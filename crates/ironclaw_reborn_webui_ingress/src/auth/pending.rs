//! In-memory store of pending OAuth flows awaiting callback.
//!
//! Each `/auth/login/{provider}` request mints a CSRF state token
//! plus a PKCE code verifier and persists them under the state
//! token. The callback handler atomically `take`s the entry by
//! state, validates the provider name matches the
//! authorization-stage provider, exchanges the code with the PKCE
//! verifier, and discards the entry.
//!
//! Bounded (capacity cap + TTL) so a flood of unauthenticated
//! `/auth/login` calls cannot grow the map unbounded — the cap is
//! enforced before insertion. Entries are single-use: a `take`
//! consumes the entry, so a replayed callback cannot re-use a state
//! token.
//!
//! The cache is intentionally process-local. A future multi-replica
//! deployment must replace this module with a shared store (matches
//! the `ironclaw_reborn_composition` CLAUDE.md note that the first
//! WebUI-mounted OAuth route keeps raw PKCE verifiers in a bounded,
//! expiring process-local cache because `ironclaw_auth` durable
//! records may store hashes only).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

/// State entries older than this are evicted on every access.
const STATE_TTL: Duration = Duration::from_secs(300);
/// Hard cap on pending-flow entries to bound memory under flood.
const MAX_PENDING_STATES: usize = 1024;

/// A pending OAuth flow awaiting callback completion.
#[derive(Clone)]
pub(super) struct PendingFlow {
    /// Provider name the login was initiated for. The callback
    /// rejects cross-provider state replay by comparing this against
    /// the URL `{provider}` segment.
    pub provider: String,
    /// PKCE code verifier — the original 32-byte random value
    /// (base64url-encoded). The callback hands it to the provider's
    /// token exchange unchanged.
    pub code_verifier: String,
    /// Validated redirect target the SPA should land on after the
    /// callback completes. Always starts with `/`; the validator
    /// rejected anything that could escape the same origin.
    pub redirect_after: Option<String>,
    created_at: Instant,
}

/// Thread-safe pending-flow store.
#[derive(Default)]
pub(super) struct PendingFlowStore {
    inner: Mutex<HashMap<String, PendingFlow>>,
}

impl PendingFlowStore {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Generate a PKCE code verifier: 32 random bytes, base64url
    /// (no padding). RFC 7636 requires 43-128 chars; this yields 43.
    pub(super) fn generate_code_verifier() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Compute the PKCE S256 code challenge from a verifier:
    /// `base64url_no_pad(sha256(verifier))`.
    pub(super) fn code_challenge(verifier: &str) -> String {
        let hash = Sha256::digest(verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
    }

    /// Mint a new pending flow and return the CSRF state token the
    /// browser will round-trip through the provider.
    pub(super) fn insert(
        &self,
        provider: impl Into<String>,
        redirect_after: Option<String>,
    ) -> (String, PendingFlow) {
        let provider = provider.into();
        let now = Instant::now();
        let mut guard = self.inner.lock();

        // Opportunistic GC on insert: if at capacity, sweep expired
        // entries first, and if still full, drop the oldest. This
        // keeps the map size bounded under flood without a background
        // task.
        if guard.len() >= MAX_PENDING_STATES {
            guard.retain(|_, flow| now.duration_since(flow.created_at) < STATE_TTL);
        }
        if guard.len() >= MAX_PENDING_STATES
            && let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, flow)| flow.created_at)
                .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        }

        let flow = PendingFlow {
            provider,
            code_verifier: Self::generate_code_verifier(),
            redirect_after,
            created_at: now,
        };
        let state = mint_state_token();
        guard.insert(state.clone(), flow.clone());
        (state, flow)
    }

    /// Atomically remove and return the flow for `state`. Returns
    /// `None` if the state is unknown or expired. Single-use: a
    /// successful take consumes the entry, so a replayed callback
    /// cannot re-use the state token.
    pub(super) fn take(&self, state: &str) -> Option<PendingFlow> {
        let mut guard = self.inner.lock();
        let flow = guard.remove(state)?;
        if Instant::now().duration_since(flow.created_at) >= STATE_TTL {
            return None;
        }
        Some(flow)
    }
}

/// Mint a 32-byte hex CSRF state token. Hex (not base64) so it round-
/// trips cleanly through URL query parameters without escaping.
fn mint_state_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Sanitize a caller-supplied `redirect_after` value: must start with
/// `/`, must not start with `//` or `/\` (protocol-relative), and
/// must contain only RFC-3986 path/query/fragment characters. The
/// percent-decoded form must also pass — `%2f%2f` decodes to `//`,
/// and a naive check on the raw value would miss that.
pub(super) fn sanitize_redirect(input: Option<String>) -> Option<String> {
    input.filter(|raw| is_safe_redirect(raw))
}

pub(super) fn is_safe_redirect(url: &str) -> bool {
    if !check_redirect_chars(url) {
        return false;
    }
    let Ok(decoded) = urlencoding::decode(url) else {
        return false;
    };
    check_redirect_chars(&decoded)
}

fn check_redirect_chars(url: &str) -> bool {
    if !url.starts_with('/') || url.starts_with("//") || url.starts_with("/\\") {
        return false;
    }
    url.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/_-.~:@!$&'()*+,;=?#[]%".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_challenge_is_deterministic_per_verifier() {
        let a = PendingFlowStore::code_challenge("abc");
        let b = PendingFlowStore::code_challenge("abc");
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn insert_then_take_returns_same_flow() {
        let store = PendingFlowStore::new();
        let (state, flow) = store.insert("google", Some("/v2".to_string()));
        assert!(!state.is_empty());
        let taken = store.take(&state).expect("flow present");
        assert_eq!(taken.provider, "google");
        assert_eq!(taken.code_verifier, flow.code_verifier);
        assert_eq!(taken.redirect_after.as_deref(), Some("/v2"));
    }

    #[test]
    fn take_is_single_use() {
        let store = PendingFlowStore::new();
        let (state, _) = store.insert("google", None);
        assert!(store.take(&state).is_some());
        assert!(store.take(&state).is_none(), "second take must be empty");
    }

    #[test]
    fn unknown_state_token_returns_none() {
        let store = PendingFlowStore::new();
        assert!(store.take("nonexistent").is_none());
    }

    #[test]
    fn safe_redirects_pass_validation() {
        assert!(is_safe_redirect("/"));
        assert!(is_safe_redirect("/v2"));
        assert!(is_safe_redirect("/v2/threads/abc"));
        assert!(is_safe_redirect("/v2?tab=settings#section"));
    }

    #[test]
    fn open_redirects_are_blocked() {
        assert!(!is_safe_redirect("//evil.example"));
        assert!(!is_safe_redirect("/\\evil.example"));
        assert!(!is_safe_redirect("https://evil.example"));
        assert!(!is_safe_redirect("javascript:alert(1)"));
        // Percent-encoded smuggling: %2f%2f → //
        assert!(!is_safe_redirect("/%2f%2fevil.example"));
        // Percent-encoded backslash: %5c → \
        assert!(!is_safe_redirect("/%5cevil.example"));
    }

    #[test]
    fn sanitize_redirect_strips_unsafe_inputs() {
        assert_eq!(
            sanitize_redirect(Some("/v2".to_string())),
            Some("/v2".to_string())
        );
        assert_eq!(sanitize_redirect(Some("//attacker".to_string())), None);
        assert_eq!(sanitize_redirect(None), None);
    }
}
