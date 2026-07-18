//! In-memory stores for pending OAuth flows and one-time session
//! exchange tickets.
//!
//! Each `/auth/login/{provider}` request mints a CSRF state token
//! plus a PKCE code verifier and persists them under the state
//! token. The callback handler atomically `take`s the entry by
//! state, validates the provider name matches the
//! authorization-stage provider, exchanges the code with the PKCE
//! verifier, mints a server-side session, then stores that session
//! bearer behind a short-lived one-time ticket. The browser receives
//! only the ticket in the redirect URL and redeems it via
//! `/auth/session/exchange`.
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
use rand::RngExt as _;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use super::provider_name::OAuthProviderName;

/// State entries older than this are evicted on every access.
const STATE_TTL: Duration = Duration::from_secs(300);
/// Hard cap on pending-flow entries to bound memory under flood.
const MAX_PENDING_STATES: usize = 1024;
/// Session exchange tickets live only long enough for the SPA to
/// finish loading and POST the ticket back to the same-origin host.
const SESSION_TICKET_TTL: Duration = Duration::from_secs(60);
/// Hard cap on session tickets to bound memory if callbacks are
/// completed but tickets are never redeemed.
const MAX_SESSION_TICKETS: usize = 1024;

/// A pending OAuth flow awaiting callback completion. The
/// `code_verifier` is wrapped in [`SecretString`] so accidental
/// `Debug`/`Serialize` of the struct (e.g. into trace logs) does
/// not leak the PKCE material — the verifier is one half of the
/// only secret a tampering middleman could use to complete a token
/// exchange against a captured authorization code.
pub(super) struct PendingFlow {
    /// Provider name the login was initiated for. The callback
    /// rejects cross-provider state replay by comparing this against
    /// the [`OAuthProviderName`] parsed from the URL `{provider}`
    /// segment.
    pub provider: OAuthProviderName,
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
        rand::rng().fill(&mut bytes);
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
        provider: OAuthProviderName,
        redirect_after: Option<String>,
    ) -> (String, PendingFlow) {
        let verifier_raw = Self::generate_code_verifier();
        let challenge = Self::code_challenge(&verifier_raw);
        let flow = PendingFlow {
            provider,
            code_verifier: SecretString::from(verifier_raw),
            code_challenge: challenge,
            redirect_after,
            created_at: Instant::now(),
        };
        let state = mint_state_token();

        let mut guard = self.inner.lock();

        // Opportunistic GC on insert: if at capacity, sweep expired
        // entries first, and if still full, drop the oldest. This
        // keeps the map size bounded under flood without a background
        // task.
        if guard.len() >= MAX_PENDING_STATES {
            guard.retain(|_, flow| flow.created_at.elapsed() < STATE_TTL);
        }
        if guard.len() >= MAX_PENDING_STATES
            && let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, flow)| flow.created_at)
                .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        }

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
        if flow.created_at.elapsed() >= STATE_TTL {
            return None;
        }
        Some(flow)
    }
}

pub(super) struct SessionTicket {
    bearer: SecretString,
    created_at: Instant,
}

/// One-time, short-lived bearer exchange store. The OAuth callback
/// returns only the random ticket in the redirect `Location`; the SPA
/// redeems it once via `/auth/session/exchange` to receive the real
/// bearer over a same-origin JSON response.
#[derive(Default)]
pub(super) struct SessionTicketStore {
    inner: Mutex<HashMap<String, SessionTicket>>,
}

impl SessionTicketStore {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert(&self, bearer: SecretString) -> String {
        let ticket = mint_state_token();
        let entry = SessionTicket {
            bearer,
            created_at: Instant::now(),
        };

        let mut guard = self.inner.lock();
        if guard.len() >= MAX_SESSION_TICKETS {
            guard.retain(|_, ticket| ticket.created_at.elapsed() < SESSION_TICKET_TTL);
        }
        if guard.len() >= MAX_SESSION_TICKETS
            && let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, ticket)| ticket.created_at)
                .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        }
        guard.insert(ticket.clone(), entry);
        ticket
    }

    pub(super) fn take(&self, ticket: &str) -> Option<SecretString> {
        let mut guard = self.inner.lock();
        let entry = guard.remove(ticket)?;
        if entry.created_at.elapsed() >= SESSION_TICKET_TTL {
            return None;
        }
        Some(entry.bearer)
    }
}

/// Mint a 32-byte hex CSRF state token. Hex (not base64) so it round-
/// trips cleanly through URL query parameters without escaping.
fn mint_state_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Sanitize a caller-supplied `redirect_after` value: must start with
/// `/`, must not start with `//` or `/\` (protocol-relative), must
/// contain only RFC-3986 path/query characters, and must NOT contain
/// a `#` fragment marker.
///
/// `#` is deliberately rejected because the OAuth success redirect
/// appends `?login_ticket=<ticket>` / `&login_ticket=<ticket>` to the
/// validated path. A caller-supplied fragment would not be sent back
/// to the server, and it would also leave the SPA on a confusing
/// post-login URL. The percent-decoded form is also checked so `%23`
/// smuggling fails.
pub(crate) fn sanitize_redirect(input: Option<String>) -> Option<String> {
    input.filter(|raw| is_safe_redirect(raw))
}

pub(crate) fn is_safe_redirect(url: &str) -> bool {
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

    fn google() -> OAuthProviderName {
        OAuthProviderName::new("google").unwrap()
    }

    #[test]
    fn insert_then_take_returns_same_flow() {
        let store = PendingFlowStore::new();
        let (state, flow) = store.insert(google(), Some("/".to_string()));
        assert!(!state.is_empty());
        let taken = store.take(&state).expect("flow present");
        assert_eq!(taken.provider, google());
        assert_eq!(
            taken.code_verifier.expose_secret(),
            flow.code_verifier.expose_secret()
        );
        assert_eq!(taken.code_challenge, flow.code_challenge);
        assert_eq!(taken.redirect_after.as_deref(), Some("/"));
    }

    // Regression: the challenge stored on the flow MUST equal the
    // SHA-256 of the actual verifier the provider's token endpoint
    // will receive. A bug that decoupled `code_verifier` from
    // `code_challenge` would make every Google token exchange fail
    // with `invalid_grant`.
    #[test]
    fn stored_challenge_matches_sha256_of_verifier() {
        let store = PendingFlowStore::new();
        let (state, flow) = store.insert(google(), None);
        let recomputed = PendingFlowStore::code_challenge(flow.code_verifier.expose_secret());
        assert_eq!(flow.code_challenge, recomputed);
        let taken = store.take(&state).expect("flow");
        assert_eq!(taken.code_challenge, recomputed);
    }

    #[test]
    fn take_is_single_use() {
        let store = PendingFlowStore::new();
        let (state, _) = store.insert(google(), None);
        assert!(store.take(&state).is_some());
        assert!(store.take(&state).is_none(), "second take must be empty");
    }

    #[test]
    fn take_removes_expired_entry_from_store() {
        let store = PendingFlowStore::new();
        let state = "expired-state".to_string();
        {
            let mut guard = store.inner.lock();
            guard.insert(
                state.clone(),
                PendingFlow {
                    provider: google(),
                    code_verifier: SecretString::from("expired-verifier".to_string()),
                    code_challenge: "expired-challenge".to_string(),
                    redirect_after: Some("/".to_string()),
                    created_at: Instant::now() - STATE_TTL - Duration::from_secs(1),
                },
            );
        }

        assert!(store.take(&state).is_none());
        assert!(
            !store.inner.lock().contains_key(&state),
            "expired entry must be removed after take",
        );
    }

    // Reviewer-requested regression (#4116 review, "Pending-flow
    // capacity eviction is untested"): the store relies on the
    // 1024-entry cap + oldest-evict tail as its flood bound, but
    // until this test only single-entry behavior was covered. Fill
    // past the cap and assert (a) the map stays bounded, (b) the
    // oldest entry is the one evicted.
    #[test]
    fn pending_store_evicts_oldest_when_capacity_exceeded() {
        let store = PendingFlowStore::new();
        // Insert one extra entry past the documented cap.
        let mut states = Vec::with_capacity(MAX_PENDING_STATES + 1);
        for _ in 0..=MAX_PENDING_STATES {
            let (state, _) = store.insert(google(), None);
            states.push(state);
        }
        let guard = store.inner.lock();
        assert!(
            guard.len() <= MAX_PENDING_STATES,
            "store must stay bounded; got {}",
            guard.len(),
        );
        // The OLDEST state minted is gone.
        let oldest = &states[0];
        assert!(
            !guard.contains_key(oldest),
            "oldest entry must be evicted; map still has it",
        );
        // The NEWEST is still present.
        let newest = states.last().unwrap();
        assert!(
            guard.contains_key(newest),
            "newest entry must survive eviction",
        );
    }

    #[test]
    fn unknown_state_token_returns_none() {
        let store = PendingFlowStore::new();
        assert!(store.take("nonexistent").is_none());
    }

    #[test]
    fn session_ticket_is_single_use() {
        let store = SessionTicketStore::new();
        let ticket = store.insert(SecretString::from("bearer-1".to_string()));

        let first = store.take(&ticket).expect("ticket present");
        assert_eq!(first.expose_secret(), "bearer-1");
        assert!(store.take(&ticket).is_none(), "ticket must be consumed");
    }

    #[test]
    fn expired_session_ticket_returns_none_and_is_removed() {
        let store = SessionTicketStore::new();
        let ticket = "expired-ticket".to_string();
        {
            let mut guard = store.inner.lock();
            guard.insert(
                ticket.clone(),
                SessionTicket {
                    bearer: SecretString::from("expired-bearer".to_string()),
                    created_at: Instant::now() - SESSION_TICKET_TTL - Duration::from_secs(1),
                },
            );
        }

        assert!(store.take(&ticket).is_none());
        assert!(
            !store.inner.lock().contains_key(&ticket),
            "expired ticket must be removed after take",
        );
    }

    #[test]
    fn safe_redirects_pass_validation() {
        assert!(is_safe_redirect("/"));
        // Cached pre-root-migration clients may still request the legacy SPA
        // mount. Keep accepting it so the compatibility redirect can preserve
        // the one-time login ticket on the way to `/`.
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

    #[test]
    fn percent_encoded_crlf_redirect_is_blocked() {
        assert!(!is_safe_redirect("/%0d%0aLocation:%20https://evil.example"));
    }

    // Regression: fragments in the caller-supplied redirect would
    // leave the user on a confusing post-login URL, and would not be
    // visible to the server. Reject both raw and percent-encoded
    // forms.
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
            sanitize_redirect(Some("/".to_string())),
            Some("/".to_string())
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
