//! U20 — bounded network capabilities.
//!
//! The module-facing boundary is a **dial**, not a socket. A `Net` handle is
//! issued by the host, and every dial goes through [`NetPolicy`]: the host
//! re-checks the requested endpoint, the addresses the name actually resolved
//! to, and (when TLS is requested) the peer's certificate identity. There is no
//! ambient networking here — a host that grants no endpoint issues no policy,
//! and a policy with an empty address range refuses every dial.
//!
//! What this module deliberately does not do: UDP, listening sockets and raw
//! sockets. They are refused by name ([`NetPolicy::unsupported`]) rather than
//! left to a library default, so a module cannot acquire an ambient network
//! capability by asking for a feature the host did not enumerate.
//!
//! HONESTY NOTE (the same Tier 1 caveat as `capstd_mod`): a module that can
//! call `std::net` directly is not contained by this. cap-std-style coupling
//! defines which capabilities cross the boundary; the VM tier is the isolation.
//! A redirect, proxy hop or reconnect is a *new* dial through this same policy,
//! so it is checked again instead of inheriting the first connection's verdict.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One endpoint the host approves: a host pattern and a port. A pattern of the
/// form `*.example.com` matches subdomains only; every other pattern is exact
/// (case-insensitive). A bare host in a manifest is port 443, the only default
/// this parser applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("an approved network endpoint must not be empty".to_string());
        }
        // `[::1]:443` — the bracket form exists so an IPv6 literal is not split
        // at its own colons.
        if let Some(rest) = text.strip_prefix('[') {
            let (host, tail) = rest
                .split_once(']')
                .ok_or_else(|| format!("endpoint `{text}` has no closing `]`"))?;
            let port = match tail.strip_prefix(':') {
                Some(port) => parse_port(port, text)?,
                None if tail.is_empty() => 443,
                None => {
                    return Err(format!(
                        "endpoint `{text}` must be `[address]:port` or `[address]`"
                    ))
                }
            };
            return Ok(Endpoint {
                host: host.to_ascii_lowercase(),
                port,
            });
        }
        // A bare IPv6 literal (`::1`) must not be read as `host ":" port 1`.
        if text.parse::<IpAddr>().is_ok() {
            return Ok(Endpoint {
                host: text.to_ascii_lowercase(),
                port: 443,
            });
        }
        match text.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                Ok(Endpoint {
                    host: host.to_ascii_lowercase(),
                    port: parse_port(port, text)?,
                })
            }
            Some(_) => Err(format!(
                "endpoint `{text}` must be `host:port` (a non-numeric port is refused rather \
                 than silently defaulted)"
            )),
            None => Ok(Endpoint {
                host: text.to_ascii_lowercase(),
                port: 443,
            }),
        }
    }

    /// Does this endpoint rule admit a dial to `host:port`?
    pub fn matches(&self, host: &str, port: u16) -> bool {
        if self.port != port {
            return false;
        }
        let host = host.trim().to_ascii_lowercase();
        match self.host.strip_prefix("*.") {
            // Wildcards match one label or more, never the bare suffix: a grant
            // for `*.example.com` is not a grant for `example.com` itself.
            Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
            None => host == self.host,
        }
    }
}

fn parse_port(text: &str, endpoint: &str) -> Result<u16, String> {
    text.parse::<u16>()
        .map_err(|_| format!("endpoint `{endpoint}` has an invalid port `{text}`"))
}

/// One approved address range (`10.0.0.0/8`, or a single address, which is a
/// `/32` or `/128`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => {
                let addr = addr.parse::<IpAddr>().map_err(|_| {
                    format!("network range `{text}` has an invalid address `{addr}`")
                })?;
                let limit = if addr.is_ipv4() { 32 } else { 128 };
                let prefix = prefix.parse::<u8>().map_err(|_| {
                    format!("network range `{text}` has an invalid prefix `{prefix}`")
                })?;
                if prefix > limit {
                    return Err(format!(
                        "network range `{text}` has prefix /{prefix}, longer than /{limit}"
                    ));
                }
                (addr, prefix)
            }
            None => {
                let addr = text
                    .parse::<IpAddr>()
                    .map_err(|_| format!("network range `{text}` is not an address or CIDR"))?;
                (addr, if addr.is_ipv4() { 32 } else { 128 })
            }
        };
        Ok(IpNet { addr, prefix })
    }

    pub fn contains(&self, candidate: IpAddr) -> bool {
        match (self.addr, candidate) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - self.prefix)
                };
                (u32::from(net) & mask) == (u32::from(ip) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - self.prefix)
                };
                (u128::from(net) & mask) == (u128::from(ip) & mask)
            }
            // A v4 range says nothing about a v6 address, or the reverse.
            _ => false,
        }
    }
}

/// Why a network operation was refused. The variants map to distinct ABI codes
/// so a module can tell "you may not" from "not there" from "the deadline".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    Denied(String),
    NotFound(String),
    Timeout(String),
    Tls(String),
    Unsupported(String),
    Io(String),
    TooLarge(String),
}

impl NetError {
    pub fn message(&self) -> &str {
        match self {
            NetError::Denied(m)
            | NetError::NotFound(m)
            | NetError::Timeout(m)
            | NetError::Tls(m)
            | NetError::Unsupported(m)
            | NetError::Io(m)
            | NetError::TooLarge(m) => m,
        }
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// The endpoints, address ranges, trust anchors and deadline one instance was
/// granted. Constructing one is the host's act; a module never sees it.
pub struct NetPolicy {
    endpoints: Vec<Endpoint>,
    ranges: Vec<IpNet>,
    tls: Option<Arc<rustls::ClientConfig>>,
    timeout: Duration,
    /// The manifest spelling, kept for refusal messages.
    spec: String,
}

impl std::fmt::Debug for NetPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetPolicy")
            .field("endpoints", &self.endpoints)
            .field("ranges", &self.ranges)
            .field("tls", &self.tls.is_some())
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl NetPolicy {
    /// Build a policy from the manifest spellings. `ca_der` is the host's trust
    /// anchor bundle already read from disk (DER certificates, PEM accepted by
    /// [`certs_from_pem`]); an empty bundle means TLS dials are refused rather
    /// than downgraded to plaintext.
    pub fn new(
        endpoints: &[String],
        ranges: &[String],
        ca_der: Vec<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Self, String> {
        let mut parsed = Vec::new();
        for text in endpoints {
            let endpoint = Endpoint::parse(text)?;
            if !parsed.contains(&endpoint) {
                parsed.push(endpoint);
            }
        }
        let mut nets = Vec::new();
        for text in ranges {
            let net = IpNet::parse(text)?;
            if !nets.contains(&net) {
                nets.push(net);
            }
        }
        let tls = if ca_der.is_empty() {
            None
        } else {
            let mut roots = rustls::RootCertStore::empty();
            for der in &ca_der {
                roots
                    .add(rustls::pki_types::CertificateDer::from(der.clone()))
                    .map_err(|e| format!("trust anchor is not a certificate: {e}"))?;
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("cannot configure TLS: {e}"))?
                .with_root_certificates(roots)
                .with_no_client_auth();
            Some(Arc::new(config))
        };
        let spec = format!("{endpoints:?} over {ranges:?}");
        Ok(Self {
            endpoints: parsed,
            ranges: nets,
            tls,
            timeout,
            spec,
        })
    }

    /// The endpoint rules this policy carries, for a caller that wants to
    /// report the grant without dialing.
    pub fn endpoints(&self) -> &[Endpoint] {
        &self.endpoints
    }

    /// Does any rule approve this endpoint? A host outside the list is refused
    /// before resolution, so a denied name never reaches DNS.
    pub fn approves_endpoint(&self, host: &str, port: u16) -> bool {
        self.endpoints.iter().any(|rule| rule.matches(host, port))
    }

    /// Does the approved address range admit this resolved address?
    pub fn approves_address(&self, addr: IpAddr) -> bool {
        self.ranges.iter().any(|net| net.contains(addr))
    }

    /// Refuse an operation the host did not enumerate. Only the three forms the
    /// policy has no rights model for are named as unsupported; any other
    /// request is a malformed argument, so `"tcp"` is not silently accepted as
    /// "nothing to do".
    pub fn unsupported(&self, operation: &str) -> NetError {
        match operation.trim().to_ascii_lowercase().as_str() {
            "udp" | "udp-dial" | "listen" | "listen-tcp" | "raw" | "raw-socket" => {
                NetError::Unsupported(format!(
                    "network operation `{operation}` is not provided by this capability; only \
                     outbound TCP dials (optionally TLS) are granted"
                ))
            }
            other => NetError::Denied(format!(
                "`{other}` is not a network operation this capability understands"
            )),
        }
    }

    /// Dial `host:port`. Resolution and every connection attempt are bounded by
    /// the policy's deadline, and **every** address the name resolved to must be
    /// inside an approved range; a name that resolves to one denied address is
    /// refused instead of being retried through it. The returned peer address is
    /// the one that actually accepted the connection.
    pub fn dial(&self, host: &str, port: u16, tls: bool) -> Result<Connection, NetError> {
        if !self.approves_endpoint(host, port) {
            return Err(NetError::Denied(format!(
                "endpoint `{host}:{port}` is not approved by `{}`",
                self.spec
            )));
        }
        if tls && self.tls.is_none() {
            return Err(NetError::Tls(
                "TLS was requested but this host granted no trust anchor; refusing rather than \
                 falling back to an unverified connection"
                    .to_string(),
            ));
        }
        let deadline = Instant::now() + self.timeout;
        let addrs = resolve_within(host, port, self.timeout, |target| {
            target
                .to_socket_addrs()
                .map(|iter| iter.collect::<Vec<SocketAddr>>())
        })
        .map_err(|error| match error {
            NetError::Timeout(_) => NetError::Timeout(format!(
                "resolving `{host}:{port}` exceeded the {}-ms deadline",
                self.timeout.as_millis()
            )),
            other => other,
        })?;
        if addrs.is_empty() {
            return Err(NetError::NotFound(format!(
                "`{host}:{port}` resolved to no address"
            )));
        }
        let denied: Vec<String> = addrs
            .iter()
            .filter(|addr| !self.approves_address(addr.ip()))
            .map(|addr| addr.ip().to_string())
            .collect();
        if !denied.is_empty() {
            return Err(NetError::Denied(format!(
                "`{host}:{port}` resolved to {} address(es) outside the approved ranges ({})",
                denied.len(),
                denied.join(", ")
            )));
        }
        let mut last = None;
        for addr in &addrs {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(NetError::Timeout(format!(
                    "dialing `{host}:{port}` exceeded the {}-ms deadline",
                    self.timeout.as_millis()
                )));
            }
            match TcpStream::connect_timeout(addr, remaining) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(self.timeout));
                    let _ = stream.set_write_timeout(Some(self.timeout));
                    let stream = if tls {
                        self.wrap_tls(host, stream)?
                    } else {
                        Stream::Plain(stream)
                    };
                    return Ok(Connection {
                        stream,
                        peer: *addr,
                        server: format!("{host}:{port}"),
                        tls,
                    });
                }
                Err(error) => last = Some(error),
            }
        }
        Err(NetError::Io(format!(
            "dialing `{host}:{port}` failed: {}",
            last.map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    fn wrap_tls(&self, host: &str, stream: TcpStream) -> Result<Stream, NetError> {
        let config = self
            .tls
            .clone()
            .ok_or_else(|| NetError::Tls("TLS was requested without a trust anchor".to_string()))?;
        let server = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|e| NetError::Tls(format!("`{host}` is not a valid TLS server name: {e}")))?;
        let session = rustls::ClientConnection::new(config, server)
            .map_err(|e| NetError::Tls(format!("cannot start TLS with `{host}`: {e}")))?;
        let mut stream = rustls::StreamOwned::new(session, stream);
        // Finish the handshake here so a wrong name, an untrusted chain or a
        // peer that closes early is the dial's typed error, not the first
        // read's `ConnectionReset`.
        while stream.conn.is_handshaking() {
            stream
                .conn
                .complete_io(&mut stream.sock)
                .map_err(|e| NetError::Tls(format!("TLS handshake with `{host}` failed: {e}")))?;
        }
        Ok(Stream::Tls(Box::new(stream)))
    }
}

/// One established connection. It is owned by the capability handle: closing
/// the handle (or revoking it) drops the socket, and a module cannot re-present
/// the handle under another identity.
pub struct Connection {
    stream: Stream,
    /// The address that accepted the connection, not the name that resolved.
    peer: SocketAddr,
    server: String,
    tls: bool,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("peer", &self.peer)
            .field("server", &self.server)
            .field("tls", &self.tls)
            .finish()
    }
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stream::Plain(_) => f.write_str("plain"),
            Stream::Tls(_) => f.write_str("tls"),
        }
    }
}

impl Connection {
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn is_tls(&self) -> bool {
        self.tls
    }

    /// Write one bounded frame and report the bytes accepted. A failed write is
    /// an error, never a partial success quietly reported as the full length.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, NetError> {
        let stream: &mut dyn Write = match &mut self.stream {
            Stream::Plain(stream) => stream,
            Stream::Tls(stream) => stream,
        };
        stream
            .write_all(data)
            .map(|()| data.len())
            .map_err(|e| io_error("write", e))
    }

    /// Read **once**, up to `max` bytes. An empty return is EOF; a module that
    /// wants a full frame loops on this call and owns its own frame limit.
    pub fn read(&mut self, max: usize) -> Result<Vec<u8>, NetError> {
        let mut buf = vec![0u8; max];
        let stream: &mut dyn Read = match &mut self.stream {
            Stream::Plain(stream) => stream,
            Stream::Tls(stream) => stream,
        };
        let read = stream.read(&mut buf).map_err(|e| io_error("read", e))?;
        buf.truncate(read);
        Ok(buf)
    }

    /// Half-close the write side. The read side stays open so a protocol can
    /// still receive its response after the request is complete.
    pub fn shutdown_write(&mut self) {
        if let Stream::Plain(stream) = &mut self.stream {
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    }
}

/// Resolve `host:port`, bounding how long the caller waits. `getaddrinfo` has
/// no portable cancellation, so the resolver runs on its own thread and the
/// caller stops waiting at the deadline; the thread exits when the lookup
/// eventually returns, but it can no longer stall the dial. `resolver` is a
/// parameter so the timeout path is testable without a slow DNS server.
fn resolve_within<F>(
    host: &str,
    port: u16,
    timeout: Duration,
    resolver: F,
) -> Result<Vec<SocketAddr>, NetError>
where
    F: FnOnce(String) -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
{
    let target = format!("{host}:{port}");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("avm-net-resolve".to_string())
        .spawn(move || {
            let _ = sender.send(resolver(target));
        })
        .map_err(|e| NetError::Io(format!("cannot start the resolver thread: {e}")))?;
    match receiver.recv_timeout(timeout) {
        Ok(Ok(addrs)) => Ok(addrs),
        Ok(Err(error)) => Err(NetError::NotFound(format!(
            "cannot resolve `{host}:{port}`: {error}"
        ))),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(NetError::Timeout(format!(
            "resolving `{host}:{port}` did not answer within {}",
            timeout.as_millis()
        ))),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(NetError::Io(format!(
            "the resolver thread for `{host}:{port}` ended without answering"
        ))),
    }
}

fn io_error(operation: &str, error: std::io::Error) -> NetError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => NetError::Timeout(
            format!("{operation} exceeded the connection deadline: {error}"),
        ),
        _ => NetError::Io(format!("{operation} failed: {error}")),
    }
}

/// Read DER certificates from a PEM bundle. A bundle that is already DER (no
/// `-----BEGIN` marker) is passed through as one certificate.
pub fn certs_from_pem(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains("-----BEGIN") {
        if bytes.is_empty() {
            return Err("trust anchor file is empty".to_string());
        }
        return Ok(vec![bytes.to_vec()]);
    }
    pem_blocks(bytes, "CERTIFICATE")
}

/// Read every `BEGIN <label>` block in a PEM file as DER bytes. Used for the
/// certificate bundle and, by the tests, for a PKCS#8 private key.
pub fn pem_blocks(bytes: &[u8], label: &str) -> Result<Vec<Vec<u8>>, String> {
    let begin = format!("-----BEGIN {label}-----");
    let end_marker = format!("-----END {label}-----");
    let text = String::from_utf8_lossy(bytes);
    let mut blocks = Vec::new();
    let mut rest = text.as_ref();
    while let Some(start) = rest.find(&begin) {
        let after = &rest[start + begin.len()..];
        let end = after
            .find(&end_marker)
            .ok_or_else(|| format!("PEM has an unterminated {label} block"))?;
        let body: String = after[..end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        blocks.push(
            base64_decode(&body)
                .ok_or_else(|| format!("PEM has a {label} block that is not valid base64"))?,
        );
        rest = &after[end + end_marker.len()..];
    }
    if blocks.is_empty() {
        return Err(format!("PEM contains no {label} block"));
    }
    Ok(blocks)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u8;
    for byte in text.bytes() {
        acc = (acc << 6) | u32::from(value(byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_patterns_are_exact_or_one_or_more_subdomain_labels() {
        let rule = Endpoint::parse("*.example.com:443").unwrap();
        assert!(rule.matches("api.example.com", 443));
        assert!(rule.matches("a.b.example.com", 443));
        assert!(!rule.matches("example.com", 443));
        assert!(!rule.matches("badexample.com", 443));
        assert!(!rule.matches("api.example.com", 444));
        let exact = Endpoint::parse("127.0.0.1:8080").unwrap();
        assert!(exact.matches("127.0.0.1", 8080));
        let v6 = Endpoint::parse("[::1]:993").unwrap();
        assert_eq!(v6.host, "::1");
        assert_eq!(v6.port, 993);
        assert_eq!(Endpoint::parse("api.example.com").unwrap().port, 443);
        assert!(Endpoint::parse("").is_err());
        assert!(Endpoint::parse("host:notaport").is_err());
    }

    #[test]
    fn address_ranges_admit_only_their_own_family_and_prefix() {
        let net = IpNet::parse("10.0.0.0/8").unwrap();
        assert!(net.contains("10.2.3.4".parse().unwrap()));
        assert!(!net.contains("11.2.3.4".parse().unwrap()));
        assert!(!net.contains("::1".parse().unwrap()));
        let one = IpNet::parse("127.0.0.1").unwrap();
        assert!(one.contains("127.0.0.1".parse().unwrap()));
        assert!(!one.contains("127.0.0.2".parse().unwrap()));
        let all = IpNet::parse("0.0.0.0/0").unwrap();
        assert!(all.contains("203.0.113.9".parse().unwrap()));
        assert!(!all.contains("::1".parse().unwrap()));
        assert!(IpNet::parse("10.0.0.0/33").is_err());
        assert!(IpNet::parse("not-an-address").is_err());
    }

    /// U20: a resolver that does not answer cannot hold the dial past the
    /// deadline, and an answering one still returns its addresses.
    #[test]
    fn resolution_is_bounded_by_the_deadline() {
        let start = Instant::now();
        let slow = resolve_within("slow.test", 1, Duration::from_millis(50), |_| {
            std::thread::sleep(Duration::from_millis(300));
            Ok(vec![])
        });
        assert!(matches!(slow, Err(NetError::Timeout(_))), "{slow:?}");
        assert!(
            start.elapsed() < Duration::from_millis(250),
            "the caller waited {:?}",
            start.elapsed()
        );

        let fast = resolve_within("localhost", 1, Duration::from_millis(500), |target| {
            target
                .to_socket_addrs()
                .map(|iter| iter.collect::<Vec<SocketAddr>>())
        });
        let addrs = fast.expect("localhost resolves");
        assert!(!addrs.is_empty());
        assert!(addrs.iter().all(|addr| addr.port() == 1));
    }

    #[test]
    fn a_policy_with_no_address_range_refuses_a_resolvable_endpoint() {
        let policy = NetPolicy::new(
            &["localhost:9".to_string()],
            &[],
            Vec::new(),
            Duration::from_millis(50),
        )
        .unwrap();
        assert!(policy.approves_endpoint("localhost", 9));
        let error = policy.dial("localhost", 9, false).unwrap_err();
        assert!(matches!(error, NetError::Denied(_)), "{error:?}");
    }

    #[test]
    fn unsupported_operations_are_named_and_unknown_ones_are_not_accepted() {
        let policy = NetPolicy::new(
            &["localhost:9".into()],
            &[],
            Vec::new(),
            Duration::from_millis(1),
        )
        .unwrap();
        for operation in ["udp", "LISTEN", "raw-socket"] {
            assert!(matches!(
                policy.unsupported(operation),
                NetError::Unsupported(_)
            ));
        }
        assert!(matches!(
            policy.unsupported("something-else"),
            NetError::Denied(_)
        ));
    }

    #[test]
    fn pem_and_der_trust_anchors_are_both_read() {
        // A one-byte DER blob is not a certificate, but the reader's job is
        // only to hand the bytes to the root store.
        assert_eq!(
            certs_from_pem(&[0x30, 0x00]).unwrap(),
            vec![vec![0x30, 0x00]]
        );
        let pem = b"-----BEGIN CERTIFICATE-----\nMAA=\n-----END CERTIFICATE-----\n";
        assert_eq!(certs_from_pem(pem).unwrap(), vec![vec![0x30, 0x00]]);
        assert!(certs_from_pem(b"").is_err());
        assert!(
            certs_from_pem(b"-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n")
                .is_err()
        );
    }
}
