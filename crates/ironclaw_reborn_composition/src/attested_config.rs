//! Attested-signing external-wallet provider configuration (attested-signing
//! PR13).
//!
//! The [`AttestedSignerContinuationDriver`]'s [`ProviderRegistry`] dispatches a
//! resolved attested gate to the external-wallet provider bound on the gate
//! (`window.ethereum`/`window.solana` injected, NEAR redirect, or
//! WalletConnect v2). The injected provider is stateless and always
//! registrable; the NEAR-redirect and WalletConnect providers need ceremony
//! configuration before they can verify a proof:
//!
//! - **NEAR redirect**: the wallet base URL + the callback URL the wallet
//!   redirects back to, and a server-side `state_secret` (HMAC key) that
//!   MAC-binds the redirect `state` parameter to the gate (defeats callback /
//!   deep-link interception). The `state_secret` is a secret and is sourced
//!   from the environment only — never from the operator TOML (mirrors the
//!   `CUSTODIAL_MAINNET_ENABLED` env convention and the "secrets are env-only"
//!   config policy).
//! - **WalletConnect**: the WalletConnect Cloud `ProjectId` — a *publishable*
//!   app-identity key (shareable across tenants, not a per-tenant secret), so
//!   it is plain config, sourced from the environment.
//!
//! ## Fail-closed (and fail-CLOSED on invalid present config)
//!
//! A provider is registered **only** when its full configuration is present
//! **and valid**. Two distinct failure modes:
//!
//! - *Absent* config → the provider stays unregistered: its wire variant still
//!   decodes and reaches the driver, which fails closed as
//!   [`ContinuationError::ProviderMismatch`].
//! - *Present-but-invalid* config (empty URL, a placeholder / low-entropy
//!   `state_secret`, a malformed WalletConnect `ProjectId`) is a hard error
//!   ([`AttestedConfigError`]) at construction / env-resolution time — never
//!   silently accepted. We never register a provider with a placeholder secret:
//!   a bogus `state_secret` would make every NEAR `state` verify, weakening the
//!   attestation boundary.
//!
//! Validated newtypes (private fields + `Result`-returning constructors) make
//! "valid whenever `Some`" a type-level invariant: no caller can hand
//! [`build_provider_registry`] an empty URL or a 1-byte secret.

use std::sync::Arc;

use ironclaw_attestation::SealedGrantStore;
use ironclaw_attested_runtime::ProviderRegistry;
use ironclaw_wallet_external::{
    InjectedSigningProvider, NearRedirectSigningProvider, ProjectId, WalletConnectSigningProvider,
};
use secrecy::{ExposeSecret, SecretString};

/// Env var holding the NEAR-redirect wallet base URL (e.g. the MyNearWallet /
/// NEAR wallet sign endpoint the user is redirected to).
pub const NEAR_WALLET_BASE_URL_ENV: &str = "ATTESTED_NEAR_WALLET_BASE_URL";
/// Env var holding the NEAR-redirect callback URL the wallet returns to.
pub const NEAR_CALLBACK_URL_ENV: &str = "ATTESTED_NEAR_CALLBACK_URL";
/// Env var holding the NEAR-redirect `state` HMAC secret. **Secret** — env-only.
pub const NEAR_STATE_SECRET_ENV: &str = "ATTESTED_NEAR_STATE_SECRET";
/// Env var holding the WalletConnect Cloud project id (publishable).
pub const WALLETCONNECT_PROJECT_ID_ENV: &str = "ATTESTED_WALLETCONNECT_PROJECT_ID";

/// Minimum NEAR `state_secret` length, in bytes. An HMAC key shorter than the
/// hash block / output is trivially brute-forceable; 32 bytes (256 bits) is the
/// floor for a binding MAC key.
pub const MIN_STATE_SECRET_BYTES: usize = 32;

/// Minimum number of *distinct* bytes a `state_secret`'s key material must
/// contain. A key drawn from a tiny alphabet (e.g. 8 distinct symbols) carries
/// far less entropy than its length implies; requiring at least 16 distinct
/// bytes rejects such degenerate low-entropy keys while leaving any real
/// CSPRNG-generated 32-byte secret (~30 distinct bytes in expectation)
/// comfortably above the floor.
///
/// The floor is applied to the *decoded* key material when the env value is
/// hex text: lowercase hex has at most 16 distinct characters total, so
/// counting distinct bytes of the text form would false-reject a substantial
/// fraction of valid `openssl rand -hex 32` secrets (any whose 64 digits don't
/// happen to use all 16 symbols). Base64 needs no such special-casing — its
/// 64-symbol alphabet keeps any high-entropy key's text form far above the
/// floor. For decoded forms shorter than 32 bytes the floor scales to half the
/// key length (see [`StateSecret::has_min_distinct`]).
pub(crate) const MIN_DISTINCT_SECRET_BYTES: usize = 16;

/// Substrings that mark an obvious placeholder / dev secret. A `state_secret`
/// containing any of these (case-insensitive) is rejected: it would otherwise
/// make every NEAR redirect `state` verify. Matched as substrings so
/// `changeme-please`, `my-test-secret`, etc. are all caught.
const PLACEHOLDER_SECRET_MARKERS: &[&str] = &[
    "changeme",
    "change-me",
    "placeholder",
    "example",
    "password",
    "secret123",
    "default",
    "dummy",
    "sample",
    "xxxx",
    "0000",
    "aaaa",
    "todo",
    "fixme",
];

/// Errors raised when present attested-provider config is invalid. Fail-closed:
/// an invalid present value is an error, never a silently-accepted weak config.
///
/// `Debug`/`Display` never include the secret material — only the field name
/// and the reason class.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AttestedConfigError {
    /// A required URL field was empty after trimming.
    #[error("attested config: {field} must not be empty")]
    EmptyUrl { field: &'static str },
    /// A multi-field ceremony was half-configured: some env vars were present
    /// while a required one (`field`) was absent. Distinct from [`Self::EmptyUrl`]
    /// (a *present* but empty value) so the operator sees "you didn't set this"
    /// rather than "your URL was blank" — the field may not even be a URL (e.g.
    /// the NEAR `state_secret`).
    #[error("attested config: {field} is required but was not set (partial configuration)")]
    MissingRequired { field: &'static str },
    /// A URL field was present but not a valid http(s) URL with a host.
    #[error("attested config: {field} is not a valid http(s) URL with a host")]
    InvalidUrl { field: &'static str },
    /// The NEAR `state_secret` was shorter than the entropy floor. The actual
    /// secret is never included.
    #[error(
        "attested config: NEAR state_secret is too short \
         (got {got} bytes, need >= {min})"
    )]
    StateSecretTooShort { got: usize, min: usize },
    /// The NEAR `state_secret` looked like a low-entropy placeholder / dev
    /// value (known marker, all-same-byte, or too few distinct bytes). The
    /// actual secret is never included.
    #[error("attested config: NEAR state_secret is low-entropy or a known placeholder")]
    StateSecretLowEntropy,
    /// The WalletConnect project id was empty after trimming.
    #[error("attested config: WalletConnect project id must not be empty")]
    EmptyProjectId,
    /// The WalletConnect project id was not a well-formed publishable id
    /// (WalletConnect Cloud ids decode to 16 bytes / 32 lowercase hex chars).
    #[error("attested config: WalletConnect project id is malformed (expected 32 hex chars)")]
    InvalidProjectId,
}

/// A validated http(s) URL. Constructed only via [`ValidatedUrl::parse`], so a
/// value of this type is always a syntactically-valid http(s) URL with a host.
#[derive(Clone, PartialEq, Eq)]
struct ValidatedUrl(String);

impl ValidatedUrl {
    fn parse(field: &'static str, raw: &str) -> Result<Self, AttestedConfigError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AttestedConfigError::EmptyUrl { field });
        }
        let parsed =
            url::Url::parse(trimmed).map_err(|_| AttestedConfigError::InvalidUrl { field })?;
        match parsed.scheme() {
            "http" | "https" => {}
            _ => return Err(AttestedConfigError::InvalidUrl { field }),
        }
        // A host must be present (rejects `http:///foo`, `https://` etc.).
        match parsed.host_str() {
            Some(host) if !host.is_empty() => {}
            _ => return Err(AttestedConfigError::InvalidUrl { field }),
        }
        Ok(Self(trimmed.to_string()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ValidatedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact the query and fragment: an RPC / wallet URL can carry an API
        // key or token in `?apikey=...` / `#...`, and `Debug` is reachable from
        // structured logs (the parent `NearRedirectConfig`/error `Debug`). Keep
        // scheme + authority + path so the value is still diagnosable.
        let rendered = match url::Url::parse(&self.0) {
            Ok(mut parsed) if parsed.query().is_some() || parsed.fragment().is_some() => {
                parsed.set_query(None);
                parsed.set_fragment(None);
                format!("{parsed}<redacted-query>")
            }
            _ => self.0.clone(),
        };
        f.debug_tuple("ValidatedUrl").field(&rendered).finish()
    }
}

/// A validated NEAR redirect `state` HMAC key. Private; constructed only via
/// [`StateSecret::new`], which enforces the length + entropy policy. Held in a
/// zeroizing, redacted-`Debug` [`SecretString`]; never logged or rendered.
#[derive(Clone)]
struct StateSecret(SecretString);

impl StateSecret {
    fn new(raw: &str) -> Result<Self, AttestedConfigError> {
        let trimmed = raw.trim();
        let bytes = trimmed.as_bytes();
        if bytes.len() < MIN_STATE_SECRET_BYTES {
            return Err(AttestedConfigError::StateSecretTooShort {
                got: bytes.len(),
                min: MIN_STATE_SECRET_BYTES,
            });
        }
        if Self::is_low_entropy(trimmed) {
            return Err(AttestedConfigError::StateSecretLowEntropy);
        }
        Ok(Self(SecretString::from(trimmed.to_string())))
    }

    /// Reject obvious placeholders and degenerate low-entropy keys: known
    /// marker substrings, single repeated byte, or key material with fewer
    /// than [`MIN_DISTINCT_SECRET_BYTES`] distinct bytes (e.g. "abababab…" or
    /// 8-symbol patterns).
    ///
    /// The distinct-byte floor is checked against the raw text *and*, when the
    /// value is hex, against the decoded key bytes — passing either form is
    /// sufficient. Hex text caps out at 16 distinct characters, so judging the
    /// text form alone would false-reject valid `openssl rand -hex 32`
    /// secrets; see [`MIN_DISTINCT_SECRET_BYTES`]. The decoded check can only
    /// ever *accept* values the raw check would reject, never the reverse, so
    /// non-hex secrets are unaffected.
    ///
    /// Note: an exact-match list of short words ("test", "key", "near", …) was
    /// dropped as dead code — every such word is shorter than
    /// [`MIN_STATE_SECRET_BYTES`] (32) and is therefore already rejected by the
    /// length floor in [`StateSecret::new`] before this function runs, so the
    /// exact list could never fire. The substring markers below plus the
    /// distinct-byte floor cover the real >=32-byte degenerate cases.
    fn is_low_entropy(value: &str) -> bool {
        let lower = value.to_ascii_lowercase();
        if PLACEHOLDER_SECRET_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
        {
            return true;
        }
        let bytes = value.as_bytes();
        // All identical bytes (e.g. "aaaa…", 32x same char).
        if bytes.windows(2).all(|w| w[0] == w[1]) {
            return true;
        }
        if Self::has_min_distinct(bytes) {
            return false;
        }
        // Very small alphabet across a long string indicates a trivial
        // pattern — unless the string is hex, whose alphabet is small by
        // construction. Judge the decoded key material instead.
        match Self::decode_hex_text(value) {
            Some(decoded) => !Self::has_min_distinct(&decoded),
            None => true,
        }
    }

    /// Whether `bytes` clears the distinct-byte entropy floor. The floor is
    /// [`MIN_DISTINCT_SECRET_BYTES`], scaled down to `len / 2` for inputs
    /// shorter than 32 bytes (a random 16-byte key has ~15.5 distinct bytes in
    /// expectation, so a flat floor of 16 would reject roughly half of them).
    fn has_min_distinct(bytes: &[u8]) -> bool {
        let floor = MIN_DISTINCT_SECRET_BYTES.min(bytes.len() / 2);
        let mut seen = [false; 256];
        let mut count = 0usize;
        for &b in bytes {
            if !seen[b as usize] {
                seen[b as usize] = true;
                count += 1;
                if count >= floor {
                    return true;
                }
            }
        }
        count >= floor
    }

    /// Decode `value` as hex text (optional `0x`/`0X` prefix, even length,
    /// ASCII hex digits only, case-insensitive). Returns `None` when the value
    /// is not hex — callers then judge the raw bytes as before.
    fn decode_hex_text(value: &str) -> Option<Vec<u8>> {
        let stripped = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        if stripped.is_empty()
            || !stripped.len().is_multiple_of(2)
            || !stripped.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        let nibble = |b: u8| -> u8 {
            match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                _ => b - b'A' + 10,
            }
        };
        Some(
            stripped
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
                .collect(),
        )
    }

    fn expose_bytes(&self) -> Vec<u8> {
        self.0.expose_secret().as_bytes().to_vec()
    }
}

/// Resolved NEAR-redirect ceremony config. Validated: every value of this type
/// has a valid http(s) wallet base URL, a valid http(s) callback URL, and a
/// `state_secret` that passes the length + entropy policy. Construct via
/// [`NearRedirectConfig::new`].
#[derive(Clone)]
pub struct NearRedirectConfig {
    wallet_base_url: ValidatedUrl,
    callback_url: ValidatedUrl,
    /// HMAC key binding the redirect `state` to the gate. Secret.
    state_secret: StateSecret,
}

impl NearRedirectConfig {
    /// Validate and construct. Trims and validates both URLs (scheme/host) and
    /// enforces the `state_secret` length + entropy policy. Returns an error
    /// (fail-closed) for any present-but-invalid field.
    pub fn new(
        wallet_base_url: impl AsRef<str>,
        callback_url: impl AsRef<str>,
        state_secret: impl AsRef<str>,
    ) -> Result<Self, AttestedConfigError> {
        let wallet_base_url =
            ValidatedUrl::parse("near_wallet_base_url", wallet_base_url.as_ref())?;
        let callback_url = ValidatedUrl::parse("near_callback_url", callback_url.as_ref())?;
        let state_secret = StateSecret::new(state_secret.as_ref())?;
        Ok(Self {
            wallet_base_url,
            callback_url,
            state_secret,
        })
    }

    /// The validated wallet base URL.
    pub fn wallet_base_url(&self) -> &str {
        self.wallet_base_url.as_str()
    }

    /// The validated callback URL.
    pub fn callback_url(&self) -> &str {
        self.callback_url.as_str()
    }
}

impl std::fmt::Debug for NearRedirectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the state_secret.
        f.debug_struct("NearRedirectConfig")
            .field("wallet_base_url", &self.wallet_base_url)
            .field("callback_url", &self.callback_url)
            .field("state_secret", &"<redacted>")
            .finish()
    }
}

/// A validated WalletConnect Cloud project id (publishable). Construct via
/// [`WalletConnectConfig::new`]; the inner [`ProjectId`] is guaranteed to decode
/// to a well-formed 16-byte id.
#[derive(Clone, Debug)]
pub struct WalletConnectConfig {
    project_id: ProjectId,
}

impl WalletConnectConfig {
    /// Validate and construct. Trims, rejects empty, and verifies the id is a
    /// well-formed publishable WalletConnect Cloud id (decodes to 16 bytes).
    pub fn new(project_id: impl AsRef<str>) -> Result<Self, AttestedConfigError> {
        let trimmed = project_id.as_ref().trim();
        if trimmed.is_empty() {
            return Err(AttestedConfigError::EmptyProjectId);
        }
        let id = ProjectId::from(trimmed);
        // WalletConnect Cloud project ids are 16-byte ids rendered as 32 hex
        // chars; `decode` enforces that shape.
        id.decode()
            .map_err(|_| AttestedConfigError::InvalidProjectId)?;
        Ok(Self { project_id: id })
    }
}

/// Configuration for the external-wallet providers that need ceremony config.
/// Each field is independently optional and independently fail-closed. Because
/// each field is a validated newtype, a `Some` value is always valid.
#[derive(Clone, Debug, Default)]
pub struct AttestedProvidersConfig {
    /// NEAR-redirect ceremony config. `None` -> NEAR provider unregistered.
    pub near_redirect: Option<NearRedirectConfig>,
    /// WalletConnect Cloud project id. `None` -> WalletConnect unregistered.
    pub walletconnect: Option<WalletConnectConfig>,
}

impl AttestedProvidersConfig {
    /// Resolve from the process environment, fail-closed.
    ///
    /// NEAR is configured only when **all** of base URL, callback URL, and the
    /// `state_secret` are present; WalletConnect is configured only when a
    /// project id is present. A present-but-invalid value (empty URL, weak
    /// secret, malformed project id) is a hard [`AttestedConfigError`] — never
    /// silently dropped — so misconfiguration fails closed at startup rather
    /// than weakening a verifier.
    ///
    /// Partial NEAR config (some of the three vars present, others absent) is
    /// also an error: a half-configured ceremony is a misconfiguration, not an
    /// "unconfigured provider".
    pub fn from_env() -> Result<Self, AttestedConfigError> {
        let near_redirect = Self::near_from_env()?;
        let walletconnect = match present_env(WALLETCONNECT_PROJECT_ID_ENV) {
            Some(raw) => Some(WalletConnectConfig::new(raw)?),
            None => None,
        };
        Ok(Self {
            near_redirect,
            walletconnect,
        })
    }

    fn near_from_env() -> Result<Option<NearRedirectConfig>, AttestedConfigError> {
        let wallet_base_url = present_env(NEAR_WALLET_BASE_URL_ENV);
        let callback_url = present_env(NEAR_CALLBACK_URL_ENV);
        let state_secret = present_env(NEAR_STATE_SECRET_ENV);
        match (wallet_base_url, callback_url, state_secret) {
            (None, None, None) => Ok(None),
            (Some(wallet_base_url), Some(callback_url), Some(state_secret)) => Ok(Some(
                NearRedirectConfig::new(wallet_base_url, callback_url, state_secret)?,
            )),
            // Partial config: treat as invalid present config (fail-closed). The
            // missing field was *absent*, not present-but-empty, so report it as
            // `MissingRequired` — `near_state_secret` is not a URL, and even the
            // URL fields read more clearly as "not set" than "must not be empty".
            (wallet_base_url, callback_url, _) => Err(AttestedConfigError::MissingRequired {
                field: if wallet_base_url.is_none() {
                    "near_wallet_base_url"
                } else if callback_url.is_none() {
                    "near_callback_url"
                } else {
                    "near_state_secret"
                },
            }),
        }
    }

    /// Build the [`ProviderRegistry`] for the attested driver.
    ///
    /// The injected provider is always registered over `grants` (the SAME
    /// sealed-grant store the custodial signer uses, so the one-shot grant CAS
    /// — threat #1 — is authoritative across every path). The NEAR-redirect and
    /// WalletConnect providers are registered only when their (validated) config
    /// is present (fail-closed otherwise). Because the config types are
    /// validated, this method cannot register a provider with placeholder /
    /// empty config.
    pub fn build_provider_registry(&self, grants: Arc<dyn SealedGrantStore>) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new()
            .with_provider(Arc::new(InjectedSigningProvider::new(Arc::clone(&grants))));

        if let Some(near) = &self.near_redirect {
            registry = registry.with_provider(Arc::new(NearRedirectSigningProvider::new(
                near.wallet_base_url.as_str().to_string(),
                near.callback_url.as_str().to_string(),
                near.state_secret.expose_bytes(),
                Arc::clone(&grants),
            )));
        }

        if let Some(wc) = &self.walletconnect {
            registry = registry.with_provider(Arc::new(WalletConnectSigningProvider::new(
                wc.project_id.clone(),
                Arc::clone(&grants),
            )));
        }

        registry
    }
}

/// Read an env var, treating absent / empty / whitespace-only as unset. The
/// returned string is the raw (un-trimmed) value so the validated constructors
/// can apply their own trimming + checks.
pub(crate) fn present_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strong_secret() -> String {
        // 36 chars, high distinct-byte count, no placeholder markers.
        "f3K9pLm2QzR7vWx1Yb4Nc8Hd6Ts0Ug5Ej2Aq".to_string()
    }

    #[test]
    fn near_config_accepts_valid_input() {
        let cfg = NearRedirectConfig::new(
            "https://wallet.testnet.near.org/sign",
            "https://app.example/near/callback",
            strong_secret(),
        )
        .expect("valid near config");
        assert_eq!(
            cfg.wallet_base_url(),
            "https://wallet.testnet.near.org/sign"
        );
    }

    #[test]
    fn near_config_rejects_empty_url() {
        let err = NearRedirectConfig::new("  ", "https://app.example/cb", strong_secret())
            .expect_err("empty url rejected");
        assert_eq!(
            err,
            AttestedConfigError::EmptyUrl {
                field: "near_wallet_base_url"
            }
        );
    }

    #[test]
    fn near_config_rejects_non_http_scheme() {
        let err = NearRedirectConfig::new(
            "ftp://wallet.near.org/sign",
            "https://app.example/cb",
            strong_secret(),
        )
        .expect_err("non-http scheme rejected");
        assert_eq!(
            err,
            AttestedConfigError::InvalidUrl {
                field: "near_wallet_base_url"
            }
        );
    }

    #[test]
    fn near_config_rejects_url_without_host() {
        let err = NearRedirectConfig::new("https://", "https://app.example/cb", strong_secret())
            .expect_err("hostless url rejected");
        assert_eq!(
            err,
            AttestedConfigError::InvalidUrl {
                field: "near_wallet_base_url"
            }
        );
    }

    #[test]
    fn state_secret_rejects_short() {
        let err = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            "tooshort",
        )
        .expect_err("short secret rejected");
        assert!(matches!(
            err,
            AttestedConfigError::StateSecretTooShort { .. }
        ));
    }

    #[test]
    fn state_secret_rejects_placeholder_changeme() {
        // 32+ bytes but contains a placeholder marker.
        let err = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            "changeme-changeme-changeme-changeme",
        )
        .expect_err("placeholder secret rejected");
        assert_eq!(err, AttestedConfigError::StateSecretLowEntropy);
    }

    #[test]
    fn state_secret_rejects_all_same_byte() {
        let err = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            "a".repeat(40),
        )
        .expect_err("all-same-byte secret rejected");
        assert_eq!(err, AttestedConfigError::StateSecretLowEntropy);
    }

    #[test]
    fn state_secret_rejects_exact_placeholder_word() {
        // Exactly "test" is too short anyway; use a padded low-distinct one.
        let err = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            "abababababababababababababababababab",
        )
        .expect_err("low-distinct secret rejected");
        assert_eq!(err, AttestedConfigError::StateSecretLowEntropy);
    }

    /// A deterministic stand-in for `openssl rand -hex 32` output: 32 bytes
    /// with 32 distinct values whose hex rendering happens to never use the
    /// symbol `f` — i.e. only 15 distinct *characters*. Counting distinct
    /// bytes of the text form would reject it; the decoded key material is
    /// maximally distinct.
    fn csprng_like_hex_missing_f() -> String {
        const KEY: [u8; 32] = [
            0x01, 0x12, 0x23, 0x34, 0x45, 0x56, 0x67, 0x78, 0x89, 0x9a, 0xab, 0xbc, 0xcd, 0xde,
            0xe0, 0x02, 0x13, 0x24, 0x35, 0x46, 0x57, 0x68, 0x79, 0x8a, 0x9b, 0xac, 0xbd, 0xce,
            0xd0, 0xe1, 0x03, 0x14,
        ];
        let hex: String = KEY.iter().map(|b| format!("{b:02x}")).collect();
        // Premise guard: the regression only reproduces when the text form has
        // fewer than MIN_DISTINCT_SECRET_BYTES distinct characters.
        assert!(!hex.contains('f'));
        assert_eq!(hex.len(), 64);
        hex
    }

    #[test]
    fn state_secret_accepts_csprng_hex() {
        NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            csprng_like_hex_missing_f(),
        )
        .expect("valid CSPRNG hex secret accepted");
    }

    #[test]
    fn state_secret_accepts_csprng_hex_uppercase_and_prefixed() {
        NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            format!("0x{}", csprng_like_hex_missing_f().to_ascii_uppercase()),
        )
        .expect("0x-prefixed uppercase hex secret accepted");
    }

    #[test]
    fn state_secret_rejects_degenerate_hex() {
        // Valid hex, but the decoded key is a single repeated byte.
        let err = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            "1b".repeat(32),
        )
        .expect_err("repeated-byte hex secret rejected");
        assert_eq!(err, AttestedConfigError::StateSecretLowEntropy);
    }

    #[test]
    fn walletconnect_accepts_valid_id() {
        WalletConnectConfig::new("00000000000000000000000000000000")
            .expect("valid 32-hex project id");
    }

    #[test]
    fn walletconnect_rejects_empty() {
        let err = WalletConnectConfig::new("   ").expect_err("empty id rejected");
        assert_eq!(err, AttestedConfigError::EmptyProjectId);
    }

    #[test]
    fn walletconnect_rejects_malformed() {
        let err = WalletConnectConfig::new("not-a-valid-project-id").expect_err("malformed");
        assert_eq!(err, AttestedConfigError::InvalidProjectId);
    }

    #[test]
    fn validated_url_debug_redacts_query_and_fragment() {
        // A wallet/callback URL can carry an API key in the query string; the
        // `Debug` impl (reachable from structured logs via NearRedirectConfig)
        // must not render it.
        let cfg = NearRedirectConfig::new(
            "https://wallet.near.org/sign?apikey=supersecrettoken",
            "https://app.example/cb#frag-secret",
            strong_secret(),
        )
        .expect("valid");
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("supersecrettoken"),
            "query secret leaked into Debug: {rendered}"
        );
        assert!(
            !rendered.contains("frag-secret"),
            "fragment leaked into Debug: {rendered}"
        );
        // Scheme + host + path are retained for diagnosability.
        assert!(rendered.contains("wallet.near.org"));
        assert!(rendered.contains("redacted-query"));
    }

    #[test]
    fn state_secret_rejects_low_distinct_byte_count() {
        // 32+ bytes drawn from a 10-symbol alphabet: above the old (<8) floor but
        // below the hardened (<16) distinct-byte floor.
        let low_distinct = "0123456789".repeat(4); // 40 bytes, 10 distinct
        let err = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            low_distinct,
        )
        .expect_err("low-distinct secret rejected");
        assert_eq!(err, AttestedConfigError::StateSecretLowEntropy);
    }

    #[test]
    fn near_config_debug_never_renders_secret() {
        let cfg = NearRedirectConfig::new(
            "https://wallet.near.org/sign",
            "https://app.example/cb",
            strong_secret(),
        )
        .expect("valid");
        let rendered = format!("{cfg:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains(&strong_secret()));
    }
}
