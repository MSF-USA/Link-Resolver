//! Lenient recognition of web links.
//!
//! People paste links in every shape: `https://example.com/page`, `www.example.com` or just
//! `example.com/page`. Everything that accepts links goes through [`parse_link`] so the rules
//! are the same in the text box, text files, spreadsheet columns and on the command line.
//! A bare host gets `https://`. Email addresses, other schemes and free text are rejected.

use reqwest::Url;

/// Interpret `text` as a web link, adding `https://` when no scheme is given.
/// Returns `None` for anything that is not an http(s) link: other schemes such as `ftp://`
/// or `mailto:`, email addresses, hosts without a dot, and text containing spaces.
///
/// ```
/// use link_resolver::links::parse_link;
/// assert_eq!(parse_link("www.example.com").unwrap().as_str(), "https://www.example.com/");
/// assert!(parse_link("someone@example.com").is_none());
/// ```
pub fn parse_link(text: &str) -> Option<Url> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return web_url(text);
    }
    if text.is_empty() || text.contains("://") || text.chars().any(char::is_whitespace) {
        return None;
    }
    let authority_end = text.find(['/', '?', '#']).unwrap_or(text.len());
    let authority = &text[..authority_end];
    if authority.contains('@') {
        return None;
    }
    let host = match authority.split_once(':') {
        Some((host, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            host
        }
        Some(_) => return None,
        None => authority,
    };
    if !looks_like_host(host) {
        return None;
    }
    web_url(&format!("https://{text}"))
}

/// Whether `text` would be accepted by [`parse_link`].
pub fn is_link(text: &str) -> bool {
    parse_link(text).is_some()
}

/// Why a piece of text was not accepted as a link, in words for the end user.
pub fn rejection_reason(text: &str) -> String {
    if text.contains("://") {
        "The link must start with https:// or http://.".to_owned()
    } else {
        "This is not a web link. Use a full address such as https://example.com/page or example.com/page.".to_owned()
    }
}

/// A dotted name whose last label is alphabetic (a domain) or four numbers (an IPv4 address).
fn looks_like_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let well_formed = labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        });
    if !well_formed {
        return false;
    }
    let ipv4 = labels.len() == 4 && labels.iter().all(|label| label.parse::<u8>().is_ok());
    let top_level = labels[labels.len() - 1];
    ipv4 || (top_level.chars().count() >= 2 && top_level.chars().all(char::is_alphabetic))
}

fn web_url(text: &str) -> Option<Url> {
    let url = Url::parse(text).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_addresses_bare_hosts_and_www_are_links() {
        for (input, expected) in [
            (
                "https://example.com/page?x=1",
                "https://example.com/page?x=1",
            ),
            ("HTTP://Example.com", "http://example.com/"),
            ("  www.example.com  ", "https://www.example.com/"),
            ("example.com", "https://example.com/"),
            (
                "sub.example.co.uk/path#frag",
                "https://sub.example.co.uk/path#frag",
            ),
            ("example.com:8080/x", "https://example.com:8080/x"),
            ("192.168.0.1/admin", "https://192.168.0.1/admin"),
            ("münchen.de", "https://xn--mnchen-3ya.de/"),
        ] {
            assert_eq!(
                parse_link(input).map(|url| url.to_string()).as_deref(),
                Some(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn email_addresses_other_schemes_and_prose_are_not_links() {
        for input in [
            "",
            "   ",
            "someone@example.com",
            "mailto:someone@example.com",
            "ftp://example.com",
            "file:///etc/hosts",
            "not a URL",
            "see example.com for details",
            "localhost",
            "localhost:3000",
            "example",
            "example.c0m",
            "-bad.com",
            "example.com.",
            "12.34",
            "3.14159",
        ] {
            assert!(parse_link(input).is_none(), "{input:?} was accepted");
        }
        assert!(
            is_link("http://localhost:3000"),
            "explicit scheme allows localhost"
        );
    }

    #[test]
    fn rejection_reasons_distinguish_wrong_schemes_from_plain_text() {
        assert!(rejection_reason("ftp://x").contains("https:// or http://"));
        assert!(rejection_reason("hello").contains("not a web link"));
    }
}
