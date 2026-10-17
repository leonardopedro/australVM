//! Sprint 5.1 — the remote binary cache (spec §3.2 "S3 / REST remote
//! sync", roadmap 5.1: *asynchronous chunk upload and download using
//! reqwest and tokio … only missing BLAKE3 chunk hashes are requested over
//! HTTP*).
//!
//! Protocol (REST; each verb maps 1:1 onto an S3 object operation at
//! `s3://<bucket>/chunks/<hex>` / `…/manifests/<hex>` — HTTPS + SigV4 land
//! with the TLS feature, which is deferred because this machine has no
//! egress; recorded as a deviation in PLAN_avm_pkg.md §4):
//!
//! ```text
//! HEAD   /chunks/<64-hex>              -> 200 present | 404 absent
//! POST   /chunks/missing {"hashes":[]} -> 200 {"missing": [hex, ...]}
//! PUT    /chunks/<64-hex>  <raw bytes> -> 201 stored | 409 digest mismatch
//! GET    /chunks/<64-hex>              -> 200 <raw bytes> | 404
//! HEAD   /manifests/<64-hex>           -> 200 | 404
//! PUT    /manifests/<64-hex> <bytes>   -> 201 | 409 identity mismatch
//! GET    /manifests/<64-hex>           -> 200 <canonical manifest bytes>
//! ```
//!
//! Two invariants this module exists to keep:
//!
//! * **Only missing hashes travel.** A push learns the remote's gaps with
//!   one `POST /chunks/missing` round-trip; a pull computes its gaps
//!   locally first (`ChunkStore::has_chunk`), so the request list itself
//!   *is* the missing set — bytes the caller already has are never
//!   requested over HTTP in either direction.
//! * **The client re-verifies everything it receives.** A pulled chunk is
//!   BLAKE3-hashed before it may touch the store, and a pulled manifest's
//!   content identity must equal the id that was asked for — a lying cache
//!   fails the transfer, it cannot poison the CAS.
//!
//! The client is plain HTTP (`reqwest` without TLS features): acceptance
//! tests run against a loopback mock, and this workspace has no network
//! egress. The CLI drives it through [`block_on`].

use crate::engine::{hex, unhex, ChunkStore, Digest, Manifest};
use crate::CasError;

/// A remote content-addressed binary cache (REST shape).
pub struct Remote {
    base: String,
    client: reqwest::Client,
}

impl Remote {
    pub fn new(base: impl Into<String>) -> Self {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        Remote {
            base,
            client: reqwest::Client::new(),
        }
    }

    /// Base URL, trailing slashes trimmed (`http://host:port`, later
    /// `https://bucket.s3…` once TLS lands).
    pub fn base(&self) -> &str {
        &self.base
    }

    fn chunk_url(&self, id: &Digest) -> String {
        format!("{}/chunks/{}", self.base, hex(id))
    }

    fn manifest_url(&self, id: &Digest) -> String {
        format!("{}/manifests/{}", self.base, hex(id))
    }

    fn status_err(what: &str, url: &str, status: reqwest::StatusCode) -> CasError {
        CasError::Remote(format!("{what} {url}: HTTP {status}"))
    }

    /// Which of `ids` the remote does *not* hold yet — the one round-trip
    /// that makes every transfer missing-only (spec 5.1).
    pub async fn missing_chunks(&self, ids: &[Digest]) -> Result<Vec<Digest>, CasError> {
        let hashes: Vec<String> = ids.iter().map(|d| hex(d)).collect();
        let url = format!("{}/chunks/missing", self.base);
        let resp = self
            .client
            .post(&url)
            .body(serde_json::json!({ "hashes": hashes }).to_string())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .send()
            .await
            .map_err(|e| CasError::Remote(format!("POST {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(Self::status_err("POST", &url, resp.status()));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| CasError::Remote(format!("POST {url} body: {e}")))?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| CasError::Remote(format!("missing-list {e}")))?;
        let arr = v
            .get("missing")
            .and_then(|m| m.as_array())
            .ok_or_else(|| CasError::Remote("missing-list: no `missing` array".into()))?;
        let mut out = Vec::with_capacity(arr.len());
        for h in arr {
            let s = h
                .as_str()
                .ok_or_else(|| CasError::Remote("missing-list: entry not a string".into()))?;
            let d = unhex(s).map_err(|e| CasError::Remote(format!("missing-list hex: {e}")))?;
            out.push(d);
        }
        Ok(out)
    }

    /// Upload `ids` the remote lacks. Bytes are read from the local store,
    /// which verifies them on read; the server re-checks the digest.
    /// Returns how many chunks actually travelled.
    pub async fn push_chunks(&self, store: &ChunkStore, ids: &[Digest]) -> Result<usize, CasError> {
        let missing = self.missing_chunks(ids).await?;
        for id in &missing {
            let bytes = store.get_chunk(id)?;
            let url = self.chunk_url(id);
            let resp = self
                .client
                .put(&url)
                .body(bytes)
                .send()
                .await
                .map_err(|e| CasError::Remote(format!("PUT {url}: {e}")))?;
            if !resp.status().is_success() {
                return Err(Self::status_err("PUT", &url, resp.status()));
            }
        }
        Ok(missing.len())
    }

    /// Download every `id` the local store lacks. What "missing" means
    /// here is the *local* view, so it costs zero round-trips to know:
    /// only those hashes are requested. Each payload is BLAKE3-verified
    /// against the requested id *before* it is allowed near the store,
    /// then all new chunks land in one batch append. Returns how many
    /// chunks actually travelled.
    pub async fn pull_chunks(&self, store: &ChunkStore, ids: &[Digest]) -> Result<usize, CasError> {
        let mut seen: std::collections::HashSet<Digest> = std::collections::HashSet::new();
        let want: Vec<Digest> = ids
            .iter()
            .copied()
            .filter(|id| !store.has_chunk(id) && seen.insert(*id))
            .collect();
        let mut fetched: Vec<Vec<u8>> = Vec::with_capacity(want.len());
        for id in &want {
            let url = self.chunk_url(id);
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| CasError::Remote(format!("GET {url}: {e}")))?;
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(CasError::Remote(format!(
                    "chunk {} vanished between listing and fetch",
                    hex(id)
                )));
            }
            if !resp.status().is_success() {
                return Err(Self::status_err("GET", &url, resp.status()));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| CasError::Remote(format!("GET {url} body: {e}")))?;
            let got = *blake3::hash(&bytes).as_bytes();
            if &got != id {
                return Err(CasError::Corrupt(format!(
                    "remote chunk {} does not match its digest (got {})",
                    hex(id),
                    hex(&got)
                )));
            }
            fetched.push(bytes.to_vec());
        }
        if !fetched.is_empty() {
            store.put_chunk_batch(fetched)?;
        }
        Ok(want.len())
    }

    /// True when the remote holds the chunk (single `HEAD`).
    pub async fn has_chunk(&self, id: &Digest) -> Result<bool, CasError> {
        let url = self.chunk_url(id);
        let resp = self
            .client
            .head(&url)
            .send()
            .await
            .map_err(|e| CasError::Remote(format!("HEAD {url}: {e}")))?;
        match resp.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            s => Err(CasError::Remote(format!("HEAD {url}: HTTP {s}"))),
        }
    }

    /// Upload a manifest's canonical bytes (after its chunks, or the
    /// server cannot have a reconstructible entry — `put_manifest` on the
    /// receiving side enforces that again, fail-closed).
    pub async fn push_manifest(&self, store: &ChunkStore, id: &Digest) -> Result<(), CasError> {
        let m = store.get_manifest(id)?;
        let url = self.manifest_url(id);
        let resp = self
            .client
            .put(&url)
            .body(m.to_bytes())
            .send()
            .await
            .map_err(|e| CasError::Remote(format!("PUT {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(Self::status_err("PUT", &url, resp.status()));
        }
        Ok(())
    }

    /// Fetch a manifest, verify its content identity equals the id that
    /// was asked for, then persist it locally — which re-verifies that
    /// every referenced chunk is already present (fail-closed: chunks
    /// first, manifest second).
    pub async fn pull_manifest(&self, store: &ChunkStore, id: &Digest) -> Result<(), CasError> {
        let url = self.manifest_url(id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| CasError::Remote(format!("GET {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CasError::NotFound(hex(id)));
        }
        if !resp.status().is_success() {
            return Err(Self::status_err("GET", &url, resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| CasError::Remote(format!("GET {url} body: {e}")))?;
        let m = Manifest::from_bytes(&bytes)
            .map_err(|e| CasError::Remote(format!("manifest {} undecodable: {e}", hex(id))))?;
        if &m.identity_digest() != id {
            return Err(CasError::Corrupt(format!(
                "remote manifest {} has identity {}",
                hex(id),
                hex(&m.identity_digest())
            )));
        }
        store.put_manifest(&m)?;
        Ok(())
    }

    /// True when the remote holds the manifest (single `HEAD`).
    pub async fn has_manifest(&self, id: &Digest) -> Result<bool, CasError> {
        let url = self.manifest_url(id);
        let resp = self
            .client
            .head(&url)
            .send()
            .await
            .map_err(|e| CasError::Remote(format!("HEAD {url}: {e}")))?;
        match resp.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            s => Err(CasError::Remote(format!("HEAD {url}: HTTP {s}"))),
        }
    }
}

/// Drive a [`Remote`] future from synchronous code (the CLI). Tests use
/// `#[tokio::test]` directly.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("avm-cas: tokio runtime")
        .block_on(future)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// A loopback HTTP/1.1 mock of the binary cache: real sockets, real
    /// protocol, content-addressed — everything below the client is the
    /// production contract, not a fake of it.
    struct MockCache {
        base: String,
        chunks: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        gets: Arc<AtomicUsize>,
        puts: Arc<AtomicUsize>,
        /// Serve these chunk ids' *content tampered* (a lying cache).
        poison: Arc<Mutex<Vec<String>>>,
    }

    impl MockCache {
        fn seed_chunk(&self, id: &Digest, bytes: &[u8]) {
            self.chunks.lock().unwrap().insert(hex(id), bytes.to_vec());
        }
        fn has_chunk(&self, id: &Digest) -> bool {
            self.chunks.lock().unwrap().contains_key(&hex(id))
        }
    }

    fn start_mock() -> MockCache {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let chunks: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let manifests: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let gets = Arc::new(AtomicUsize::new(0));
        let puts = Arc::new(AtomicUsize::new(0));
        let poison = Arc::new(Mutex::new(Vec::new()));
        let state = (
            chunks.clone(),
            manifests.clone(),
            gets.clone(),
            puts.clone(),
            poison.clone(),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let state = state.clone();
                // One thread per connection; requests arrive sequentially
                // from the test anyway, but hyper may pipeline.
                std::thread::spawn(move || {
                    let _ = handle(stream, state);
                });
            }
        });
        MockCache {
            base: format!("http://127.0.0.1:{port}"),
            chunks,
            gets,
            puts,
            poison,
        }
    }

    type State = (
        Arc<Mutex<HashMap<String, Vec<u8>>>>,
        Arc<Mutex<HashMap<String, Vec<u8>>>>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<String>>>,
    );

    fn respond(mut stream: TcpStream, code: u16, reason: &str, body: &[u8]) -> std::io::Result<()> {
        let head = format!(
            "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        stream.flush()
    }

    fn handle(stream: TcpStream, state: State) -> std::io::Result<()> {
        let (chunks, manifests, gets, puts, poison) = state;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut content_length = 0usize;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h)?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            reader.read_exact(&mut body)?;
        }

        let mut segs = path.splitn(3, '/').skip(1); // "", kind, id
        let kind = segs.next().unwrap_or("");
        let id = segs.next().unwrap_or("");

        match (method.as_str(), kind, id) {
            ("POST", "chunks", "missing") => {
                let v: serde_json::Value = serde_json::from_slice(&body)
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                let want = v.get("hashes").and_then(|h| h.as_array()).unwrap();
                let map = chunks.lock().unwrap();
                let missing: Vec<&str> = want
                    .iter()
                    .filter_map(|h| h.as_str())
                    .filter(|h| !map.contains_key(*h))
                    .collect();
                drop(map);
                respond(
                    stream,
                    200,
                    "OK",
                    serde_json::json!({ "missing": missing })
                        .to_string()
                        .as_bytes(),
                )
            }
            ("HEAD", "chunks", h) => {
                let ok = chunks.lock().unwrap().contains_key(h);
                respond(
                    stream,
                    if ok { 200 } else { 404 },
                    if ok { "OK" } else { "Not Found" },
                    b"",
                )
            }
            ("GET", "chunks", h) => {
                gets.fetch_add(1, Ordering::Relaxed);
                let poisoned = poison.lock().unwrap().iter().any(|p| p == h);
                let data = chunks.lock().unwrap().get(h).cloned();
                match data {
                    Some(mut d) => {
                        if poisoned {
                            // Content differs under the same address: the
                            // client's re-hash MUST catch this.
                            d.push(0xEE);
                        }
                        respond(stream, 200, "OK", &d)
                    }
                    None => respond(stream, 404, "Not Found", b""),
                }
            }
            ("PUT", "chunks", h) => {
                puts.fetch_add(1, Ordering::Relaxed);
                let got = hex(blake3::hash(&body).as_bytes());
                if got != h {
                    return respond(stream, 409, "Digest Mismatch", b"");
                }
                chunks.lock().unwrap().insert(h.to_string(), body);
                respond(stream, 201, "Created", b"")
            }
            ("HEAD", "manifests", h) => {
                let ok = manifests.lock().unwrap().contains_key(h);
                respond(
                    stream,
                    if ok { 200 } else { 404 },
                    if ok { "OK" } else { "Not Found" },
                    b"",
                )
            }
            ("PUT", "manifests", h) => {
                // Server-side identity check, exactly as an honest cache
                // would: decode, recompute identity, compare to the path.
                match Manifest::from_bytes(&body) {
                    Ok(m) if hex(&m.identity_digest()) == h => {
                        manifests.lock().unwrap().insert(h.to_string(), body);
                        respond(stream, 201, "Created", b"")
                    }
                    _ => respond(stream, 409, "Identity Mismatch", b""),
                }
            }
            ("GET", "manifests", h) => {
                let data = manifests.lock().unwrap().get(h).cloned();
                match data {
                    Some(d) => respond(stream, 200, "OK", &d),
                    None => respond(stream, 404, "Not Found", b""),
                }
            }
            _ => respond(stream, 404, "Not Found", b""),
        }
    }

    fn store() -> (tempfile::TempDir, ChunkStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::open(dir.path().join("store")).unwrap();
        (dir, store)
    }

    fn filler(n: usize, salt: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(n + 32);
        let mut ctr: u64 = u64::from(salt) << 32;
        while out.len() < n {
            out.extend_from_slice(blake3::hash(&ctr.to_le_bytes()).as_bytes());
            ctr += 1;
        }
        out.truncate(n);
        out
    }

    #[tokio::test]
    async fn push_uploads_only_missing() {
        let mock = start_mock();
        let (_d, store) = store();
        let raws: Vec<Vec<u8>> = (0..5).map(|i| filler(96 * 1024, i)).collect();
        let ids = store.put_chunk_batch(raws.clone()).unwrap();
        // Remote already has two of the five.
        for i in 0..2 {
            mock.seed_chunk(&ids[i], &raws[i]);
        }

        let remote = Remote::new(&mock.base);
        let n = remote.push_chunks(&store, &ids).await.unwrap();
        assert_eq!(n, 3, "only the 3 missing chunks may travel");
        assert_eq!(mock.puts.load(Ordering::Relaxed), 3, "exactly 3 PUTs");
        for id in &ids {
            assert!(mock.has_chunk(id), "remote must now hold every chunk");
        }

        // Idempotent second push: nothing moves.
        let n2 = remote.push_chunks(&store, &ids).await.unwrap();
        assert_eq!(n2, 0, "already-present chunks must not re-upload");
        assert_eq!(mock.puts.load(Ordering::Relaxed), 3, "no extra PUTs");
    }

    #[tokio::test]
    async fn pull_fetches_only_missing_and_verifies() {
        let mock = start_mock();
        let raws: Vec<Vec<u8>> = (0..5).map(|i| filler(96 * 1024, 50 + i)).collect();
        // Populate the remote first.
        {
            let (_d, remote_store) = store();
            let ids = remote_store.put_chunk_batch(raws.clone()).unwrap();
            for (id, raw) in ids.iter().zip(&raws) {
                mock.seed_chunk(id, raw);
            }
        }
        // Local store already has two of them.
        let (_d, local) = store();
        let all_ids = {
            // Digests are content-derived: recompute without a store.
            raws.iter()
                .map(|r| *blake3::hash(r).as_bytes())
                .collect::<Vec<Digest>>()
        };
        local
            .put_chunk_batch(vec![raws[0].clone(), raws[1].clone()])
            .unwrap();

        let remote = Remote::new(&mock.base);
        let before = mock.gets.load(Ordering::Relaxed);
        let n = remote.pull_chunks(&local, &all_ids).await.unwrap();
        assert_eq!(n, 3, "only the 3 missing chunks may be requested");
        assert_eq!(
            mock.gets.load(Ordering::Relaxed) - before,
            3,
            "exactly 3 GETs — missing-only streaming"
        );
        for (id, raw) in all_ids.iter().zip(&raws) {
            assert!(local.has_chunk(id), "local store must hold every chunk");
            assert_eq!(&local.get_chunk(id).unwrap(), raw, "bytes round-trip");
        }
    }

    #[tokio::test]
    async fn pull_rejects_a_lying_cache() {
        let mock = start_mock();
        let (_d, local) = store();
        let good = filler(64 * 1024, 9);
        let id = *blake3::hash(&good).as_bytes();
        // Remote *claims* the address but serves tampered bytes.
        mock.seed_chunk(&id, &good);
        mock.poison.lock().unwrap().push(hex(&id));

        let remote = Remote::new(&mock.base);
        let err = remote.pull_chunks(&local, &[id]).await.unwrap_err();
        assert!(
            matches!(err, CasError::Corrupt(_)),
            "digest mismatch must be Corrupt, got: {err}"
        );
        assert!(
            !local.has_chunk(&id),
            "poisoned bytes must never reach the store"
        );
    }

    #[tokio::test]
    async fn manifest_sync_is_fail_closed_then_roundtrips() {
        let mock = start_mock();
        let (_da, src) = store();
        let data = filler(300 * 1024, 3);
        let m = src.put_buffer("pkg", "1.2.3", &data, None).unwrap();
        let id = m.identity_digest();

        let remote = Remote::new(&mock.base);
        // Chunks first, then the manifest (the reconstructible order).
        let n = remote.push_chunks(&src, &m.chunks).await.unwrap();
        assert_eq!(n, m.chunks.len());
        remote.push_manifest(&src, &id).await.unwrap();
        assert!(remote.has_manifest(&id).await.unwrap());

        // Fresh store: manifest WITHOUT chunks must refuse (fail-closed).
        let (_db, dst) = store();
        let err = remote.pull_manifest(&dst, &id).await.unwrap_err();
        assert!(
            matches!(err, CasError::NotFound(_)),
            "pulling a manifest whose chunks are absent must fail-closed, got: {err}"
        );
        assert!(dst.get_manifest(&id).is_err(), "nothing persisted locally");

        // Now the legal order: chunks, then manifest → byte-exact identity.
        remote.pull_chunks(&dst, &m.chunks).await.unwrap();
        remote.pull_manifest(&dst, &id).await.unwrap();
        let back = dst.get_manifest(&id).unwrap();
        assert_eq!(back.identity_digest(), id);
        assert_eq!(back.to_bytes(), m.to_bytes(), "canonical bytes identical");
        assert_eq!(
            dst.get_chunk(&m.chunks[0]).unwrap(),
            src.get_chunk(&m.chunks[0]).unwrap()
        );
    }
}
