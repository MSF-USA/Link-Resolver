//! File input and interactive path handling.
//!
//! [`load_file`] is the one entry point for "load a file": it reads a CSV or TSV file as a
//! [`Table`] and anything else as one link per line, judged by the file extension.

use std::fs;
use std::path::{Path, PathBuf};

use crate::table::{Table, is_table_file};

/// What a loaded file turned out to contain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// One link per line, in file order.
    Links(Vec<String>),
    /// A spreadsheet-style file; the caller picks the column (see [`Table::link_columns`]).
    Table(Table),
}

/// Load a file of links, choosing the format from the extension: `.csv` and `.tsv` are
/// tables, everything else is one link per line.
pub fn load_file(path: &Path) -> Result<Loaded, String> {
    if is_table_file(path) {
        Table::load(path).map(Loaded::Table)
    } else {
        load_urls(path).map(Loaded::Links)
    }
}

/// Parse UTF-8 text into ordered URLs, ignoring a BOM, blank lines and # comments.
/// Invalid URLs are retained so the resolver can report each failed input.
pub fn parse_urls(text: &str) -> Vec<String> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Read one URL per line from `path`; return a contextual error for unreadable,
/// non-UTF-8 or empty files. Does not discard duplicate URLs.
pub fn load_urls(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let urls = parse_urls(&text);
    if urls.is_empty() {
        return Err(format!("{} contains no URLs.", path.display()));
    }
    Ok(urls)
}

/// Remove matching shell-added outer quotes from a typed or dragged file path.
/// The returned path is not expanded or interpreted as a shell command.
pub fn unquote_path(text: &str) -> PathBuf {
    let text = text.trim();
    let unquoted = text
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(text);
    PathBuf::from(unquoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_handles_bom_crlf_comments_duplicates_and_bad_urls() {
        assert_eq!(
            parse_urls(
                "\u{feff}# links\r\n\r\n https://example.com/a \r\ninvalid\nhttps://example.com/a\n"
            ),
            ["https://example.com/a", "invalid", "https://example.com/a"]
        );
    }

    #[test]
    fn load_file_picks_the_format_from_the_extension() {
        let dir = std::env::temp_dir().join(format!("link-resolver-input-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let text = dir.join("links.txt");
        fs::write(
            &text,
            "https://example.com/a
",
        )
        .unwrap();
        let csv = dir.join("links.csv");
        fs::write(
            &csv,
            "Website
https://example.com/a
",
        )
        .unwrap();
        assert_eq!(
            load_file(&text).unwrap(),
            Loaded::Links(vec!["https://example.com/a".into()])
        );
        match load_file(&csv).unwrap() {
            Loaded::Table(table) => assert_eq!(table.link_columns(), [0]),
            other => panic!("expected a table, got {other:?}"),
        }
        assert!(
            load_file(&dir.join("missing.txt"))
                .unwrap_err()
                .contains("Cannot read")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_support_spaces_and_matching_quotes() {
        assert_eq!(
            unquote_path("  \"my links.txt\"  "),
            PathBuf::from("my links.txt")
        );
        assert_eq!(
            unquote_path("'my links.txt'"),
            PathBuf::from("my links.txt")
        );
        assert_eq!(unquote_path("my links.txt"), PathBuf::from("my links.txt"));
    }
}
