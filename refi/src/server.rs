//! A small HTTP/1.1 server: one thread per connection, bounded header/body
//! sizes, read/write timeouts, `Connection: close`.
//!
//! This replaced the Pingora front that the first draft used. Pingora 0.8.1
//! builds its accepted streams through `BufStream::with_capacity(0, 0)`, which
//! tokio 1.54 rejects with `capacity must be greater than zero`; every
//! connection panicked the service's proxy thread before a request reached the
//! router (observed on this host, aarch64). The transport is deliberately the
//! least interesting part of the service — `service.rs` owns all behaviour —
//! so a dependency-free server is the honest fix rather than pinning a broken
//! dependency tree.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::service::Service;
use crate::util::{HttpRequest, HttpResponse, url_decode};

/// Concurrent connection ceiling. Beyond this the listener answers 503 rather
/// than letting a client exhaust the instance's memory.
const MAX_CONNECTIONS: usize = 256;
const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a shed connection gets to finish reading its 503 before the socket
/// is closed underneath it.
const SHED_TIMEOUT: Duration = Duration::from_secs(1);

pub fn serve(service: Arc<Service>, listen: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen)?;
    tracing::info!(listen = %listen, "listening (HTTP/1.1)");
    serve_listener(listener, service, MAX_CONNECTIONS)
}

/// Serve an already-bound listener with an explicit connection ceiling. Split
/// out of [`serve`] so tests can bind port 0, cap the pool at a handful of
/// sockets and drive the wire protocol directly.
pub fn serve_listener(
    listener: TcpListener,
    service: Arc<Service>,
    max_connections: usize,
) -> std::io::Result<()> {
    // Per listener, not per process: the ceiling bounds the threads this server
    // spawns, so two servers in one process do not share a quota (and tests do
    // not interfere with each other's accounting).
    let active = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                continue;
            }
        };
        if active.load(Ordering::Relaxed) >= max_connections {
            // Shed off the accept path: the refusal has to be written and the
            // socket half-closed, which takes a round trip the loop must not
            // wait for. Not counted against the ceiling — it does no parsing
            // and lives at most SHED_TIMEOUT.
            std::thread::Builder::new()
                .name("refi-shed".to_string())
                .spawn(move || shed(stream))
                .ok();
            continue;
        }
        active.fetch_add(1, Ordering::Relaxed);
        let svc = Arc::clone(&service);
        let counter = Arc::clone(&active);
        let spawned = std::thread::Builder::new()
            .name("refi-conn".to_string())
            .spawn(move || {
                handle_connection(&svc, stream);
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        if spawned.is_err() {
            // The closure was dropped without running, so its decrement never
            // happens: give the slot back or the ceiling leaks capacity.
            active.fetch_sub(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Refuse one connection with a 503, then half-close and drain what the
/// client already sent.
///
/// The drain is not politeness: closing a socket that still has unread request
/// bytes makes the kernel answer with RST instead of FIN, and an RST discards
/// the 503 the client has not read yet — it sees `ConnectionReset` instead of
/// the refusal. Reading until the peer's FIN (or a short timeout) lets the
/// kernel close cleanly.
fn shed(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(SHED_TIMEOUT));
    let _ = stream.write_all(
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(SHED_TIMEOUT));
    let mut scratch = [0u8; 2048];
    let mut drained = 0usize;
    while drained < MAX_HEAD {
        match stream.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained += n,
        }
    }
}

fn handle_connection(svc: &Service, stream: TcpStream) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);

    let (resp, head_only) = match read_request(&mut reader, &peer) {
        Ok((req, head_only)) => (svc.handle(req), head_only),
        Err((status, message)) => (
            HttpResponse::error(status, "UK-1000", "BadRequest", &message),
            false,
        ),
    };
    write_response(&mut writer, &resp, head_only);
}

fn read_request(
    reader: &mut BufReader<TcpStream>,
    peer: &str,
) -> Result<(HttpRequest, bool), (u16, String)> {
    let mut head_bytes = 0usize;
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .map_err(|_| (400u16, "could not read the request line".to_string()))?;
    if n == 0 {
        return Err((400, "empty request".to_string()));
    }
    head_bytes += n;
    let request_line = line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("");
    if method.is_empty() || !target.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err((400, "malformed request line".to_string()));
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|_| (400u16, "could not read headers".to_string()))?;
        if n == 0 {
            return Err((400, "connection closed mid-headers".to_string()));
        }
        head_bytes += n;
        if head_bytes > MAX_HEAD {
            return Err((431, "request headers too large".to_string()));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            return Err((400, "malformed header".to_string()));
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        match name.as_str() {
            "content-length" => {
                content_length = Some(
                    value
                        .parse()
                        .map_err(|_| (400u16, "invalid Content-Length".to_string()))?,
                );
            }
            "transfer-encoding" if value.eq_ignore_ascii_case("chunked") => chunked = true,
            _ => {}
        }
        headers.push((name, value));
        if headers.len() > 100 {
            return Err((431, "too many headers".to_string()));
        }
    }

    if chunked {
        return Err((
            411,
            "chunked bodies are not supported; send Content-Length".to_string(),
        ));
    }
    let len = content_length.unwrap_or(0);
    if len > MAX_BODY {
        return Err((413, "request body exceeds 256 kB".to_string()));
    }
    let mut body = vec![0u8; len];
    if len > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|_| (400u16, "truncated request body".to_string()))?;
    }

    let (path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let query = raw_query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(k), url_decode(v))
        })
        .collect();

    // HEAD is GET without the body (RFC 9110 §9.3.2), so route it as GET and
    // drop the bytes at write time.
    let head_only = method.eq_ignore_ascii_case("HEAD");
    let method = if head_only { "GET".to_string() } else { method };
    Ok((
        HttpRequest {
            method,
            path,
            query,
            headers,
            body,
            peer: peer.to_string(),
            // TLS is terminated by the tunnel/reverse proxy in this
            // deployment; see deploy/README.md.
            secure: false,
        },
        head_only,
    ))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

fn write_response(stream: &mut TcpStream, resp: &HttpResponse, head_only: bool) {
    let mut out = Vec::with_capacity(resp.body.len() + 512);
    out.extend_from_slice(
        format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status)).as_bytes(),
    );
    out.extend_from_slice(format!("Content-Type: {}\r\n", resp.content_type).as_bytes());
    out.extend_from_slice(format!("Content-Length: {}\r\n", resp.body.len()).as_bytes());
    out.extend_from_slice(b"Connection: close\r\n");
    for (name, value) in &resp.headers {
        // Header injection is impossible: names are fixed by the service and
        // values are validated (cookies, Location) before they reach here.
        if !value.contains('\r') && !value.contains('\n') {
            out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
    }
    out.extend_from_slice(b"\r\n");
    // A HEAD response carries the representation's length but no body
    // (RFC 9110 §9.3.2): Content-Length above is the GET length, and only the
    // bytes are dropped.
    if !head_only {
        out.extend_from_slice(&resp.body);
    }
    let _ = stream.write_all(&out);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reasons_cover_the_service() {
        assert_eq!(reason(200), "OK");
        assert_eq!(reason(422), "Unprocessable Entity");
        assert_eq!(reason(429), "Too Many Requests");
        assert_eq!(reason(303), "See Other");
    }
}
