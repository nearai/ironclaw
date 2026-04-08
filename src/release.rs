//! Release version helpers.
//!
//! Shared utilities for checking whether a newer stable IronClaw release exists.

use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/nearai/ironclaw/releases/latest";
const GITHUB_RELEASE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    name: Option<String>,
    prerelease: bool,
    draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCheck {
    pub current: Version,
    pub latest: Version,
    pub release_name: Option<String>,
    pub release_url: String,
}

impl ReleaseCheck {
    pub fn update_available(&self) -> bool {
        self.latest > self.current
    }
}

pub fn current_version() -> anyhow::Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| {
        anyhow::anyhow!(
            "invalid built-in version '{}': {e}",
            env!("CARGO_PKG_VERSION")
        )
    })
}

pub async fn check_for_update(current: Version) -> anyhow::Result<ReleaseCheck> {
    let release =
        fetch_latest_release_at(GITHUB_LATEST_RELEASE_URL, GITHUB_RELEASE_CHECK_TIMEOUT).await?;
    let latest = parse_release_version(&release.tag_name)?;

    Ok(ReleaseCheck {
        current,
        latest,
        release_name: release.name,
        release_url: release.html_url,
    })
}

async fn fetch_latest_release_at(url: &str, timeout: Duration) -> anyhow::Result<GitHubRelease> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(format!(
            "{}/{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))?;

    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("failed to query GitHub releases: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "GitHub releases request failed with HTTP {}",
            status
        ));
    }

    let release = response
        .json::<GitHubRelease>()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse GitHub release response: {e}"))?;

    if release.draft || release.prerelease {
        return Err(anyhow::anyhow!(
            "latest GitHub release response is not a stable release"
        ));
    }

    Ok(release)
}

fn parse_release_version(tag_name: &str) -> anyhow::Result<Version> {
    let trimmed = tag_name.strip_prefix('v').unwrap_or(tag_name);
    Version::parse(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "latest release tag '{}' is not a semver version: {e}",
            tag_name
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::Instant;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio::time::sleep;

    async fn spawn_stalled_release_server(
        stall_delay: Duration,
    ) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut stream = stream;
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf).await;
                    sleep(stall_delay).await;
                });
            }
        });

        Ok((addr, handle))
    }

    #[test]
    fn parses_v_prefixed_release_tags() {
        let version = parse_release_version("v0.18.0").expect("version should parse");
        assert_eq!(version, Version::new(0, 18, 0));
    }

    #[test]
    fn parses_plain_release_tags() {
        let version = parse_release_version("0.19.1").expect("version should parse");
        assert_eq!(version, Version::new(0, 19, 1));
    }

    #[test]
    fn rejects_non_semver_tags() {
        let err = parse_release_version("release-latest").expect_err("tag should be rejected");
        assert!(err.to_string().contains("not a semver version"));
    }

    #[test]
    fn compares_versions_correctly() {
        let current = Version::new(0, 18, 0);
        let latest = Version::new(0, 19, 0);
        assert!(latest > current);
    }

    #[tokio::test]
    async fn fetch_latest_release_times_out_on_slow_server() {
        let (addr, _server) = spawn_stalled_release_server(Duration::from_millis(200))
            .await
            .expect("test server should start");
        let url = format!("http://{addr}/repos/nearai/ironclaw/releases/latest");

        let started = Instant::now();
        let err = fetch_latest_release_at(&url, Duration::from_millis(50))
            .await
            .expect_err("slow server should time out");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "request should have been bounded, took {elapsed:?} and returned {err}"
        );
    }
}
