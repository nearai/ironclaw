//! WebChat v2 SSO startup config for `ironclaw-reborn serve`.
//!
//! Compiled under the `webui-v2-beta` feature (the same feature that
//! compiles the `serve` command). This module owns the host config side
//! of SSO login startup policy: reading the operator's env vars into a
//! provider list, resolving the public base URL, refusing cleartext
//! OAuth on a public interface, and requiring a fail-closed
//! verified-email-domain admission allowlist. The auth/session model
//! itself — the signed-token session store, the composite authenticator,
//! and the route wiring — lives in
//! `ironclaw_reborn_webui_ingress` (see
//! [`ironclaw_reborn_webui_ingress::build_signed_session_login`]), which
//! is where this crate's guardrails place `WebuiAuthenticator` /
//! `SessionStore` implementations. `serve.rs` calls
//! [`sso_startup_config_from_env`] and, when it returns `Some`, hands
//! the result plus the operator identity/secret to that builder.

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use ironclaw_reborn_webui_ingress::{
    GitHubOAuthConfig, GitHubProvider, GoogleOAuthConfig, GoogleProvider, OAuthProvider,
};
use secrecy::SecretString;

const WEBUI_BASE_URL_ENV: &str = "IRONCLAW_REBORN_WEBUI_BASE_URL";

#[cfg(test)]
pub(crate) static WEBUI_BASE_URL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Resolved SSO startup config: the providers to mount plus the public
/// base URL their callback URLs are built from. Constructed by
/// [`sso_startup_config_from_env`]; consumed by `serve.rs`, which adds
/// the operator identity/secret and calls the ingress builder.
pub(crate) struct SsoStartupConfig {
    pub(crate) providers: Vec<Arc<dyn OAuthProvider>>,
    pub(crate) base_url: String,
    /// Lowercased verified-email domains allowed to log in. The host
    /// [`WebuiUserDirectory`](crate::commands::user_directory) admits a
    /// login only when the profile's verified email is in this set;
    /// startup fails when providers are configured but this is empty, so
    /// it is never empty here in practice.
    pub(crate) allowed_email_domains: Vec<String>,
}

/// Resolve the SSO startup config from environment, applying all
/// startup-time SSO policy in one place: provider discovery, base-URL
/// resolution, and the cleartext-OAuth guard.
///
/// Returns `Ok(None)` when no provider is configured (the listener then
/// keeps its plain env-bearer auth and mounts no public login routes),
/// `Err` on misconfiguration (a provider opted in without its secret, or
/// an http:// base URL on a non-loopback bind).
pub(crate) fn sso_startup_config_from_env(
    listen_addr: SocketAddr,
) -> anyhow::Result<Option<SsoStartupConfig>> {
    let providers = oauth_providers_from_env()?;
    if providers.is_empty() {
        return Ok(None);
    }

    let base_url = webui_oauth_base_url_from_env(listen_addr)?;
    reject_cleartext_oauth(&base_url, listen_addr)?;

    let allowed_email_domains = parse_allowed_email_domains(
        non_empty_env("IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS")
            .as_deref()
            .unwrap_or_default(),
    );
    require_admission_allowlist(&allowed_email_domains)?;

    Ok(Some(SsoStartupConfig {
        providers,
        base_url,
        allowed_email_domains,
    }))
}

/// Resolve the externally visible WebUI base URL used for OAuth redirects.
///
/// Precedence stays Reborn-scoped: explicit Reborn env first, then the bound
/// listener address for local development.
pub(crate) fn webui_oauth_base_url_from_env(listen_addr: SocketAddr) -> anyhow::Result<String> {
    Ok(webui_public_base_url_from_env()?.unwrap_or_else(|| format!("http://{listen_addr}")))
}

/// Resolve a hosted WebUI base URL from deployment env without falling back to
/// the listener address. Product-auth uses this to avoid turning `0.0.0.0`
/// hosted deployments into provider-visible `localhost` callbacks.
pub(crate) fn webui_public_base_url_from_env() -> anyhow::Result<Option<String>> {
    if let Some(raw) = non_empty_env(WEBUI_BASE_URL_ENV) {
        return Ok(Some(normalize_base_url(WEBUI_BASE_URL_ENV, raw)?));
    }
    Ok(None)
}

/// Validate the hosted WebUI OAuth base URL against the current listen address.
///
/// This keeps the cleartext-OAuth policy local to the reborn CLI command
/// module while letting `serve.rs` fail startup instead of silently skipping
/// product-auth wiring when an explicit public `http://` base URL is bound to a
/// non-loopback interface.
pub(crate) fn validate_webui_public_base_url(
    public_base_url: Option<&str>,
    listen_addr: SocketAddr,
) -> anyhow::Result<()> {
    if let Some(base_url) = public_base_url {
        reject_cleartext_oauth(base_url, listen_addr)?;
    }
    Ok(())
}

fn normalize_base_url(env_var: &'static str, raw: impl AsRef<str>) -> anyhow::Result<String> {
    let normalized = raw.as_ref().trim().trim_end_matches('/');
    if normalized.is_empty() {
        anyhow::bail!("{env_var} must not be empty after trimming whitespace and trailing slashes")
    } else {
        Ok(normalized.to_string())
    }
}

/// Parse the comma-separated verified-email-domain allowlist, trimmed,
/// lowercased, and de-blanked.
fn parse_allowed_email_domains(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|domain| domain.trim().to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect()
}

/// Fail closed when SSO providers are configured without an admission
/// allowlist. GitHub has no org/team allowlist and Google only an
/// optional hosted-domain check, so a configured provider with no
/// verified-email-domain allowlist would let *any* Google/GitHub account
/// mint a session on a protected WebUI — open registration. Refuse to
/// start rather than silently expose that.
fn require_admission_allowlist(allowed_email_domains: &[String]) -> anyhow::Result<()> {
    if allowed_email_domains.is_empty() {
        anyhow::bail!(
            "WebChat v2 SSO providers are configured but \
             IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS is empty. Without an \
             allowlist, any Google/GitHub account that completes login would be \
             admitted (open registration). Set it to a comma-separated list of \
             allowed verified-email domains (e.g. example.com,team.example.com)."
        );
    }
    Ok(())
}

/// Refuse cleartext OAuth on a public interface: `http://` redirect URIs
/// leak authorization codes in transit (and Google/GitHub reject them for
/// production apps). Loopback `http://` stays allowed for local dev.
fn reject_cleartext_oauth(base_url: &str, listen_addr: SocketAddr) -> anyhow::Result<()> {
    if is_cleartext_http_scheme(base_url) && !listen_addr.ip().is_loopback() {
        anyhow::bail!(
            "hosted WebUI OAuth base URL `{base_url}` uses http:// on a non-loopback interface, \
             which would transmit OAuth authorization codes in cleartext. Set \
             IRONCLAW_REBORN_WEBUI_BASE_URL to an https:// URL."
        );
    }
    Ok(())
}

/// Whether `base_url` uses the cleartext `http` scheme. URL schemes are
/// case-insensitive (RFC 3986 §3.1), so `HTTP://` and `Http://` are
/// cleartext too. A literal `starts_with("http://")` check would let
/// `IRONCLAW_REBORN_WEBUI_BASE_URL=HTTP://example.com` slip past the
/// non-loopback guard while still building a cleartext callback URL —
/// comparing the scheme case-insensitively closes that bypass.
pub(crate) fn is_cleartext_http_scheme(base_url: &str) -> bool {
    base_url
        .split_once("://")
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("http"))
}

/// Collect the configured OAuth providers from env. A provider is
/// opted in by setting its `IRONCLAW_REBORN_WEBUI_*_CLIENT_ID`. The
/// matching `*_CLIENT_SECRET` is then REQUIRED: opting a provider in
/// without its secret is a misconfiguration, not an optional feature —
/// registering it would surface a login button whose code exchange
/// always fails at the provider — so it fails startup loudly rather than
/// silently skipping with a buried log line.
///
/// These WebChat-login vars live in the `IRONCLAW_REBORN_WEBUI_*`
/// namespace — the same one as `IRONCLAW_REBORN_WEBUI_TOKEN` /
/// `IRONCLAW_REBORN_WEBUI_USER_ID` — deliberately separate from the
/// bare `GOOGLE_CLIENT_ID` / `IRONCLAW_REBORN_GOOGLE_*` vars that the
/// product-auth credential-connection flow reads. Sharing the bare
/// names would couple two unrelated surfaces: setting `GOOGLE_CLIENT_ID`
/// just to enable login would also activate the product-auth Google
/// resolver, which hard-errors when its required redirect URI is absent
/// and would take down every `ironclaw-reborn` command. The distinct
/// namespace keeps SSO login and product-auth independently
/// configurable. (The browser-user *login* client and the product
/// *credential* client are usually distinct OAuth clients anyway —
/// different registered redirect URIs.)
fn oauth_providers_from_env() -> anyhow::Result<Vec<Arc<dyn OAuthProvider>>> {
    let mut providers: Vec<Arc<dyn OAuthProvider>> = Vec::new();
    // Optional operator override for the provider HTTP timeout, applied to
    // every configured provider. Useful on a slow / cross-border path to
    // the provider (e.g. `github.com`) where the default times out.
    let http_timeout = oauth_http_timeout_from_env();

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID") {
        let client_secret = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET")
            .ok_or_else(|| {
                anyhow!(
                    "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID is set but \
                     IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET is missing"
                )
            })?;
        let allowed_hd = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD");
        // `allowed_hd` is an additional Google-side restriction (the ID
        // token's `hd` claim must match a Workspace domain). It is
        // independent of — and narrower than — the cross-provider
        // verified-email-domain allowlist the host `UserDirectory`
        // enforces. When unset, Google login is still gated by
        // `IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS` (admission cannot
        // be open), but consumer Gmail accounts in an allowed email domain
        // are not additionally excluded; warn so operators can opt into
        // the stricter Workspace-only check.
        if allowed_hd.is_none() {
            tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                "IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD is unset — Google login is gated \
                 only by IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS, not by a Workspace \
                 hosted domain; set it to also require a specific hd claim",
            );
        }
        let provider = GoogleProvider::new(GoogleOAuthConfig {
            client_id,
            client_secret: SecretString::from(client_secret),
            allowed_hd,
            http_timeout,
        })
        .context("failed to build Google OAuth provider")?;
        providers.push(Arc::new(provider));
    }

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID") {
        let client_secret = non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET")
            .ok_or_else(|| {
                anyhow!(
                    "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID is set but \
                     IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET is missing"
                )
            })?;
        let provider = GitHubProvider::new(GitHubOAuthConfig {
            client_id,
            client_secret: SecretString::from(client_secret),
            http_timeout,
        })
        .context("failed to build GitHub OAuth provider")?;
        providers.push(Arc::new(provider));
    }

    Ok(providers)
}

/// Read an env var, returning `None` when it is unset or blank.
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|raw| !raw.trim().is_empty())
}

/// Operator override for the OAuth provider HTTP per-call timeout, in
/// whole seconds. `None` (unset) keeps each provider's default. A
/// non-positive or unparseable value is ignored with a warning rather
/// than failing startup — a timeout typo should not take down `serve`.
fn oauth_http_timeout_from_env() -> Option<Duration> {
    let raw = non_empty_env("IRONCLAW_REBORN_WEBUI_OAUTH_HTTP_TIMEOUT_SECS")?;
    match raw.trim().parse::<u64>() {
        Ok(secs) if secs > 0 => Some(Duration::from_secs(secs)),
        _ => {
            tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                value = %raw,
                "IRONCLAW_REBORN_WEBUI_OAUTH_HTTP_TIMEOUT_SECS is not a positive integer; \
                 using the provider default timeout",
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(raw: &str) -> SocketAddr {
        raw.parse().expect("valid socket addr")
    }

    #[test]
    fn cleartext_scheme_is_detected_case_insensitively() {
        for raw in [
            "http://example.com",
            "HTTP://example.com",
            "Http://example.com",
        ] {
            assert!(is_cleartext_http_scheme(raw), "{raw} should be cleartext");
        }
        for raw in [
            "https://example.com",
            "HTTPS://example.com",
            "Https://example.com",
        ] {
            assert!(
                !is_cleartext_http_scheme(raw),
                "{raw} should not be cleartext"
            );
        }
    }

    #[test]
    fn cleartext_oauth_rejected_on_non_loopback_for_any_scheme_case() {
        let public = addr("203.0.113.1:3000");
        for raw in [
            "http://example.com",
            "HTTP://example.com",
            "Http://example.com",
        ] {
            assert!(
                reject_cleartext_oauth(raw, public).is_err(),
                "{raw} on a non-loopback bind must be rejected",
            );
        }
        assert!(reject_cleartext_oauth("https://example.com", public).is_ok());
        assert!(reject_cleartext_oauth("HTTPS://example.com", public).is_ok());
    }

    #[test]
    fn cleartext_oauth_allowed_on_loopback_regardless_of_scheme_case() {
        let loopback = addr("127.0.0.1:3000");
        for raw in ["http://localhost:3000", "HTTP://localhost:3000"] {
            assert!(
                reject_cleartext_oauth(raw, loopback).is_ok(),
                "{raw} on loopback stays allowed for local dev",
            );
        }
    }

    #[test]
    fn hosted_webui_public_base_url_validation_fails_closed_for_public_cleartext() {
        let public = addr("203.0.113.1:3000");
        let error = validate_webui_public_base_url(Some("http://example.com"), public)
            .expect_err("public cleartext base URL must abort startup");
        let message = error.to_string();
        assert!(
            message.contains(WEBUI_BASE_URL_ENV),
            "message should name {WEBUI_BASE_URL_ENV}, got: {message}"
        );
        assert!(
            message.contains("hosted WebUI OAuth base URL"),
            "message should mention hosted WebUI OAuth base URL, got: {message}"
        );
    }

    #[test]
    fn hosted_webui_public_base_url_validation_allows_loopback_cleartext() {
        let loopback = addr("127.0.0.1:3000");
        assert!(validate_webui_public_base_url(Some("http://127.0.0.1:3000"), loopback).is_ok());
        assert!(validate_webui_public_base_url(None, loopback).is_ok());
    }

    #[test]
    fn allowed_email_domains_are_trimmed_lowercased_and_deblanked() {
        assert_eq!(
            parse_allowed_email_domains(" Example.com , ,team.EXAMPLE.org ,"),
            vec!["example.com".to_string(), "team.example.org".to_string()],
        );
        assert!(parse_allowed_email_domains("").is_empty());
        assert!(parse_allowed_email_domains("  , ,").is_empty());
    }

    #[test]
    fn admission_allowlist_required_when_providers_configured() {
        // The fail-closed startup gate: an empty allowlist (which would be
        // open registration) must error; a non-empty one must pass.
        assert!(require_admission_allowlist(&[]).is_err());
        assert!(require_admission_allowlist(&["example.com".to_string()]).is_ok());
    }

    const SSO_ENV_VARS: &[&str] = &[
        "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID",
        "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET",
        "IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD",
        "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID",
        "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET",
        WEBUI_BASE_URL_ENV,
        "IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS",
    ];

    fn clear_sso_env() {
        for var in SSO_ENV_VARS {
            // SAFETY: tests are serialized by `WEBUI_BASE_URL_ENV_LOCK`; no other thread
            // reads or writes these vars while the guard is held.
            unsafe { std::env::remove_var(var) };
        }
    }

    /// Caller-level coverage: with a provider configured but the
    /// verified-email-domain allowlist absent, `sso_startup_config_from_env`
    /// (the caller that wires provider discovery and admission together)
    /// must fail closed — error out and produce NO mounted SSO config —
    /// rather than admitting every account. The positive case confirms the
    /// same caller succeeds once the allowlist is supplied.
    #[test]
    fn startup_fails_closed_when_providers_set_but_allowlist_missing() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");
        clear_sso_env();
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID", "client-id");
            std::env::set_var(
                "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET",
                "client-secret",
            );
        }

        // Allowlist unset → fail closed (no config mounted).
        let missing = sso_startup_config_from_env(addr("127.0.0.1:3000"));
        assert!(
            missing.is_err(),
            "a configured provider with no allowlist must abort startup, not mount open registration",
        );

        // Blank allowlist → same fail-closed result.
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK.
        unsafe {
            std::env::set_var("IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS", "  , ,");
        }
        assert!(
            sso_startup_config_from_env(addr("127.0.0.1:3000")).is_err(),
            "a blank allowlist is equivalent to none and must also fail closed",
        );

        // Supplying the allowlist makes the same caller succeed.
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK.
        unsafe {
            std::env::set_var("IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS", "example.com");
        }
        let ok = sso_startup_config_from_env(addr("127.0.0.1:3000"))
            .expect("a configured provider WITH an allowlist must start")
            .expect("providers configured → Some(config)");
        assert_eq!(ok.allowed_email_domains, vec!["example.com".to_string()]);
        assert!(!ok.providers.is_empty());

        clear_sso_env();
    }

    /// Caller-level coverage of the missing-secret fail-closed arms (Google
    /// and GitHub) inside `oauth_providers_from_env`: opting a provider in
    /// by its CLIENT_ID without the matching CLIENT_SECRET must abort
    /// startup, not silently skip the provider (which would surface a login
    /// button whose code exchange always fails). Exercised through the
    /// caller `sso_startup_config_from_env`, since the helper is private.
    #[test]
    fn client_id_without_secret_fails_startup() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");

        for id_var in [
            "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID",
            "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID",
        ] {
            clear_sso_env();
            // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleared before/after.
            unsafe { std::env::set_var(id_var, "client-id") };
            assert!(
                sso_startup_config_from_env(addr("127.0.0.1:3000")).is_err(),
                "{id_var} set without its CLIENT_SECRET must abort startup",
            );
        }
        clear_sso_env();
    }

    /// With no provider CLIENT_ID configured, the caller returns
    /// `Ok(None)` — the listener keeps its plain env-bearer auth and mounts
    /// no public login routes. (No allowlist is required in this branch,
    /// since the allowlist gate only fires once a provider is configured.)
    #[test]
    fn no_provider_configured_returns_none() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");
        clear_sso_env();
        let resolved = sso_startup_config_from_env(addr("127.0.0.1:3000"))
            .expect("no provider configured is not an error");
        assert!(resolved.is_none(), "no CLIENT_ID → no SSO config mounted",);
    }

    #[test]
    fn webui_oauth_base_url_prefers_explicit_reborn_env() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");
        clear_sso_env();
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, " https://configured.example/ ");
        }

        assert_eq!(
            webui_oauth_base_url_from_env(addr("0.0.0.0:8080")).expect("base url"),
            "https://configured.example"
        );

        clear_sso_env();
    }

    #[test]
    fn webui_public_base_url_from_env_rejects_blank_normalized_values() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");
        clear_sso_env();
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, "/");
        }
        let error = webui_public_base_url_from_env()
            .expect_err("slash-only explicit base URL must fail closed");
        assert!(
            error.to_string().contains(WEBUI_BASE_URL_ENV),
            "error should name env var, got: {error}"
        );

        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, " / ");
        }
        assert!(
            webui_public_base_url_from_env().is_err(),
            "normalized-empty explicit base URL must fail closed"
        );

        clear_sso_env();
    }

    #[test]
    fn sso_startup_config_uses_explicit_webui_base_url() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");
        clear_sso_env();
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID", "client-id");
            std::env::set_var(
                "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET",
                "client-secret",
            );
            std::env::set_var("IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS", "example.com");
            std::env::set_var(WEBUI_BASE_URL_ENV, " https://configured.example/ ");
        }

        let resolved = sso_startup_config_from_env(addr("0.0.0.0:8080"))
            .expect("configured provider and allowlist should start")
            .expect("providers configured → Some(config)");

        assert_eq!(
            resolved.base_url, "https://configured.example",
            "explicit WebUI base URL must become the SSO callback base URL"
        );

        clear_sso_env();
    }

    #[test]
    fn webui_oauth_base_url_falls_back_to_listener() {
        let _guard = WEBUI_BASE_URL_ENV_LOCK.lock().expect("env lock");
        clear_sso_env();

        assert_eq!(
            webui_oauth_base_url_from_env(addr("127.0.0.1:3000")).expect("base url"),
            "http://127.0.0.1:3000"
        );
    }
}
