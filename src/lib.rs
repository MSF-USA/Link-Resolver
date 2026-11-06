//! Shared input parsing, redirect resolution and CSV export for the GUI and CLI.

/// The product name shown in window titles, headings, help text and the HTTP user agent.
/// Binary and crate names in `Cargo.toml` and `packaging/` are the kebab-case form of this.
pub const APP_NAME: &str = "Link Resolver";

pub mod input;
pub mod links;
pub mod resolver;
pub mod results;
pub mod table;
