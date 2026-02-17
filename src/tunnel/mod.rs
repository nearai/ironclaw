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
    pub public_url: String,
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
        let cfg = TunnelProviderConfig {
            provider: "cloudflare".into(),
            cloudflare: Some(CloudflareTunnelConfig {
                token: "test-token".into(),
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
                public_url: "https://example.test".into(),
            });
        }

        kill_shared(&proc).await.unwrap();
        assert!(proc.lock().await.is_none());
    }
}
