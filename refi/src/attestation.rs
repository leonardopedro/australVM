//! `GET /attestation?nonce=…` — this process's own measurement.
//!
//! The guest's supervisor (`svcmux`) measures every service from outside:
//! the bytes it execs, the argv and env it passes, the state root it names.
//! This endpoint measures the same process from *inside* — its own
//! `/proc/self/exe`, its own argv, its own environment — and binds the
//! result to the caller's nonce. Two independent measurements of one
//! service must agree; when they stop agreeing, something changed
//! underneath one of them, which is exactly what an auditor needs to know.
//!
//! The canonical form is `svcmux-bundle-v1`, implemented for both
//! webservers by `unfer_measure` and pinned there against svcmux's own
//! copy of the same vectors.

use crate::util::{ApiError, HttpResponse};
use unfer_measure::{Bundle, Identity, attestation_binding};

/// The identity this service claims when unsupervised. Under svcmux the
/// supervisor's `SVCMUX_*` bookkeeping overrides id/state_root/listen and
/// pins the config keys, so both measurements describe the same launch.
const SERVICE_ID: &str = "refi";
const ENV_PREFIXES: &[&str] = &["REFI_", "AUSTRAL_BIN"];

/// A nonce is a challenge, not a payload.
const MAX_NONCE: usize = 512;

/// Measure this process and answer the challenge. The JSON carries the
/// same field names as the supervisor's manifest entry, so a verifier can
/// diff them key by key.
pub fn attestation(
    state_root: &std::path::Path,
    listen: &str,
    nonce: Option<&str>,
) -> Result<HttpResponse, ApiError> {
    let nonce = nonce
        .ok_or_else(|| ApiError::bad_request("attestation requires ?nonce=<challenge>"))?;
    if nonce.is_empty() || nonce.len() > MAX_NONCE {
        return Err(ApiError::bad_request(format!(
            "nonce must be 1..{MAX_NONCE} bytes"
        )));
    }
    let state = state_root.to_string_lossy().into_owned();
    let identity = Identity {
        id: SERVICE_ID,
        state_root: &state,
        listen,
        env_prefixes: ENV_PREFIXES,
    };
    let bundle = Bundle::for_service(&identity)
        .map_err(|e| ApiError::internal(format!("measurement failed: {e}")))?;
    let binding = attestation_binding(&bundle.bundle, nonce);

    let mut body = serde_json::to_value(&bundle)
        .map_err(|e| ApiError::internal(format!("serialize: {e}")))?;
    body["nonce"] = serde_json::json!(nonce);
    body["binding"] = serde_json::json!(binding);
    Ok(HttpResponse::json(200, body))
}
