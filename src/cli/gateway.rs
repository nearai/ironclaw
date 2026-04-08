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

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use clap::Subcommand;

use crate::app::{AppBuilder, AppBuilderFlags};
use crate::bootstrap::{PidLock, gateway_log_path, gateway_pid_lock_path, gateway_token_path};
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
    // Start and stop rely on Unix process signals (kill, setsid).
    // Status uses the health probe (cross-platform) but skips PID checks on non-Unix.
    #[cfg(not(unix))]
    match cmd {
        GatewayCommand::Serve | GatewayCommand::Status => {}
        GatewayCommand::Start => anyhow::bail!("`gateway start` is currently Unix-only"),
        GatewayCommand::Stop => anyhow::bail!("`gateway stop` is currently Unix-only"),
    }

    match cmd {
        GatewayCommand::Serve => cmd_serve(config_path).await,
        GatewayCommand::Start => cmd_start(config_path).await,
        GatewayCommand::Stop => cmd_stop().await,
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

    // Capture values before config is moved into AppBuilder.
    let owner_id = config.owner_id.clone();
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

    // Wire the GatewayChannel via shared factory — no more manual duplication
    // with main.rs. Agent-loop-only features (scheduler, job_manager, etc.)
    // are intentionally omitted; those APIs return 503 in standalone mode.
    //
    // Note: `enable_db_auth: true` causes `from_components()` to bootstrap an
    // admin user if none exist. This is intentional — standalone mode needs a
    // DB user so the Web UI auth works. The pairing store is intentionally NOT
    // wired; pairing endpoints return 503 (handlers check the Option).
    let session_manager =
        Arc::new(crate::agent::SessionManager::new().with_hooks(components.hooks.clone()));
    let gateway_base_url = tunnel_public_url
        .unwrap_or_else(|| format!("http://{}:{}", gw_config.host, gw_config.port));
    // Build workspace pool for multi-user isolation if DB is available.
    let workspace_pool = components.create_workspace_pool();

    let gw = GatewayChannel::from_components(
        gw_config.clone(),
        owner_id,
        crate::channels::web::GatewayComponents {
            llm: Arc::clone(&components.llm),
            workspace: components.workspace.clone(),
            session_manager,
            log_broadcaster: Arc::clone(&log_broadcaster),
            log_level_handle: Arc::clone(&log_level_handle),
            tools: Arc::clone(&components.tools),
            extension_manager: components.extension_manager.clone(),
            catalog_entries: components.catalog_entries.clone(),
            db: components.db.clone(),
            skill_registry: components.skill_registry.clone(),
            skill_catalog: components.skill_catalog.clone(),
            cost_guard: Arc::clone(&components.cost_guard),
            secrets_store: components.secrets_store.clone(),
            gateway_base_url: Some(gateway_base_url),
            workspace_pool,
            enable_db_auth: true,
        },
    )
    .await;

    // Start the HTTP server directly — do NOT call Channel::start() so
    // msg_tx stays None and chat send/ws naturally return 503.
    let addr: SocketAddr = format!("{}:{}", gw_config.host, gw_config.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid gateway address: {e}"))?;

    let state = Arc::clone(gw.state());
    let auth_token = gw.auth_token().to_string();

    let (bound_addr, server_handle) =
        server::start_server(addr, Arc::clone(&state), gw.auth().clone()).await?;

    // Write auth token to a dedicated file so `gateway start` can read it
    // without parsing log output (which is brittle if tracing interleaves).
    let token_path = gateway_token_path();
    if let Err(e) = write_gateway_token_file(&token_path, &auth_token) {
        tracing::warn!("Failed to write token file {}: {e}", token_path.display());
    }

    println!("Gateway running at http://{bound_addr}/");
    // Only print the auth token when stdout is a real terminal. When
    // `gateway start` redirects stdout to gateway.log, suppress the token
    // to avoid writing credentials to the log file in plaintext. The token
    // is always available via the dedicated token file (gateway.token).
    if std::io::stdout().is_terminal() {
        println!("Auth token: {auth_token}");
        println!("Web UI: http://{bound_addr}/?token={auth_token}");
    } else {
        println!("Auth token written to {}", token_path.display());
    }
    println!();
    println!("Standalone mode: chat endpoints return 503 (no agent loop).");
    println!("Press Ctrl-C to stop.");

    // Wait for shutdown signal, then bridge to axum's graceful shutdown.
    wait_for_shutdown_signal().await;

    tracing::info!("Shutting down gateway...");
    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(());
    }

    // Await the server task so in-flight requests can drain gracefully.
    let _ = server_handle.await;

    // Clean up token file and PID lock.
    cleanup_gateway_token_file(&token_path);
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

fn write_gateway_token_file(path: &Path, auth_token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(auth_token.as_bytes())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::io::Write;

        // Write the file first.
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(auth_token.as_bytes())?;
        drop(file);

        // Restrict to owner-only via Windows ACLs: remove inherited ACEs and
        // grant only the current user full control.
        restrict_windows_file_permissions(path);
    }

    #[cfg(not(any(unix, windows)))]
    {
        std::fs::write(path, auth_token)?;
    }

    Ok(())
}

/// Best-effort restriction of file permissions on Windows using `icacls`.
///
/// Removes inherited ACEs and grants only the current user full control,
/// mirroring the Unix `chmod 0600` behaviour. Failures are logged as
/// warnings but do not prevent gateway startup.
#[cfg(windows)]
fn restrict_windows_file_permissions(path: &Path) {
    // `icacls <path> /inheritance:r /grant:r "%USERNAME%:F"` removes inherited
    // permissions and grants only the current user full control.
    let username = std::env::var("USERNAME").unwrap_or_default();
    if username.is_empty() {
        tracing::warn!(
            "Cannot restrict token file permissions: %USERNAME% not set"
        );
        return;
    }

    let path_str = path.to_string_lossy();
    let grant_arg = format!("{username}:F");
    let result = std::process::Command::new("icacls")
        .args([path_str.as_ref(), "/inheritance:r", "/grant:r", &grant_arg])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => {}
        Ok(status) => {
            tracing::warn!(
                "icacls returned non-zero ({}) for {}; token file may be world-readable",
                status,
                path.display()
            );
        }
        Err(e) => {
            tracing::warn!(
                "Failed to run icacls for {}: {e}; token file may be world-readable",
                path.display()
            );
        }
    }
}

/// Open the gateway log file in append mode. On Unix, restrict to owner-only
/// (0600) because the auth token is printed to stdout which lands in this file
/// when `gateway start` redirects output.
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(windows)]
    {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        restrict_windows_file_permissions(path);
        Ok(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    }
}

fn cleanup_gateway_token_file(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("Failed to remove token file {}: {e}", path.display());
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

    // Best-effort pre-flight check: try to acquire the PID lock to detect
    // an already-running instance. This is NOT authoritative — there is a
    // TOCTOU window between this check and the child's PidLock::acquire_at
    // in `cmd_serve`. The child's lock is the real mutual exclusion point.
    // If two `gateway start` commands race, the loser's child will fail to
    // acquire the lock and exit, which the parent detects via health-check
    // timeout or process exit.
    let pid_path = gateway_pid_lock_path();
    match PidLock::acquire_at(pid_path.clone()) {
        Ok(lock) => {
            // Lock acquired — no other instance holds it. Release immediately
            // so the child can acquire it.
            drop(lock);
        }
        Err(crate::bootstrap::PidLockError::AlreadyRunning { pid }) => {
            anyhow::bail!("Gateway is already running (PID {pid})");
        }
        Err(crate::bootstrap::PidLockError::Io(e)) => {
            anyhow::bail!("Cannot check gateway PID lock: {e}");
        }
    }

    // Spawn `ironclaw gateway serve` as a detached child process.
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Cannot determine own executable path: {e}"))?;

    let log_path = gateway_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Cannot create directory {}: {e}", parent.display()))?;
    }
    let log_file = open_log_file(&log_path)
        .map_err(|e| anyhow::anyhow!("Cannot open log file {}: {e}", log_path.display()))?;
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
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
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
    // Normalize unspecified bind addresses to localhost for probing.
    let health_host = normalize_probe_host(&gw_config.host);
    let health_url = format!("http://{health_host}:{}/api/health", gw_config.port);
    let deadline = std::time::Instant::now() + START_HEALTH_TIMEOUT;
    let mut healthy = false;
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| anyhow::anyhow!("Cannot create HTTP client: {e}"))?;

    while std::time::Instant::now() < deadline {
        tokio::time::sleep(START_HEALTH_POLL).await;

        // Also verify the child hasn't crashed.
        if !is_process_alive(child_pid) {
            anyhow::bail!(
                "Gateway process exited unexpectedly. Check logs: {}",
                log_path.display()
            );
        }

        if probe_health(&http_client, &health_url)
            .await
            .unwrap_or(false)
        {
            healthy = true;
            break;
        }
    }

    if healthy {
        let base_url = format!("http://{}:{}", gw_config.host, gw_config.port);
        println!("Gateway is running at {base_url}/");

        // Read auth token from the dedicated token file written by `serve`.
        let token_path = gateway_token_path();
        if let Ok(token) = std::fs::read_to_string(&token_path) {
            let token = token.trim();
            if !token.is_empty() {
                println!("Auth token: {token}");
                println!("Web UI: {base_url}/?token={token}");
            }
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

// ─── stop ────────────────────────────────────────────────────────

async fn cmd_stop() -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        anyhow::bail!("gateway stop is currently Unix-only");
    }

    #[cfg(unix)]
    {
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

        // Verify the gateway still holds the flock on the PID file. This is
        // the authoritative ownership signal — `is_process_alive` alone can
        // return false positives if the OS has recycled the PID to an
        // unrelated process.
        if !is_pid_lock_held(&pid_path) {
            // No flock held — the gateway has exited and the PID may have
            // been reused. Clean up the stale PID file instead of signalling.
            let _ = std::fs::remove_file(&pid_path);
            cleanup_gateway_token_file(&gateway_token_path());
            anyhow::bail!("Gateway process (PID {pid}) is not running (stale PID file removed)");
        }

        if !is_process_alive(pid) {
            // Flock held but process not found — theoretically impossible,
            // but handle gracefully (kernel should release flock on exit).
            let _ = std::fs::remove_file(&pid_path);
            cleanup_gateway_token_file(&gateway_token_path());
            anyhow::bail!("Gateway process (PID {pid}) is not running (stale PID file removed)");
        }

        let pid_t = to_pid_t(pid).ok_or_else(|| anyhow::anyhow!("PID {pid} overflows i32"))?;
        // SAFETY: We are sending a signal to a process identified by our PID
        // file whose flock we verified above.
        let ret = unsafe { libc::kill(pid_t, libc::SIGTERM) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                let _ = std::fs::remove_file(&pid_path);
                cleanup_gateway_token_file(&gateway_token_path());
                anyhow::bail!(
                    "Gateway process (PID {pid}) exited before SIGTERM could be delivered (stale PID file removed)"
                );
            }
            anyhow::bail!("Failed to send SIGTERM to PID {pid}: {err}");
        }
        println!("Sent SIGTERM to gateway (PID {pid}).");

        // Poll for process exit, then clean up the PID file if the serve
        // process didn't remove it (e.g. killed by SIGKILL or OOM).
        let stop_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < stop_deadline {
            if !is_process_alive(pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // If the PID file still exists and the process has exited, remove it.
        if pid_path.exists() && !is_process_alive(pid) {
            let _ = std::fs::remove_file(&pid_path);
        }

        if is_process_alive(pid) {
            println!(
                "Warning: process (PID {pid}) still running after 5s. You may need to `kill -9 {pid}`."
            );
        } else {
            cleanup_gateway_token_file(&gateway_token_path());
            println!("Gateway stopped.");
        }

        Ok(())
    }
}

// ─── status ──────────────────────────────────────────────────────

async fn cmd_status(config_path: Option<&Path>) -> anyhow::Result<()> {
    // PID-based liveness check is Unix-only; on other platforms we skip
    // straight to the health probe.
    #[cfg(unix)]
    {
        let pid_path = gateway_pid_lock_path();
        match std::fs::read_to_string(&pid_path) {
            Ok(pid_str) => match pid_str.trim().parse::<u32>() {
                Ok(pid) => {
                    if is_process_alive(pid) {
                        println!("Gateway is running (PID {pid}).");
                    } else {
                        println!("Gateway is not running (stale PID {pid}).");
                        return Ok(());
                    }
                }
                Err(_) => {
                    println!("Gateway PID file exists but is invalid.");
                    return Ok(());
                }
            },
            Err(_) => {
                println!("No PID file found.");
            }
        }
    }

    // Health probe — works on all platforms.
    let config = Config::from_env_with_toml(config_path).await.ok();
    let probe_addr = config
        .as_ref()
        .and_then(|c| c.channels.gateway.as_ref())
        .map(|gw| {
            let host = normalize_probe_host(&gw.host);
            format!("{host}:{}", gw.port)
        });

    if let Some(ref addr) = probe_addr {
        println!("Address: {addr}");

        // Probe /api/health (unauthenticated endpoint)
        let url = format!("http://{addr}/api/health");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build();
        match client {
            Ok(c) => match probe_health(&c, &url).await {
                Ok(true) => println!("Health:  ok"),
                Ok(false) => println!("Health:  unhealthy"),
                Err(reason) => println!("Health:  unreachable ({reason})"),
            },
            Err(e) => println!("Health:  cannot create client ({e})"),
        }
    }

    Ok(())
}

/// Check whether the PID file's exclusive flock is currently held by another
/// process.
///
/// Returns `true` if the lock IS held (meaning the gateway is still running),
/// `false` if we can acquire it (meaning the gateway has exited and the PID
/// may be stale/reused).
#[cfg(unix)]
fn is_pid_lock_held(path: &Path) -> bool {
    use fs4::FileExt;
    let Ok(file) = std::fs::OpenOptions::new().read(true).open(path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            // We acquired the lock — nobody else holds it. Release immediately.
            let _ = file.unlock();
            false
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            // Lock held by the gateway process.
            true
        }
        Err(_) => {
            // Other I/O error (permissions, etc.) — assume not held.
            false
        }
    }
}

/// Check if a process is alive by sending signal 0.
///
/// **PID reuse caveat:** `kill(pid, 0)` only checks that *some* process with
/// that PID exists. After the original gateway exits, the OS may reassign
/// the PID to an unrelated process. Callers (especially `cmd_stop`) should
/// be aware that a `true` return does not guarantee the process is *our*
/// gateway. The `PidLock` flock is the authoritative ownership signal.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Some(pid_t) = to_pid_t(pid) else {
            return false;
        };
        // SAFETY: Signal 0 is a null signal used only for existence checking.
        unsafe { libc::kill(pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Safely convert a u32 PID to libc::pid_t (i32), returning None on overflow.
#[cfg(unix)]
fn to_pid_t(pid: u32) -> Option<libc::pid_t> {
    libc::pid_t::try_from(pid).ok()
}

/// Normalize a bind host for health probing. Unspecified addresses like
/// `0.0.0.0` or `::` cannot be connected to; use `127.0.0.1` instead.
fn normalize_probe_host(host: &str) -> &str {
    match host {
        "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other => other,
    }
}

async fn probe_health(client: &reqwest::Client, url: &str) -> Result<bool, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("{e}"))?;
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
    fn normalize_probe_host_rewrites_unspecified() {
        assert_eq!(normalize_probe_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(normalize_probe_host("::"), "127.0.0.1");
        assert_eq!(normalize_probe_host("[::]"), "127.0.0.1");
        assert_eq!(normalize_probe_host("10.0.0.1"), "10.0.0.1");
        assert_eq!(normalize_probe_host("127.0.0.1"), "127.0.0.1");
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
    #[cfg(unix)]
    fn is_pid_lock_held_returns_true_when_locked() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pid_path = dir.path().join("gateway.pid");

        // Acquire a PidLock — this holds the flock.
        let _lock = PidLock::acquire_at(pid_path.clone()).expect("acquire lock");

        // is_pid_lock_held should see the lock as held.
        assert!(
            is_pid_lock_held(&pid_path),
            "expected lock to be reported as held"
        );
    }

    #[test]
    #[cfg(unix)]
    fn is_pid_lock_held_returns_false_when_unlocked() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pid_path = dir.path().join("gateway.pid");

        // Write a file but do NOT hold a flock.
        std::fs::write(&pid_path, "12345").expect("write pid file");

        assert!(
            !is_pid_lock_held(&pid_path),
            "expected lock to be reported as not held"
        );
    }

    #[test]
    fn gateway_token_path_is_in_base_dir() {
        let path = gateway_token_path();
        assert!(
            path.ends_with("gateway.token"),
            "expected gateway.token, got: {}",
            path.display()
        );
    }

    #[test]
    fn token_file_round_trip() {
        let dir = tempfile::tempdir().expect("create temp dir"); // safety: test
        let token_path = dir.path().join("gateway.token");
        let token = "abc123def456";
        write_gateway_token_file(&token_path, token).expect("write token"); // safety: test
        let read_back = std::fs::read_to_string(&token_path).expect("read token"); // safety: test
        assert_eq!(read_back.trim(), token);
    }

    #[test]
    #[cfg(unix)]
    fn token_file_permissions_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("create temp dir");
        let token_path = dir.path().join("gateway.token");

        write_gateway_token_file(&token_path, "secret").expect("write token");

        let mode = std::fs::metadata(&token_path)
            .expect("token metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
