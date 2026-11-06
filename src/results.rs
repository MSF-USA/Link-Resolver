//! Result records and spreadsheet-friendly CSV export.

use std::fs::OpenOptions;
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

/// Save ordered results to a new CSV at `path` with input_url, resolved_url,
/// status and error columns. Returns contextual I/O errors and refuses to
/// overwrite existing files. Spreadsheet formulas are escaped as literal text.
pub fn save_csv(path: &Path, results: &[Resolution]) -> Result<(), String> {
    let file = OpenOptions::new()
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
        })?;
    write_csv(file, results).map_err(|error| {
        format!(
            "Could not finish writing {}: {error}. The file may be incomplete.",
            path.display()
        )
    })
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
