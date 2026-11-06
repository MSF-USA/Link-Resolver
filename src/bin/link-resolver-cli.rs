//! Interactive and command-line entry points for the URL resolver.

use link_resolver::{APP_NAME, input, resolver, results};

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use input::Loaded;
use link_resolver::table::Table;
use resolver::Resolver;
use results::{Resolution, save_csv};

const HELP: &str = "Resolve links to their final destinations, including Vertex AI grounding links.

Open without arguments for a friendly menu.

Command-line usage:
  link-resolver-cli URL [URL ...]
  link-resolver-cli --file urls.txt [--output results.csv]
  link-resolver-cli --file links.csv [--column Website] [--no-header] [--output results.csv]
  link-resolver-cli --output results.csv URL [URL ...]

Text files contain one link per line. Blank lines and # comments are ignored.
CSV and TSV files are read as tables: the column holding links is used
automatically when there is exactly one; otherwise name it with --column
(a header name or a 1-based number). The first row counts as a header when
it holds no link; --header and --no-header override that. Rows without a
link are skipped and listed. Links may be written as https://example.com, www.example.com or
example.com/page. URLs and --file can be combined. Existing output files
are never overwritten. Only use trusted links: the app contacts each
redirected destination.
";

#[derive(Debug, Default)]
struct Options {
    urls: Vec<String>,
    files: Vec<PathBuf>,
    /// Header name or 1-based number of the link column in CSV/TSV files.
    column: Option<String>,
    /// Force the first row of CSV/TSV files to be a header (`true`) or data (`false`).
    header: Option<bool>,
    output: Option<PathBuf>,
    help: bool,
}

fn parse_options(arguments: Vec<String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => options.help = true,
            "--file" | "-f" => options.files.push(PathBuf::from(
                arguments.next().ok_or("--file needs a file path.")?,
            )),
            "--header" => options.header = Some(true),
            "--no-header" => options.header = Some(false),
            "--column" | "-c" => {
                if options.column.is_some() {
                    return Err("Use --column only once.".into());
                }
                options.column = Some(
                    arguments
                        .next()
                        .ok_or("--column needs a column name or number.")?,
                );
            }
            "--output" | "-o" => {
                if options.output.is_some() {
                    return Err("Use --output only once.".into());
                }
                options.output = Some(PathBuf::from(
                    arguments.next().ok_or("--output needs a file path.")?,
                ));
            }
            "--" => {
                options.urls.extend(arguments);
                break;
            }
            _ if argument.starts_with('-') => {
                return Err(format!("Unknown option: {argument}. Use --help for usage."));
            }
            _ => options.urls.push(argument),
        }
    }
    Ok(options)
}

fn prompt(message: &str) -> io::Result<Option<String>> {
    print!("{message}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

/// Which table column to take links from when the file does not settle it.
enum ColumnPicker<'a> {
    /// A `--column` argument, or nothing: ambiguity is an error.
    Named(Option<&'a str>),
    /// Ask on the terminal; `None` from the prompt cancels the load.
    Ask,
}

/// Load a text or table file. Tables use their single link column, the `--column` choice,
/// or a prompt, and print the rows they skipped so nobody misses them. `header` overrides
/// the header detection when given.
fn load_links(
    path: &Path,
    picker: ColumnPicker<'_>,
    header: Option<bool>,
) -> Result<Option<Vec<String>>, String> {
    let mut table = match input::load_file(path)? {
        Loaded::Links(links) => return Ok(Some(links)),
        Loaded::Table(table) => table,
    };
    if let Some(has_header) = header {
        table.set_header(has_header);
    }
    let Some(column) = choose_column(&table, picker)? else {
        return Ok(None);
    };
    let extraction = table.extract(column);
    let summary = extraction.describe(&table.columns[column].name);
    if extraction.skipped_rows.is_empty() {
        println!("{summary}");
    } else {
        eprintln!("Warning: {summary}");
    }
    if extraction.links.is_empty() {
        return Err(format!(
            "{} has no links in column \"{}\".",
            table.name, table.columns[column].name
        ));
    }
    Ok(Some(extraction.links))
}

fn choose_column(table: &Table, picker: ColumnPicker<'_>) -> Result<Option<usize>, String> {
    let names = || {
        table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let link_columns = table.link_columns();
    match picker {
        ColumnPicker::Named(Some(wanted)) => table.find_column(wanted).map(Some).ok_or_else(|| {
            format!(
                "{} has no column \"{wanted}\". Columns: {}.",
                table.name,
                names()
            )
        }),
        ColumnPicker::Named(None) => match link_columns[..] {
            [only] => Ok(Some(only)),
            [] => Err(format!(
                "No column in {} holds only links. Columns: {}. Add --column NAME to use one anyway; rows without a link are skipped.",
                table.name,
                names()
            )),
            _ => Err(format!(
                "{} has several columns with links: {}. Add --column NAME to choose one.",
                table.name,
                link_columns
                    .iter()
                    .map(|&index| table.columns[index].name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        },
        ColumnPicker::Ask => {
            if let [only] = link_columns[..] {
                return Ok(Some(only));
            }
            if link_columns.is_empty() {
                println!(
                    "No column in {} holds only links. Rows without a link will be skipped.",
                    table.name
                );
            } else {
                println!("{} has several columns with links.", table.name);
            }
            for (number, column) in table.columns.iter().enumerate() {
                let sample = column
                    .sample
                    .as_deref()
                    .map_or(String::new(), |sample| format!(" (e.g. {sample})"));
                println!(
                    "{}. {} - {}{sample}",
                    number + 1,
                    column.name,
                    column.summary()
                );
            }
            loop {
                let Some(answer) = prompt("Column number (Enter to cancel): ")
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(None);
                };
                if answer.is_empty() {
                    return Ok(None);
                }
                match table.find_column(&answer) {
                    Some(column) => return Ok(Some(column)),
                    None => println!("Please enter a number from 1 to {}.", table.columns.len()),
                }
            }
        }
    }
}

fn resolve_batch(resolver: &Resolver, urls: &[String]) -> Vec<Resolution> {
    urls.iter()
        .enumerate()
        .map(|(index, url)| {
            println!("\n[{}/{}] {}", index + 1, urls.len(), url);
            let result = resolver.resolve(url);
            match &result.outcome {
                Ok(destination) => println!("  -> {destination}"),
                Err(error) => println!("  Failed: {error}"),
            }
            result
        })
        .collect()
}

fn show_summary(results: &[Resolution]) {
    let succeeded = results
        .iter()
        .filter(|result| result.outcome.is_ok())
        .count();
    println!(
        "\nFinished: {succeeded} resolved, {} failed.",
        results.len() - succeeded
    );
}

fn interactive(resolver: &Resolver) -> Result<(), String> {
    println!("{APP_NAME}\nFind the final destination of a redirect link.");
    println!("Only use trusted links. Each URL can take up to 30 seconds.");
    let mut results = Vec::new();
    loop {
        println!(
            "\n1. Paste URLs\n2. Load a text, CSV or TSV file\n3. Save last results as CSV\n4. Exit"
        );
        let Some(choice) = prompt("Choose 1-4: ").map_err(|error| error.to_string())? else {
            return Ok(());
        };
        match choice.as_str() {
            "1" => {
                println!("Paste one URL per line. Press Enter on an empty line to start.");
                let mut urls = Vec::new();
                while let Some(line) = prompt("URL: ").map_err(|error| error.to_string())? {
                    if line.is_empty() {
                        break;
                    }
                    urls.extend(input::parse_urls(&line));
                }
                if urls.is_empty() {
                    println!("No URLs entered. Previous results have been kept.");
                    continue;
                }
                results = resolve_batch(resolver, &urls);
                show_summary(&results);
                println!("Choose 3 to save these results.");
            }
            "2" => {
                println!(
                    "Use a text file with one link per line, or a CSV/TSV file with a link column."
                );
                let Some(path) = prompt("File path (or drag the file here): ")
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(());
                };
                match load_links(&input::unquote_path(&path), ColumnPicker::Ask, None) {
                    Ok(Some(urls)) => {
                        results = resolve_batch(resolver, &urls);
                        show_summary(&results);
                        println!("Choose 3 to save these results.");
                    }
                    Ok(None) => println!("Load cancelled. Previous results have been kept."),
                    Err(error) => println!("Could not load file: {error}"),
                }
            }
            "3" => {
                if results.is_empty() {
                    println!("Resolve some URLs first, then save the results here.");
                    continue;
                }
                let Some(path) = prompt("Save as (for example results.csv): ")
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(());
                };
                let path = input::unquote_path(&path);
                if path.as_os_str().is_empty() {
                    println!("Save cancelled.");
                    continue;
                }
                match save_csv(&path, &results) {
                    Ok(()) => println!("Saved {} results to {}.", results.len(), path.display()),
                    Err(error) => println!("Could not save: {error}"),
                }
            }
            "4" => return Ok(()),
            _ => println!("Please enter 1, 2, 3, or 4."),
        }
    }
}

fn run(arguments: Vec<String>) -> Result<bool, String> {
    if arguments.is_empty() {
        interactive(&Resolver::new()?)?;
        return Ok(true);
    }
    let options = parse_options(arguments)?;
    if options.help {
        print!("{HELP}");
        return Ok(true);
    }
    let mut urls = options.urls;
    for file in options.files {
        urls.extend(
            load_links(
                &file,
                ColumnPicker::Named(options.column.as_deref()),
                options.header,
            )?
            .unwrap_or_default(),
        );
    }
    if urls.is_empty() {
        return Err("No URLs supplied. Open without arguments for the menu, or use --help.".into());
    }
    let results = resolve_batch(&Resolver::new()?, &urls);
    show_summary(&results);
    if let Some(output) = options.output {
        save_csv(&output, &results)?;
        println!("Saved results to {}.", output.display());
    }
    Ok(results.iter().all(|result| result.outcome.is_ok()))
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let interactive_mode = arguments.is_empty();
    match run(arguments) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Error: {error}");
            if interactive_mode {
                let _ = prompt("Press Enter to close.");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_accept_files_urls_and_output() {
        let options = parse_options(
            [
                "https://example.com",
                "--file",
                "links.txt",
                "--output",
                "results.csv",
            ]
            .map(String::from)
            .to_vec(),
        )
        .unwrap();
        assert_eq!(options.urls, ["https://example.com"]);
        assert_eq!(options.files, [PathBuf::from("links.txt")]);
        assert_eq!(options.output, Some(PathBuf::from("results.csv")));
    }

    #[test]
    fn options_reject_missing_values_unknown_flags_and_duplicate_output() {
        for arguments in [
            vec!["--file"],
            vec!["--output"],
            vec!["--column"],
            vec!["--unknown"],
            vec!["--output", "a.csv", "--output", "b.csv"],
            vec!["--column", "a", "--column", "b"],
        ] {
            assert!(parse_options(arguments.into_iter().map(String::from).collect()).is_err());
        }
        let options = parse_options(vec!["--column".into(), "Website".into()]).unwrap();
        assert_eq!(options.column.as_deref(), Some("Website"));
        assert_eq!(options.header, None);
        assert_eq!(
            parse_options(vec!["--no-header".into()]).unwrap().header,
            Some(false)
        );
        assert_eq!(
            parse_options(vec!["--header".into()]).unwrap().header,
            Some(true)
        );
    }

    #[test]
    fn column_choice_uses_the_only_link_column_or_the_named_one_and_explains_otherwise() {
        let one = Table::parse("a.csv", "Name,Website\nA,https://x.com\n", b',').unwrap();
        assert_eq!(
            choose_column(&one, ColumnPicker::Named(None)).unwrap(),
            Some(1)
        );
        assert_eq!(choose_column(&one, ColumnPicker::Ask).unwrap(), Some(1));
        assert_eq!(
            choose_column(&one, ColumnPicker::Named(Some("name"))).unwrap(),
            Some(0)
        );
        assert_eq!(
            choose_column(&one, ColumnPicker::Named(Some("2"))).unwrap(),
            Some(1)
        );
        let missing = choose_column(&one, ColumnPicker::Named(Some("Nope"))).unwrap_err();
        assert!(missing.contains("no column \"Nope\"") && missing.contains("Name, Website"));

        let several = Table::parse(
            "b.csv",
            "Source,Target\nhttps://x.com,https://y.com\n",
            b',',
        )
        .unwrap();
        let error = choose_column(&several, ColumnPicker::Named(None)).unwrap_err();
        assert!(
            error.contains("several columns")
                && error.contains("Source, Target")
                && error.contains("--column")
        );

        let none = Table::parse("c.csv", "Note\nhello\n", b',').unwrap();
        let error = choose_column(&none, ColumnPicker::Named(None)).unwrap_err();
        assert!(error.contains("holds only links") && error.contains("--column"));
    }
}
