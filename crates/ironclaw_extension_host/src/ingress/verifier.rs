//! Host-side execution of `[channel.ingress.verification]` recipes.
//!
//! The router runs this before an adapter sees anything: signing secrets are
//! resolved by the host, compared constant-time, and never enter adapter
//! scope. Recipes are pure data ([`IngressVerificationRecipe`]); this module
//! is the one implementation of each verification *kind*
//! (`docs/ironclaw/extension-runtime/overview.md` §3, §5.3).
//!
//! Multi-candidate resolution: with several candidate installations on one
//! route the verifier tries each within [`MAX_VERIFICATION_CANDIDATES`] and
//! resolves exactly one — zero verifying is an authentication failure, more
//! than one is ambiguous (checklist ING-6).

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use ironclaw_host_api::{
    HmacSha256VerificationRecipe, IngressVerificationRecipe, SharedSecretHeaderRecipe,
    SignatureEncoding, SignedPayloadSegment,
};

/// Fixed bound on installations tried per request (overview.md §5.3: "small
/// constant").
pub const MAX_VERIFICATION_CANDIDATES: usize = 8;

/// One candidate installation's verification secret.
#[derive(Clone)]
pub struct VerificationCandidate {
    pub installation_id: String,
    pub secret: Vec<u8>,
}

impl std::fmt::Debug for VerificationCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerificationCandidate")
            .field("installation_id", &self.installation_id)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// The installation a request verified against, plus which headers the
/// verification consumed (the router strips them before the adapter runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedInstallation {
    pub installation_id: String,
    /// Header names consumed by verification (signature, timestamp, shared
    /// secret). Never forwarded to the adapter.
    pub consumed_headers: Vec<String>,
}

/// Typed verification failures. Wire responses map every variant to a
/// generic 401 — the distinctions exist for tests and logs, not callers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerificationFailure {
    #[error("required verification header `{header}` is missing")]
    MissingHeader { header: String },
    #[error("verification header `{header}` appears more than once")]
    DuplicateHeader { header: String },
    #[error("verification header `{header}` is malformed")]
    MalformedHeader { header: String },
    #[error("request timestamp is outside the {max_age_seconds}s replay window")]
    StaleTimestamp { max_age_seconds: u32 },
    #[error("signature mismatch")]
    SignatureMismatch,
    #[error("more than one candidate installation verified — ambiguous")]
    Ambiguous,
    #[error("no candidate installation is configured for this route")]
    NoCandidates,
    #[error("candidate installations exceed the fixed verification bound")]
    TooManyCandidates,
    #[error("verification secret is misconfigured (empty)")]
    MisconfiguredSecret,
}

/// Case-insensitive single-instance header lookup over raw request headers.
/// Duplicate instances of a verification-relevant header fail closed.
pub struct IngressHeaders<'a> {
    entries: &'a [(String, Vec<u8>)],
}

impl<'a> IngressHeaders<'a> {
    pub fn new(entries: &'a [(String, Vec<u8>)]) -> Self {
        Self { entries }
    }

    /// The single value of `name` (case-insensitive). Missing or duplicated
    /// instances are typed failures — a duplicated signature/secret header is
    /// a smuggling attempt, never "pick one".
    fn single(&self, name: &str) -> Result<&'a [u8], VerificationFailure> {
        let mut found: Option<&'a [u8]> = None;
        for (key, value) in self.entries {
            if key.eq_ignore_ascii_case(name) {
                if found.is_some() {
                    return Err(VerificationFailure::DuplicateHeader {
                        header: name.to_string(),
                    });
                }
                found = Some(value.as_slice());
            }
        }
        found.ok_or_else(|| VerificationFailure::MissingHeader {
            header: name.to_string(),
        })
    }
}

/// Execute a verification recipe against a request, resolving exactly one
/// candidate installation.
///
/// `now_unix_seconds` is injected so the replay window is deterministic in
/// tests.
pub fn verify_recipe(
    recipe: &IngressVerificationRecipe,
    headers: &IngressHeaders<'_>,
    body: &[u8],
    now_unix_seconds: u64,
    candidates: &[VerificationCandidate],
) -> Result<VerifiedInstallation, VerificationFailure> {
    match recipe {
        IngressVerificationRecipe::None => {
            // No verification declared (explicit). The route still needs an
            // installation identity; exactly one candidate must exist.
            match candidates {
                [] => Err(VerificationFailure::NoCandidates),
                [single] => Ok(VerifiedInstallation {
                    installation_id: single.installation_id.clone(),
                    consumed_headers: Vec::new(),
                }),
                _ => Err(VerificationFailure::Ambiguous),
            }
        }
        IngressVerificationRecipe::HmacSha256(recipe) => {
            verify_hmac(recipe, headers, body, now_unix_seconds, candidates)
        }
        IngressVerificationRecipe::SharedSecretHeader(recipe) => {
            verify_shared_secret(recipe, headers, candidates)
        }
    }
}

fn check_candidate_budget(candidates: &[VerificationCandidate]) -> Result<(), VerificationFailure> {
    if candidates.is_empty() {
        return Err(VerificationFailure::NoCandidates);
    }
    if candidates.len() > MAX_VERIFICATION_CANDIDATES {
        return Err(VerificationFailure::TooManyCandidates);
    }
    Ok(())
}

/// Resolve exactly one verified candidate from per-candidate outcomes.
fn resolve_exactly_one(
    verified: Vec<&VerificationCandidate>,
    first_failure: Option<VerificationFailure>,
    consumed_headers: Vec<String>,
) -> Result<VerifiedInstallation, VerificationFailure> {
    match verified.as_slice() {
        [] => Err(first_failure.unwrap_or(VerificationFailure::SignatureMismatch)),
        [single] => Ok(VerifiedInstallation {
            installation_id: single.installation_id.clone(),
            consumed_headers,
        }),
        _ => Err(VerificationFailure::Ambiguous),
    }
}

fn verify_hmac(
    recipe: &HmacSha256VerificationRecipe,
    headers: &IngressHeaders<'_>,
    body: &[u8],
    now_unix_seconds: u64,
    candidates: &[VerificationCandidate],
) -> Result<VerifiedInstallation, VerificationFailure> {
    check_candidate_budget(candidates)?;

    let mut consumed = vec![recipe.signature_header.clone()];

    // Timestamp/replay window before any HMAC work.
    if let (Some(timestamp_header), Some(max_age_seconds)) =
        (&recipe.timestamp_header, recipe.max_age_seconds)
    {
        consumed.push(timestamp_header.clone());
        let raw = headers.single(timestamp_header)?;
        let text = std::str::from_utf8(raw).map_err(|_| VerificationFailure::MalformedHeader {
            header: timestamp_header.clone(),
        })?;
        // i128: an attacker-controlled `i64::MIN`-shaped header must not
        // overflow the drift arithmetic in overflow-checked builds.
        let timestamp: i128 =
            text.trim()
                .parse()
                .map_err(|_| VerificationFailure::MalformedHeader {
                    header: timestamp_header.clone(),
                })?;
        let drift = (i128::from(now_unix_seconds) - timestamp).abs();
        if drift > i128::from(max_age_seconds) {
            return Err(VerificationFailure::StaleTimestamp { max_age_seconds });
        }
    }

    // The presented signature, minus the declared prefix.
    let signature_raw = headers.single(&recipe.signature_header)?;
    let signature_text =
        std::str::from_utf8(signature_raw).map_err(|_| VerificationFailure::MalformedHeader {
            header: recipe.signature_header.clone(),
        })?;
    let presented = match &recipe.signature_prefix {
        Some(prefix) => signature_text
            .strip_prefix(prefix.as_str())
            .ok_or_else(|| VerificationFailure::MalformedHeader {
                header: recipe.signature_header.clone(),
            })?,
        None => signature_text,
    };

    // The signed payload bytes, exactly as the recipe declares them.
    let mut signed_payload: Vec<u8> = Vec::new();
    for segment in &recipe.signed_payload {
        match segment {
            SignedPayloadSegment::Literal { literal } => {
                signed_payload.extend_from_slice(literal.as_bytes());
            }
            SignedPayloadSegment::Header { header } => {
                signed_payload.extend_from_slice(headers.single(header)?);
                if !consumed
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(header))
                {
                    consumed.push(header.clone());
                }
            }
            SignedPayloadSegment::Body { body: _ } => {
                signed_payload.extend_from_slice(body);
            }
        }
    }

    let mut verified = Vec::new();
    let mut first_failure = None;
    for candidate in candidates {
        if candidate.secret.is_empty() {
            first_failure.get_or_insert(VerificationFailure::MisconfiguredSecret);
            continue;
        }
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&candidate.secret) else {
            first_failure.get_or_insert(VerificationFailure::MisconfiguredSecret);
            continue;
        };
        mac.update(&signed_payload);
        let digest = mac.finalize().into_bytes();
        let expected = match recipe.signature_encoding {
            SignatureEncoding::Hex => hex_encode(&digest),
            SignatureEncoding::Base64 => base64_encode(&digest),
        };
        if bool::from(expected.as_bytes().ct_eq(presented.as_bytes())) {
            verified.push(candidate);
        } else {
            first_failure.get_or_insert(VerificationFailure::SignatureMismatch);
        }
    }
    resolve_exactly_one(verified, first_failure, consumed)
}

fn verify_shared_secret(
    recipe: &SharedSecretHeaderRecipe,
    headers: &IngressHeaders<'_>,
    candidates: &[VerificationCandidate],
) -> Result<VerifiedInstallation, VerificationFailure> {
    check_candidate_budget(candidates)?;
    let presented = headers.single(&recipe.header)?;

    let mut verified = Vec::new();
    let mut first_failure = None;
    for candidate in candidates {
        // Fail closed on a misconfigured (empty) secret: `ct_eq(b"", b"")`
        // is true, so an empty configured secret would authenticate any
        // request carrying an empty header value.
        if candidate.secret.is_empty() {
            first_failure.get_or_insert(VerificationFailure::MisconfiguredSecret);
            continue;
        }
        if bool::from(candidate.secret.as_slice().ct_eq(presented)) {
            verified.push(candidate);
        } else {
            first_failure.get_or_insert(VerificationFailure::SignatureMismatch);
        }
    }
    resolve_exactly_one(verified, first_failure, vec![recipe.header.clone()])
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard (padded) base64. Local implementation keeps the crate's
/// dependency surface minimal for one 32-byte digest per request.
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(BASE64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The acme fixture's recipe, byte-for-byte
    /// (`tests/fixtures/extensions/acme-messenger/manifest.toml`): hex
    /// HMAC-SHA256 over `v0:{timestamp}:{body}` with a `v0=` prefix and a
    /// 300s replay window.
    fn acme_recipe() -> IngressVerificationRecipe {
        toml::from_str(
            r#"
kind = "hmac_sha256"
secret_handle = "acme_signing_secret"
signature_header = "X-Acme-Signature"
signature_prefix = "v0="
signature_encoding = "hex"
timestamp_header = "X-Acme-Request-Timestamp"
max_age_seconds = 300
signed_payload = [
  { literal = "v0:" },
  { header = "X-Acme-Request-Timestamp" },
  { literal = ":" },
  { body = true },
]
"#,
        )
        .expect("acme recipe parses")
    }

    fn shared_secret_recipe() -> IngressVerificationRecipe {
        toml::from_str(
            r#"
kind = "shared_secret_header"
secret_handle = "vendor_webhook_secret"
header = "X-Vendor-Secret-Token"
"#,
        )
        .expect("shared secret recipe parses")
    }

    fn candidate(id: &str, secret: &[u8]) -> VerificationCandidate {
        VerificationCandidate {
            installation_id: id.to_string(),
            secret: secret.to_vec(),
        }
    }

    fn sign_acme(secret: &[u8], timestamp: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        format!("v0={}", hex_encode(&mac.finalize().into_bytes()))
    }

    fn headers(entries: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        entries
            .iter()
            .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
            .collect()
    }

    const NOW: u64 = 1_700_000_000;

    #[test]
    fn hmac_recipe_verifies_the_exact_acme_byte_construction() {
        let secret = b"acme-signing-secret";
        let body = br#"{"type":"message","text":"hi"}"#;
        let timestamp = NOW.to_string();
        let signature = sign_acme(secret, &timestamp, body);
        let entries = headers(&[
            ("X-Acme-Signature", &signature),
            ("X-Acme-Request-Timestamp", &timestamp),
            ("Content-Type", "application/json"),
        ]);

        let verified = verify_recipe(
            &acme_recipe(),
            &IngressHeaders::new(&entries),
            body,
            NOW,
            &[candidate("install-1", secret)],
        )
        .expect("canonical signature verifies");
        assert_eq!(verified.installation_id, "install-1");
        // Verification-consumed headers are reported for stripping.
        assert!(
            verified
                .consumed_headers
                .iter()
                .any(|name| name == "X-Acme-Signature")
        );
        assert!(
            verified
                .consumed_headers
                .iter()
                .any(|name| name == "X-Acme-Request-Timestamp")
        );
        assert!(
            !verified
                .consumed_headers
                .iter()
                .any(|h| h == "Content-Type")
        );
    }

    #[test]
    fn hmac_recipe_rejects_tampered_body_missing_and_bad_signatures() {
        let secret = b"acme-signing-secret";
        let body = br#"{"n":1}"#;
        let timestamp = NOW.to_string();
        let signature = sign_acme(secret, &timestamp, body);
        let recipe = acme_recipe();
        let candidates = [candidate("install-1", secret)];

        // Tampered body.
        let entries = headers(&[
            ("X-Acme-Signature", &signature),
            ("X-Acme-Request-Timestamp", &timestamp),
        ]);
        assert_eq!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                br#"{"n":2}"#,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::SignatureMismatch
        );

        // Missing signature header.
        let entries = headers(&[("X-Acme-Request-Timestamp", &timestamp)]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::MissingHeader { .. }
        ));

        // Signature without the declared prefix.
        let entries = headers(&[
            ("X-Acme-Signature", signature.trim_start_matches("v0=")),
            ("X-Acme-Request-Timestamp", &timestamp),
        ]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::MalformedHeader { .. }
        ));
    }

    #[test]
    fn hmac_recipe_enforces_the_replay_window_before_any_hmac() {
        let secret = b"acme-signing-secret";
        let body = b"{}";
        let recipe = acme_recipe();
        let candidates = [candidate("install-1", secret)];

        // 301s stale (window is 300s) — a correctly signed but replayed
        // capture is rejected.
        let stale_ts = (NOW - 301).to_string();
        let stale_sig = sign_acme(secret, &stale_ts, body);
        let entries = headers(&[
            ("X-Acme-Signature", &stale_sig),
            ("X-Acme-Request-Timestamp", &stale_ts),
        ]);
        assert_eq!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::StaleTimestamp {
                max_age_seconds: 300
            }
        );

        // Far-future timestamps are equally forged.
        let future_ts = (NOW + 301).to_string();
        let future_sig = sign_acme(secret, &future_ts, body);
        let entries = headers(&[
            ("X-Acme-Signature", &future_sig),
            ("X-Acme-Request-Timestamp", &future_ts),
        ]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::StaleTimestamp { .. }
        ));

        // Boundary (exactly 300s) passes — closed window.
        let boundary_ts = (NOW - 300).to_string();
        let boundary_sig = sign_acme(secret, &boundary_ts, body);
        let entries = headers(&[
            ("X-Acme-Signature", &boundary_sig),
            ("X-Acme-Request-Timestamp", &boundary_ts),
        ]);
        assert!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .is_ok()
        );

        // Extreme attacker-controlled values must fail closed, not panic.
        for extreme in ["-9223372036854775808", "9223372036854775807"] {
            let sig = sign_acme(secret, extreme, body);
            let entries = headers(&[
                ("X-Acme-Signature", &sig),
                ("X-Acme-Request-Timestamp", extreme),
            ]);
            assert!(
                verify_recipe(
                    &recipe,
                    &IngressHeaders::new(&entries),
                    body,
                    NOW,
                    &candidates
                )
                .is_err()
            );
        }

        // Malformed timestamp.
        let sig = sign_acme(secret, "not-a-number", body);
        let entries = headers(&[
            ("X-Acme-Signature", &sig),
            ("X-Acme-Request-Timestamp", "not-a-number"),
        ]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::MalformedHeader { .. }
        ));
    }

    #[test]
    fn hmac_recipe_rejects_duplicated_signature_or_timestamp_headers() {
        let secret = b"acme-signing-secret";
        let body = b"{}";
        let timestamp = NOW.to_string();
        let signature = sign_acme(secret, &timestamp, body);
        let recipe = acme_recipe();
        let candidates = [candidate("install-1", secret)];

        let entries = headers(&[
            ("X-Acme-Signature", &signature),
            ("X-Acme-Signature", &signature),
            ("X-Acme-Request-Timestamp", &timestamp),
        ]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::DuplicateHeader { .. }
        ));

        let entries = headers(&[
            ("X-Acme-Signature", &signature),
            ("X-Acme-Request-Timestamp", &timestamp),
            ("x-acme-request-timestamp", &timestamp),
        ]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::DuplicateHeader { .. }
        ));
    }

    #[test]
    fn hmac_recipe_supports_base64_encoding_and_no_prefix() {
        let recipe: IngressVerificationRecipe = toml::from_str(
            r#"
kind = "hmac_sha256"
secret_handle = "vendor_secret"
signature_header = "X-Vendor-Signature"
signature_encoding = "base64"
signed_payload = [ { body = true } ]
"#,
        )
        .expect("recipe parses");
        let secret = b"vendor-secret";
        let body = b"payload-bytes";
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
        mac.update(body);
        let signature = base64_encode(&mac.finalize().into_bytes());

        let entries = headers(&[("X-Vendor-Signature", &signature)]);
        let verified = verify_recipe(
            &recipe,
            &IngressHeaders::new(&entries),
            body,
            NOW,
            &[candidate("install-1", secret)],
        )
        .expect("base64 signature verifies");
        assert_eq!(verified.installation_id, "install-1");
    }

    #[test]
    fn hmac_recipe_resolves_exactly_one_of_multiple_candidates() {
        let body = b"{}";
        let timestamp = NOW.to_string();
        let recipe = acme_recipe();
        let signature = sign_acme(b"secret-b", &timestamp, body);
        let entries = headers(&[
            ("X-Acme-Signature", &signature),
            ("X-Acme-Request-Timestamp", &timestamp),
        ]);

        // Exactly one candidate verifies → resolved.
        let verified = verify_recipe(
            &recipe,
            &IngressHeaders::new(&entries),
            body,
            NOW,
            &[
                candidate("install-a", b"secret-a"),
                candidate("install-b", b"secret-b"),
            ],
        )
        .expect("single matching candidate resolves");
        assert_eq!(verified.installation_id, "install-b");

        // Two candidates verify → ambiguous, fail closed.
        assert_eq!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &[
                    candidate("install-b1", b"secret-b"),
                    candidate("install-b2", b"secret-b"),
                ],
            )
            .unwrap_err(),
            VerificationFailure::Ambiguous
        );

        // Over the fixed bound → rejected before any HMAC.
        let too_many: Vec<_> = (0..=MAX_VERIFICATION_CANDIDATES)
            .map(|index| candidate(&format!("install-{index}"), b"secret-b"))
            .collect();
        assert_eq!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &too_many
            )
            .unwrap_err(),
            VerificationFailure::TooManyCandidates
        );

        // No candidates configured → typed failure.
        assert_eq!(
            verify_recipe(&recipe, &IngressHeaders::new(&entries), body, NOW, &[]).unwrap_err(),
            VerificationFailure::NoCandidates
        );
    }

    #[test]
    fn hmac_recipe_rejects_empty_secret_as_misconfigured() {
        let body = b"{}";
        let timestamp = NOW.to_string();
        let signature = sign_acme(b"", &timestamp, body);
        let entries = headers(&[
            ("X-Acme-Signature", &signature),
            ("X-Acme-Request-Timestamp", &timestamp),
        ]);
        assert_eq!(
            verify_recipe(
                &acme_recipe(),
                &IngressHeaders::new(&entries),
                body,
                NOW,
                &[candidate("install-1", b"")],
            )
            .unwrap_err(),
            VerificationFailure::MisconfiguredSecret
        );
    }

    #[test]
    fn shared_secret_header_verifies_constant_time_and_rejects_missing_duplicate() {
        let recipe = shared_secret_recipe();
        let candidates = [candidate("install-1", b"webhook-secret")];

        // Match.
        let entries = headers(&[("X-Vendor-Secret-Token", "webhook-secret")]);
        let verified = verify_recipe(
            &recipe,
            &IngressHeaders::new(&entries),
            b"",
            NOW,
            &candidates,
        )
        .expect("matching shared secret verifies");
        assert_eq!(verified.installation_id, "install-1");
        assert_eq!(verified.consumed_headers, vec!["X-Vendor-Secret-Token"]);

        // Mismatch.
        let entries = headers(&[("X-Vendor-Secret-Token", "wrong")]);
        assert_eq!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                b"",
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::SignatureMismatch
        );

        // Missing header.
        let entries = headers(&[]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                b"",
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::MissingHeader { .. }
        ));

        // Duplicate header (case-insensitive) fails closed.
        let entries = headers(&[
            ("X-Vendor-Secret-Token", "webhook-secret"),
            ("x-vendor-secret-token", "webhook-secret"),
        ]);
        assert!(matches!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                b"",
                NOW,
                &candidates
            )
            .unwrap_err(),
            VerificationFailure::DuplicateHeader { .. }
        ));

        // Empty configured secret is a misconfiguration, not a match — even
        // for an empty header value.
        let entries = headers(&[("X-Vendor-Secret-Token", "")]);
        assert_eq!(
            verify_recipe(
                &recipe,
                &IngressHeaders::new(&entries),
                b"",
                NOW,
                &[candidate("install-1", b"")],
            )
            .unwrap_err(),
            VerificationFailure::MisconfiguredSecret
        );
    }

    #[test]
    fn none_recipe_requires_exactly_one_candidate() {
        let recipe = IngressVerificationRecipe::None;
        let entries = headers(&[]);
        let ingress_headers = IngressHeaders::new(&entries);

        let verified = verify_recipe(
            &recipe,
            &ingress_headers,
            b"",
            NOW,
            &[candidate("install-1", b"")],
        )
        .expect("single candidate resolves without verification");
        assert_eq!(verified.installation_id, "install-1");
        assert!(verified.consumed_headers.is_empty());

        assert_eq!(
            verify_recipe(&recipe, &ingress_headers, b"", NOW, &[]).unwrap_err(),
            VerificationFailure::NoCandidates
        );
        assert_eq!(
            verify_recipe(
                &recipe,
                &ingress_headers,
                b"",
                NOW,
                &[candidate("a", b""), candidate("b", b"")],
            )
            .unwrap_err(),
            VerificationFailure::Ambiguous
        );
    }
}
