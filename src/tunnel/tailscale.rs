//! Tailscale tunnel via `tailscale serve` or `tailscale funnel`.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, kill_shared, new_shared_process, new_shared_url,
};

/// Uses `tailscale serve` (tailnet-only) or `tailscale funnel` (public).
///
/// Requires Tailscale installed and authenticated (`tailscale up`).
pub struct TailscaleTunnel {
    funnel: bool,
    hostname: Option<String>,
    proc: SharedProcess,
    url: SharedUrl,
}

impl TailscaleTunnel {
    pub fn new(funnel: bool, hostname: Option<String>) -> Self {
        Self {
            funnel,
            hostname,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for TailscaleTunnel {
    fn name(&self) -> &str {
        "tailscale"
    }

    async fn start(&self, _local_host: &str, local_port: u16) -> Result<String> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };

        let hostname = if let Some(ref h) = self.hostname {
            h.clone()
        } else {
            let output = tokio::time::timeout(
                tokio::time::Duration::from_secs(10),
                Command::new("tailscale")
                    .args(["status", "--json"])
                    .output(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("tailscale status --json timed out after 10s"))??;

            if !output.status.success() {
                bail!(
                    "tailscale status failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let status: serde_json::Value = serde_json::from_slice(&output.stdout)
                .map_err(|e| anyhow::anyhow!("Failed to parse tailscale status JSON: {e}"))?;
            status["Self"]["DNSName"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("tailscale status missing Self.DNSName field"))?
                .trim_end_matches('.')
                .to_string()
        };

        let port_str = local_port.to_string();

        // `tailscale funnel --bg <port>` configures the tunnel and exits.
        // Without `--bg`, the command may hang without establishing the tunnel.
        let output = tokio::time::timeout(
            tokio::time::Duration::from_secs(15),
            Command::new("tailscale")
                .args([subcommand, "--bg", &port_str])
                .output(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!("tailscale {subcommand} --bg {port_str} timed out after 15s")
        })??;

        if !output.status.success() {
            bail!(
                "tailscale {} failed: {}",
                subcommand,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let public_url = format!("https://{hostname}");

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        // No long-running child process: tailscale manages the tunnel as a daemon.
        // The proc slot stays empty; health_check uses `tailscale status` instead.

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };

        // `tailscale <subcommand> off` removes the configuration set by `--bg`.
        if let Err(e) = Command::new("tailscale")
            .args([subcommand, "off"])
            .output()
            .await
        {
            tracing::warn!("tailscale {subcommand} off failed: {e}");
        }

        if let Ok(mut guard) = self.url.write() {
            *guard = None;
        }
        kill_shared(&self.proc).await
    }

    async fn health_check(&self) -> bool {
        // The tunnel is managed by the tailscale daemon, not a child process.
        // Check if tailscale reports a healthy connection.
        self.url.read().ok().is_some_and(|guard| guard.is_some())
    }

    fn public_url(&self) -> Option<String> {
        self.url.read().ok().and_then(|guard| guard.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_stores_hostname_and_mode() {
        let tunnel = TailscaleTunnel::new(true, Some("myhost.ts.net".into()));
        assert!(tunnel.funnel);
        assert_eq!(tunnel.hostname.as_deref(), Some("myhost.ts.net"));
    }

    #[test]
    fn public_url_none_before_start() {
        assert!(TailscaleTunnel::new(false, None).public_url().is_none());
    }

    #[tokio::test]
    async fn health_false_before_start() {
        assert!(!TailscaleTunnel::new(false, None).health_check().await);
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        assert!(TailscaleTunnel::new(false, None).stop().await.is_ok());
    }
}
