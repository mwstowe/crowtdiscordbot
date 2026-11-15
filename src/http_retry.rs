//! Limited retries for idempotent GET requests to the Frinkiac-family APIs.
//!
//! A single dropped connection, timeout, or 429/5xx from the site shouldn't turn into an
//! error message for the user (or, worse, be mistaken for "no results"). Requests are
//! retried a couple of times with a short pause; anything else (404, 400, ...) is
//! returned straight away because repeating it can't help.

use reqwest::{Client, Response, StatusCode};
use std::time::Duration;
use tracing::warn;

/// Pauses before the second and third attempts, in milliseconds. Three attempts in total,
/// so a request that keeps failing costs about 1.2 seconds extra.
const BACKOFF_MS: [u64; 2] = [300, 900];

/// A status worth retrying: rate limited or a server-side failure.
pub(crate) fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// A transport failure worth retrying: timeouts, refused or dropped connections.
/// Errors from building the request itself (a bad URL) are not.
fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

/// GET `url`, retrying transport errors and 429/5xx responses.
///
/// Returns the response from the last attempt, whatever its status, so the caller decides
/// what a persistent failure means. Returns `Err` only if every attempt failed to get any
/// response (or the error is not retryable).
pub(crate) async fn get_with_retry(client: &Client, url: &str) -> Result<Response, reqwest::Error> {
    get_with_backoff(client, url, &BACKOFF_MS).await
}

async fn get_with_backoff(
    client: &Client,
    url: &str,
    backoff_ms: &[u64],
) -> Result<Response, reqwest::Error> {
    let attempts = backoff_ms.len() + 1;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let last = attempt == attempts;
        match client.get(url).send().await {
            Ok(response) if last || !is_retryable_status(response.status()) => {
                return Ok(response);
            }
            Ok(response) => warn!(
                "GET {} returned {} (attempt {}/{}), retrying",
                url,
                response.status(),
                attempt,
                attempts
            ),
            Err(error) if last || !is_retryable_error(&error) => return Err(error),
            Err(error) => warn!(
                "GET {} failed (attempt {}/{}): {}, retrying",
                url, attempt, attempts, error
            ),
        }
        tokio::time::sleep(Duration::from_millis(backoff_ms[attempt - 1])).await;
    }
}

/// A tiny scripted HTTP server for tests, so retry behaviour is checked against real
/// responses and dropped connections instead of assumptions.
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// One scripted reply, served to one connection.
    pub(crate) enum Reply {
        /// Respond with this status and an empty body.
        Status(u16),
        /// Respond 200 with this JSON body.
        Json(&'static str),
        /// Read the request, then close the connection without answering.
        Drop,
    }

    /// Serve `script` in order, one reply per connection, then stop. Returns the base URL
    /// (no trailing slash) and a counter of connections served.
    pub(crate) fn serve(script: Vec<Reply>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let base = format!("http://{}", listener.local_addr().expect("local addr"));
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();

        std::thread::spawn(move || {
            for reply in script {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);

                // Read the request head.
                let mut head = Vec::new();
                let mut buf = [0u8; 2048];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }

                let response = match reply {
                    Reply::Drop => continue,
                    Reply::Status(code) => format!(
                        "HTTP/1.1 {code} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    ),
                    Reply::Json(body) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    ),
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });

        (base, hits)
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{serve, Reply};
    use super::*;
    use std::sync::atomic::Ordering;

    /// Short pauses so the tests stay fast; the retry logic is the same.
    const FAST: [u64; 2] = [1, 1];

    async fn get(url: &str) -> Result<Response, reqwest::Error> {
        get_with_backoff(&Client::new(), url, &FAST).await
    }

    #[test]
    fn retryable_statuses() {
        for code in [429u16, 500, 502, 503, 504] {
            assert!(
                is_retryable_status(StatusCode::from_u16(code).unwrap()),
                "{code}"
            );
        }
        for code in [200u16, 301, 400, 404, 422] {
            assert!(
                !is_retryable_status(StatusCode::from_u16(code).unwrap()),
                "{code}"
            );
        }
    }

    #[tokio::test]
    async fn success_is_not_retried() {
        let (base, hits) = serve(vec![Reply::Status(200)]);
        let response = get(&base).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn server_error_then_success_is_retried() {
        let (base, hits) = serve(vec![Reply::Status(503), Reply::Status(200)]);
        let response = get(&base).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rate_limit_is_retried_up_to_three_attempts() {
        let (base, hits) = serve(vec![
            Reply::Status(429),
            Reply::Status(429),
            Reply::Status(200),
        ]);
        let response = get(&base).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn persistent_failure_returns_the_last_response() {
        let (base, hits) = serve(vec![
            Reply::Status(503),
            Reply::Status(503),
            Reply::Status(503),
        ]);
        let response = get(&base).await.unwrap();
        // The caller decides what a persistent 503 means.
        assert_eq!(response.status(), 503);
        assert_eq!(hits.load(Ordering::SeqCst), 3, "exactly three attempts");
    }

    #[tokio::test]
    async fn client_errors_are_not_retried() {
        let (base, hits) = serve(vec![Reply::Status(404), Reply::Status(200)]);
        let response = get(&base).await.unwrap();
        assert_eq!(response.status(), 404);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dropped_connection_is_retried() {
        // A one-off transport failure: the server hangs up without answering.
        let (base, hits) = serve(vec![Reply::Drop, Reply::Json("[]")]);
        let response = get(&base).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn refused_connection_gives_up_after_the_last_attempt() {
        // Bind then drop a listener so the port is closed.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let result = get(&format!("http://127.0.0.1:{port}")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_connect());
    }

    #[tokio::test]
    async fn unusable_url_is_not_retried() {
        // Builder errors can't succeed on a retry, so the error comes back immediately.
        let result = get("not a url").await;
        assert!(result.is_err());
    }
}
