//! Local HTTP endpoint that CS2 POSTs game state to.
//!
//! GSI is push: the game is the client. This is a deliberately minimal
//! HTTP/1.1 server — one known client on loopback posting small JSON bodies
//! with Content-Length — not a general web server. Payloads with the wrong
//! auth token are answered 200 (so the game doesn't buffer and hammer
//! retries) but never forwarded.
//!
//! Lifecycle: the accept loop polls non-blocking and exits once the paired
//! [`GsiSource`] is dropped, releasing the port. The recorder service
//! restarts between recording sessions, so the previous listener must free
//! the port promptly; `bind` also retries briefly to absorb that race.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::payload::GsiPayload;

const ACCEPT_POLL: Duration = Duration::from_millis(200);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const BIND_RETRIES: u32 = 10;
const BIND_RETRY_DELAY: Duration = Duration::from_millis(100);

/// The receiving end of a GSI listener. Dropping it stops the accept loop
/// and frees the port (within one poll interval).
pub struct GsiSource {
    rx: Receiver<GsiPayload>,
    /// Liveness anchor: the accept loop holds the matching [`Weak`].
    _alive: Arc<()>,
}

impl GsiSource {
    pub fn receiver(&self) -> &Receiver<GsiPayload> {
        &self.rx
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<GsiPayload, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }
}

/// Bind the GSI endpoint and start serving on background threads.
/// `expected_token`: posts must carry this auth token to be forwarded;
/// `None` accepts everything (tests, spikes).
pub fn bind(addr: &str, expected_token: Option<String>) -> std::io::Result<(GsiSource, String)> {
    let listener = bind_with_retry(addr)?;
    listener.set_nonblocking(true)?;
    let local_addr = listener.local_addr()?.to_string();

    let (tx, rx) = mpsc::channel();
    let alive = Arc::new(());
    let watch = Arc::downgrade(&alive);

    std::thread::Builder::new()
        .name("clipline-cs2-gsi".into())
        .spawn(move || accept_loop(listener, tx, watch, expected_token))
        .map_err(|e| std::io::Error::other(format!("spawn gsi listener: {e}")))?;

    Ok((GsiSource { rx, _alive: alive }, local_addr))
}

fn bind_with_retry(addr: &str) -> std::io::Result<TcpListener> {
    let mut last_err = None;
    for _ in 0..BIND_RETRIES {
        match TcpListener::bind(addr) {
            Ok(listener) => return Ok(listener),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(BIND_RETRY_DELAY);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("gsi bind failed")))
}

fn accept_loop(
    listener: TcpListener,
    tx: Sender<GsiPayload>,
    watch: Weak<()>,
    expected_token: Option<String>,
) {
    loop {
        if watch.upgrade().is_none() {
            return; // GsiSource dropped — free the port.
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let tx = tx.clone();
                let token = expected_token.clone();
                let watch = watch.clone();
                let _ = std::thread::Builder::new()
                    .name("clipline-cs2-gsi-conn".into())
                    .spawn(move || serve_connection(stream, tx, watch, token));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(_) => std::thread::sleep(ACCEPT_POLL),
        }
    }
}

fn serve_connection(
    stream: TcpStream,
    tx: Sender<GsiPayload>,
    watch: Weak<()>,
    expected_token: Option<String>,
) {
    if stream.set_nonblocking(false).is_err() || stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
    {
        return;
    }
    let mut reader = BufReader::new(stream);

    // CS2 keeps the connection alive and posts repeatedly.
    loop {
        if watch.upgrade().is_none() {
            return;
        }
        let request = match read_request(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => return, // clean EOF between requests
            Err(_) => return,
        };

        let status: &str = if request.method != "POST" {
            "405 Method Not Allowed"
        } else {
            match GsiPayload::from_json(&request.body) {
                Ok(payload) => {
                    let authorized = expected_token
                        .as_deref()
                        .is_none_or(|expected| payload.auth_token() == expected);
                    if authorized && tx.send(payload).is_err() {
                        return; // consumer gone
                    }
                    "200 OK"
                }
                Err(_) => "400 Bad Request",
            }
        };

        let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n");
        if reader.get_mut().write_all(response.as_bytes()).is_err() {
            return;
        }
    }
}

struct Request {
    method: String,
    body: Vec<u8>,
}

/// Read one HTTP/1.1 request. `Ok(None)` on clean EOF before a request line.
fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let method = line.split_whitespace().next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    let mut header_bytes = line.len();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Err(std::io::Error::other("eof in headers"));
        }
        header_bytes += header.len();
        if header_bytes > MAX_HEADER_BYTES {
            return Err(std::io::Error::other("headers too large"));
        }
        let trimmed = header.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value
                    .trim()
                    .parse()
                    .map_err(|_| std::io::Error::other("bad content-length"))?;
            }
        }
    }

    if content_length > MAX_BODY_BYTES {
        return Err(std::io::Error::other("body too large"));
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    Ok(Some(Request { method, body }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_post(stream: &mut TcpStream, body: &str) -> String {
        let request = format!(
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        // Drain the response headers so the next request starts clean.
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim_end().is_empty() {
                break;
            }
        }
        status
    }

    #[test]
    fn forwards_authorized_posts_and_keeps_the_connection_alive() {
        let (source, addr) = bind("127.0.0.1:0", Some("secret".into())).unwrap();
        let mut stream = TcpStream::connect(&addr).unwrap();

        let status = http_post(
            &mut stream,
            r#"{ "auth": { "token": "secret" }, "provider": { "steamid": "1" } }"#,
        );
        assert!(status.contains("200"), "{status}");
        let payload = source.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(payload.provider.unwrap().steamid, "1");

        // Second post on the same connection (CS2 keep-alive behavior).
        let status = http_post(
            &mut stream,
            r#"{ "auth": { "token": "secret" }, "provider": { "steamid": "2" } }"#,
        );
        assert!(status.contains("200"), "{status}");
        let payload = source.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(payload.provider.unwrap().steamid, "2");
    }

    #[test]
    fn wrong_token_gets_200_but_is_never_forwarded() {
        let (source, addr) = bind("127.0.0.1:0", Some("secret".into())).unwrap();
        let mut stream = TcpStream::connect(&addr).unwrap();

        let status = http_post(
            &mut stream,
            r#"{ "auth": { "token": "wrong" }, "provider": { "steamid": "1" } }"#,
        );
        assert!(status.contains("200"), "{status}");
        assert!(source.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn malformed_json_is_rejected_without_killing_the_connection() {
        let (source, addr) = bind("127.0.0.1:0", None).unwrap();
        let mut stream = TcpStream::connect(&addr).unwrap();

        let status = http_post(&mut stream, "not json");
        assert!(status.contains("400"), "{status}");

        let status = http_post(&mut stream, r#"{ "provider": { "steamid": "3" } }"#);
        assert!(status.contains("200"), "{status}");
        assert_eq!(
            source
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .provider
                .unwrap()
                .steamid,
            "3"
        );
    }

    #[test]
    fn dropping_the_source_frees_the_port() {
        let (source, addr) = bind("127.0.0.1:0", None).unwrap();
        drop(source);
        // The accept loop notices within one poll interval; rebinding the
        // same port must then succeed (bind retries absorb the gap).
        let (_source, addr2) = bind(&addr, None).unwrap();
        assert_eq!(addr, addr2);
    }
}
