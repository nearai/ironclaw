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

        let stdout = child.stdout.take();

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
            let error_detail = if let Some(stdout) = stdout {
                let mut out_reader = tokio::io::BufReader::new(stdout).lines();
                let mut lines = Vec::new();
                while lines.len() < 10 {
                    match tokio::time::timeout(
                        tokio::time::Duration::from_secs(1),
                        out_reader.next_line(),
                    )
                    .await
                    {
                        Ok(Ok(Some(line))) => lines.push(line),
                        _ => break,
                    }
                }
                lines.join("\n")
            } else {
                String::new()
            };

            child.kill().await.ok();
            if error_detail.is_empty() {
                bail!("cloudflared did not produce a public URL within 30s");
            } else {
                bail!("cloudflared failed to start: {error_detail}");
            }
        }

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        // We took ownership of cloudflared's stderr pipe above to parse the URL.
        // cloudflared continues writing logs for its entire lifetime. If we drop
        // the reader, the pipe closes and cloudflared gets SIGPIPE on its next
        // write. We can't just store the reader without reading — the OS pipe
        // buffer fills up and cloudflared blocks. So we drain it in a background
        // task. The task exits naturally when cloudflared is killed (EOF).
        let drain_handle = tokio::spawn(async move {
            while let Ok(Ok(Some(line))) = reader.next_line().await.map(Ok::<_, std::io::Error>) {
                tracing::trace!("cloudflared: {line}");
            }
        });

        // Drain stdout silently to prevent SIGPIPE/buffer stalls.
        if let Some(stdout) = stdout {
            tokio::spawn(async move {
                let mut out_reader = tokio::io::BufReader::new(stdout).lines();
                while let Ok(Some(_)) = out_reader.next_line().await {}
            });
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess {
            child,
            _pipe_drain: Some(drain_handle),
        });

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
