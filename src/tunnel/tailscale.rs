//! Tailscale tunnel via `tailscale serve` or `tailscale funnel`.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, kill_shared, new_shared_process,
    new_shared_url,
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

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
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

        let target = format!("http://{local_host}:{local_port}");
        let child = Command::new("tailscale")
            .args([subcommand, &target])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let public_url = format!("https://{hostname}");

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess { child });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };
        if let Err(e) = Command::new("tailscale")
            .args([subcommand, "reset"])
            .output()
            .await
        {
            tracing::warn!("tailscale {subcommand} reset failed: {e}");
        }

        if let Ok(mut guard) = self.url.write() {
            *guard = None;
        }
        kill_shared(&self.proc).await
    }

    async fn health_check(&self) -> bool {
        let guard = self.proc.lock().await;
        guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
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
