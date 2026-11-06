//! End-to-end executable, file-input, interactive-menu and export checks.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

/// Own an isolated test directory and remove only its files on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let root = if cfg!(target_os = "linux") && PathBuf::from("/tmp/opencode").is_dir() {
            PathBuf::from("/tmp/opencode")
        } else {
            std::env::temp_dir()
        };
        let path = root.join(format!(
            "link-resolver-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn application() -> Command {
    Command::new(env!("CARGO_BIN_EXE_link-resolver-cli"))
}

fn redirect_server() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        for response in [
            "HTTP/1.1 302 Found\r\nLocation: /destination\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ] {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "Application never connected");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET "));
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (base, thread)
}

#[test]
fn file_batch_continues_after_invalid_url_and_exports_all_results() {
    let scratch = Scratch::new();
    let input = scratch.0.join("my links.txt");
    let output = scratch.0.join("results.csv");
    let (base, server) = redirect_server();
    fs::write(
        &input,
        format!("\u{feff}# Links\r\nnot-a-url\r\n{base}/start\r\n"),
    )
    .unwrap();
    let result = application()
        .arg("--file")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("1 resolved, 1 failed")
    );
    let records = csv::Reader::from_path(output)
        .unwrap()
        .records()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(&records[0][2], "FAILED");
    assert_eq!(&records[1][1], format!("{base}/destination"));
    assert_eq!(&records[1][2], "OK");
}

#[test]
fn csv_files_use_their_link_column_and_ambiguity_needs_the_column_option() {
    let scratch = Scratch::new();
    let (base, server) = redirect_server();
    let single = scratch.0.join("links.csv");
    fs::write(
        &single,
        format!("Name,Website\r\n\"Smith, Jo\",{base}/start\r\nLee,\r\n"),
    )
    .unwrap();
    let result = application().arg("--file").arg(&single).output().unwrap();
    server.join().unwrap();
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(result.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("Loaded 1 link from column \"Website\". 1 row was blank."));
    assert!(stdout.contains(&format!("{base}/destination")));

    let several = scratch.0.join("two.csv");
    fs::write(
        &several,
        "Source,Target\nhttps://a.invalid,https://b.invalid\n",
    )
    .unwrap();
    let result = application().arg("--file").arg(&several).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(
        stderr.contains("several columns") && stderr.contains("--column"),
        "{stderr}"
    );

    let (base, server) = redirect_server();
    let mixed = scratch.0.join("mixed.csv");
    fs::write(
        &mixed,
        format!("Note;Site\nhello;{base}/start\nworld;not a link\n"),
    )
    .unwrap();
    let result = application()
        .arg("--file")
        .arg(&mixed)
        .arg("--column")
        .arg("site")
        .output()
        .unwrap();
    server.join().unwrap();
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert_eq!(result.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("Warning: Loaded 1 link from column \"Site\". Skipped 1 row that do not contain a link: row 3."), "{stderr}");

    // --no-header keeps a first row that only looks like a header as data; it is then
    // skipped as a non-link and the column is named by position.
    let (base, server) = redirect_server();
    let no_header = scratch.0.join("plain.csv");
    fs::write(&no_header, format!("label,target\nfirst,{base}/start\n")).unwrap();
    let result = application()
        .arg("--file")
        .arg(&no_header)
        .arg("--no-header")
        .arg("--column")
        .arg("2")
        .output()
        .unwrap();
    server.join().unwrap();
    let stdout = String::from_utf8(result.stdout).unwrap();
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert_eq!(result.status.code(), Some(0), "{stdout}{stderr}");
    assert!(
        stderr.contains(
            "Loaded 1 link from column \"Column 2\". Skipped 1 row that do not contain a link: row 1."
        ),
        "{stderr}"
    );
}

#[test]
fn interactive_menu_loads_quoted_path_and_saves_results() {
    let scratch = Scratch::new();
    let input = scratch.0.join("links with spaces.txt");
    let output = scratch.0.join("results.csv");
    let (base, server) = redirect_server();
    fs::write(&input, format!("{base}/start\n")).unwrap();
    let mut child = application()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let menu_input = format!("2\n\"{}\"\n3\n{}\n4\n", input.display(), output.display());
    child
        .stdin
        .take()
        .unwrap()
        .write_all(menu_input.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    server.join().unwrap();
    assert!(result.status.success(), "{:?}", result);
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("1 resolved, 0 failed"));
    assert!(stdout.contains("Saved 1 results"));
    assert!(
        fs::read_to_string(output)
            .unwrap()
            .contains(&format!("{base}/destination"))
    );
}

#[test]
fn interactive_paste_handles_errors_and_keeps_menu_open() {
    let mut child = application()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"3\n1\ninvalid\n\n4\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("Resolve some URLs first"));
    assert!(stdout.contains("0 resolved, 1 failed"));
    assert!(stdout.matches("Choose 1-4:").count() >= 3);
}

#[test]
fn existing_export_is_not_overwritten() {
    let scratch = Scratch::new();
    let output = scratch.0.join("results.csv");
    fs::write(&output, "keep this content").unwrap();
    let result = application()
        .arg("--output")
        .arg(&output)
        .arg("invalid")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("already exists")
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "keep this content");
}

#[test]
fn help_missing_file_and_empty_file_have_predictable_exit_codes() {
    let version = application().arg("--version").output().unwrap();
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        format!("Link Resolver {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        application()
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
    let scratch = Scratch::new();
    let path = scratch.0.join("links.txt");
    assert!(
        !application()
            .arg("--file")
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(&path, "# No links\n").unwrap();
    let result = application().arg("--file").arg(&path).output().unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("contains no URLs")
    );
}
