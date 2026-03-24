//! Tunnel abstraction for exposing the agent to the internet.
//!
//! Wraps external tunnel binaries (cloudflared, ngrok, tailscale, etc.) behind
//! a common trait. The gateway starts a tunnel after binding its local port
//! and stops it on shutdown.
//!
//! Supported providers:
//! - **cloudflare** - Zero Trust tunnels via `cloudflared`
//! - **tailscale** - `tailscale serve` (tailnet) or `tailscale funnel` (public)
//! - **ngrok** - instant public URLs via `ngrok`
//! - **custom** - any command with `{host}`/`{port}` placeholders
//! - **none** - local-only, no external exposure

mod cloudflare;
mod custom;
mod ngrok;
mod none;
mod tailscale;

pub use cloudflare::CloudflareTunnel;
pub use custom::CustomTunnel;
pub use ngrok::NgrokTunnel;
pub use none::NoneTunnel;
pub use tailscale::TailscaleTunnel;

use std::sync::Arc;

use anyhow::{Result, bail};
use tokio::sync::Mutex;

/// Lock-free URL storage. Uses `std::sync::RwLock` so `public_url()` (sync)
/// never returns a spurious `None` due to async lock contention.
pub(crate) type SharedUrl = Arc<std::sync::RwLock<Option<String>>>;

pub(crate) fn new_shared_url() -> SharedUrl {
    Arc::new(std::sync::RwLock::new(None))
}

// ── Tunnel trait ─────────────────────────────────────────────────

/// Provider-agnostic tunnel with lifecycle management.
///
/// Implementations wrap an external tunnel binary. The gateway calls
/// `start()` after binding its local port and `stop()` on shutdown.
#[async_trait::async_trait]
pub trait Tunnel: Send + Sync {
    /// Human-readable provider name (e.g. "cloudflare", "tailscale").
    fn name(&self) -> &str;

    /// Start the tunnel exposing `local_host:local_port` externally.
    /// Returns the public URL on success.
    async fn start(&self, local_host: &str, local_port: u16) -> Result<String>;

    /// Stop the tunnel process gracefully.
    async fn stop(&self) -> Result<()>;

    /// Check if the tunnel process is still alive.
    async fn health_check(&self) -> bool;

    /// Return the public URL if the tunnel is running, `None` otherwise.
    fn public_url(&self) -> Option<String>;
}

// ── Shared child-process handle ──────────────────────────────────

/// Wraps a spawned tunnel child process.
pub(crate) struct TunnelProcess {
    pub child: tokio::process::Child,
    /// Background task that drains the process's output pipe (stdout or stderr).
    /// Must stay alive or the process dies (SIGPIPE from closed pipe) or hangs
    /// (OS pipe buffer fills up, blocking the process's writes).
    pub _pipe_drain: Option<tokio::task::JoinHandle<()>>,
}

pub(crate) type SharedProcess = Arc<Mutex<Option<TunnelProcess>>>;

pub(crate) fn new_shared_process() -> SharedProcess {
    Arc::new(Mutex::new(None))
}

/// Kill a shared tunnel process if running.
pub(crate) async fn kill_shared(proc: &SharedProcess) -> Result<()> {
    let mut guard = proc.lock().await;
    if let Some(ref mut tp) = *guard {
        tp.child.kill().await.ok();
        tp.child.wait().await.ok();
    }
    *guard = None;
    Ok(())
}

// ── Configuration types ──────────────────────────────────────────

/// Provider-specific config for Cloudflare tunnels.
#[derive(Debug, Clone, Default)]
pub struct CloudflareTunnelConfig {
    /// Token from the Cloudflare Zero Trust dashboard.
    pub token: String,
}

/// Provider-specific config for Tailscale tunnels.
#[derive(Debug, Clone, Default)]
pub struct TailscaleTunnelConfig {
    /// Use `tailscale funnel` (public) instead of `tailscale serve` (tailnet).
    pub funnel: bool,
    /// Override the hostname (default: auto-detect from `tailscale status`).
    pub hostname: Option<String>,
}

/// Provider-specific config for ngrok tunnels.
#[derive(Debug, Clone, Default)]
pub struct NgrokTunnelConfig {
    /// ngrok auth token (required).
    pub auth_token: String,
    /// Custom domain (requires ngrok paid plan).
    pub domain: Option<String>,
}

/// Provider-specific config for custom tunnel commands.
#[derive(Debug, Clone, Default)]
pub struct CustomTunnelConfig {
    /// Shell command with `{port}` and `{host}` placeholders.
    pub start_command: String,
    /// HTTP endpoint to poll for health checks.
    pub health_url: Option<String>,
    /// Substring to match in stdout for URL extraction.
    pub url_pattern: Option<String>,
}

/// Full tunnel configuration.
#[derive(Debug, Clone, Default)]
pub struct TunnelProviderConfig {
    /// Provider name: "none", "cloudflare", "tailscale", "ngrok", "custom".
    pub provider: String,
    pub cloudflare: Option<CloudflareTunnelConfig>,
    pub tailscale: Option<TailscaleTunnelConfig>,
    pub ngrok: Option<NgrokTunnelConfig>,
    pub custom: Option<CustomTunnelConfig>,
}

// ── Factory ──────────────────────────────────────────────────────

/// Create a tunnel from config. Returns `None` for provider "none" or empty.
pub fn create_tunnel(config: &TunnelProviderConfig) -> Result<Option<Box<dyn Tunnel>>> {
    match config.provider.as_str() {
        "none" | "" => Ok(None),

        "cloudflare" => {
            let cf = config.cloudflare.as_ref().ok_or_else(|| {
                anyhow::anyhow!("TUNNEL_PROVIDER=cloudflare but no TUNNEL_CF_TOKEN configured")
            })?;
            Ok(Some(Box::new(CloudflareTunnel::new(cf.token.clone()))))
        }

        "tailscale" => {
            let ts = config.tailscale.as_ref().cloned().unwrap_or_default();
            Ok(Some(Box::new(TailscaleTunnel::new(ts.funnel, ts.hostname))))
        }

        "ngrok" => {
            let ng = config.ngrok.as_ref().ok_or_else(|| {
                anyhow::anyhow!("TUNNEL_PROVIDER=ngrok but no TUNNEL_NGROK_TOKEN configured")
            })?;
            Ok(Some(Box::new(NgrokTunnel::new(
                ng.auth_token.clone(),
                ng.domain.clone(),
            ))))
        }

        "custom" => {
            let cu = config.custom.as_ref().ok_or_else(|| {
                anyhow::anyhow!("TUNNEL_PROVIDER=custom but no TUNNEL_CUSTOM_COMMAND configured")
            })?;
            Ok(Some(Box::new(CustomTunnel::new(
                cu.start_command.clone(),
                cu.health_url.clone(),
                cu.url_pattern.clone(),
            ))))
        }

        other => bail!(
            "Unknown tunnel provider: \"{other}\". Valid: none, cloudflare, tailscale, ngrok, custom"
        ),
    }
}

// ── Managed tunnel startup ───────────────────────────────────────

/// Determine which local address the tunnel should forward traffic to.
///
/// Prefers the webhook server (`HTTP_PORT`) since that's where webhook routes
/// (Telegram, etc.) are served. Falls back to the gateway port if configured,
/// otherwise defaults to 0.0.0.0:8080 (the same fallback the webhook server
/// uses in main.rs when no HTTP config is present).
fn resolve_tunnel_target(channels: &crate::config::ChannelsConfig) -> (&str, u16) {
    if let Some(ref http) = channels.http {
        return (http.host.as_str(), http.port);
    }
    if let Some(ref gw) = channels.gateway {
        return (gw.host.as_str(), gw.port);
    }
    ("0.0.0.0", 8080)
}

/// Start a managed tunnel if configured and no static URL is already set.
///
/// Returns the (potentially mutated) config with `tunnel.public_url` set,
/// plus the active tunnel handle (if one was started) for later shutdown.
pub async fn start_managed_tunnel(
    mut config: crate::config::Config,
) -> (crate::config::Config, Option<Box<dyn Tunnel>>) {
    if config.tunnel.public_url.is_some() {
        tracing::debug!(
            "Static tunnel URL in use: {}",
            config.tunnel.public_url.as_deref().unwrap_or("?")
        );
        return (config, None);
    }

    let Some(ref provider_config) = config.tunnel.provider else {
        return (config, None);
    };

    let (tunnel_host, tunnel_port) = resolve_tunnel_target(&config.channels);

    match create_tunnel(provider_config) {
        Ok(Some(tunnel)) => {
            tracing::debug!(
                "Starting {} tunnel on {}:{}...",
                tunnel.name(),
                tunnel_host,
                tunnel_port
            );
            match tunnel.start(tunnel_host, tunnel_port).await {
                Ok(url) => {
                    tracing::debug!("Tunnel started: {}", url);
                    config.tunnel.public_url = Some(url);
                    (config, Some(tunnel))
                }
                Err(e) => {
                    tracing::error!("Failed to start tunnel: {}", e);
                    (config, None)
                }
            }
        }
        Ok(None) => (config, None),
        Err(e) => {
            tracing::error!("Failed to create tunnel: {}", e);
            (config, None)
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    fn assert_tunnel_err(cfg: &TunnelProviderConfig, needle: &str) {
        match create_tunnel(cfg) {
            Err(e) => assert!(
                e.to_string().contains(needle),
                "Expected error containing \"{needle}\", got: {e}"
            ),
            Ok(_) => panic!("Expected error containing \"{needle}\", but got Ok"),
        }
    }

    #[test]
    fn factory_none_returns_none() {
        let cfg = TunnelProviderConfig::default();
        assert!(create_tunnel(&cfg).unwrap().is_none());
    }

    #[test]
    fn factory_empty_returns_none() {
        let cfg = TunnelProviderConfig {
            provider: String::new(),
            ..Default::default()
        };
        assert!(create_tunnel(&cfg).unwrap().is_none());
    }

    #[test]
    fn factory_unknown_provider_errors() {
        let cfg = TunnelProviderConfig {
            provider: "wireguard".into(),
            ..Default::default()
        };
        assert_tunnel_err(&cfg, "Unknown tunnel provider");
    }

    #[test]
    fn factory_cloudflare_missing_config_errors() {
        let cfg = TunnelProviderConfig {
            provider: "cloudflare".into(),
            ..Default::default()
        };
        assert_tunnel_err(&cfg, "TUNNEL_CF_TOKEN");
    }

    #[test]
    fn factory_cloudflare_with_config_ok() {
        use crate::testing::credentials::TEST_BEARER_TOKEN;
        let cfg = TunnelProviderConfig {
            provider: "cloudflare".into(),
            cloudflare: Some(CloudflareTunnelConfig {
                token: TEST_BEARER_TOKEN.into(),
            }),
            ..Default::default()
        };
        let t = create_tunnel(&cfg).unwrap().unwrap();
        assert_eq!(t.name(), "cloudflare");
    }

    #[test]
    fn factory_tailscale_defaults_ok() {
        let cfg = TunnelProviderConfig {
            provider: "tailscale".into(),
            ..Default::default()
        };
        let t = create_tunnel(&cfg).unwrap().unwrap();
        assert_eq!(t.name(), "tailscale");
    }

    #[test]
    fn factory_ngrok_missing_config_errors() {
        let cfg = TunnelProviderConfig {
            provider: "ngrok".into(),
            ..Default::default()
        };
        assert_tunnel_err(&cfg, "TUNNEL_NGROK_TOKEN");
    }

    #[test]
    fn factory_ngrok_with_config_ok() {
        let cfg = TunnelProviderConfig {
            provider: "ngrok".into(),
            ngrok: Some(NgrokTunnelConfig {
                auth_token: "tok".into(),
                domain: None,
            }),
            ..Default::default()
        };
        let t = create_tunnel(&cfg).unwrap().unwrap();
        assert_eq!(t.name(), "ngrok");
    }

    #[test]
    fn factory_custom_missing_config_errors() {
        let cfg = TunnelProviderConfig {
            provider: "custom".into(),
            ..Default::default()
        };
        assert_tunnel_err(&cfg, "TUNNEL_CUSTOM_COMMAND");
    }

    #[test]
    fn factory_custom_with_config_ok() {
        let cfg = TunnelProviderConfig {
            provider: "custom".into(),
            custom: Some(CustomTunnelConfig {
                start_command: "echo tunnel".into(),
                health_url: None,
                url_pattern: None,
            }),
            ..Default::default()
        };
        let t = create_tunnel(&cfg).unwrap().unwrap();
        assert_eq!(t.name(), "custom");
    }

    #[tokio::test]
    async fn kill_shared_no_process_is_ok() {
        let proc = new_shared_process();
        assert!(kill_shared(&proc).await.is_ok());
        assert!(proc.lock().await.is_none());
    }

    #[tokio::test]
    async fn kill_shared_terminates_child() {
        let proc = new_shared_process();

        let child = Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("sleep should spawn");

        {
            let mut guard = proc.lock().await;
            *guard = Some(TunnelProcess {
                child,
                _pipe_drain: None,
            });
        }

        kill_shared(&proc).await.unwrap();
        assert!(proc.lock().await.is_none());
    }

    // ── Port selection regression tests ──────────────────────────────

    fn base_channels() -> crate::config::ChannelsConfig {
        crate::config::ChannelsConfig {
            cli: crate::config::CliConfig { enabled: false },
            http: None,
            gateway: None,
            signal: None,
            wasm_channels_dir: std::env::temp_dir().join("ironclaw-test-channels"),
            wasm_channels_enabled: false,
            wasm_channel_owner_ids: std::collections::HashMap::new(),
        }
    }

    fn channels_with_http(host: &str, port: u16) -> crate::config::ChannelsConfig {
        let mut c = base_channels();
        c.http = Some(crate::config::HttpConfig {
            host: host.to_string(),
            port,
            webhook_secret: None,
            user_id: "test".to_string(),
        });
        c.gateway = Some(crate::config::GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
            auth_token: None,
            user_id: "test".to_string(),
            workspace_read_scopes: Vec::new(),
            memory_layers: crate::workspace::layer::MemoryLayer::default_for_user("test"),
            user_tokens: None,
        });
        c
    }

    fn channels_gateway_only(host: &str, port: u16) -> crate::config::ChannelsConfig {
        let mut c = base_channels();
        c.gateway = Some(crate::config::GatewayConfig {
            host: host.to_string(),
            port,
            auth_token: None,
            user_id: "test".to_string(),
            workspace_read_scopes: Vec::new(),
            memory_layers: crate::workspace::layer::MemoryLayer::default_for_user("test"),
            user_tokens: None,
        });
        c
    }

    fn channels_neither() -> crate::config::ChannelsConfig {
        base_channels()
    }

    #[test]
    fn tunnel_target_prefers_http_port() {
        let channels = channels_with_http("0.0.0.0", 8080);
        let (host, port) = resolve_tunnel_target(&channels);
        assert_eq!(host, "0.0.0.0"); // safety: test-only
        assert_eq!(port, 8080); // safety: test-only
    }

    #[test]
    fn tunnel_target_falls_back_to_gateway() {
        let channels = channels_gateway_only("10.0.0.1", 4000);
        let (host, port) = resolve_tunnel_target(&channels);
        assert_eq!(host, "10.0.0.1"); // safety: test-only
        assert_eq!(port, 4000); // safety: test-only
    }

    #[test]
    fn tunnel_target_defaults_to_webhook_fallback() {
        let channels = channels_neither();
        let (host, port) = resolve_tunnel_target(&channels);
        // Matches the webhook server's hardcoded fallback in main.rs
        assert_eq!(host, "0.0.0.0"); // safety: test-only
        assert_eq!(port, 8080); // safety: test-only
    }

    #[test]
    fn tunnel_target_http_takes_priority_over_gateway() {
        let channels = channels_with_http("192.168.1.1", 9090);
        let (host, port) = resolve_tunnel_target(&channels);
        // Should use HTTP config, not gateway's 127.0.0.1:3000
        assert_eq!(host, "192.168.1.1"); // safety: test-only
        assert_eq!(port, 9090); // safety: test-only
    }

    #[test]
    fn tunnel_target_no_http_no_gateway_matches_webhook_fallback() {
        // When HTTP_PORT is not set and gateway is not configured (e.g. WASM
        // channels exist but no explicit HTTP config), the webhook server in
        // main.rs binds to 0.0.0.0:8080 as a hardcoded fallback. The tunnel
        // must target the same address so webhook traffic reaches the right
        // server.
        let channels = channels_neither();
        let (host, port) = resolve_tunnel_target(&channels);
        assert_eq!((host, port), ("0.0.0.0", 8080)); // safety: test-only
    }
}
