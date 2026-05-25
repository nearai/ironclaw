use std::path::{Path, PathBuf};

use ironclaw_safety::params_contain_manual_credentials;

use crate::RuntimeProcessError;

use super::reject_nul;

const REBORN_NETWORK_MODE_ENV: &str = "IRONCLAW_REBORN_NETWORK_MODE";
const REBORN_HTTP_PROXY_ENV: &str = "IRONCLAW_REBORN_HTTP_PROXY";
const REBORN_HTTP_BROKER_SOCKET_ENV: &str = "IRONCLAW_REBORN_HTTP_BROKER_SOCKET";
const REBORN_HTTP_BROKER_URL_ENV: &str = "IRONCLAW_REBORN_HTTP_BROKER_URL";
const REBORN_SECRET_MODE_ENV: &str = "IRONCLAW_REBORN_SECRET_MODE";
const REBORN_SECRET_BROKER_ENV: &str = "IRONCLAW_REBORN_SECRET_BROKER_URL";
const REBORN_SECRET_BROKER_SOCKET_ENV: &str = "IRONCLAW_REBORN_SECRET_BROKER_SOCKET";
const HTTP_PROXY_ENV_KEYS: &[&str] = &["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];
pub(super) const RESERVED_BROKER_ENV_KEYS: &[&str] = &[
    REBORN_NETWORK_MODE_ENV,
    REBORN_HTTP_PROXY_ENV,
    REBORN_HTTP_BROKER_SOCKET_ENV,
    REBORN_HTTP_BROKER_URL_ENV,
    REBORN_SECRET_MODE_ENV,
    REBORN_SECRET_BROKER_ENV,
    REBORN_SECRET_BROKER_SOCKET_ENV,
    "http_proxy",
    "https_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
];
const CONTAINER_HTTP_BROKER_SOCKET: &str = "/tmp/ironclaw-http-broker.sock";
const CONTAINER_SECRET_BROKER_SOCKET: &str = "/tmp/ironclaw-secret-broker.sock";
const CONTAINER_BROKER_URL: &str = "http://ironclaw-broker";

/// Broker affordance exposed to tenant sandbox commands.
///
/// The Unix-socket variant preserves Docker `--network none`; the HTTP-proxy
/// variant intentionally requires Docker network attachment and is for
/// compositions that accept proxy-enforced rather than Docker-enforced egress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSandboxNetworkBroker {
    kind: RebornSandboxNetworkBrokerKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RebornSandboxNetworkBrokerKind {
    HttpProxy { proxy_url: BrokerUrl },
    UnixSocket { host_socket: BrokerSocket },
}

impl RebornSandboxNetworkBroker {
    pub fn new(proxy_url: impl Into<String>) -> Result<Self, RuntimeProcessError> {
        Ok(Self {
            kind: RebornSandboxNetworkBrokerKind::HttpProxy {
                proxy_url: BrokerUrl::new("network broker proxy URL", proxy_url.into())?,
            },
        })
    }

    pub fn from_port(port: u16) -> Self {
        let proxy_url = format!("http://{}:{port}", docker_host_gateway());
        debug_assert!(validate_broker_url("network broker proxy URL", &proxy_url).is_ok());

        Self {
            kind: RebornSandboxNetworkBrokerKind::HttpProxy {
                proxy_url: BrokerUrl::trusted(proxy_url),
            },
        }
    }

    pub fn unix_socket(host_socket: impl Into<PathBuf>) -> Result<Self, RuntimeProcessError> {
        Ok(Self {
            kind: RebornSandboxNetworkBrokerKind::UnixSocket {
                host_socket: BrokerSocket::new("network broker socket", host_socket.into())?,
            },
        })
    }

    pub(super) fn requires_docker_network(&self) -> bool {
        matches!(self.kind, RebornSandboxNetworkBrokerKind::HttpProxy { .. })
    }

    fn push_env(&self, env: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        push_reserved_env(env, REBORN_NETWORK_MODE_ENV, "brokered")?;
        match &self.kind {
            RebornSandboxNetworkBrokerKind::HttpProxy { proxy_url } => {
                push_reserved_env(env, REBORN_HTTP_PROXY_ENV, proxy_url.as_str())?;
                for key in HTTP_PROXY_ENV_KEYS {
                    push_reserved_env(env, key, proxy_url.as_str())?;
                }
            }
            RebornSandboxNetworkBrokerKind::UnixSocket { .. } => {
                push_reserved_env(
                    env,
                    REBORN_HTTP_BROKER_SOCKET_ENV,
                    CONTAINER_HTTP_BROKER_SOCKET,
                )?;
                push_reserved_env(env, REBORN_HTTP_BROKER_URL_ENV, CONTAINER_BROKER_URL)?;
            }
        }
        Ok(())
    }

    fn append_bind(&self, binds: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        let RebornSandboxNetworkBrokerKind::UnixSocket { host_socket } = &self.kind else {
            return Ok(());
        };
        binds.push(docker_file_bind(
            host_socket.as_path(),
            CONTAINER_HTTP_BROKER_SOCKET,
            "network broker socket",
        )?);
        Ok(())
    }
}

/// Secret broker affordance exposed to tenant sandbox commands.
///
/// The value is an endpoint, not secret material. Concrete brokers remain
/// responsible for authentication, one-shot leases, redaction, and audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSandboxSecretBroker {
    kind: RebornSandboxSecretBrokerKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RebornSandboxSecretBrokerKind {
    HttpEndpoint { broker_url: BrokerUrl },
    UnixSocket { host_socket: BrokerSocket },
}

impl RebornSandboxSecretBroker {
    pub fn new(broker_url: impl Into<String>) -> Result<Self, RuntimeProcessError> {
        Ok(Self {
            kind: RebornSandboxSecretBrokerKind::HttpEndpoint {
                broker_url: BrokerUrl::new("secret broker URL", broker_url.into())?,
            },
        })
    }

    pub fn unix_socket(host_socket: impl Into<PathBuf>) -> Result<Self, RuntimeProcessError> {
        Ok(Self {
            kind: RebornSandboxSecretBrokerKind::UnixSocket {
                host_socket: BrokerSocket::new("secret broker socket", host_socket.into())?,
            },
        })
    }

    fn push_env(&self, env: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        push_reserved_env(env, REBORN_SECRET_MODE_ENV, "brokered")?;
        match &self.kind {
            RebornSandboxSecretBrokerKind::HttpEndpoint { broker_url } => {
                push_reserved_env(env, REBORN_SECRET_BROKER_ENV, broker_url.as_str())?;
            }
            RebornSandboxSecretBrokerKind::UnixSocket { .. } => {
                push_reserved_env(
                    env,
                    REBORN_SECRET_BROKER_SOCKET_ENV,
                    CONTAINER_SECRET_BROKER_SOCKET,
                )?;
            }
        }
        Ok(())
    }

    fn append_bind(&self, binds: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        let RebornSandboxSecretBrokerKind::UnixSocket { host_socket } = &self.kind else {
            return Ok(());
        };
        binds.push(docker_file_bind(
            host_socket.as_path(),
            CONTAINER_SECRET_BROKER_SOCKET,
            "secret broker socket",
        )?);
        Ok(())
    }
}

pub(super) fn push_broker_env(
    network_broker: Option<&RebornSandboxNetworkBroker>,
    secret_broker: Option<&RebornSandboxSecretBroker>,
    env: &mut Vec<String>,
) -> Result<(), RuntimeProcessError> {
    reject_reserved_broker_env_overrides(env)?;
    if let Some(broker) = network_broker {
        broker.push_env(env)?;
    } else {
        push_reserved_env(env, REBORN_NETWORK_MODE_ENV, "disabled")?;
    }
    if let Some(broker) = secret_broker {
        broker.push_env(env)?;
    } else {
        push_reserved_env(env, REBORN_SECRET_MODE_ENV, "disabled")?;
    }
    Ok(())
}

pub(super) fn append_broker_binds(
    network_broker: Option<&RebornSandboxNetworkBroker>,
    secret_broker: Option<&RebornSandboxSecretBroker>,
    binds: &mut Vec<String>,
) -> Result<(), RuntimeProcessError> {
    if let Some(broker) = network_broker {
        broker.append_bind(binds)?;
    }
    if let Some(broker) = secret_broker {
        broker.append_bind(binds)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokerUrl(String);

impl BrokerUrl {
    fn new(label: &str, value: String) -> Result<Self, RuntimeProcessError> {
        validate_broker_url(label, &value)?;
        Ok(Self(value))
    }

    fn trusted(value: String) -> Self {
        Self(value)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokerSocket(PathBuf);

impl BrokerSocket {
    fn new(label: &str, path: PathBuf) -> Result<Self, RuntimeProcessError> {
        validate_host_socket_path(label, &path)?;
        Ok(Self(path))
    }

    fn as_path(&self) -> &Path {
        &self.0
    }
}

fn reject_reserved_broker_env_overrides(env: &[String]) -> Result<(), RuntimeProcessError> {
    for entry in env {
        let Some((key, _)) = entry.split_once('=') else {
            continue;
        };
        if RESERVED_BROKER_ENV_KEYS.contains(&key) {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "environment variable {key} is reserved for the Reborn sandbox"
            )));
        }
    }
    Ok(())
}

fn push_reserved_env(
    env: &mut Vec<String>,
    key: &str,
    value: &str,
) -> Result<(), RuntimeProcessError> {
    if env
        .iter()
        .any(|entry| entry.starts_with(&format!("{key}=")))
    {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "environment variable {key} is reserved for the Reborn sandbox"
        )));
    }
    reject_nul("environment variable name", key)?;
    reject_nul("environment variable value", value)?;
    env.push(format!("{key}={value}"));
    Ok(())
}

fn validate_broker_url(label: &str, value: &str) -> Result<(), RuntimeProcessError> {
    reject_nul(label, value)?;
    let parsed = url::Url::parse(value).map_err(|_| {
        RuntimeProcessError::ExecutionFailed(format!(
            "{label} must be an http(s) URL without credentials, fragments, or control characters"
        ))
    })?;
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} must be an http(s) URL without credentials, fragments, or control characters"
        )));
    }
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || broker_url_contains_manual_credentials(value)
    {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} must be an http(s) URL without credentials, fragments, or control characters"
        )));
    }
    Ok(())
}

fn broker_url_contains_manual_credentials(value: &str) -> bool {
    params_contain_manual_credentials(&serde_json::json!({ "url": value }))
}

fn validate_host_socket_path(label: &str, path: &Path) -> Result<(), RuntimeProcessError> {
    let raw = path.to_string_lossy();
    reject_nul(label, &raw)?;
    if !path.is_absolute() || raw.contains(':') || raw.is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} must be an absolute host path without ':'"
        )));
    }
    Ok(())
}

fn docker_file_bind(
    host_path: &Path,
    container_path: &str,
    label: &str,
) -> Result<String, RuntimeProcessError> {
    validate_host_socket_path(label, host_path)?;
    reject_nul("container broker path", container_path)?;
    Ok(format!("{}:{container_path}:rw", host_path.display()))
}

pub(super) fn docker_host_gateway() -> &'static str {
    if cfg!(target_os = "linux") {
        "172.17.0.1"
    } else {
        "host.docker.internal"
    }
}
