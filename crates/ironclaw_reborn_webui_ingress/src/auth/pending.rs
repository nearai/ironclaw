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
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

/// State entries older than this are evicted on every access.
const STATE_TTL: Duration = Duration::from_secs(300);
/// Hard cap on pending-flow entries to bound memory under flood.
const MAX_PENDING_STATES: usize = 1024;

/// A pending OAuth flow awaiting callback completion. The
/// `code_verifier` is wrapped in [`SecretString`] so accidental
/// `Debug`/`Serialize` of the struct (e.g. into trace logs) does
/// not leak the PKCE material — the verifier is one half of the
/// only secret a tampering middleman could use to complete a token
/// exchange against a captured authorization code.
pub(super) struct PendingFlow {
    /// Provider name the login was initiated for. The callback
    /// rejects cross-provider state replay by comparing this against
    /// the URL `{provider}` segment.
    pub provider: String,
    /// PKCE code verifier — the original 32-byte random value
    /// (base64url-encoded), wrapped in `SecretString` for redacted
    /// `Debug`. The callback hands the raw value to the provider's
    /// token exchange unchanged.
    pub code_verifier: SecretString,
    /// PKCE S256 code challenge pre-computed at mint time so the
    /// login handler doesn't recompute SHA-256 every redirect. The
    /// challenge is non-secret (it's emitted in the authorization
    /// URL), so it stays a plain `String`.
    pub code_challenge: String,
    /// Validated redirect target the SPA should land on after the
    /// callback completes. Always starts with `/`; the validator
    /// rejected anything that could escape the same origin.
    pub redirect_after: Option<String>,
    created_at: Instant,
}

impl Clone for PendingFlow {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            code_verifier: SecretString::from(self.code_verifier.expose_secret().to_string()),
            code_challenge: self.code_challenge.clone(),
            redirect_after: self.redirect_after.clone(),
            created_at: self.created_at,
        }
    }
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
    fn generate_code_verifier() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Compute the PKCE S256 code challenge from a verifier:
    /// `base64url_no_pad(sha256(verifier))`. Called once at mint
    /// time and cached on the `PendingFlow`; the login handler
    /// reads `flow.code_challenge` directly.
    fn code_challenge(verifier: &str) -> String {
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

        let verifier_raw = Self::generate_code_verifier();
        let challenge = Self::code_challenge(&verifier_raw);
        let flow = PendingFlow {
            provider,
            code_verifier: SecretString::from(verifier_raw),
            code_challenge: challenge,
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
    hex::encode(bytes)
}

/// Sanitize a caller-supplied `redirect_after` value: must start with
/// `/`, must not start with `//` or `/\` (protocol-relative), must
/// contain only RFC-3986 path/query characters, and must NOT contain
/// a `#` fragment marker.
///
/// `#` is deliberately rejected because the OAuth success redirect
/// concatenates the bearer as `{redirect_after}#token=<bearer>`. A
/// caller-supplied `redirect_after` carrying its own `#…` would
/// produce `…#user_fragment#token=<bearer>`, which the SPA's
/// `URLSearchParams` parses as `token=user_fragment#token=<bearer>`
/// — the user gets stuck on a malformed bearer rather than being
/// logged in. Not an exfiltration path (the real bearer never
/// reaches the attacker), but a clean DoS we can close at the
/// boundary. The percent-decoded form is also checked so `%23`
/// smuggling fails.
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
    // Allowed: alphanumerics + pchar/query subset from RFC 3986,
    // MINUS `#` (see `sanitize_redirect` docstring).
    url.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/_-.~:@!$&'()*+,;=?[]%".contains(&b))
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
        assert_eq!(
            taken.code_verifier.expose_secret(),
            flow.code_verifier.expose_secret()
        );
        assert_eq!(taken.code_challenge, flow.code_challenge);
        assert_eq!(taken.redirect_after.as_deref(), Some("/v2"));
    }

    // Regression: the challenge stored on the flow MUST equal the
    // SHA-256 of the actual verifier the provider's token endpoint
    // will receive. A bug that decoupled `code_verifier` from
    // `code_challenge` would make every Google token exchange fail
    // with `invalid_grant`.
    #[test]
    fn stored_challenge_matches_sha256_of_verifier() {
        let store = PendingFlowStore::new();
        let (state, flow) = store.insert("google", None);
        let recomputed = PendingFlowStore::code_challenge(flow.code_verifier.expose_secret());
        assert_eq!(flow.code_challenge, recomputed);
        let taken = store.take(&state).expect("flow");
        assert_eq!(taken.code_challenge, recomputed);
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
        assert!(is_safe_redirect("/v2?tab=settings"));
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

    // Regression: `#` in the caller-supplied redirect would collide
    // with the OAuth success transport (`/v2#token=<bearer>`) and
    // strand the user on a malformed token. Reject both raw and
    // percent-encoded forms.
    #[test]
    fn redirects_with_fragment_marker_are_blocked() {
        assert!(!is_safe_redirect("/v2#token=fake"));
        assert!(!is_safe_redirect("/v2#section"));
        assert!(!is_safe_redirect("/v2/threads/abc#detail"));
        // Percent-encoded `#` (%23) decodes to `#`.
        assert!(!is_safe_redirect("/v2%23token=fake"));
        assert!(!is_safe_redirect("/v2%23section"));
    }

    #[test]
    fn sanitize_redirect_strips_unsafe_inputs() {
        assert_eq!(
            sanitize_redirect(Some("/v2".to_string())),
            Some("/v2".to_string())
        );
        assert_eq!(sanitize_redirect(Some("//attacker".to_string())), None);
        assert_eq!(
            sanitize_redirect(Some("/v2#token=fake".to_string())),
            None,
            "`#` in redirect must be stripped to prevent fragment collision",
        );
        assert_eq!(sanitize_redirect(None), None);
    }
}
