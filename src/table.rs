//! Spreadsheet-style input: CSV and TSV files with the links in one column.
//!
//! A [`Table`] scores every column by how many of its filled cells are links (see
//! [`crate::links`]). A column where every filled cell is a link is a *link column*. With
//! exactly one link column the caller can use it straight away; otherwise the user chooses a
//! column, and [`Table::extract`] reports the rows it had to skip.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::links::is_link;

/// File extensions treated as tables rather than one-link-per-line text.
const TABLE_EXTENSIONS: [&str; 2] = ["csv", "tsv"];
/// Delimiters tried when sniffing a `.csv` file; the most frequent one on the first line wins.
const CANDIDATE_DELIMITERS: &[u8] = b",;\t";
/// Longest sample value shown for a column before it is cut with an ellipsis.
const SAMPLE_LENGTH: usize = 60;
/// Rows a preview grid shows before saying how many more there are.
pub const PREVIEW_ROWS: usize = 14;
/// How many skipped row numbers a summary lists before saying "and N more".
const LISTED_ROWS: usize = 5;

/// Whether `path` should be read as a table, judged by its extension.
pub fn is_table_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            TABLE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// What is known about one column, for choosing the one that holds links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    /// Header text, or `Column N` when the file has no header row.
    pub name: String,
    /// Cells that contain something other than whitespace.
    pub filled_cells: usize,
    /// Filled cells that are links.
    pub link_cells: usize,
    /// First filled cell, shortened for display.
    pub sample: Option<String>,
}

impl Column {
    /// A column whose every filled cell is a link.
    pub fn is_link_column(&self) -> bool {
        self.filled_cells > 0 && self.link_cells == self.filled_cells
    }

    /// Short description such as `links in 12 of 15 cells`.
    pub fn summary(&self) -> String {
        match (self.filled_cells, self.link_cells) {
            (0, _) => "empty".to_owned(),
            (filled, links) if filled == links => count(filled, "link"),
            (filled, 0) => format!("no links in {}", count(filled, "cell")),
            (filled, links) => format!("links in {links} of {filled} cells"),
        }
    }
}

/// A parsed CSV or TSV file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// File name without directories, for messages.
    pub name: String,
    pub columns: Vec<Column>,
    /// Every non-empty row of the file, padded so each has one cell per column. The first
    /// is the header when `has_header` is set.
    all_rows: Vec<Vec<String>>,
    has_header: bool,
}

/// The links taken from one column, with the rows that had to be skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extraction {
    /// Links in file order, exactly as they appear in the cells.
    pub links: Vec<String>,
    /// Spreadsheet row numbers (1-based, counting the header) of filled cells that were not links.
    pub skipped_rows: Vec<usize>,
    /// Rows whose cell was empty.
    pub blank_rows: usize,
}

impl Table {
    /// Read and parse a CSV or TSV file. The delimiter is sniffed for `.csv` and forced to a
    /// tab for `.tsv`. Fails with a readable message for unreadable, non-UTF-8 or empty files.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let is_tsv = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tsv"));
        let delimiter = if is_tsv {
            b'\t'
        } else {
            sniff_delimiter(&text)
        };
        Self::parse(&name, &text, delimiter)
    }

    /// Parse table text with a known delimiter. The first row is taken as a header when none
    /// of its cells is a link; otherwise columns are named `Column 1`, `Column 2`, …
    pub fn parse(name: &str, text: &str, delimiter: u8) -> Result<Self, String> {
        let text = text.trim_start_matches('\u{feff}');
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(text.as_bytes());
        let mut rows: Vec<Vec<String>> = Vec::new();
        for record in reader.records() {
            let record = record.map_err(|error| format!("{name} is not a valid table: {error}"))?;
            let row: Vec<String> = record.iter().map(str::to_owned).collect();
            if row.iter().any(|cell| !cell.is_empty()) {
                rows.push(row);
            }
        }
        if rows.is_empty() {
            return Err(format!("{name} is empty."));
        }
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        for row in &mut rows {
            row.resize(width, String::new());
        }
        // A first row with no link in it reads as a header; the user can override this.
        let has_header = rows.len() > 1 && !rows[0].iter().any(|cell| is_link(cell));
        let mut table = Self {
            name: name.to_owned(),
            columns: Vec::new(),
            all_rows: rows,
            has_header,
        };
        table.rebuild_columns();
        Ok(table)
    }

    /// Whether the first row is treated as column names rather than data.
    pub fn has_header(&self) -> bool {
        self.has_header
    }

    /// Treat the first row as a header (`true`) or as data (`false`) and rescore the columns.
    /// A one-row table always keeps its row as data.
    pub fn set_header(&mut self, has_header: bool) {
        self.has_header = has_header && self.all_rows.len() > 1;
        self.rebuild_columns();
    }

    /// The header row, when there is one.
    pub fn header(&self) -> Option<&[String]> {
        self.has_header.then(|| self.all_rows[0].as_slice())
    }

    /// Data rows in file order, each with one cell per column.
    pub fn rows(&self) -> &[Vec<String>] {
        &self.all_rows[usize::from(self.has_header)..]
    }

    /// Number of data rows.
    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    fn rebuild_columns(&mut self) {
        let width = self.all_rows.first().map_or(0, Vec::len);
        self.columns = (0..width)
            .map(|index| {
                let name = self
                    .header()
                    .map(|header| header[index].trim())
                    .filter(|name| !name.is_empty())
                    .map_or_else(|| format!("Column {}", index + 1), str::to_owned);
                let filled: Vec<&String> = self
                    .rows()
                    .iter()
                    .map(|row| &row[index])
                    .filter(|cell| !cell.is_empty())
                    .collect();
                Column {
                    name,
                    filled_cells: filled.len(),
                    link_cells: filled.iter().filter(|cell| is_link(cell)).count(),
                    sample: filled.first().map(|cell| abbreviate(cell, SAMPLE_LENGTH)),
                }
            })
            .collect();
    }

    /// Indexes of the columns whose every filled cell is a link.
    pub fn link_columns(&self) -> Vec<usize> {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.is_link_column())
            .map(|(index, _)| index)
            .collect()
    }

    /// Index of a column by header name (case-insensitive) or 1-based position.
    pub fn find_column(&self, name_or_number: &str) -> Option<usize> {
        let wanted = name_or_number.trim();
        self.columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(wanted))
            .or_else(|| {
                wanted
                    .parse::<usize>()
                    .ok()
                    .filter(|number| (1..=self.columns.len()).contains(number))
                    .map(|number| number - 1)
            })
    }

    /// Take the links from `column`, skipping blank cells and noting cells that are not links.
    ///
    /// # Panics
    /// If `column` is out of range; callers get indexes from this table.
    pub fn extract(&self, column: usize) -> Extraction {
        assert!(column < self.columns.len(), "column {column} out of range");
        let first_data_row = if self.has_header { 2 } else { 1 };
        let mut extraction = Extraction {
            links: Vec::new(),
            skipped_rows: Vec::new(),
            blank_rows: 0,
        };
        for (offset, row) in self.rows().iter().enumerate() {
            let cell = &row[column];
            if cell.is_empty() {
                extraction.blank_rows += 1;
            } else if is_link(cell) {
                extraction.links.push(cell.clone());
            } else {
                extraction.skipped_rows.push(first_data_row + offset);
            }
        }
        extraction
    }
}

impl Extraction {
    /// One or two sentences for the status line, naming skipped rows so they can be checked.
    ///
    /// ```
    /// # use link_resolver::table::Extraction;
    /// let extraction = Extraction { links: vec!["a.com".into()], skipped_rows: vec![3, 7], blank_rows: 0 };
    /// assert_eq!(
    ///     extraction.describe("Website"),
    ///     "Loaded 1 link from column \"Website\". Skipped 2 rows that do not contain a link: rows 3 and 7."
    /// );
    /// ```
    pub fn describe(&self, column_name: &str) -> String {
        let mut text = format!(
            "Loaded {} from column \"{column_name}\".",
            count(self.links.len(), "link")
        );
        if !self.skipped_rows.is_empty() {
            let _ = write!(
                text,
                " Skipped {} that do not contain a link: {}.",
                count(self.skipped_rows.len(), "row"),
                list_rows(&self.skipped_rows)
            );
        }
        if self.blank_rows > 0 {
            let _ = write!(
                text,
                " {} blank.",
                if self.blank_rows == 1 {
                    "1 row was".to_owned()
                } else {
                    format!("{} rows were", self.blank_rows)
                }
            );
        }
        text
    }
}

/// The delimiter that occurs most on the first non-empty line. Ties go to the earlier entry
/// in [`CANDIDATE_DELIMITERS`], and a line with none of them means comma.
fn sniff_delimiter(text: &str) -> u8 {
    let first_line = text
        .trim_start_matches('\u{feff}')
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let occurrences = |delimiter: u8| first_line.bytes().filter(|byte| *byte == delimiter).count();
    CANDIDATE_DELIMITERS
        .iter()
        .copied()
        .filter(|delimiter| occurrences(*delimiter) > 0)
        .fold(None, |best: Option<u8>, delimiter| match best {
            Some(current) if occurrences(current) >= occurrences(delimiter) => Some(current),
            _ => Some(delimiter),
        })
        .unwrap_or(b',')
}

/// `text` cut to at most `max_chars` characters, ending in an ellipsis when it was longer.
pub fn abbreviate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        let mut short: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        short.push('…');
        short
    }
}

fn count(number: usize, noun: &str) -> String {
    if number == 1 {
        format!("1 {noun}")
    } else {
        format!("{number} {noun}s")
    }
}

/// `row 3`, `rows 3 and 7`, `rows 1, 2, 3, 4, 5 and 12 more`.
fn list_rows(rows: &[usize]) -> String {
    let shown: Vec<String> = rows
        .iter()
        .take(LISTED_ROWS)
        .map(usize::to_string)
        .collect();
    match rows.len() {
        1 => format!("row {}", shown[0]),
        n if n <= LISTED_ROWS => format!("rows {} and {}", shown[..n - 1].join(", "), shown[n - 1]),
        n => format!("rows {} and {} more", shown.join(", "), n - LISTED_ROWS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        Table::parse("links.csv", text, b',').unwrap()
    }

    #[test]
    fn one_link_column_is_found_with_header_detection_and_quoting() {
        let table = table(
            "\u{feff}Name,Website,Notes\r\n\"Smith, Jo\",https://example.com/a,hi\r\nLee,www.example.org,\r\n,,\r\n",
        );
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.link_columns(), [1]);
        assert_eq!(table.columns[1].name, "Website");
        assert_eq!(table.columns[0].summary(), "no links in 2 cells");
        assert_eq!(
            Column {
                name: "x".into(),
                filled_cells: 3,
                link_cells: 1,
                sample: None
            }
            .summary(),
            "links in 1 of 3 cells"
        );
        assert_eq!(table.columns[1].summary(), "2 links");
        assert_eq!(table.columns[2].summary(), "no links in 1 cell");
        let extraction = table.extract(1);
        assert_eq!(
            extraction.links,
            ["https://example.com/a", "www.example.org"]
        );
        assert!(extraction.skipped_rows.is_empty());
        assert_eq!(extraction.blank_rows, 0);
        assert_eq!(
            extraction.describe("Website"),
            "Loaded 2 links from column \"Website\"."
        );
    }

    #[test]
    fn headerless_files_name_columns_by_position() {
        let table = table("https://example.com/a,x\nhttps://example.com/b,y\n");
        assert_eq!(table.columns[0].name, "Column 1");
        assert_eq!(table.link_columns(), [0]);
        assert_eq!(table.extract(0).links.len(), 2);
        assert_eq!(table.find_column("2"), Some(1));
        assert_eq!(table.find_column("column 1"), Some(0));
        assert_eq!(table.find_column("3"), None);
    }

    #[test]
    fn mixed_columns_report_skipped_and_blank_rows_with_spreadsheet_numbers() {
        let table = table("Link\nexample.com\nnot a link\n\nanother bad one\nwww.ok.org\n");
        assert!(table.link_columns().is_empty());
        let extraction = table.extract(0);
        assert_eq!(extraction.links, ["example.com", "www.ok.org"]);
        // Row 1 is the header; the empty line is dropped entirely, so numbering follows
        // the non-empty rows of the file as a spreadsheet would show them after import.
        assert_eq!(extraction.skipped_rows, [3, 4]);
        assert_eq!(
            extraction.describe("Link"),
            "Loaded 2 links from column \"Link\". Skipped 2 rows that do not contain a link: rows 3 and 4."
        );
    }

    #[test]
    fn several_link_columns_are_all_reported() {
        let table = table("Source,Target\nhttps://a.com,https://b.com\nhttps://c.com,\n");
        assert_eq!(table.link_columns(), [0, 1]);
        let target = table.extract(1);
        assert_eq!(target.links, ["https://b.com"]);
        assert_eq!(target.blank_rows, 1);
        assert_eq!(
            target.describe("Target"),
            "Loaded 1 link from column \"Target\". 1 row was blank."
        );
    }

    #[test]
    fn delimiters_are_sniffed_and_tsv_is_forced() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3"), b';');
        assert_eq!(sniff_delimiter("a\tb\n"), b'\t');
        assert_eq!(sniff_delimiter("a,b;c\n"), b',');
        assert_eq!(sniff_delimiter("single\n"), b',');
        let dir = std::env::temp_dir().join(format!("link-resolver-table-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("links.TSV");
        fs::write(
            &path,
            "Name\tWebsite\nA\thttps://example.com/a, with comma\n",
        )
        .unwrap();
        let table = Table::load(&path).unwrap();
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.link_columns(), [1]);
        assert!(is_table_file(&path));
        assert!(is_table_file(Path::new("x.csv")));
        assert!(!is_table_file(Path::new("x.txt")));
        assert!(!is_table_file(Path::new("csv")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_files_are_errors_and_a_single_row_is_always_data() {
        assert!(
            Table::parse("a.csv", "", b',')
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            Table::parse("a.csv", "\n\n", b',')
                .unwrap_err()
                .contains("empty")
        );
        let lone = table("Name,Website\n");
        assert!(!lone.has_header());
        assert_eq!(lone.row_count(), 1);
        assert_eq!(lone.columns[0].name, "Column 1");
    }

    #[test]
    fn the_header_choice_can_be_overridden_either_way() {
        let mut titled = table("Name,Website\nA,https://example.com/a\nB,https://example.com/b\n");
        assert!(titled.has_header());
        assert_eq!(titled.header().unwrap()[1], "Website");
        titled.set_header(false);
        assert!(!titled.has_header());
        assert_eq!(titled.row_count(), 3);
        assert_eq!(titled.columns[1].name, "Column 2");
        // "Website" is now a data cell that is not a link, so the column is no longer pure.
        assert!(titled.link_columns().is_empty());
        assert_eq!(titled.extract(1).skipped_rows, [1]);
        titled.set_header(true);
        assert_eq!(titled.link_columns(), [1]);
        assert_eq!(titled.extract(1).links.len(), 2);

        let mut headerless = table("https://a.com,x\nhttps://b.com,y\n");
        assert!(!headerless.has_header());
        headerless.set_header(true);
        assert_eq!(headerless.columns[0].name, "https://a.com");
        assert_eq!(headerless.row_count(), 1);
    }

    #[test]
    fn row_lists_read_naturally() {
        assert_eq!(list_rows(&[3]), "row 3");
        assert_eq!(list_rows(&[3, 7]), "rows 3 and 7");
        assert_eq!(list_rows(&[1, 2, 3]), "rows 1, 2 and 3");
        assert_eq!(
            list_rows(&[1, 2, 3, 4, 5, 6, 7]),
            "rows 1, 2, 3, 4, 5 and 2 more"
        );
        assert_eq!(
            abbreviate(&"x".repeat(70), SAMPLE_LENGTH).chars().count(),
            SAMPLE_LENGTH
        );
        assert_eq!(abbreviate("short", 10), "short");
    }
}
