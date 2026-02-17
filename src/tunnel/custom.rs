//! Custom tunnel via an arbitrary shell command.

use anyhow::{Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, kill_shared, new_shared_process,
    new_shared_url,
};

/// Bring-your-own tunnel binary.
///
/// `start_command` supports `{port}` and `{host}` placeholders.
/// If `url_pattern` is set, stdout is scanned for a URL matching that
/// substring. If `health_url` is set, health checks poll that endpoint.
///
/// **Note:** The command is split on whitespace, so quoted arguments like
/// `--arg "hello world"` won't work. Each token must be a single word.
///
/// Examples:
/// - `bore local {port} --to bore.pub`
/// - `ssh -R 80:localhost:{port} serveo.net`
pub struct CustomTunnel {
    start_command: String,
    health_url: Option<String>,
    url_pattern: Option<String>,
    proc: SharedProcess,
    url: SharedUrl,
}

impl CustomTunnel {
    pub fn new(
        start_command: String,
        health_url: Option<String>,
        url_pattern: Option<String>,
    ) -> Self {
        Self {
            start_command,
            health_url,
            url_pattern,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for CustomTunnel {
    fn name(&self) -> &str {
        "custom"
    }

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
        let cmd = self
            .start_command
            .replace("{port}", &local_port.to_string())
            .replace("{host}", local_host);

        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            bail!("Custom tunnel start_command is empty");
        }

        let mut child = Command::new(parts[0])
            .args(&parts[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let mut public_url = format!("http://{local_host}:{local_port}");

        if self.url_pattern.is_some()
            && let Some(stdout) = child.stdout.take()
        {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);

            while tokio::time::Instant::now() < deadline {
                let line =
                    tokio::time::timeout(tokio::time::Duration::from_secs(3), reader.next_line())
                        .await;

                match line {
                    Ok(Ok(Some(l))) => {
                        tracing::debug!("custom-tunnel: {l}");
                        if let Some(url) = extract_url(&l) {
                            let matches_pattern = self
                                .url_pattern
                                .as_ref()
                                .is_none_or(|pat| url.contains(pat.as_str()));
                            if matches_pattern {
                                public_url = url;
                                break;
                            }
                        }
                    }
                    Ok(Ok(None) | Err(_)) => break,
                    Err(_) => {}
                }
            }
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
        if let Some(ref url) = self.health_url {
            return reqwest::Client::new()
                .get(url)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .is_ok();
        }

        let guard = self.proc.lock().await;
        guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
    }

    fn public_url(&self) -> Option<String> {
        self.url.read().ok().and_then(|guard| guard.clone())
    }
}

/// Extract the first `https://` or `http://` URL from a line of text.
fn extract_url(line: &str) -> Option<String> {
    let idx = line.find("https://").or_else(|| line.find("http://"))?;
    let url_part = &line[idx..];
    let end = url_part
        .find(|c: char| c.is_whitespace())
        .unwrap_or(url_part.len());
    Some(url_part[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_command_returns_error() {
        let tunnel = CustomTunnel::new("   ".into(), None, None);
        let result = tunnel.start("127.0.0.1", 8080).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("start_command is empty")
        );
    }

    #[tokio::test]
    async fn start_without_pattern_returns_local() {
        let tunnel = CustomTunnel::new("sleep 1".into(), None, None);
        let url = tunnel.start("127.0.0.1", 4455).await.unwrap();
        assert_eq!(url, "http://127.0.0.1:4455");
        tunnel.stop().await.unwrap();
    }

    #[tokio::test]
    async fn start_with_pattern_extracts_url() {
        let tunnel = CustomTunnel::new(
            "echo https://public.example".into(),
            None,
            Some("public.example".into()),
        );
        let url = tunnel.start("localhost", 9999).await.unwrap();
        assert_eq!(url, "https://public.example");
        tunnel.stop().await.unwrap();
    }

    #[tokio::test]
    async fn pattern_filters_non_matching_urls() {
        // The command outputs two lines: first a non-matching URL, then a matching one.
        // The pattern filter should skip the first and grab the second.
        // No shell quoting needed; Command passes args directly to the binary.
        let tunnel = CustomTunnel::new(
            r"printf http://internal:1234\nhttps://real.tunnel.io/abc\n".into(),
            None,
            Some("tunnel.io".into()),
        );
        let url = tunnel.start("localhost", 9999).await.unwrap();
        assert_eq!(url, "https://real.tunnel.io/abc");
        tunnel.stop().await.unwrap();
    }

    #[tokio::test]
    async fn replaces_host_and_port_placeholders() {
        let tunnel = CustomTunnel::new(
            "echo http://{host}:{port}".into(),
            None,
            Some("http://".into()),
        );
        let url = tunnel.start("10.1.2.3", 4321).await.unwrap();
        assert_eq!(url, "http://10.1.2.3:4321");
        tunnel.stop().await.unwrap();
    }

    #[tokio::test]
    async fn health_with_unreachable_url_is_false() {
        let tunnel = CustomTunnel::new(
            "sleep 1".into(),
            Some("http://127.0.0.1:9/healthz".into()),
            None,
        );
        assert!(!tunnel.health_check().await);
    }

    #[test]
    fn extract_url_finds_https() {
        assert_eq!(
            extract_url("tunnel ready at https://foo.bar.com/path more text"),
            Some("https://foo.bar.com/path".to_string())
        );
    }

    #[test]
    fn extract_url_finds_http() {
        assert_eq!(
            extract_url("url=http://localhost:8080"),
            Some("http://localhost:8080".to_string())
        );
    }

    #[test]
    fn extract_url_none_when_absent() {
        assert_eq!(extract_url("no url here"), None);
    }
}
