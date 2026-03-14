//! Gateway management CLI commands.
//!
//! Provides standalone gateway lifecycle management:
//!
//! - `gateway serve`  — foreground mode (Ctrl-C to stop), for dev/debug
//! - `gateway start`  — background daemon, spawns `serve` as detached child
//! - `gateway stop`   — sends SIGTERM to background daemon via PID file
//! - `gateway status` — checks PID liveness + health probe
//!
//! Read-only APIs work in standalone mode (health, threads, history, memory,
//! settings, skills, extensions, logs). Write/control APIs that require the
//! agent loop (chat send/ws, routine trigger, job restart/cancel/prompt)
//! return 503. For full agent mode, use `ironclaw run`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use clap::Subcommand;

use crate::app::{AppBuilder, AppBuilderFlags};
use crate::bootstrap::{PidLock, gateway_log_path, gateway_pid_lock_path};
use crate::channels::GatewayChannel;
use crate::channels::web::log_layer::{LogBroadcaster, init_tracing};
use crate::channels::web::server;
use crate::config::Config;
use crate::llm::create_session_manager;

/// Maximum time to wait for the background gateway to become healthy.
const START_HEALTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Polling interval when waiting for health check.
const START_HEALTH_POLL: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Subcommand, Debug, Clone)]
pub enum GatewayCommand {
    /// Run the web gateway in the foreground (Ctrl-C to stop).
    ///
    /// Starts the gateway API server without the agent loop. Useful for
    /// development and debugging. Use `gateway start` for background mode.
    Serve,

    /// Start the web gateway as a background daemon.
    ///
    /// Spawns the gateway in the background, writes a PID file, and returns
    /// immediately. Use `gateway stop` to shut it down.
    Start,

    /// Stop a running background gateway.
    ///
    /// Reads the PID from ~/.ironclaw/gateway.pid and sends SIGTERM
    /// for graceful shutdown.
    Stop,

    /// Show gateway status.
    ///
    /// Checks the PID file and probes the health endpoint.
    Status,
}

/// Run the gateway CLI subcommand.
pub async fn run_gateway_command(
    cmd: GatewayCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    match cmd {
        GatewayCommand::Serve => cmd_serve(config_path).await,
        GatewayCommand::Start => cmd_start(config_path).await,
        GatewayCommand::Stop => cmd_stop(),
        GatewayCommand::Status => cmd_status(config_path).await,
    }
}

// ─── serve (foreground) ─────────────────────────────────────────

async fn cmd_serve(config_path: Option<&Path>) -> anyhow::Result<()> {
    // Load config first.
    let config = Config::from_env_with_toml(config_path).await?;

    let gw_config = config
        .channels
        .gateway
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Gateway is not enabled. Set GATEWAY_ENABLED=true"))?;

    // Acquire PID lock before any heavy init — ensures single instance.
    let _pid_lock = PidLock::acquire_at(gateway_pid_lock_path())
        .map_err(|e| anyhow::anyhow!("Cannot start gateway: {e}"))?;

    // Initialize full tracing (with LogBroadcaster for /api/logs/events).
    let log_broadcaster = Arc::new(LogBroadcaster::new());
    let log_level_handle = init_tracing(Arc::clone(&log_broadcaster));

    tracing::info!("Starting standalone gateway...");

    // Capture tunnel URL before config is moved into AppBuilder.
    let tunnel_public_url = config.tunnel.public_url.clone();

    // Build core components via the standard AppBuilder pipeline.
    let session = create_session_manager(config.llm.session.clone()).await;
    let flags = AppBuilderFlags { no_db: false };
    let components = AppBuilder::new(
        config,
        flags,
        config_path.map(std::path::PathBuf::from),
        session,
        Arc::clone(&log_broadcaster),
    )
    .build_all()
    .await?;

    // Wire the GatewayChannel with available components.
    // NOTE: keep in sync with main.rs gateway setup (~line 468). Standalone mode
    // intentionally omits: scheduler, job_manager, routine_engine, prompt_queue,
    // ext_mgr gateway_mode. Those APIs return 503 in standalone.
    let session_manager =
        Arc::new(crate::agent::SessionManager::new().with_hooks(components.hooks.clone()));
    let mut gw =
        GatewayChannel::new(gw_config.clone()).with_llm_provider(Arc::clone(&components.llm));
    if let Some(ref ws) = components.workspace {
        gw = gw.with_workspace(Arc::clone(ws));
    }
    gw = gw.with_session_manager(session_manager);
    gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
    gw = gw.with_log_level_handle(Arc::clone(&log_level_handle));
    gw = gw.with_tool_registry(Arc::clone(&components.tools));
    if let Some(ref ext_mgr) = components.extension_manager {
        // Enable gateway mode so MCP OAuth returns auth URLs to the frontend
        // instead of calling open::that() on the server.
        let gw_base = tunnel_public_url
            .unwrap_or_else(|| format!("http://{}:{}", gw_config.host, gw_config.port));
        ext_mgr.enable_gateway_mode(gw_base).await;
        gw = gw.with_extension_manager(Arc::clone(ext_mgr));
    }
    if !components.catalog_entries.is_empty() {
        gw = gw.with_registry_entries(components.catalog_entries.clone());
    }
    if let Some(ref d) = components.db {
        gw = gw.with_store(Arc::clone(d));
    }
    if let Some(ref sr) = components.skill_registry {
        gw = gw.with_skill_registry(Arc::clone(sr));
    }
    if let Some(ref sc) = components.skill_catalog {
        gw = gw.with_skill_catalog(Arc::clone(sc));
    }
    gw = gw.with_cost_guard(Arc::clone(&components.cost_guard));

    // Start the HTTP server directly — do NOT call Channel::start() so
    // msg_tx stays None and chat send/ws naturally return 503.
    let addr: SocketAddr = format!("{}:{}", gw_config.host, gw_config.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid gateway address: {e}"))?;

    let state = Arc::clone(gw.state());
    let auth_token = gw.auth_token().to_string();

    let (bound_addr, server_handle) =
        server::start_server(addr, Arc::clone(&state), auth_token.clone()).await?;

    println!("Gateway running at http://{bound_addr}/");
    println!("Auth token: {auth_token}");
    println!("Web UI: http://{bound_addr}/?token={auth_token}");
    println!();
    println!("Standalone mode: chat endpoints return 503 (no agent loop).");
    println!("Press Ctrl-C to stop.");

    // Wait for shutdown signal, then bridge to axum's graceful shutdown.
    wait_for_shutdown_signal().await;

    tracing::info!("Shutting down gateway...");
    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(());
    }

    // Wait for the server to finish draining in-flight requests.
    let _ = server_handle.await;

    // _pid_lock is dropped here, cleaning up gateway.pid.
    Ok(())
}

/// Wait for SIGTERM or Ctrl-C.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        } else {
            // SIGTERM registration failed — fall back to Ctrl-C only.
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// ─── start (background daemon) ──────────────────────────────────

async fn cmd_start(config_path: Option<&Path>) -> anyhow::Result<()> {
    // Pre-flight: check that gateway is enabled before spawning.
    let config = Config::from_env_with_toml(config_path).await?;
    let gw_config = config
        .channels
        .gateway
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Gateway is not enabled. Set GATEWAY_ENABLED=true"))?;

    // Check if already running.
    let pid_path = gateway_pid_lock_path();
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path)
        && let Ok(pid) = pid_str.trim().parse::<u32>()
        && is_process_alive(pid)
    {
        anyhow::bail!("Gateway is already running (PID {pid})");
    }

    // Spawn `ironclaw gateway serve` as a detached child process.
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Cannot determine own executable path: {e}"))?;

    let log_path = gateway_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Cannot create directory {}: {e}", parent.display()))?;
    }
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| anyhow::anyhow!("Cannot create log file {}: {e}", log_path.display()))?;
    let stderr_file = log_file
        .try_clone()
        .map_err(|e| anyhow::anyhow!("Cannot clone log file handle: {e}"))?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("gateway").arg("serve");
    if let Some(cp) = config_path {
        cmd.arg("--config").arg(cp);
    }
    cmd.stdout(log_file)
        .stderr(stderr_file)
        .stdin(std::process::Stdio::null());

    // On Unix, start a new session so the child survives parent exit.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid() is async-signal-safe and is the standard way to
        // detach a child process from the parent's terminal session.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn gateway process: {e}"))?;

    let child_pid = child.id();
    println!("Gateway starting in background (PID {child_pid})...");
    println!("Log file: {}", log_path.display());

    // Poll health endpoint until it responds or timeout.
    let health_url = format!("http://{}:{}/api/health", gw_config.host, gw_config.port);
    let deadline = std::time::Instant::now() + START_HEALTH_TIMEOUT;
    let mut healthy = false;

    while std::time::Instant::now() < deadline {
        tokio::time::sleep(START_HEALTH_POLL).await;

        // Also verify the child hasn't crashed.
        if !is_process_alive(child_pid) {
            anyhow::bail!(
                "Gateway process exited unexpectedly. Check logs: {}",
                log_path.display()
            );
        }

        if probe_health(&health_url).await.unwrap_or(false) {
            healthy = true;
            break;
        }
    }

    if healthy {
        let base_url = format!("http://{}:{}", gw_config.host, gw_config.port);
        println!("Gateway is running at {base_url}/");

        // Extract auth token from the child's log output so the user can
        // access the Web UI without manually reading the log file.
        if let Some(token) = extract_auth_token_from_log(&log_path) {
            println!("Auth token: {token}");
            println!("Web UI: {base_url}/?token={token}");
        } else {
            println!("Auth token: see {}", log_path.display());
        }
    } else {
        println!(
            "Warning: gateway process started but health check did not pass within {}s.",
            START_HEALTH_TIMEOUT.as_secs()
        );
        println!("Check logs: {}", log_path.display());
    }

    Ok(())
}

/// Extract the auth token from the gateway log file.
///
/// The `serve` subcommand prints `Auth token: <token>` to stdout, which is
/// redirected to the log file in background mode.
fn extract_auth_token_from_log(log_path: &std::path::Path) -> Option<String> {
    let contents = std::fs::read_to_string(log_path).ok()?;
    for line in contents.lines() {
        if let Some(token) = line.strip_prefix("Auth token: ") {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

// ─── stop ────────────────────────────────────────────────────────

fn cmd_stop() -> anyhow::Result<()> {
    let pid_path = gateway_pid_lock_path();
    let pid_str = std::fs::read_to_string(&pid_path).map_err(|_| {
        anyhow::anyhow!(
            "Gateway is not running (no PID file at {})",
            pid_path.display()
        )
    })?;

    let pid: u32 = pid_str.trim().parse().map_err(|_| {
        anyhow::anyhow!(
            "Invalid PID in {}: '{}'",
            pid_path.display(),
            pid_str.trim()
        )
    })?;

    #[cfg(unix)]
    {
        // SAFETY: We are sending a signal to a process we own.
        let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                // Process doesn't exist — stale PID file
                let _ = std::fs::remove_file(&pid_path);
                anyhow::bail!(
                    "Gateway process (PID {pid}) is not running (stale PID file removed)"
                );
            }
            anyhow::bail!("Failed to send SIGTERM to PID {pid}: {err}");
        }
        println!("Sent SIGTERM to gateway (PID {pid}).");
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        anyhow::bail!("gateway stop is currently Unix-only");
    }

    Ok(())
}

// ─── status ──────────────────────────────────────────────────────

async fn cmd_status(config_path: Option<&Path>) -> anyhow::Result<()> {
    let pid_path = gateway_pid_lock_path();

    // Check PID file
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => {
            println!("Gateway is not running (no PID file).");
            return Ok(());
        }
    };

    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            println!("Gateway PID file exists but is invalid.");
            return Ok(());
        }
    };

    // Check if process is alive
    let alive = is_process_alive(pid);
    if !alive {
        println!("Gateway is not running (stale PID {pid}).");
        return Ok(());
    }

    println!("Gateway is running (PID {pid}).");

    // Try to determine configured address and probe health
    let config = Config::from_env_with_toml(config_path).await.ok();
    let addr = config
        .as_ref()
        .and_then(|c| c.channels.gateway.as_ref())
        .map(|gw| format!("{}:{}", gw.host, gw.port));

    if let Some(ref addr) = addr {
        println!("Address: {addr}");

        // Probe /api/health (unauthenticated endpoint)
        let url = format!("http://{addr}/api/health");
        match probe_health(&url).await {
            Ok(true) => println!("Health:  ok"),
            Ok(false) => println!("Health:  unhealthy"),
            Err(_) => println!("Health:  unreachable"),
        }
    }

    Ok(())
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks if process exists without sending a signal.
        // SAFETY: Signal 0 is a null signal used only for existence checking.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

async fn probe_health(url: &str) -> Result<bool, ()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|_| ())?;

    let resp = client.get(url).send().await.map_err(|_| ())?;
    Ok(resp.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_pid_lock_path_is_in_base_dir() {
        let path = gateway_pid_lock_path();
        assert!(
            path.ends_with("gateway.pid"),
            "expected gateway.pid, got: {}",
            path.display()
        );
    }

    #[test]
    fn gateway_log_path_is_in_base_dir() {
        let path = gateway_log_path();
        assert!(
            path.ends_with("gateway.log"),
            "expected gateway.log, got: {}",
            path.display()
        );
    }

    #[test]
    fn stop_reports_missing_pid_file() {
        // With no gateway.pid, stop should fail with a clear message
        let result = cmd_stop();
        // It may or may not fail depending on whether a gateway is running,
        // but it should not panic.
        match result {
            Ok(()) => {} // gateway was running and got SIGTERM
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("not running") || msg.contains("PID"),
                    "expected descriptive error, got: {msg}"
                );
            }
        }
    }

    #[test]
    fn is_process_alive_for_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid), "current process should be alive");
    }

    #[test]
    fn is_process_alive_for_nonexistent_pid() {
        // PID 4_000_000 is extremely unlikely to exist
        assert!(
            !is_process_alive(4_000_000),
            "nonexistent PID should not be alive"
        );
    }

    #[test]
    fn pid_lock_prevents_double_start() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pid_path = dir.path().join("gateway.pid");

        let lock1 = PidLock::acquire_at(pid_path.clone());
        assert!(lock1.is_ok(), "first lock should succeed");

        let lock2 = PidLock::acquire_at(pid_path);
        assert!(lock2.is_err(), "second lock should fail");

        let err = lock2.unwrap_err().to_string();
        assert!(
            err.contains("already running"),
            "expected 'already running' in error, got: {err}"
        );
    }

    #[test]
    fn extract_auth_token_from_log_finds_token() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = dir.path().join("gateway.log");
        std::fs::write(
            &log,
            "2026-03-12T16:03:30.100Z  INFO Starting standalone gateway...\n\
             Gateway running at http://127.0.0.1:3000/\n\
             Auth token: abc123def456\n\
             Web UI: http://127.0.0.1:3000/?token=abc123def456\n",
        )
        .expect("write test log");

        assert_eq!(
            extract_auth_token_from_log(&log),
            Some("abc123def456".to_string())
        );
    }

    #[test]
    fn extract_auth_token_from_log_returns_none_when_missing() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = dir.path().join("gateway.log");
        std::fs::write(&log, "some other log output\n").expect("write test log");

        assert_eq!(extract_auth_token_from_log(&log), None);
    }

    #[test]
    fn extract_auth_token_from_log_returns_none_for_nonexistent_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = dir.path().join("nonexistent.log");

        assert_eq!(extract_auth_token_from_log(&log), None);
    }
}
