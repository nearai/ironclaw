//! Cloudflare Tunnel via the `cloudflared` binary.

use anyhow::{Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, kill_shared, new_shared_process,
    new_shared_url,
};

/// Wraps `cloudflared` with token-based auth from the Zero Trust dashboard.
pub struct CloudflareTunnel {
    token: String,
    proc: SharedProcess,
    url: SharedUrl,
}

impl CloudflareTunnel {
    pub fn new(token: String) -> Self {
        Self {
            token,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for CloudflareTunnel {
    fn name(&self) -> &str {
        "cloudflare"
    }

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
        let origin = format!("http://{local_host}:{local_port}");
        let mut child = Command::new("cloudflared")
            .args([
                "tunnel",
                "--no-autoupdate",
                "run",
                "--token",
                &self.token,
                "--url",
                &origin,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        // cloudflared prints the public URL on stderr
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture cloudflared stderr"))?;

        let mut reader = tokio::io::BufReader::new(stderr).lines();
        let mut public_url = String::new();

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let line =
                tokio::time::timeout(tokio::time::Duration::from_secs(5), reader.next_line()).await;

            match line {
                Ok(Ok(Some(l))) => {
                    tracing::debug!("cloudflared: {l}");
                    if let Some(idx) = l.find("https://") {
                        let url_part = &l[idx..];
                        let end = url_part
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(url_part.len());
                        public_url = url_part[..end].to_string();
                        break;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => bail!("Error reading cloudflared output: {e}"),
                Err(_) => {} // line timeout, keep waiting
            }
        }

        if public_url.is_empty() {
            child.kill().await.ok();
            bail!("cloudflared did not produce a public URL within 30s. Is the token valid?");
        }

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess { child });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
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
    fn constructor_stores_token() {
        let tunnel = CloudflareTunnel::new("cf-token".into());
        assert_eq!(tunnel.token, "cf-token");
    }

    #[test]
    fn public_url_none_before_start() {
        assert!(CloudflareTunnel::new("tok".into()).public_url().is_none());
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        assert!(CloudflareTunnel::new("tok".into()).stop().await.is_ok());
    }

    #[tokio::test]
    async fn health_false_before_start() {
        assert!(!CloudflareTunnel::new("tok".into()).health_check().await);
    }
}
