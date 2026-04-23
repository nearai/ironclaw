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
//! settings, skills, extensions, logs). Endpoints that require the agent loop
//! (chat send/ws, routine trigger, job restart/cancel/prompt) return 503. For
//! the full runtime, use `ironclaw run`.

use std::io::IsTerminal;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use clap::Subcommand;

use crate::app::{AppBuilder, AppBuilderFlags};
use crate::bootstrap::{PidLock, gateway_log_path, gateway_pid_lock_path, gateway_token_path};
use crate::channels::GatewayChannel;
use crate::channels::web::log_layer::{LogBroadcaster, init_tracing};
use crate::channels::web::platform::{router as gateway_router, state::ActiveConfigSnapshot};
use crate::config::Config;
use crate::llm::create_session_manager;

/// Maximum time to wait for the background gateway to become healthy.
const START_HEALTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Polling interval when waiting for health check.
const START_HEALTH_POLL: std::time::Duration = std::time::Duration::from_millis(300);

/// Maximum time to wait for graceful shutdown before aborting the server task.
const GRACEFUL_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Subcommand, Debug, Clone)]
pub enum GatewayCommand {
    /// Run the web gateway in the foreground (Ctrl-C to stop).
    Serve,

    /// Start the web gateway as a background daemon.
    Start,

    /// Stop a running background gateway.
    Stop,

    /// Show standalone gateway status.
    Status,
}

/// Run the gateway CLI subcommand.
pub async fn run_gateway_command(
    cmd: GatewayCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
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

async fn cmd_serve(config_path: Option<&Path>) -> anyhow::Result<()> {
    let config = Config::from_env_with_toml(config_path).await?;
    let gw_config = config
        .channels
        .gateway
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Gateway is not enabled. Set GATEWAY_ENABLED=true"))?;
    let _validated_addr = parse_gateway_addr(&gw_config.host, gw_config.port)?;

    let _pid_lock = PidLock::acquire_at(gateway_pid_lock_path())
        .map_err(|e| anyhow::anyhow!("Cannot start gateway: {e}"))?;

    let log_broadcaster = Arc::new(LogBroadcaster::new());
    let log_level_handle = init_tracing(Arc::clone(&log_broadcaster), false);

    tracing::info!("Starting standalone gateway...");

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

    let runtime_config = &components.config;
    let gw_config = runtime_config
        .channels
        .gateway
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Gateway is not enabled. Set GATEWAY_ENABLED=true"))?;

    let mut gw = GatewayChannel::new(gw_config.clone(), runtime_config.owner_id.clone());
    gw = gw.with_multi_tenant_mode(runtime_config.is_multi_tenant_deployment());
    gw = gw.with_llm_provider(Arc::clone(&components.llm));
    if let Some(ref ws) = components.workspace {
        gw = gw.with_workspace(Arc::clone(ws));
    }
    if let Some(ref db) = components.db {
        gw = gw.with_db_backing_from_config(
            runtime_config,
            Arc::clone(db),
            components.embeddings.clone(),
        );
    }
    gw = gw.with_session_manager(Arc::clone(&components.agent_session_manager));
    gw = gw.with_llm_session_manager(Arc::clone(&components.session));
    if let Some(ref reload) = components.llm_reload {
        gw = gw.with_llm_reload(Arc::clone(reload));
    }
    if let Some(config_path) = config_path {
        gw = gw.with_config_toml_path(std::path::PathBuf::from(config_path));
    }
    gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
    gw = gw.with_log_level_handle(Arc::clone(&log_level_handle));
    gw = gw.with_tool_registry(Arc::clone(&components.tools));
    if let Some(ref db) = components.db {
        let dispatcher = Arc::new(crate::tools::dispatch::ToolDispatcher::new(
            Arc::clone(&components.tools),
            Arc::clone(&components.safety),
            Arc::clone(db),
        ));
        gw = gw.with_tool_dispatcher(dispatcher);
    }
    if let Some(ref ext_mgr) = components.extension_manager {
        let gw_base = runtime_config
            .tunnel
            .public_url
            .clone()
            .unwrap_or_else(|| oauth_base_url(&gw_config.host, gw_config.port));
        ext_mgr.enable_gateway_mode(gw_base).await;
        gw = gw.with_extension_manager(Arc::clone(ext_mgr));
    }
    if !components.catalog_entries.is_empty() {
        gw = gw.with_registry_entries(components.catalog_entries.clone());
    }
    if let Some(ref db) = components.db {
        gw = gw.with_store(Arc::clone(db));
        if let Some(ref settings_cache) = components.settings_cache {
            gw = gw.with_settings_cache(Arc::clone(settings_cache));
        }
        gw = gw.with_db_auth(Arc::clone(db));
        if let Some(ref secrets_store) = components.secrets_store {
            gw = gw.with_secrets_store(Arc::clone(secrets_store));
        }
    }
    if let Some(ref skill_registry) = components.skill_registry {
        gw = gw.with_skill_registry(Arc::clone(skill_registry));
    }
    if let Some(ref skill_catalog) = components.skill_catalog {
        gw = gw.with_skill_catalog(Arc::clone(skill_catalog));
    }
    gw = gw.with_cost_guard(Arc::clone(&components.cost_guard));
    gw = gw.with_oauth(runtime_config.oauth.clone(), gw_config.port);
    gw = gw.with_active_config(ActiveConfigSnapshot {
        llm_backend: runtime_config.llm.backend.to_string(),
        llm_model: components.llm.model_name().to_string(),
        enabled_channels: vec!["gateway".to_string()],
        default_timezone: runtime_config.agent.default_timezone.clone(),
    });

    let addr = parse_gateway_addr(&gw_config.host, gw_config.port)?;

    let state = Arc::clone(gw.state());
    let auth_token = gw.auth_token().to_string();
    let (bound_addr, server_handle) =
        gateway_router::start_server(addr, Arc::clone(&state), gw.auth().clone()).await?;
    let base_url = format_http_base_url(&gw_config.host, bound_addr.port());

    let token_path = gateway_token_path();
    if let Err(e) = write_gateway_token_file(&token_path, &auth_token) {
        tracing::warn!("Failed to write token file {}: {e}", token_path.display());
    }

    println!("Gateway running at {base_url}/");
    if std::io::stdout().is_terminal() {
        println!("Auth token: {auth_token}");
        println!("Web UI: {base_url}/?token={auth_token}");
    } else {
        println!("Auth token written to {}", token_path.display());
    }
    println!();
    println!("Standalone mode: chat endpoints return 503 (no agent loop).");
    println!("Press Ctrl-C to stop.");

    wait_for_shutdown_signal().await;

    tracing::info!("Shutting down gateway...");
    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(());
    }

    let mut server_handle = server_handle;
    tokio::select! {
        result = &mut server_handle => {
            if let Err(error) = result {
                tracing::warn!(%error, "Gateway server task exited during shutdown");
            }
        }
        _ = tokio::time::sleep(GRACEFUL_SHUTDOWN_TIMEOUT) => {
            tracing::warn!(
                timeout_secs = GRACEFUL_SHUTDOWN_TIMEOUT.as_secs(),
                "Gateway server did not shut down in time; aborting task"
            );
            server_handle.abort();
            let _ = server_handle.await;
        }
    }

    cleanup_gateway_token_file(&token_path);
    Ok(())
}

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

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(auth_token.as_bytes())?;
        drop(file);

        restrict_windows_file_permissions(path);
    }

    #[cfg(not(any(unix, windows)))]
    {
        std::fs::write(path, auth_token)?;
    }

    Ok(())
}

#[cfg(windows)]
fn restrict_windows_file_permissions(path: &Path) {
    let username = std::env::var("USERNAME").unwrap_or_default();
    if username.is_empty() {
        tracing::warn!("Cannot restrict token file permissions: %USERNAME% not set");
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

fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        restrict_unix_file_permissions(path)?;
        Ok(file)
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

#[cfg(unix)]
fn restrict_unix_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

fn cleanup_gateway_token_file(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("Failed to remove token file {}: {e}", path.display());
    }
}

async fn cmd_start(config_path: Option<&Path>) -> anyhow::Result<()> {
    let config = Config::from_env_with_toml(config_path).await?;
    let gw_config = config
        .channels
        .gateway
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Gateway is not enabled. Set GATEWAY_ENABLED=true"))?;
    if gw_config.port == 0 {
        anyhow::bail!(
            "gateway start does not support port 0; set a fixed GATEWAY_PORT or use `gateway serve`"
        );
    }
    let _validated_addr = parse_gateway_addr(&gw_config.host, gw_config.port)?;

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
    // Args are passed directly to the child process — no shell involved.
    cmd.arg("gateway").arg("serve");
    if let Some(cp) = config_path {
        cmd.arg("--config").arg(cp);
    }
    cmd.stdout(log_file)
        .stderr(stderr_file)
        .stdin(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn gateway process: {e}"))?;

    let child_pid = child.id();
    println!("Gateway starting in background (PID {child_pid})...");
    println!("Log file: {}", log_path.display());

    let health_url = format!(
        "{}/api/health",
        format_http_base_url(&gw_config.host, gw_config.port)
    );
    let deadline = std::time::Instant::now() + START_HEALTH_TIMEOUT;
    let mut healthy = false;
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| anyhow::anyhow!("Cannot create HTTP client: {e}"))?;

    while std::time::Instant::now() < deadline {
        tokio::time::sleep(START_HEALTH_POLL).await;

        match child.try_wait() {
            Ok(Some(status)) => {
                anyhow::bail!(
                    "Gateway process exited with {status}. Check logs: {}",
                    log_path.display()
                );
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("Failed to check child process status: {e}");
            }
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
        let base_url = format_http_base_url(&gw_config.host, gw_config.port);
        println!("Gateway is running at {base_url}/");

        let token_path = gateway_token_path();
        if let Ok(token) = std::fs::read_to_string(&token_path) {
            let token = token.trim();
            if !token.is_empty() {
                println!("Auth token: {token}");
                println!("Web UI: {base_url}/?token={token}");
            }
        } else {
            println!(
                "Auth token: could not read {}; the gateway may still be starting",
                token_path.display()
            );
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

        if !is_pid_lock_held(&pid_path) {
            let _ = std::fs::remove_file(&pid_path);
            cleanup_gateway_token_file(&gateway_token_path());
            anyhow::bail!("Gateway process (PID {pid}) is not running (stale PID file removed)");
        }

        if !is_process_alive(pid) {
            let _ = std::fs::remove_file(&pid_path);
            cleanup_gateway_token_file(&gateway_token_path());
            anyhow::bail!("Gateway process (PID {pid}) is not running (stale PID file removed)");
        }

        if !is_pid_lock_held(&pid_path) {
            let _ = std::fs::remove_file(&pid_path);
            cleanup_gateway_token_file(&gateway_token_path());
            anyhow::bail!(
                "Gateway process (PID {pid}) no longer holds the lock file (stale PID file removed)"
            );
        }

        let pid_t = to_pid_t(pid).ok_or_else(|| anyhow::anyhow!("PID {pid} overflows i32"))?;
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

        let stop_deadline = std::time::Instant::now() + GRACEFUL_SHUTDOWN_TIMEOUT;
        while std::time::Instant::now() < stop_deadline {
            if !is_process_alive(pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        if pid_path.exists() && !is_process_alive(pid) {
            let _ = std::fs::remove_file(&pid_path);
        }

        if is_process_alive(pid) {
            println!(
                "Warning: process (PID {pid}) still running after {}s. You may need to `kill -9 {pid}`.",
                GRACEFUL_SHUTDOWN_TIMEOUT.as_secs()
            );
        } else {
            cleanup_gateway_token_file(&gateway_token_path());
            println!("Gateway stopped.");
        }

        Ok(())
    }
}

async fn cmd_status(config_path: Option<&Path>) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let pid_path = gateway_pid_lock_path();
        match std::fs::read_to_string(&pid_path) {
            Ok(pid_str) => match pid_str.trim().parse::<u32>() {
                Ok(pid) => {
                    if is_process_alive(pid) && is_pid_lock_held(&pid_path) {
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

    let config = Config::from_env_with_toml(config_path).await.ok();
    let probe_addr = config
        .as_ref()
        .and_then(|c| c.channels.gateway.as_ref())
        .map(|gw| {
            let addr = format_http_authority(&gw.host, gw.port);
            let url = format_http_base_url(&gw.host, gw.port);
            (addr, url)
        });

    if let Some((addr, base_url)) = probe_addr {
        println!("Address: {addr}");

        let url = format!("{base_url}/api/health");
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

#[cfg(unix)]
fn is_pid_lock_held(path: &Path) -> bool {
    use fs4::FileExt;
    let Ok(file) = std::fs::OpenOptions::new().read(true).open(path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
        Err(e) => {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "Failed to inspect PID lock state"
            );
            false
        }
    }
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Some(pid_t) = to_pid_t(pid) else {
            return false;
        };
        unsafe { libc::kill(pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
fn to_pid_t(pid: u32) -> Option<libc::pid_t> {
    libc::pid_t::try_from(pid).ok()
}

fn normalize_probe_host(host: &str) -> &str {
    match host {
        "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other => other,
    }
}

fn format_http_host(host: &str) -> String {
    let normalized = normalize_probe_host(host);
    let trimmed = normalized.trim_start_matches('[').trim_end_matches(']');
    match trimmed.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{trimmed}]"),
        _ => trimmed.to_string(),
    }
}

fn format_http_authority(host: &str, port: u16) -> String {
    format!("{}:{port}", format_http_host(host))
}

fn format_http_base_url(host: &str, port: u16) -> String {
    format!("http://{}", format_http_authority(host, port))
}

async fn probe_health(client: &reqwest::Client, url: &str) -> Result<bool, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("{e}"))?;
    Ok(resp.status().is_success())
}

fn parse_gateway_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    let authority = match trimmed.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{trimmed}]:{port}"),
        _ => format!("{trimmed}:{port}"),
    };
    authority
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid gateway address '{host}:{port}': {e}"))
}

fn oauth_base_url(host: &str, port: u16) -> String {
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    match trimmed.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) if ip.is_unspecified() => format!("http://localhost:{port}"),
        Ok(IpAddr::V6(ip)) if ip.is_unspecified() => format!("http://localhost:{port}"),
        Ok(IpAddr::V6(_)) => format!("http://[{trimmed}]:{port}"),
        Ok(IpAddr::V4(_)) => format!("http://{trimmed}:{port}"),
        Err(_) => format!("http://{host}:{port}"),
    }
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
    fn gateway_token_path_is_in_base_dir() {
        let path = gateway_token_path();
        assert!(
            path.ends_with("gateway.token"),
            "expected gateway.token, got: {}",
            path.display()
        );
    }

    #[test]
    fn normalize_probe_host_rewrites_unspecified() {
        assert_eq!(normalize_probe_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(normalize_probe_host("::"), "127.0.0.1");
        assert_eq!(normalize_probe_host("[::]"), "127.0.0.1");
        assert_eq!(normalize_probe_host("10.0.0.1"), "10.0.0.1");
    }

    #[test]
    fn oauth_base_url_maps_unspecified_to_localhost() {
        assert_eq!(oauth_base_url("0.0.0.0", 3033), "http://localhost:3033");
        assert_eq!(oauth_base_url("::", 3033), "http://localhost:3033");
        assert_eq!(oauth_base_url("[::]", 3033), "http://localhost:3033");
    }

    #[test]
    fn format_http_base_url_handles_ipv6_and_wildcards() {
        assert_eq!(
            format_http_base_url("0.0.0.0", 3000),
            "http://127.0.0.1:3000"
        );
        assert_eq!(format_http_base_url("::", 3000), "http://127.0.0.1:3000");
        assert_eq!(format_http_base_url("::1", 3000), "http://[::1]:3000");
        assert_eq!(format_http_base_url("[::1]", 3000), "http://[::1]:3000");
    }

    #[test]
    fn parse_gateway_addr_accepts_bracketed_and_unbracketed_ipv6() {
        assert!(parse_gateway_addr("::1", 3000).is_ok());
        assert!(parse_gateway_addr("[::1]", 3000).is_ok());
    }

    #[test]
    fn token_file_round_trip() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let token_path = dir.path().join("gateway.token");
        let token = "abc123def456";
        write_gateway_token_file(&token_path, token).expect("write token");
        let read_back = std::fs::read_to_string(&token_path).expect("read token");
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

    #[test]
    #[cfg(unix)]
    fn open_log_file_tightens_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("create temp dir");
        let log_path = dir.path().join("gateway.log");
        std::fs::write(&log_path, "old log").expect("seed log file");
        std::fs::set_permissions(&log_path, std::fs::Permissions::from_mode(0o644))
            .expect("loosen permissions");

        let _file = open_log_file(&log_path).expect("open log file");

        let mode = std::fs::metadata(&log_path)
            .expect("log metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn pid_lock_prevents_double_start() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pid_path = dir.path().join("gateway.pid");

        let lock1 = PidLock::acquire_at(pid_path.clone());
        assert!(lock1.is_ok(), "first lock should succeed");

        let lock2 = PidLock::acquire_at(pid_path);
        assert!(lock2.is_err(), "second lock should fail");
    }
}
