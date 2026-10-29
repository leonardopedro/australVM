//! Small, dependency-free helpers: time, tokens, hashing, escaping and the
//! request/response types the transport adapter shares with the service.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Unix seconds. Saturates rather than panicking on a pre-epoch clock.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Unix day number, used by the daily peg-in cap.
pub fn today() -> u64 {
    now_unix() / 86_400
}

/// `sha256(bytes)` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A fresh 32-byte token, hex encoded. `prefix` namespaces it for operators
/// reading a log line (`refi_...`) so a leaked token is recognisable.
pub fn random_token(prefix: &str) -> String {
    let bytes: [u8; 32] = rand::random();
    format!("{prefix}{}", hex::encode(bytes))
}

/// Constant-time comparison of two byte strings (length leaks only).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse a 32-byte hex string (cert, reserve and dedup ids).
pub fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(s.trim()).ok()?;
    bytes.as_slice().try_into().ok()
}

/// Is this a DOI? `10.<registrant>/<suffix>` — the shape Zenodo mints, and
/// alongside `unfccc:vc:` the provenance a certificate may carry.
pub fn is_doi(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 || s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let Some(rest) = s.strip_prefix("10.") else {
        return false;
    };
    let Some((registrant, suffix)) = rest.split_once('/') else {
        return false;
    };
    // A registrant is 4–9 digits (ISO 26622), the suffix is non-empty.
    (4..=9).contains(&registrant.len())
        && registrant.chars().all(|c| c.is_ascii_digit())
        && !suffix.is_empty()
}

/// HTML-escape text. Every interpolation into the site goes through this.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Percent-decode a form/query component (`+` becomes a space).
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode `application/x-www-form-urlencoded` into key/value pairs.
pub fn parse_form(body: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(body);
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

/// Look a decoded form field up.
pub fn form_get<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    form.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Percent-encode a path segment (page slugs, page titles in links).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A minimal snake-case slugifier (ASCII; everything else collapses to `-`).
pub fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = true;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "page".to_string()
    } else {
        trimmed
    }
}

/// Write a file atomically and durably: temp file in the same directory,
/// fsync, rename, fsync the directory. A crash either leaves the previous
/// file intact or the new one complete — never a truncated state file.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8], secret: bool) -> Result<(), String> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if secret {
                opts.mode(0o600);
            }
        }
        let mut f = opts
            .open(&tmp)
            .map_err(|e| format!("open {}: {e}", tmp.display()))?;
        f.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        f.sync_all()
            .map_err(|e| format!("fsync {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))?;
    // Persist the directory entry too, so a power loss cannot resurrect the
    // old name after a successful rename.
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Read a secret file and expect 32 hex-encoded bytes (or a 32-byte hex
/// token). Returns the decoded bytes.
pub fn read_hex32(path: &std::path::Path) -> Result<[u8; 32], String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_hex32(text.trim()).ok_or_else(|| {
        format!(
            "{} must contain 32 hex-encoded bytes (64 hex chars), got {} chars",
            path.display(),
            text.trim().len()
        )
    })
}

/// A transport-neutral request. `server.rs` fills this from the socket; tests
/// fill it directly, so the whole service is testable without a socket.
#[derive(Debug, Clone, Default)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub peer: String,
    /// True when the connection arrived over TLS (drives the Secure cookie
    /// attribute).
    pub secure: bool,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn query_get(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The request body as JSON, if it parses.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// A transport-neutral response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl HttpResponse {
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8".to_string(),
            body: serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
            headers: Vec::new(),
        }
    }

    pub fn html(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "text/html; charset=utf-8".to_string(),
            body: body.into_bytes(),
            headers: Vec::new(),
        }
    }

    pub fn text(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: body.into_bytes(),
            headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// An API error in one shape everywhere:
    /// `{"error":{"code","name","message"}}`.
    pub fn error(status: u16, code: &str, name: &str, message: &str) -> Self {
        Self::json(
            status,
            json!({"error": {"code": code, "name": name, "message": message}}),
        )
    }
}

/// A structured API error the engine and service pass around.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub name: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: u16, code: &str, name: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            name: name.to_string(),
            message: message.into(),
        }
    }

    pub fn to_response(&self) -> HttpResponse {
        HttpResponse::error(self.status, &self.code, &self.name, &self.message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "UK-1001", "BadRequest", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, "UK-4001", "Unauthorized", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "UK-1002", "NotFound", message)
    }

    pub fn limit(message: impl Into<String>) -> Self {
        Self::new(429, "UK-RATE-LIMIT", "RateLimited", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "UK-5000", "Internal", message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(503, "UK-5001", "Unavailable", message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escaping_covers_the_dangerous_five() {
        assert_eq!(
            escape_html("<script>\"a\" & 'b'</script>"),
            "&lt;script&gt;&quot;a&quot; &amp; &#x27;b&#x27;&lt;/script&gt;"
        );
    }

    #[test]
    fn form_and_query_decoding() {
        assert_eq!(url_decode("a+b%20c%2F"), "a b c/");
        let form = parse_form(b"title=Hello+world&body=%3Cb%3Ex%3C%2Fb%3E");
        assert_eq!(form_get(&form, "title"), Some("Hello world"));
        assert_eq!(form_get(&form, "body"), Some("<b>x</b>"));
        assert_eq!(form_get(&form, "missing"), None);
    }

    #[test]
    fn slugs_are_stable_and_safe() {
        assert_eq!(slugify("Hello, ReFi World!"), "hello-refi-world");
        assert_eq!(slugify("  ---  "), "page");
        assert_eq!(slugify("Ünïcode 2026"), "n-code-2026");
    }

    #[test]
    fn hex32_round_trips_and_rejects_junk() {
        let id = [7u8; 32];
        assert_eq!(parse_hex32(&hex::encode(id)), Some(id));
        assert_eq!(parse_hex32("zz"), None);
        assert_eq!(parse_hex32(&hex::encode([1u8; 31])), None);
    }

    #[test]
    fn constant_time_eq_is_correct() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn doi_shapes_are_the_ones_zenodo_mints() {
        assert!(is_doi("10.5281/zenodo.123456"));
        assert!(is_doi("10.1093/mnas/abc.def-42"));
        assert!(!is_doi("10.1234"), "no separator, no suffix");
        assert!(!is_doi("10.1/x"), "registrant is too short");
        assert!(!is_doi("10.5281/zenodo.1 "));
        assert!(!is_doi("https://doi.org/10.5281/zenodo.1"));
        assert!(!is_doi("unfccc:vc:1"));
    }
}
