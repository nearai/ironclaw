//! `ironclaw doctor` - active health diagnostics.
//!
//! Probes external dependencies and validates configuration to surface
//! problems before they bite during normal operation. Each check reports
//! pass/fail with actionable guidance on failures.

use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::cli::fmt;
use crate::settings::Settings;

async fn load_acp_agents_for_doctor()
-> Result<crate::config::acp::AcpAgentsFile, crate::config::acp::AcpConfigError> {
    match crate::config::Config::from_env().await {
        Ok(config) => {
            let db: Option<std::sync::Arc<dyn crate::db::Database>> =
                crate::db::connect_from_config(&config.database)
                    .await
                    .ok()
                    .map(|db| db as std::sync::Arc<dyn crate::db::Database>);
            crate::config::acp::load_acp_agents_for_user(db.as_deref(), &config.owner_id).await
        }
        Err(_) => crate::config::acp::load_acp_agents().await,
    }
}

/// Run all diagnostic checks and print results.
pub async fn run_doctor_command() -> anyhow::Result<()> {
    println!();
    println!("  {}IronClaw Doctor{}", fmt::bold(), fmt::reset());

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    // Load settings once for checks that need them.
    let settings = Settings::load();

    // ── Core ─────────────────────────────────────────────────

    section_header("Core");

    check(
        "Settings file",
        check_settings_file(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "NEAR AI session",
        check_nearai_session(&settings).await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "LLM configuration",
        check_llm_config(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Database backend",
        check_database().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Workspace directory",
        check_workspace_dir(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Features ─────────────────────────────────────────────

    section_header("Features");

    check(
        "Embeddings",
        check_embeddings(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Routines config",
        check_routines_config(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Gateway config",
        check_gateway_config(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "MCP servers",
        check_mcp_config().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "ACP agents",
        check_acp_config().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Skills",
        check_skills().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Secrets",
        check_secrets(&settings).await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Service",
        check_service_installed(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── External ─────────────────────────────────────────────

    section_header("External");

    check(
        "Docker daemon",
        check_docker_daemon().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "cloudflared",
        check_binary("cloudflared", &["--version"]),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "ngrok",
        check_binary("ngrok", &["version"]),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "tailscale",
        check_binary("tailscale", &["version"]),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Summary ───────────────────────────────────────────────

    println!();
    println!(
        "  {}{} passed{}, {}{} failed{}, {}{} skipped{}",
        fmt::success(),
        passed,
        fmt::reset(),
        if failed > 0 { fmt::error() } else { fmt::dim() },
        failed,
        fmt::reset(),
        fmt::dim(),
        skipped,
        fmt::reset(),
    );

    if failed > 0 {
        println!("\n  Some checks failed. This is normal if you don't use those features.");
    }

    Ok(())
}

/// Print a section header with a separator and bold group name.
fn section_header(name: &str) {
    println!();
    println!("  {}", fmt::separator(36));
    println!("  {}{}{}", fmt::bold(), name, fmt::reset());
    println!();
}

// ── Individual checks ───────────────────────────────────────

fn check(name: &str, result: CheckResult, passed: &mut u32, failed: &mut u32, skipped: &mut u32) {
    match result {
        CheckResult::Pass(detail) => {
            *passed += 1;
            println!(
                "{}",
                fmt::check_line(fmt::StatusKind::Pass, name, &detail, 18)
            );
        }
        CheckResult::Fail(detail) => {
            *failed += 1;
            println!(
                "{}",
                fmt::check_line(fmt::StatusKind::Fail, name, &detail, 18)
            );
        }
        CheckResult::Skip(reason) => {
            *skipped += 1;
            println!(
                "{}",
                fmt::check_line(fmt::StatusKind::Skip, name, &reason, 18)
            );
        }
    }
}

enum CheckResult {
    Pass(String),
    Fail(String),
    Skip(String),
}

// ── Settings file ───────────────────────────────────────────

fn check_settings_file() -> CheckResult {
    let path = Settings::default_path();
    if !path.exists() {
        return CheckResult::Pass("no settings file (defaults will be used)".into());
    }

    match std::fs::read_to_string(&path) {
        Ok(data) => match serde_json::from_str::<serde_json::Value>(&data) {
            Ok(_) => CheckResult::Pass(format!("valid ({})", path.display())),
            Err(e) => CheckResult::Fail(format!(
                "settings.json is malformed: {}. Fix or delete {}",
                e,
                path.display()
            )),
        },
        Err(e) => CheckResult::Fail(format!("cannot read {}: {}", path.display(), e)),
    }
}

// ── NEAR AI session ─────────────────────────────────────────

async fn check_nearai_session(settings: &Settings) -> CheckResult {
    // Skip entirely when the configured backend is not NEAR AI.
    let llm_config = match crate::config::LlmConfig::resolve(settings) {
        Ok(config) => config,
        Err(e) => {
            // check_llm_config will report the full error; just skip here.
            return CheckResult::Skip(format!("LLM config error: {e}"));
        }
    };
    if llm_config.backend != "nearai" {
        return CheckResult::Skip(format!(
            "not using NEAR AI backend (backend={})",
            llm_config.backend
        ));
    }

    // Check if session file exists
    let session_path = crate::config::llm::default_session_path();
    if !session_path.exists() {
        // Check for API key mode
        if crate::config::helpers::env_or_override("NEARAI_API_KEY").is_some() {
            return CheckResult::Pass("API key configured".into());
        }
        return CheckResult::Fail(format!(
            "session file not found at {}. Run `ironclaw onboard`",
            session_path.display()
        ));
    }

    // Verify the session file is readable and non-empty
    match std::fs::read_to_string(&session_path) {
        Ok(content) if content.trim().is_empty() => {
            CheckResult::Fail("session file is empty".into())
        }
        Ok(_) => CheckResult::Pass(format!("session found ({})", session_path.display())),
        Err(e) => CheckResult::Fail(format!("cannot read session file: {e}")),
    }
}

// ── LLM configuration ──────────────────────────────────────

fn check_llm_config(settings: &Settings) -> CheckResult {
    match crate::llm::LlmConfig::resolve(settings) {
        Ok(config) => {
            // Show the model for the active backend, not always nearai.model.
            let model = if let Some(ref bedrock) = config.bedrock {
                &bedrock.model
            } else if let Some(ref provider) = config.provider {
                &provider.model
            } else {
                &config.nearai.model
            };
            CheckResult::Pass(format!("backend={}, model={}", config.backend, model))
        }
        Err(e) => CheckResult::Fail(format!("LLM config error: {e}")),
    }
}

// ── Database ────────────────────────────────────────────────

async fn check_database() -> CheckResult {
    let backend = std::env::var("DATABASE_BACKEND")
        .ok()
        .unwrap_or_else(|| "postgres".into());

    match backend.as_str() {
        "libsql" | "turso" | "sqlite" => {
            let path = std::env::var("LIBSQL_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| crate::config::default_libsql_path());

            if path.exists() {
                CheckResult::Pass(format!("libSQL database exists ({})", path.display()))
            } else {
                CheckResult::Pass(format!(
                    "libSQL database not found at {} (will be created on first run)",
                    path.display()
                ))
            }
        }
        _ => {
            if std::env::var("DATABASE_URL").is_ok() {
                // Try to connect
                match try_pg_connect().await {
                    Ok(()) => CheckResult::Pass("PostgreSQL connected".into()),
                    Err(e) => CheckResult::Fail(format!("PostgreSQL connection failed: {e}")),
                }
            } else {
                CheckResult::Fail("DATABASE_URL not set".into())
            }
        }
    }
}

#[cfg(feature = "postgres")]
async fn try_pg_connect() -> Result<(), String> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set".to_string())?;

    let config = deadpool_postgres::Config {
        url: Some(url),
        ..Default::default()
    };
    let pool = crate::db::tls::create_pool(&config, crate::config::SslMode::from_env())
        .map_err(|e| format!("pool error: {e}"))?;

    let client = tokio::time::timeout(std::time::Duration::from_secs(5), pool.get())
        .await
        .map_err(|_| "connection timeout (5s)".to_string())?
        .map_err(|e| format!("{e}"))?;

    client
        .execute("SELECT 1", &[])
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(())
}

#[cfg(not(feature = "postgres"))]
async fn try_pg_connect() -> Result<(), String> {
    Err("postgres feature not compiled in".into())
}

// ── Workspace directory ─────────────────────────────────────

fn check_workspace_dir() -> CheckResult {
    let dir = ironclaw_base_dir();

    if dir.exists() {
        if dir.is_dir() {
            CheckResult::Pass(format!("{}", dir.display()))
        } else {
            CheckResult::Fail(format!("{} exists but is not a directory", dir.display()))
        }
    } else {
        CheckResult::Pass(format!("{} will be created on first run", dir.display()))
    }
}

// ── Embeddings ──────────────────────────────────────────────

fn check_embeddings(settings: &Settings) -> CheckResult {
    match crate::config::EmbeddingsConfig::resolve(settings) {
        Ok(config) => {
            if !config.enabled {
                return CheckResult::Skip("disabled (set EMBEDDING_ENABLED=true)".into());
            }
            let has_creds = match config.provider.as_str() {
                "openai" => config.openai_api_key().is_some(),
                "nearai" => {
                    // NearAiEmbeddings uses SessionManager::get_token() which
                    // only returns session tokens, NOT NEARAI_API_KEY
                    // (src/workspace/embeddings.rs:309, src/llm/session.rs:132).
                    let session_path = crate::config::llm::default_session_path();
                    session_path.exists()
                        && std::fs::read_to_string(&session_path)
                            .map(|s| !s.trim().is_empty())
                            .unwrap_or(false)
                }
                "ollama" => true, // local, no creds needed
                _ => config.openai_api_key().is_some(),
            };
            if has_creds {
                CheckResult::Pass(format!(
                    "provider={}, model={}",
                    config.provider, config.model
                ))
            } else {
                let hint = match config.provider.as_str() {
                    "nearai" => "run `ironclaw onboard` to create a session",
                    _ => "set OPENAI_API_KEY",
                };
                CheckResult::Fail(format!(
                    "provider={} but credentials missing ({})",
                    config.provider, hint
                ))
            }
        }
        Err(e) => CheckResult::Fail(format!("config error: {e}")),
    }
}

// ── Routines config ─────────────────────────────────────────

fn check_routines_config(settings: &Settings) -> CheckResult {
    match crate::config::RoutineConfig::resolve(settings) {
        Ok(config) => {
            if config.enabled {
                CheckResult::Pass(format!(
                    "enabled (interval={}s, max_concurrent={})",
                    config.cron_check_interval_secs, config.max_concurrent_routines
                ))
            } else {
                CheckResult::Skip("disabled".into())
            }
        }
        Err(e) => CheckResult::Fail(format!("config error: {e}")),
    }
}

// ── Gateway config ──────────────────────────────────────────

fn check_gateway_config(settings: &Settings) -> CheckResult {
    // Use the same resolve() path as runtime so invalid env values
    // (e.g. GATEWAY_PORT=abc) are caught here too.
    let owner_id = match crate::config::resolve_owner_id(settings) {
        Ok(owner_id) => owner_id,
        Err(e) => return CheckResult::Fail(format!("config error: {e}")),
    };
    match crate::config::ChannelsConfig::resolve(settings, &owner_id) {
        Ok(channels) => match channels.gateway {
            Some(gw) => {
                if gw.auth_token.is_some() {
                    CheckResult::Pass(format!(
                        "enabled at {}:{} (auth token set)",
                        gw.host, gw.port
                    ))
                } else {
                    CheckResult::Pass(format!(
                        "enabled at {}:{} (no auth token — random token will be generated)",
                        gw.host, gw.port
                    ))
                }
            }
            None => CheckResult::Skip("disabled (GATEWAY_ENABLED=false)".into()),
        },
        Err(e) => CheckResult::Fail(format!("config error: {e}")),
    }
}

// ── MCP servers ─────────────────────────────────────────────

async fn check_mcp_config() -> CheckResult {
    match crate::tools::mcp::config::load_mcp_servers().await {
        Ok(file) => {
            let servers: Vec<_> = file.enabled_servers().collect();
            if servers.is_empty() {
                return CheckResult::Skip("no MCP servers configured".into());
            }

            let mut invalid = Vec::new();
            for server in &servers {
                if let Err(e) = server.validate() {
                    invalid.push(format!("{}: {}", server.name, e));
                }
            }

            if invalid.is_empty() {
                CheckResult::Pass(format!("{} server(s) configured, all valid", servers.len()))
            } else {
                CheckResult::Fail(format!(
                    "{} server(s), {} invalid: {}",
                    servers.len(),
                    invalid.len(),
                    invalid.join("; ")
                ))
            }
        }
        Err(e) => {
            // Distinguish no config from corrupted config
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("No such file") {
                CheckResult::Skip("no MCP config file".into())
            } else {
                CheckResult::Fail(format!("config error: {e}"))
            }
        }
    }
}

async fn check_acp_config() -> CheckResult {
    match load_acp_agents_for_doctor().await {
        Ok(file) => {
            let agents: Vec<_> = file.enabled_agents().collect();
            if agents.is_empty() {
                return CheckResult::Skip("no ACP agents configured".into());
            }

            let mut invalid = Vec::new();
            for agent in &agents {
                if let Err(e) = agent.validate() {
                    invalid.push(format!("{}: {}", agent.name, e));
                }
            }

            if invalid.is_empty() {
                CheckResult::Pass(format!("{} agent(s) configured, all valid", agents.len()))
            } else {
                CheckResult::Fail(format!(
                    "{} agent(s), {} invalid: {}",
                    agents.len(),
                    invalid.len(),
                    invalid.join("; ")
                ))
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("No such file") {
                CheckResult::Skip("no ACP config file".into())
            } else {
                CheckResult::Fail(format!("config error: {e}"))
            }
        }
    }
}

// ── Skills ──────────────────────────────────────────────────

async fn check_skills() -> CheckResult {
    let user_dir = ironclaw_base_dir().join("skills");
    let installed_dir = ironclaw_base_dir().join("installed_skills");

    let mut registry = ironclaw_skills::SkillRegistry::new(user_dir.clone());
    registry = registry.with_installed_dir(installed_dir);

    // discover_all() returns loaded skill names (not warnings).
    let _loaded_names = registry.discover_all().await;

    let count = registry.count();
    if count == 0 {
        return CheckResult::Skip("no skills discovered".into());
    }

    CheckResult::Pass(format!("{count} skill(s) loaded"))
}

// ── Secrets ─────────────────────────────────────────────────

/// Diagnose the secrets subsystem end-to-end.
///
/// The stored `settings.secrets_master_key_source` is only one signal and
/// does not capture the hosted-TEE failure mode (#1537): master key resolves
/// to `Env`/`Keychain` source at runtime, but the backing store factory
/// returns `None` because the DB handles needed by `LibSqlSecretsStore` /
/// `PostgresSecretsStore` aren't available, so WASM tool credential
/// injection silently falls back to unauthenticated requests.
///
/// This check runs the same `SecretsConfig::resolve` path `AppBuilder::init_secrets`
/// uses, then attempts to construct the backing store — the two things that
/// jointly determine whether credential injection will actually work.
async fn check_secrets(settings: &Settings) -> CheckResult {
    // 1. Master-key resolution — READ-ONLY. `SecretsConfig::resolve()`
    //    auto-generates and persists a key to `~/.ironclaw/.env` when
    //    none exists, which is the correct behavior for startup but
    //    would make `ironclaw doctor` mutate user state every time it
    //    ran on a fresh machine. Instead, probe only for an *existing*
    //    key — env var or OS keychain — so the missing-key case reports
    //    as Skip("not configured") without creating one.
    //    References: Copilot/#2753 + serrrfirat review on PR #2753.
    let resolved_key = crate::secrets::resolve_master_key().await;

    let Some(master_key_hex) = resolved_key else {
        return CheckResult::Skip("secrets not configured (run `ironclaw onboard`)".into());
    };

    // Determine which source won. Mirrors `SecretsConfig::resolve`'s
    // order: env first, keychain second, but without the auto-generate
    // fallback — so the only two reachable outcomes are `Env` or
    // `Keychain`. `KeySource::None` is *not* reachable here because the
    // `Some(master_key_hex) else Skip` guard above already returned.
    let env_wins = std::env::var("SECRETS_MASTER_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some();
    let (source, source_label) = if env_wins {
        (crate::settings::KeySource::Env, "env / ~/.ironclaw/.env")
    } else {
        (crate::settings::KeySource::Keychain, "OS keychain")
    };

    // 2. Surface a warning when settings disagree with the resolved runtime
    //    source — common when onboarding was skipped on a TEE.
    let settings_note = match (settings.secrets_master_key_source, source) {
        (s, r) if s == r => String::new(),
        (crate::settings::KeySource::None, _) => {
            " (settings say `None`; run `ironclaw onboard` to persist)".to_string()
        }
        (s, r) => format!(" (settings say `{s:?}`, runtime resolved `{r:?}`)"),
    };

    // 3. Probe the backing store — this is the #1537 axis. A missing DB
    //    handle here is the hosted-TEE symptom: master key present, but
    //    the store factory has nothing to build against.
    let config = match crate::config::Config::from_env().await {
        Ok(c) => c,
        Err(e) => {
            return CheckResult::Fail(format!(
                "master key resolves from {source_label}{settings_note}, but config load \
                 failed so the backing store cannot be probed: {e}"
            ));
        }
    };

    let crypto =
        match crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(master_key_hex)) {
            Ok(c) => std::sync::Arc::new(c),
            Err(e) => {
                return CheckResult::Fail(format!(
                    "master key resolved from {source_label} but crypto init failed: {e}"
                ));
            }
        };

    match crate::db::create_secrets_store(&config.database, crypto).await {
        Ok(_store) => CheckResult::Pass(format!(
            "master key source: {source_label}; backing store reachable{settings_note}"
        )),
        Err(e) => CheckResult::Fail(format!(
            "master key present ({source_label}){settings_note} but backing store unavailable: \
             {e}. Runtime falls back to an ephemeral in-memory store (see #1537); \
             credentials saved via `ironclaw tool auth` will not persist across restarts"
        )),
    }
}

// ── Service ─────────────────────────────────────────────────

fn check_service_installed() -> CheckResult {
    if cfg!(target_os = "macos") {
        let plist =
            dirs::home_dir().map(|h| h.join("Library/LaunchAgents/com.ironclaw.daemon.plist"));
        match plist {
            Some(path) if path.exists() => {
                CheckResult::Pass(format!("launchd plist installed ({})", path.display()))
            }
            Some(_) => CheckResult::Skip("not installed (run `ironclaw service install`)".into()),
            None => CheckResult::Skip("cannot determine home directory".into()),
        }
    } else if cfg!(target_os = "linux") {
        let unit = dirs::home_dir().map(|h| h.join(".config/systemd/user/ironclaw.service"));
        match unit {
            Some(path) if path.exists() => {
                CheckResult::Pass(format!("systemd unit installed ({})", path.display()))
            }
            Some(_) => CheckResult::Skip("not installed (run `ironclaw service install`)".into()),
            None => CheckResult::Skip("cannot determine home directory".into()),
        }
    } else {
        CheckResult::Skip("service management not supported on this platform".into())
    }
}

// ── Docker daemon ───────────────────────────────────────────

async fn check_docker_daemon() -> CheckResult {
    let detection = crate::sandbox::check_docker().await;
    match detection.status {
        crate::sandbox::DockerStatus::Available => CheckResult::Pass("running".into()),
        crate::sandbox::DockerStatus::NotInstalled => CheckResult::Skip(format!(
            "not installed. {}",
            detection.platform.install_hint()
        )),
        crate::sandbox::DockerStatus::NotRunning => CheckResult::Fail(format!(
            "installed but not running. {}",
            detection.platform.start_hint()
        )),
        crate::sandbox::DockerStatus::Disabled => CheckResult::Skip("sandbox disabled".into()),
    }
}

// ── External binary ─────────────────────────────────────────

fn check_binary(name: &str, args: &[&str]) -> CheckResult {
    match std::process::Command::new(name)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version = version.trim();
            // Some tools print version to stderr
            let version = if version.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                stderr.trim().lines().next().unwrap_or("").to_string()
            } else {
                version.lines().next().unwrap_or("").to_string()
            };

            if output.status.success() {
                CheckResult::Pass(version)
            } else {
                CheckResult::Fail(format!("exited with {}", output.status))
            }
        }
        Err(_) => CheckResult::Skip(format!("{name} not found in PATH")),
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::doctor::*;

    #[test]
    fn check_binary_finds_sh() {
        match check_binary("sh", &["-c", "echo ok"]) {
            CheckResult::Pass(_) => {}
            other => panic!("expected Pass for sh, got: {}", format_result(&other)),
        }
    }

    #[test]
    fn check_binary_skips_nonexistent() {
        match check_binary("__ironclaw_nonexistent_binary__", &["--version"]) {
            CheckResult::Skip(_) => {}
            other => panic!(
                "expected Skip for nonexistent binary, got: {}",
                format_result(&other)
            ),
        }
    }

    #[test]
    fn check_workspace_dir_does_not_panic() {
        let result = check_workspace_dir();
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[tokio::test]
    async fn check_nearai_session_does_not_panic() {
        let settings = Settings::default();
        let result = check_nearai_session(&settings).await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_nearai_session_skips_for_non_nearai_backend() {
        struct EnvGuard(&'static str, Option<String>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                // SAFETY: Under ENV_MUTEX.
                unsafe {
                    match &self.1 {
                        Some(val) => std::env::set_var(self.0, val),
                        None => std::env::remove_var(self.0),
                    }
                }
            }
        }

        let _mutex = crate::config::helpers::lock_env();
        let prev = std::env::var("LLM_BACKEND").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("LLM_BACKEND", "anthropic");
        }
        let _env_guard = EnvGuard("LLM_BACKEND", prev);

        let settings = Settings::default();
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let result = rt.block_on(check_nearai_session(&settings));
        match result {
            CheckResult::Skip(msg) => {
                assert!(
                    msg.contains("backend=anthropic"),
                    "expected backend name in skip message, got: {msg}"
                );
            }
            other => panic!(
                "expected Skip for non-nearai backend, got: {}",
                format_result(&other)
            ),
        }
    }

    #[test]
    fn check_settings_file_handles_missing() {
        // Settings::default_path() might or might not exist, but must not panic
        let result = check_settings_file();
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_llm_config_does_not_panic() {
        let settings = Settings::default();
        let result = check_llm_config(&settings);
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_routines_config_does_not_panic() {
        let settings = Settings::default();
        let result = check_routines_config(&settings);
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_gateway_config_does_not_panic() {
        let settings = Settings::default();
        let result = check_gateway_config(&settings);
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_embeddings_does_not_panic() {
        let settings = Settings::default();
        let result = check_embeddings(&settings);
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    /// `check_secrets` now resolves the master key the same way
    /// `AppBuilder::init_secrets` does (env → keychain → auto-generate to
    /// `~/.ironclaw/.env`) and probes the backing store. The exact outcome
    /// depends on the test-host environment — if `SECRETS_MASTER_KEY` is
    /// set in CI, we'll Pass; on a dev machine with no keychain we'll
    /// auto-generate. Either way the function must not panic, and the
    /// settings-snapshot `None` no longer masks a working runtime key.
    #[tokio::test]
    async fn check_secrets_does_not_panic() {
        let settings = Settings::default();
        let result = check_secrets(&settings).await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_service_installed_does_not_panic() {
        let result = check_service_installed();
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[tokio::test]
    async fn check_docker_daemon_does_not_panic() {
        let result = check_docker_daemon().await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[tokio::test]
    async fn check_mcp_config_does_not_panic() {
        let result = check_mcp_config().await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[tokio::test]
    async fn check_skills_does_not_panic() {
        let result = check_skills().await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[test]
    fn check_llm_config_shows_nearai_model_for_nearai_backend() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
        }
        let settings = Settings::default();
        match check_llm_config(&settings) {
            CheckResult::Pass(msg) => {
                assert!(
                    msg.contains("backend=nearai"),
                    "expected nearai backend, got: {msg}"
                );
                // Must NOT show a bedrock or registry model when backend is nearai
                assert!(
                    !msg.contains("anthropic.claude"),
                    "should not show bedrock model for nearai backend: {msg}"
                );
            }
            other => panic!(
                "expected Pass for default LLM config, got: {}",
                format_result(&other)
            ),
        }
    }

    #[test]
    fn check_embeddings_disabled_by_default_returns_skip() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
        }
        let settings = Settings::default();
        match check_embeddings(&settings) {
            CheckResult::Skip(msg) => {
                assert!(
                    msg.contains("disabled"),
                    "expected 'disabled' in skip message, got: {msg}"
                );
            }
            other => panic!(
                "expected Skip for disabled embeddings, got: {}",
                format_result(&other)
            ),
        }
    }

    #[test]
    fn check_routines_enabled_by_default() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("ROUTINES_ENABLED");
        }
        let settings = Settings::default();
        match check_routines_config(&settings) {
            CheckResult::Pass(msg) => {
                assert!(
                    msg.contains("enabled"),
                    "routines should be enabled by default, got: {msg}"
                );
            }
            other => panic!(
                "expected Pass for default routines, got: {}",
                format_result(&other)
            ),
        }
    }

    /// Earlier the `Env`-source branch returned Fail when
    /// `SECRETS_MASTER_KEY` was unset. With the TEE-aware rewrite the check
    /// resolves the actual master key (which may auto-generate and persist
    /// to `~/.ironclaw/.env`, producing a runtime `Env` key that differs
    /// from the settings snapshot). Just make sure settings drift no longer
    /// panics the check.
    #[tokio::test]
    async fn check_secrets_env_source_does_not_panic() {
        let settings = Settings {
            secrets_master_key_source: crate::settings::KeySource::Env,
            ..Default::default()
        };
        let result = check_secrets(&settings).await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    fn format_result(r: &CheckResult) -> String {
        match r {
            CheckResult::Pass(s) => format!("Pass({s})"),
            CheckResult::Fail(s) => format!("Fail({s})"),
            CheckResult::Skip(s) => format!("Skip({s})"),
        }
    }
}
