//! Client for `mm0web` — the mm0 proof service that verifies a development
//! (`mm0-rs` compile, then `mm0-c`) and publishes it to a Zenodo project
//! deposit.
//!
//! Plain HTTP/1.1 over `std::net::TcpStream`: the two services are deployed
//! side by side and the hop never leaves the host, so a TLS stack would be a
//! dependency and a second trust store for nothing. An `https://` base URL is
//! refused when the service starts rather than silently downgraded.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

use crate::util::url_encode;

/// mm0web verifies synchronously but answers `202` as soon as the publication
/// is queued, so a publish returns quickly. The status poll is a local index
/// read. 30 s is generous for both and keeps a hung backend from pinning a
/// request thread forever.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// What `POST /api/v1/publish` said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PublishReply {
    pub status: u16,
    pub accepted: bool,
    /// mm0web's content-addressed proof id (present whenever verification ran).
    pub proof: Option<String>,
    pub doi: Option<String>,
    pub record: Option<u64>,
    pub url: Option<String>,
    /// What the checkers said — `mm0-rs` and `mm0-c` output.
    pub log: Option<String>,
    /// The stage a rejection happened at (`mm0-rs` or `mm0-c`), when mm0web
    /// rejected the development.
    pub stage: Option<String>,
    /// What went wrong, when something did.
    pub error: Option<String>,
}

/// What `GET /proof` said about a proof mm0web already knows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProofReply {
    pub state: String,
    pub doi: Option<String>,
    pub record: Option<u64>,
    pub url: Option<String>,
    pub log: Option<String>,
}

pub struct Client {
    authority: String,
    prefix: String,
}

impl Client {
    pub fn new(base: &str) -> Result<Self, String> {
        let trimmed = base.trim().trim_end_matches('/');
        let rest = trimmed.strip_prefix("http://").ok_or_else(|| {
            format!("mm0web base URL must be http:// (got {base:?}); TLS is not supported on this hop")
        })?;
        if rest.is_empty() {
            return Err(format!("mm0web base URL {base:?} has no host"));
        }
        let (authority, prefix) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, String::new()),
        };
        if authority.is_empty() {
            return Err(format!("mm0web base URL {base:?} has no host"));
        }
        Ok(Self {
            authority: authority.to_string(),
            prefix,
        })
    }

    pub fn base(&self) -> String {
        format!("http://{}{}", self.authority, self.prefix)
    }

    /// Submit a development for verification and (if it checks out) for
    /// publication. Returns mm0web's answer verbatim; mapping it onto the
    /// ledger's states is the caller's job.
    pub fn publish(
        &self,
        project: &str,
        title: &str,
        mm0: &str,
        mm1: &str,
    ) -> Result<PublishReply, String> {
        let body = format!(
            "project={}&title={}&mm0={}&mm1={}",
            url_encode(project),
            url_encode(title),
            url_encode(mm0),
            url_encode(mm1)
        );
        let (status, bytes) = self.call("POST", "/api/v1/publish", Some(("application/x-www-form-urlencoded", &body)))?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Ok(PublishReply {
            status,
            accepted: v["accepted"] == true,
            proof: string(&v, "proof"),
            doi: string(&v, "doi"),
            record: v["record"].as_u64(),
            url: string(&v, "url"),
            log: string(&v, "log"),
            stage: string(&v, "stage"),
            error: string(&v, "error"),
        })
    }

    /// Read back the state of a proof already submitted.
    pub fn proof(&self, project: &str, id: &str) -> Result<ProofReply, String> {
        let path = format!(
            "/proof?project={}&id={}",
            url_encode(project),
            url_encode(id)
        );
        let (status, bytes) = self.call("GET", &path, None)?;
        if status != 200 {
            return Err(format!("mm0web answered {status} for proof {id}"));
        }
        let v: Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("mm0web proof {id} is not JSON: {e}"))?;
        Ok(ProofReply {
            state: string(&v, "state").unwrap_or_default(),
            doi: string(&v, "doi"),
            record: v["record"].as_u64(),
            url: string(&v, "url"),
            log: string(&v, "log"),
        })
    }

    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<(&str, &str)>,
    ) -> Result<(u16, Vec<u8>), String> {
        let mut stream = TcpStream::connect(&self.authority)
            .map_err(|e| format!("connect to mm0web at {}: {e}", self.authority))?;
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

        let mut head = format!(
            "{method} {}{path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.prefix, self.authority
        );
        let payload = match body {
            Some((content_type, body)) => {
                head.push_str(&format!(
                    "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
                    body.len()
                ));
                body.as_bytes()
            }
            None => &[][..],
        };
        head.push_str("\r\n");

        let mut request = head.into_bytes();
        request.extend_from_slice(payload);
        stream
            .write_all(&request)
            .map_err(|e| format!("send to mm0web: {e}"))?;
        stream.flush().map_err(|e| format!("send to mm0web: {e}"))?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("read mm0web status line: {e}"))?;
        let status: u16 = line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("mm0web sent a malformed status line: {line:?}"))?;

        let mut length: Option<usize> = None;
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| format!("read mm0web headers: {e}"))?;
            if n == 0 {
                break;
            }
            let header = line.trim_end_matches(['\r', '\n']);
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().ok();
            }
        }

        let mut body = Vec::with_capacity(length.unwrap_or(0));
        match length {
            Some(len) => {
                body.resize(len, 0);
                reader
                    .read_exact(&mut body)
                    .map_err(|e| format!("read mm0web body: {e}"))?;
            }
            None => {
                reader
                    .read_to_end(&mut body)
                    .map_err(|e| format!("read mm0web body: {e}"))?;
            }
        }
        Ok((status, body))
    }
}

fn string(v: &Value, key: &str) -> Option<String> {
    v[key]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_must_be_plain_http() {
        assert!(Client::new("https://prove.example").is_err());
        assert!(Client::new("http://").is_err());
        let client = Client::new("http://127.0.0.1:8081/").unwrap();
        assert_eq!(client.base(), "http://127.0.0.1:8081");
        let client = Client::new("http://127.0.0.1:8081/mm0").unwrap();
        assert_eq!(client.base(), "http://127.0.0.1:8081/mm0");
    }
}
