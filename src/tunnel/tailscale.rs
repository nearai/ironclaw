//! Tailscale tunnel via `tailscale serve` or `tailscale funnel`.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{SharedProcess, Tunnel, TunnelProcess, kill_shared, new_shared_process};

/// Uses `tailscale serve` (tailnet-only) or `tailscale funnel` (public).
///
/// Requires Tailscale installed and authenticated (`tailscale up`).
pub struct TailscaleTunnel {
    funnel: bool,
    hostname: Option<String>,
    proc: SharedProcess,
}

impl TailscaleTunnel {
    pub fn new(funnel: bool, hostname: Option<String>) -> Self {
        Self {
            funnel,
            hostname,
            proc: new_shared_process(),
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
            let output = Command::new("tailscale")
                .args(["status", "--json"])
                .output()
                .await?;

            if !output.status.success() {
                bail!(
                    "tailscale status failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let status: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or_default();
            status["Self"]["DNSName"]
                .as_str()
                .unwrap_or("localhost")
                .trim_end_matches('.')
                .to_string()
        };

        let child = Command::new("tailscale")
            .args([subcommand, &local_port.to_string()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        // tailscale serve/funnel expose on standard HTTPS port (443),
        // the local_port is what gets forwarded to, not the public endpoint.
        let public_url = format!("https://{hostname}");

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess {
            child,
            public_url: public_url.clone(),
        });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };
        Command::new("tailscale")
            .args([subcommand, "reset"])
            .output()
            .await
            .ok();

        kill_shared(&self.proc).await
    }

    async fn health_check(&self) -> bool {
        let guard = self.proc.lock().await;
        guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
    }

    fn public_url(&self) -> Option<String> {
        self.proc
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|tp| tp.public_url.clone()))
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
