//! Boot-time configuration for the standalone `ironclaw-reborn` binary.
//!
//! All inputs come from environment variables — no shared `Config` type with
//! the v1 agent. Operators run this binary against its own DB (or, in dev,
//! point it at the same one as v1).

use std::env;
use std::net::SocketAddr;

use secrecy::SecretString;

use crate::error::HostError;

/// Selects which storage backend the host wires up. Only one is active at a
/// time. `Postgres` takes precedence when both are configured.
#[derive(Debug, Clone)]
pub enum StorageBackend {
    #[cfg(feature = "libsql")]
    LibSql { path: String },
    #[cfg(feature = "postgres")]
    Postgres { url: String },
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Address to bind the axum webhook server on.
    pub listen_addr: SocketAddr,
    /// Storage backend wiring.
    pub storage: StorageBackend,
    /// Reborn installation id (one process = one install for the tracer).
    pub installation_id: String,
    /// Telegram bot token. Wrapped in `SecretString` so it zeroizes on drop
    /// and accidental `Debug` / `Display` prints reveal `[REDACTED]` rather
    /// than the literal token. The token still ends up cloned into
    /// `StaticCredentialResolver` (which holds a plain `String`) for the
    /// lifetime of the runner — fully eliminating that residual exposure
    /// requires re-reading through `EgressCredentialResolver` on each
    /// resolve, which zmanian's review on PR #3590 (item #3) flags as a
    /// major-tier follow-up before non-default-off rollout.
    pub telegram_bot_token: SecretString,
    /// Telegram webhook shared secret (sent in `X-Telegram-Bot-Api-Secret-Token`).
    pub telegram_webhook_secret: SecretString,
    /// Optional tenant id override (defaults to `tenant_default`).
    pub tenant_id: String,
    /// Optional agent id override (defaults to `agent_default`).
    pub agent_id: String,
    /// Trusted Telegram-user → canonical-Reborn-user pairings to install at
    /// boot. The shared `ProductConversationBindingService` fails closed on
    /// unpaired actors (the security invariant introduced by PR #3727); this
    /// list bootstraps the pairings the host owner trusts. Each entry is
    /// `<telegram_user_id>:<reborn_user_id>`. Read from
    /// `REBORN_TELEGRAM_PAIRINGS` (comma-separated). Empty in production
    /// deployments that pair out-of-band; required to send any message in
    /// dev/test.
    pub pairings: Vec<TelegramPairing>,
}

/// One trusted Telegram-user → Reborn-user pairing read from
/// `REBORN_TELEGRAM_PAIRINGS`. Stored as raw strings here; `boot.rs`
/// validates them as `UserId` / `ExternalActorRef` at startup and fails
/// closed on any malformed pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramPairing {
    /// Telegram user id as it appears in `from.id` on inbound updates.
    pub external_user_id: String,
    /// Canonical Reborn `UserId` this Telegram user maps to.
    pub user_id: String,
}

/// Callback the config layer uses to read env-like values. The default
/// production wiring resolves through `std::env::var`; tests inject a
/// fake closure so they exercise the same branches without touching the
/// process-global env (which is unsafe under multi-threaded test
/// scheduling).
pub(crate) type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

impl HostConfig {
    pub fn from_env() -> Result<Self, HostError> {
        Self::from_env_with(&|name| env::var(name).ok())
    }

    pub(crate) fn from_env_with(env_lookup: EnvLookup) -> Result<Self, HostError> {
        let listen_addr = env_lookup("IRONCLAW_REBORN_LISTEN_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8090".to_string())
            .parse()
            .map_err(|e| HostError::Config(format!("invalid IRONCLAW_REBORN_LISTEN_ADDR: {e}")))?;

        let storage = resolve_storage(env_lookup)?;

        // Three namespace inputs whose defaults collide if any two host
        // processes share a DB: `installation_id` keys binding uniqueness
        // and feeds `derive_user_id`, and `tenant_id`/`agent_id` scope every
        // downstream Reborn operation. Two bots started with these
        // unset against the same DB collapse into one canonical user
        // namespace — different chats, same canonical `UserId`. Fail
        // closed unless every field is explicitly configured, with an
        // explicit dev/test opt-in env var to bypass.
        // See @serrrfirat's review on PR #3590, finding #3.
        let installation_id_var = env_lookup("REBORN_TELEGRAM_V2_INSTALLATION_ID");
        let tenant_id_var = env_lookup("REBORN_TENANT_ID");
        let agent_id_var = env_lookup("REBORN_AGENT_ID");
        let allow_default_namespace = env_flag_set(env_lookup, ALLOW_DEFAULT_NAMESPACE_ENV);
        let missing_namespace_vars: Vec<&str> = [
            ("REBORN_TELEGRAM_V2_INSTALLATION_ID", &installation_id_var),
            ("REBORN_TENANT_ID", &tenant_id_var),
            ("REBORN_AGENT_ID", &agent_id_var),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.is_none().then_some(name))
        .collect();
        if !missing_namespace_vars.is_empty() && !allow_default_namespace {
            return Err(HostError::Config(format!(
                "namespace fail-closed: {missing} must be set explicitly. \
                 Defaults (`default` / `tenant_default` / `agent_default`) \
                 would collide if another host process shares this DB, \
                 collapsing distinct bots into one canonical user namespace. \
                 For dev/test, opt in with {ALLOW_DEFAULT_NAMESPACE_ENV}=1.",
                missing = missing_namespace_vars.join(", "),
            )));
        }
        if allow_default_namespace && !missing_namespace_vars.is_empty() {
            tracing::warn!(
                missing = missing_namespace_vars.join(", "),
                "Reborn host: using literal default namespace values because \
                 {ALLOW_DEFAULT_NAMESPACE_ENV} is set. Distinct bots sharing \
                 this DB will collapse into one canonical user namespace; \
                 not safe for production."
            );
        }
        let installation_id = installation_id_var.unwrap_or_else(|| "default".to_string());
        let tenant_id = tenant_id_var.unwrap_or_else(|| "tenant_default".into());
        let agent_id = agent_id_var.unwrap_or_else(|| "agent_default".into());

        let telegram_bot_token = env_lookup("TELEGRAM_BOT_TOKEN")
            .ok_or_else(|| HostError::Config("TELEGRAM_BOT_TOKEN must be set".into()))?
            .into();
        let telegram_webhook_secret = env_lookup("TELEGRAM_WEBHOOK_SECRET")
            .ok_or_else(|| HostError::Config("TELEGRAM_WEBHOOK_SECRET must be set".into()))?
            .into();

        let pairings = parse_pairings(env_lookup("REBORN_TELEGRAM_PAIRINGS").as_deref())?;

        Ok(Self {
            listen_addr,
            storage,
            installation_id,
            telegram_bot_token,
            telegram_webhook_secret,
            tenant_id,
            agent_id,
            pairings,
        })
    }
}

/// Parse `REBORN_TELEGRAM_PAIRINGS` — a comma-separated list of
/// `<telegram_user_id>:<reborn_user_id>` entries.
///
/// Trims whitespace around the list, the comma-separated entries, and either
/// side of the colon. Skips empty entries (so trailing commas are tolerated).
/// Fails closed on any entry that has more or fewer than one colon, an empty
/// external id, or an empty Reborn user id — those would silently corrupt the
/// pairing table downstream.
fn parse_pairings(raw: Option<&str>) -> Result<Vec<TelegramPairing>, HostError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (idx, entry) in raw.split(',').enumerate() {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.splitn(2, ':');
        let external_user_id = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HostError::Config(format!(
                    "REBORN_TELEGRAM_PAIRINGS entry {idx} is empty before the colon: {entry:?}"
                ))
            })?;
        let user_id = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HostError::Config(format!(
                    "REBORN_TELEGRAM_PAIRINGS entry {idx} is missing `<telegram_user_id>:<reborn_user_id>` form: {entry:?}"
                ))
            })?;
        out.push(TelegramPairing {
            external_user_id: external_user_id.to_string(),
            user_id: user_id.to_string(),
        });
    }
    Ok(out)
}

/// Opt-in env var that bypasses the default-namespace fail-closed check.
/// Intended for dev/test only; documented and warned on at startup.
const ALLOW_DEFAULT_NAMESPACE_ENV: &str = "IRONCLAW_REBORN_ALLOW_DEFAULT_NAMESPACE";

fn env_flag_set(env_lookup: EnvLookup, name: &str) -> bool {
    env_lookup(name).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Env-var name an operator can set to *explicitly* opt into ephemeral
/// in-memory storage. Required for tests and dev loops; absent in any
/// production deployment that should survive a restart. Renamed from the
/// previous silent `:memory:` fallback because that fallback made it
/// impossible to tell a misconfigured production process apart from an
/// intentional dev session (Henry's PR #3590 review item #3).
const ALLOW_EPHEMERAL_ENV: &str = "IRONCLAW_REBORN_ALLOW_EPHEMERAL";

#[allow(unreachable_code)]
fn resolve_storage(env_lookup: EnvLookup) -> Result<StorageBackend, HostError> {
    #[cfg(feature = "postgres")]
    if let Some(url) = env_lookup("DATABASE_URL") {
        return Ok(StorageBackend::Postgres { url });
    }
    #[cfg(feature = "libsql")]
    if let Some(path) = env_lookup("LIBSQL_PATH") {
        return Ok(StorageBackend::LibSql { path });
    }
    // Fail-closed default. The Reborn host's entire purpose is durable
    // idempotency + binding storage; a silent `:memory:` fallback would
    // break the idempotency contract on every restart without anyone
    // noticing. Operators who want ephemeral storage on purpose (tests,
    // dev loops) opt in explicitly via `IRONCLAW_REBORN_ALLOW_EPHEMERAL=1`.
    #[cfg(feature = "libsql")]
    if env_flag_set(env_lookup, ALLOW_EPHEMERAL_ENV) {
        tracing::warn!(
            "Reborn host: using ephemeral in-memory libSQL storage because \
             {ALLOW_EPHEMERAL_ENV} is set. Ledger and bindings will be lost \
             on restart; not safe for production."
        );
        return Ok(StorageBackend::LibSql {
            path: ":memory:".to_string(),
        });
    }
    Err(HostError::Config(format!(
        "no durable storage configured — set DATABASE_URL (postgres) or \
         LIBSQL_PATH (libsql). For tests/dev, set {ALLOW_EPHEMERAL_ENV}=1 to \
         opt into in-memory storage."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::collections::HashMap;

    /// Build an `EnvLookup` closure backed by a fake `HashMap`. Tests use
    /// this to exercise `from_env_with` without touching process globals.
    fn fake_env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn lookup<'a>(map: &'a HashMap<String, String>) -> impl Fn(&str) -> Option<String> + 'a {
        |name: &str| map.get(name).cloned()
    }

    fn baseline_explicit_namespace() -> Vec<(&'static str, &'static str)> {
        let mut v = vec![
            ("TELEGRAM_BOT_TOKEN", "test-bot-token"),
            ("TELEGRAM_WEBHOOK_SECRET", "test-webhook-secret"),
            ("REBORN_TELEGRAM_V2_INSTALLATION_ID", "install_alpha"),
            ("REBORN_TENANT_ID", "tenant_alpha"),
            ("REBORN_AGENT_ID", "agent_alpha"),
        ];
        // Provide whichever storage backend is compiled in; tests below
        // override this per-case when they need a different backend.
        if cfg!(feature = "libsql") {
            v.push(("LIBSQL_PATH", "/tmp/test-not-touched"));
        } else if cfg!(feature = "postgres") {
            v.push(("DATABASE_URL", "postgres://test/not-touched"));
        }
        v
    }

    #[test]
    fn from_env_with_accepts_fully_explicit_namespace() {
        let env = fake_env(&baseline_explicit_namespace());
        let lookup = lookup(&env);
        let config = HostConfig::from_env_with(&lookup).expect("explicit namespace should succeed");
        assert_eq!(config.installation_id, "install_alpha");
        assert_eq!(config.tenant_id, "tenant_alpha");
        assert_eq!(config.agent_id, "agent_alpha");
        assert_eq!(config.telegram_bot_token.expose_secret(), "test-bot-token");
    }

    #[test]
    fn from_env_with_rejects_missing_installation_id() {
        let mut pairs = baseline_explicit_namespace();
        pairs.retain(|(k, _)| *k != "REBORN_TELEGRAM_V2_INSTALLATION_ID");
        let env = fake_env(&pairs);
        let lookup = lookup(&env);
        let err = HostConfig::from_env_with(&lookup).expect_err("missing install id must fail");
        match err {
            HostError::Config(msg) => {
                assert!(
                    msg.contains("REBORN_TELEGRAM_V2_INSTALLATION_ID"),
                    "error must name the missing var: {msg}"
                );
                assert!(
                    msg.contains("namespace fail-closed"),
                    "error must be the namespace guard: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn from_env_with_rejects_missing_tenant_and_agent_together() {
        let mut pairs = baseline_explicit_namespace();
        pairs.retain(|(k, _)| *k != "REBORN_TENANT_ID" && *k != "REBORN_AGENT_ID");
        let env = fake_env(&pairs);
        let lookup = lookup(&env);
        let err = HostConfig::from_env_with(&lookup).expect_err("missing tenant+agent must fail");
        let msg = match err {
            HostError::Config(m) => m,
            other => panic!("expected Config error, got {other:?}"),
        };
        assert!(msg.contains("REBORN_TENANT_ID"), "msg: {msg}");
        assert!(msg.contains("REBORN_AGENT_ID"), "msg: {msg}");
    }

    #[test]
    fn from_env_with_allows_defaults_when_opt_in_flag_set() {
        let mut pairs = vec![
            ("TELEGRAM_BOT_TOKEN", "test-bot-token"),
            ("TELEGRAM_WEBHOOK_SECRET", "test-webhook-secret"),
            ("IRONCLAW_REBORN_ALLOW_DEFAULT_NAMESPACE", "1"),
        ];
        if cfg!(feature = "libsql") {
            pairs.push(("LIBSQL_PATH", "/tmp/test-not-touched"));
        } else if cfg!(feature = "postgres") {
            pairs.push(("DATABASE_URL", "postgres://test/not-touched"));
        }
        let env = fake_env(&pairs);
        let lookup = lookup(&env);
        let config =
            HostConfig::from_env_with(&lookup).expect("opt-in flag should allow literal defaults");
        assert_eq!(config.installation_id, "default");
        assert_eq!(config.tenant_id, "tenant_default");
        assert_eq!(config.agent_id, "agent_default");
    }

    #[test]
    fn parse_pairings_returns_empty_when_env_unset() {
        assert_eq!(parse_pairings(None).expect("ok"), Vec::new());
    }

    #[test]
    fn parse_pairings_returns_empty_when_env_blank() {
        assert_eq!(parse_pairings(Some("")).expect("ok"), Vec::new());
        assert_eq!(parse_pairings(Some("   ")).expect("ok"), Vec::new());
    }

    #[test]
    fn parse_pairings_handles_single_entry() {
        let parsed = parse_pairings(Some("123456:user_alice")).expect("ok");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].external_user_id, "123456");
        assert_eq!(parsed[0].user_id, "user_alice");
    }

    #[test]
    fn parse_pairings_handles_multiple_entries_with_whitespace_and_trailing_comma() {
        let parsed = parse_pairings(Some("  123:user_alice ,  456 : user_bob ,")).expect("ok");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].external_user_id, "123");
        assert_eq!(parsed[0].user_id, "user_alice");
        assert_eq!(parsed[1].external_user_id, "456");
        assert_eq!(parsed[1].user_id, "user_bob");
    }

    #[test]
    fn parse_pairings_rejects_entry_without_colon() {
        let err = parse_pairings(Some("123,456:user_bob")).expect_err("must reject");
        match err {
            HostError::Config(msg) => {
                assert!(msg.contains("REBORN_TELEGRAM_PAIRINGS"), "msg: {msg}");
                assert!(msg.contains("123"), "msg should quote the bad entry: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn parse_pairings_rejects_empty_left_side() {
        let err = parse_pairings(Some(":user_alice")).expect_err("must reject");
        assert!(matches!(err, HostError::Config(_)));
    }

    #[test]
    fn parse_pairings_rejects_empty_right_side() {
        let err = parse_pairings(Some("123:")).expect_err("must reject");
        assert!(matches!(err, HostError::Config(_)));
    }

    #[test]
    fn from_env_with_accepts_pairings_var() {
        let mut pairs = baseline_explicit_namespace();
        pairs.push(("REBORN_TELEGRAM_PAIRINGS", "111:user_a,222:user_b"));
        let env = fake_env(&pairs);
        let lookup = lookup(&env);
        let config = HostConfig::from_env_with(&lookup).expect("should succeed");
        assert_eq!(config.pairings.len(), 2);
        assert_eq!(config.pairings[0].external_user_id, "111");
        assert_eq!(config.pairings[0].user_id, "user_a");
    }

    #[test]
    fn from_env_with_treats_opt_in_flag_value_zero_as_not_set() {
        // Reviewer concern: a half-baked "off" value must not bypass the
        // guard. Match the same parsing rule used for ALLOW_EPHEMERAL_ENV
        // so the two opt-ins behave the same.
        let mut pairs = vec![
            ("TELEGRAM_BOT_TOKEN", "test-bot-token"),
            ("TELEGRAM_WEBHOOK_SECRET", "test-webhook-secret"),
            ("IRONCLAW_REBORN_ALLOW_DEFAULT_NAMESPACE", "0"),
        ];
        if cfg!(feature = "libsql") {
            pairs.push(("LIBSQL_PATH", "/tmp/test-not-touched"));
        } else if cfg!(feature = "postgres") {
            pairs.push(("DATABASE_URL", "postgres://test/not-touched"));
        }
        let env = fake_env(&pairs);
        let lookup = lookup(&env);
        let err = HostConfig::from_env_with(&lookup).expect_err("value=0 must NOT count as opt-in");
        match err {
            HostError::Config(msg) => {
                assert!(msg.contains("namespace fail-closed"), "msg: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }
}
