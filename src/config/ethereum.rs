use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Configuration for Ethereum wallet integration.
#[derive(Debug, Clone)]
pub struct EthereumConfig {
    /// Whether Ethereum tools are enabled.
    pub enabled: bool,
    /// JSON-RPC endpoint URL (e.g. https://mainnet.infura.io/v3/...).
    pub rpc_url: Option<String>,
    /// WalletConnect v2 project ID from cloud.walletconnect.com.
    pub walletconnect_project_id: Option<String>,
    /// TTL in seconds for pending transaction callbacks before timeout.
    pub callback_ttl_secs: u64,
    /// Path to persist WalletConnect session data. Defaults to ~/.ironclaw/walletconnect/.
    pub session_path: Option<String>,
}

impl Default for EthereumConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rpc_url: None,
            walletconnect_project_id: None,
            callback_ttl_secs: 600,
            session_path: None,
        }
    }
}

impl EthereumConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("ETHEREUM_ENABLED", false)?,
            rpc_url: optional_env("ETHEREUM_RPC_URL")?,
            walletconnect_project_id: optional_env("WALLETCONNECT_PROJECT_ID")?,
            callback_ttl_secs: parse_optional_env("ETHEREUM_CALLBACK_TTL_SECS", 600)?,
            session_path: optional_env("WALLETCONNECT_SESSION_PATH")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = EthereumConfig::default();
        assert!(!config.enabled);
        assert!(config.rpc_url.is_none());
        assert!(config.walletconnect_project_id.is_none());
        assert_eq!(config.callback_ttl_secs, 600);
        assert!(config.session_path.is_none());
    }

    #[test]
    fn test_from_env_with_values() {
        let config = EthereumConfig {
            enabled: true,
            rpc_url: Some("https://eth.example.com".into()),
            walletconnect_project_id: Some("test-project-id".into()),
            callback_ttl_secs: 300,
            session_path: None,
        };
        assert!(config.enabled);
        assert_eq!(config.rpc_url.as_deref(), Some("https://eth.example.com"));
        assert_eq!(config.callback_ttl_secs, 300);
    }

    #[test]
    fn test_resolve_defaults() {
        let config = EthereumConfig::resolve().expect("resolve should succeed");
        assert!(!config.enabled);
        assert!(config.rpc_url.is_none());
        assert_eq!(config.callback_ttl_secs, 600);
    }
}
