//! ngrok tunnel via the `ngrok` binary.

use anyhow::{Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, kill_shared, new_shared_process,
    new_shared_url,
};

/// Wraps `ngrok` with optional custom domain support (paid plan).
pub struct NgrokTunnel {
    auth_token: String,
    domain: Option<String>,
    proc: SharedProcess,
    url: SharedUrl,
}

impl NgrokTunnel {
    pub fn new(auth_token: String, domain: Option<String>) -> Self {
        Self {
            auth_token,
            domain,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for NgrokTunnel {
    fn name(&self) -> &str {
        "ngrok"
    }

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
        let mut args = vec!["http".to_string(), format!("{local_host}:{local_port}")];
        if let Some(ref domain) = self.domain {
            args.push("--domain".into());
            args.push(domain.clone());
        }
        args.extend(["--log", "stdout", "--log-format", "logfmt"].map(String::from));

        let mut child = Command::new("ngrok")
            .args(&args)
            .env("NGROK_AUTHTOKEN", &self.auth_token)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture ngrok stdout"))?;
        let stderr = child.stderr.take();
        let mut reader = tokio::io::BufReader::new(stdout).lines();
        let mut public_url = String::new();

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            let line =
                tokio::time::timeout(tokio::time::Duration::from_secs(3), reader.next_line()).await;

            match line {
                Ok(Ok(Some(l))) => {
                    tracing::debug!("ngrok: {l}");
                    // ngrok logfmt: url=https://xxxx.ngrok-free.app
                    if let Some(idx) = l.find("url=https://") {
                        let url_start = idx + 4; // skip "url="
                        let url_part = &l[url_start..];
                        let end = url_part
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(url_part.len());
                        public_url = url_part[..end].to_string();
                        break;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => bail!("Error reading ngrok output: {e}"),
                Err(_) => {}
            }
        }

        if public_url.is_empty() {
            let error_detail = if let Some(stderr) = stderr {
                let mut err_reader = tokio::io::BufReader::new(stderr).lines();
                let mut lines = Vec::new();
                while lines.len() < 10 {
                    match tokio::time::timeout(
                        tokio::time::Duration::from_secs(1),
                        err_reader.next_line(),
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
                bail!("ngrok did not produce a public URL within 15s");
            } else {
                bail!("ngrok failed to start: {error_detail}");
            }
        }

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        // We took ownership of ngrok's stdout pipe above to parse the URL.
        // ngrok continues writing logs to stdout for its entire lifetime.
        // If we drop the reader, the pipe closes and ngrok gets SIGPIPE on
        // its next write → process dies. We can't just store the reader
        // without reading — the OS pipe buffer (~64KB) fills up and ngrok
        // blocks. So we drain it in a background task. The task exits
        // naturally when ngrok is killed (EOF on the pipe).
        let drain_handle = tokio::spawn(async move {
            while let Ok(Ok(Some(line))) = reader.next_line().await.map(Ok::<_, std::io::Error>) {
                tracing::trace!("ngrok: {line}");
            }
        });

        // Drain stderr silently to prevent SIGPIPE/buffer stalls.
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut err_reader = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(_)) = err_reader.next_line().await {}
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
    fn constructor_stores_domain() {
        let tunnel = NgrokTunnel::new("tok".into(), Some("my.ngrok.app".into()));
        assert_eq!(tunnel.domain.as_deref(), Some("my.ngrok.app"));
    }

    #[test]
    fn public_url_none_before_start() {
        assert!(NgrokTunnel::new("tok".into(), None).public_url().is_none());
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        assert!(NgrokTunnel::new("tok".into(), None).stop().await.is_ok());
    }

    #[tokio::test]
    async fn health_false_before_start() {
        assert!(!NgrokTunnel::new("tok".into(), None).health_check().await);
    }
}
