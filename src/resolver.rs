//! Bounded HTTP redirect resolution with a reusable, certificate-validating client.

use std::error::Error;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect;

use crate::links::{parse_link, rejection_reason};
use crate::results::Resolution;

const TIMEOUT_SECONDS: u64 = 30;
const MAX_REDIRECTS: usize = 10;

/// Reuse connections across a batch while limiting each request's duration.
pub struct Resolver {
    client: Client,
}

impl Resolver {
    /// Build a client with a 30-second timeout and ten-redirect limit.
    /// Returns a contextual error if TLS or client initialization fails.
    pub fn new() -> Result<Self, String> {
        Self::with_limits(Duration::from_secs(TIMEOUT_SECONDS), MAX_REDIRECTS)
    }

    fn with_limits(timeout: Duration, max_redirects: usize) -> Result<Self, String> {
        // reqwest's no-provider mode requires explicit initialization. Reusing
        // an existing process-wide provider is normal, including in parallel tests.
        match rustls::crypto::ring::default_provider().install_default() {
            Ok(()) => {}
            Err(_) if rustls::crypto::CryptoProvider::get_default().is_some() => {}
            Err(_) => return Err("Could not initialize the TLS crypto provider.".into()),
        }
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .user_agent(format!(
                "{}/{}",
                crate::APP_NAME.replace(' ', ""),
                env!("CARGO_PKG_VERSION")
            ))
            .redirect(redirect::Policy::custom(move |attempt| {
                if !matches!(attempt.url().scheme(), "http" | "https") {
                    attempt.error("The link redirects to something that is not a web address (http:// or https://)")
                } else if attempt.previous().len() > max_redirects {
                    attempt.error("This link keeps redirecting, so it has no final destination")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|error| {
                format!("Could not initialize networking: {}", error_details(&error))
            })?;
        Ok(Self { client })
    }

    /// Resolve one absolute HTTP(S) URL using GET and return its input and outcome.
    /// Invalid URLs, connection failures and redirect loops are stored as errors;
    /// final HTTP error statuses still count as successfully resolved destinations.
    /// The response body is not downloaded, and JavaScript redirects are not run.
    pub fn resolve(&self, input: &str) -> Resolution {
        let outcome = self.resolve_url(input);
        Resolution {
            input: input.to_owned(),
            outcome,
        }
    }

    fn resolve_url(&self, input: &str) -> Result<String, String> {
        let url = parse_link(input).ok_or_else(|| rejection_reason(input))?;
        let response = self.client.get(url).send().map_err(|error| {
            if error.is_timeout() {
                "The link timed out. Check your connection or try again later.".to_owned()
            } else if error.is_redirect() {
                format!("Could not follow redirects: {}", error_details(&error))
            } else {
                format!("Request failed: {}", error_details(&error))
            }
        })?;
        // The HTTP stack can decline malformed/non-HTTP Location headers before
        // calling our policy. Such a response is not a completed resolution.
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
            && response.headers().contains_key(reqwest::header::LOCATION)
        {
            return Err("The link redirects somewhere this app cannot follow. The destination may be invalid or unsupported.".into());
        }
        Ok(response.url().to_string())
    }
}

fn error_details(error: &dyn Error) -> String {
    let mut details = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        details.push_str(": ");
        details.push_str(&cause.to_string());
        source = cause.source();
    }
    details
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve(replies: Vec<String>, delay: Duration) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
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
                thread::sleep(delay);
                // A timeout test may have closed its socket before this write.
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (url, handle)
    }

    fn reply(status: &str, headers: &str) -> String {
        format!("HTTP/1.1 {status}\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n")
    }

    #[test]
    fn resolves_relative_redirects_and_final_http_errors() {
        let (base, server) = serve(
            vec![
                reply("302 Found", "Location: /middle\r\n"),
                reply("307 Temporary Redirect", "Location: /final?source=test\r\n"),
                reply("403 Forbidden", ""),
            ],
            Duration::ZERO,
        );
        let result = Resolver::new().unwrap().resolve(&format!("{base}/start"));
        assert_eq!(result.outcome.unwrap(), format!("{base}/final?source=test"));
        server.join().unwrap();
    }

    #[test]
    fn exact_redirect_limit_succeeds_but_one_more_fails() {
        for (redirects, succeeds) in [(2, true), (3, false)] {
            let mut replies = vec![reply("302 Found", "Location: /next\r\n"); redirects];
            if succeeds {
                replies.push(reply("200 OK", ""));
            }
            let (base, server) = serve(replies, Duration::ZERO);
            let result = Resolver::with_limits(Duration::from_secs(5), 2)
                .unwrap()
                .resolve(&base);
            assert_eq!(result.outcome.is_ok(), succeeds, "{:?}", result.outcome);
            server.join().unwrap();
        }
    }

    #[test]
    fn rejects_non_http_redirect_destinations() {
        let (base, server) = serve(
            vec![reply("302 Found", "Location: file:///etc/hosts\r\n")],
            Duration::ZERO,
        );
        assert!(Resolver::new().unwrap().resolve(&base).outcome.is_err());
        server.join().unwrap();
    }

    #[test]
    fn slow_responses_time_out() {
        let (base, server) = serve(vec![reply("200 OK", "")], Duration::from_millis(300));
        let result = Resolver::with_limits(Duration::from_millis(100), 10)
            .unwrap()
            .resolve(&base);
        assert!(result.outcome.unwrap_err().contains("timed out"));
        server.join().unwrap();
    }

    #[test]
    fn invalid_inputs_fail_before_network_access() {
        let resolver = Resolver::new().unwrap();
        for input in ["not a URL", "", "file:///etc/hosts", "ftp://example.com"] {
            assert!(resolver.resolve(input).outcome.is_err());
        }
    }

    #[test]
    fn closed_port_produces_a_contextual_connection_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let error = Resolver::new().unwrap().resolve(&url).outcome.unwrap_err();
        assert!(error.starts_with("Request failed:"));
    }
}
