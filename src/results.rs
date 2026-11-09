//! Result records and their export: the whole table as spreadsheet-friendly CSV, or only the
//! resolved links as plain text.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Preserve an original input and either its final URL or a human-readable error.
#[derive(Clone, Debug)]
pub struct Resolution {
    /// The original, unmodified input URL or invalid input text.
    pub input: String,
    /// The final URL on success, or a contextual error on failure.
    pub outcome: Result<String, String>,
}

/// What an export contains and how it is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportFormat {
    /// Every result as CSV: input_url, resolved_url, status and error columns.
    #[default]
    Table,
    /// Only successful destinations, one per line, in a plain text file that can be loaded back.
    ResolvedLinks,
}

impl ExportFormat {
    /// Every format, in the order a user is offered them.
    pub const ALL: [Self; 2] = [Self::Table, Self::ResolvedLinks];

    /// File extension without the dot, added when the user types a name without one.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Table => "csv",
            Self::ResolvedLinks => "txt",
        }
    }

    /// File name suggested in the save dialog.
    pub fn default_file_name(self) -> &'static str {
        match self {
            Self::Table => "results.csv",
            Self::ResolvedLinks => "resolved-links.txt",
        }
    }

    /// Name of the file type, for save dialog filters.
    pub fn file_type(self) -> &'static str {
        match self {
            Self::Table => "CSV spreadsheet",
            Self::ResolvedLinks => "Text file",
        }
    }
}

/// Save ordered results to a new file at `path` in `format`. Returns contextual I/O errors
/// and refuses to overwrite existing files. In the CSV table, spreadsheet formulas are
/// escaped as literal text. Failed results are left out of [`ExportFormat::ResolvedLinks`].
pub fn save_results(
    path: &Path,
    results: &[Resolution],
    format: ExportFormat,
) -> Result<(), String> {
    let file = create_new(path)?;
    let written = match format {
        ExportFormat::Table => write_csv(file, results).map_err(|error| error.to_string()),
        ExportFormat::ResolvedLinks => {
            write_resolved_links(file, results).map_err(|error| error.to_string())
        }
    };
    written.map_err(|error| {
        format!(
            "Could not finish writing {}: {error}. The file may be incomplete.",
            path.display()
        )
    })
}

/// Save ordered results to a new CSV at `path`; see [`save_results`].
pub fn save_csv(path: &Path, results: &[Resolution]) -> Result<(), String> {
    save_results(path, results, ExportFormat::Table)
}

/// Create `path`, refusing to replace an existing file.
fn create_new(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "{} already exists. Choose a different file name.",
                    path.display()
                )
            } else {
                format!("Cannot create {}: {error}", path.display())
            }
        })
}

/// One resolved destination per line. Failures have no destination, so they are skipped.
fn write_resolved_links(mut writer: impl Write, results: &[Resolution]) -> std::io::Result<()> {
    for url in results.iter().filter_map(|result| result.outcome.as_ref().ok()) {
        writeln!(writer, "{url}")?;
    }
    writer.flush()
}

fn spreadsheet_text(value: &str) -> String {
    // Invalid input can be arbitrary text, including spreadsheet formulas.
    if value.starts_with(['=', '+', '-', '@', '\t', '\r', '\n'])
        || value.trim_start().starts_with(['=', '+', '-', '@'])
    {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

fn write_csv(writer: impl Write, results: &[Resolution]) -> Result<(), csv::Error> {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(writer);
    writer.write_record(["input_url", "resolved_url", "status", "error"])?;
    for result in results {
        let (url, status, error) = match &result.outcome {
            Ok(url) => (url.as_str(), "OK", ""),
            Err(error) => ("", "FAILED", error.as_str()),
        };
        writer.write_record([
            spreadsheet_text(&result.input),
            spreadsheet_text(url),
            status.to_owned(),
            spreadsheet_text(error),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_round_trips_commas_quotes_unicode_and_errors() {
        let results = vec![
            Resolution {
                input: "https://example.com/a,b".into(),
                outcome: Ok("https://example.com/final".into()),
            },
            Resolution {
                input: "=HYPERLINK(\"untrusted\")".into(),
                outcome: Err("Bad input: café, \"quoted\"".into()),
            },
        ];
        let mut output = Vec::new();
        write_csv(&mut output, &results).unwrap();
        let mut reader = csv::Reader::from_reader(output.as_slice());
        assert_eq!(
            reader.headers().unwrap().iter().collect::<Vec<_>>(),
            ["input_url", "resolved_url", "status", "error"]
        );
        let records = reader.records().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(&records[0][0], "https://example.com/a,b");
        assert_eq!(&records[0][2], "OK");
        assert_eq!(&records[1][0], "'=HYPERLINK(\"untrusted\")");
        assert_eq!(&records[1][2], "FAILED");
        assert_eq!(&records[1][3], "Bad input: café, \"quoted\"");
    }

    fn mixed_results() -> Vec<Resolution> {
        vec![
            Resolution {
                input: "https://a.example/start".into(),
                outcome: Ok("https://a.example/final".into()),
            },
            Resolution {
                input: "broken".into(),
                outcome: Err("Enter a full link.".into()),
            },
            Resolution {
                input: "https://b.example/start".into(),
                outcome: Ok("https://b.example/final".into()),
            },
        ]
    }

    #[test]
    fn each_format_writes_its_own_content_and_refuses_to_overwrite() {
        let directory = std::env::temp_dir().join(format!(
            "link-resolver-export-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        for format in ExportFormat::ALL {
            let path = directory.join(format.default_file_name());
            let _ = std::fs::remove_file(&path);
            save_results(&path, &mixed_results(), format).unwrap();
            let written = std::fs::read_to_string(&path).unwrap();
            match format {
                ExportFormat::Table => {
                    assert!(written.starts_with("input_url,resolved_url,status,error\r\n"));
                    assert!(written.contains("broken,,FAILED,Enter a full link."));
                }
                ExportFormat::ResolvedLinks => {
                    assert_eq!(written, "https://a.example/final\nhttps://b.example/final\n");
                }
            }
            assert_eq!(path.extension().unwrap(), format.extension());
            let error = save_results(&path, &mixed_results(), format).unwrap_err();
            assert!(error.contains("already exists"), "{error}");
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn resolved_links_file_loads_back_as_the_same_links() {
        let mut output = Vec::new();
        write_resolved_links(&mut output, &mixed_results()).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(
            crate::input::parse_urls(&text),
            ["https://a.example/final", "https://b.example/final"]
        );
    }

    #[test]
    fn spreadsheet_formulas_are_neutralized() {
        for text in ["=1+1", "+1", "-1", "@SUM(A1)", "\tformula", "  =1"] {
            assert!(spreadsheet_text(text).starts_with('\''));
        }
        assert_eq!(
            spreadsheet_text("https://example.com"),
            "https://example.com"
        );
    }
}
