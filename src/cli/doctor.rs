//! `ironclaw doctor` - active health diagnostics.
//!
//! Probes external dependencies and validates configuration to surface
//! problems before they bite during normal operation. Each check reports
//! pass/fail with actionable guidance on failures.

use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::cli::fmt;
use crate::settings::Settings;

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
        check_routines_config(),
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
        "Skills",
        check_skills().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    check(
        "Secrets",
        check_secrets(&settings),
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
                "openai" => embedding_api_key_present(config.openai_api_key()),
                "gemini" => embedding_api_key_present(config.gemini_api_key()),
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
                _ => embedding_api_key_present(config.openai_api_key()),
            };
            if has_creds {
                CheckResult::Pass(format!(
                    "provider={}, model={}",
                    config.provider, config.model
                ))
            } else {
                let hint = match config.provider.as_str() {
                    "nearai" => "run `ironclaw onboard` to create a session",
                    "gemini" => "set GEMINI_API_KEY",
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

fn embedding_api_key_present(api_key: Option<&str>) -> bool {
    api_key.is_some_and(|key| !key.is_empty())
}

// ── Routines config ─────────────────────────────────────────

fn check_routines_config() -> CheckResult {
    match crate::config::RoutineConfig::resolve() {
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

// ── Skills ──────────────────────────────────────────────────

async fn check_skills() -> CheckResult {
    let user_dir = ironclaw_base_dir().join("skills");
    let installed_dir = ironclaw_base_dir().join("installed_skills");

    let mut registry = crate::skills::SkillRegistry::new(user_dir.clone());
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

fn check_secrets(settings: &Settings) -> CheckResult {
    match settings.secrets_master_key_source {
        crate::settings::KeySource::Keychain => {
            CheckResult::Pass("master key source: OS keychain".into())
        }
        crate::settings::KeySource::Env => {
            if std::env::var("SECRETS_MASTER_KEY").is_ok() {
                CheckResult::Pass("master key source: env var (set)".into())
            } else {
                CheckResult::Fail(
                    "master key source: env var but SECRETS_MASTER_KEY not set".into(),
                )
            }
        }
        crate::settings::KeySource::None => {
            CheckResult::Skip("secrets not configured (run `ironclaw onboard`)".into())
        }
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
        let result = check_routines_config();
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

    #[test]
    fn check_secrets_none_returns_skip() {
        let settings = Settings::default();
        match check_secrets(&settings) {
            CheckResult::Skip(msg) => {
                assert!(
                    msg.contains("not configured"),
                    "expected 'not configured' in skip message, got: {msg}"
                );
            }
            other => panic!(
                "expected Skip for default settings, got: {}",
                format_result(&other)
            ),
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
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("GEMINI_API_KEY");
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
    fn check_embeddings_gemini_missing_key_returns_fail() {
        let _guard = crate::config::helpers::ENV_MUTEX.lock().expect("env mutex");
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "gemini");
            std::env::remove_var("GEMINI_API_KEY");
        }

        match check_embeddings(&Settings::default()) {
            CheckResult::Fail(msg) => {
                assert!(
                    msg.contains("GEMINI_API_KEY"),
                    "expected GEMINI_API_KEY hint, got: {msg}"
                );
            }
            other => panic!(
                "expected Fail for gemini embeddings without key, got: {}",
                format_result(&other)
            ),
        }

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
        }
    }

    #[test]
    fn embedding_api_key_present_rejects_empty_values() {
        assert!(!embedding_api_key_present(None));
        assert!(!embedding_api_key_present(Some("")));
        assert!(embedding_api_key_present(Some("non-empty")));
    }

    #[test]
    fn check_routines_enabled_by_default() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("ROUTINES_ENABLED");
        }
        match check_routines_config() {
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

    #[test]
    fn check_secrets_env_without_var_returns_fail() {
        let settings = Settings {
            secrets_master_key_source: crate::settings::KeySource::Env,
            ..Default::default()
        };
        match check_secrets(&settings) {
            CheckResult::Fail(msg) => {
                assert!(
                    msg.contains("SECRETS_MASTER_KEY not set"),
                    "expected mention of missing env var, got: {msg}"
                );
            }
            CheckResult::Pass(_) => {
                // If SECRETS_MASTER_KEY happens to be set in the environment,
                // Pass is correct — don't fail the test.
            }
            other => panic!(
                "expected Fail or Pass for env key source, got: {}",
                format_result(&other)
            ),
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
