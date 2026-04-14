use tokio::sync::{Mutex, RwLock};
use url::Url;
use walletconnect_client::prelude::*;

use super::error::EthereumError;

/// Status of the WalletConnect session from IronClaw's perspective.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Disconnected,
    Pairing { uri: String },
    Paired { address: String, chain_id: u64 },
    Expired,
}

/// Manages a WalletConnect v2 session, wrapping the real WC client.
pub struct WalletConnectSession {
    status: RwLock<SessionStatus>,
    wc_client: Mutex<Option<WalletConnect<NativeTransport>>>,
    project_id: Option<String>,
}

impl WalletConnectSession {
    pub fn new(project_id: Option<String>) -> Self {
        Self {
            status: RwLock::new(SessionStatus::Disconnected),
            wc_client: Mutex::new(None),
            project_id,
        }
    }

    /// Create a disconnected session with no project ID (for tests / disabled mode).
    pub fn new_disconnected() -> Self {
        Self::new(None)
    }

    /// Initiate a WalletConnect pairing session. Returns the pairing URI.
    pub async fn initiate_pairing(&self, chain_id: u64) -> Result<String, EthereumError> {
        let project_id = self.project_id.as_ref().ok_or_else(|| {
            EthereumError::NotConfigured("WALLETCONNECT_PROJECT_ID not set".into())
        })?;

        let metadata = Metadata::from(
            "IronClaw",
            "Secure personal AI assistant",
            Url::parse("https://ironclaw.dev")
                .map_err(|e| EthereumError::WalletConnect(e.to_string()))?,
            vec![],
        );

        let wc = WalletConnect::<NativeTransport>::connect(
            walletconnect_client::jwt::decode::ProjectId::from(project_id.as_str()),
            chain_id,
            metadata,
            None,
        )
        .await
        .map_err(|e| EthereumError::WalletConnect(e.to_string()))?;

        let uri = wc
            .initiate_session(None)
            .await
            .map_err(|e| EthereumError::WalletConnect(e.to_string()))?;

        *self.status.write().await = SessionStatus::Pairing { uri: uri.clone() };
        *self.wc_client.lock().await = Some(wc);

        Ok(uri)
    }

    /// Poll for the next WalletConnect event. Called by the listener task.
    ///
    /// Returns `None` if no client is active or no event is available.
    pub async fn poll_event(&self) -> Option<Event> {
        let mut client = self.wc_client.lock().await;
        if let Some(ref wc) = *client {
            match wc.next().await {
                Ok(event) => event,
                Err(e) => {
                    tracing::warn!("WalletConnect event error: {e}");
                    // On disconnect error, drop the client
                    if matches!(e, WalletConnectError::Disconnected) {
                        *client = None;
                    }
                    None
                }
            }
        } else {
            None
        }
    }

    /// Send a JSON-RPC request through the WC client (e.g. eth_sendTransaction).
    pub async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        chain_id: u64,
    ) -> Result<serde_json::Value, EthereumError> {
        let client = self.wc_client.lock().await;
        let wc = client.as_ref().ok_or(EthereumError::NotPaired)?;

        wc.request(method, params, chain_id)
            .await
            .map_err(|e| EthereumError::WalletConnect(e.to_string()))
    }

    /// Update session status from a WC Connected event by reading the client state.
    pub async fn update_from_connected(&self) {
        let client = self.wc_client.lock().await;
        if let Some(ref wc) = *client {
            let address = wc
                .get_account()
                .map(|a| format!("{a:?}"))
                .unwrap_or_default();
            let chain_id = wc.chain_id();
            drop(client);
            *self.status.write().await = SessionStatus::Paired { address, chain_id };
        }
    }

    /// Update session status when accounts change.
    pub async fn update_accounts(&self, chain_id: u64) {
        let client = self.wc_client.lock().await;
        if let Some(ref wc) = *client {
            let address = wc
                .get_accounts_for_chain_id(chain_id)
                .and_then(|accs| accs.first().map(|a| format!("{a:?}")))
                .unwrap_or_default();
            drop(client);
            *self.status.write().await = SessionStatus::Paired { address, chain_id };
        }
    }

    pub async fn is_paired(&self) -> bool {
        matches!(*self.status.read().await, SessionStatus::Paired { .. })
    }

    pub async fn active_address(&self) -> Option<String> {
        match &*self.status.read().await {
            SessionStatus::Paired { address, .. } => Some(address.clone()),
            _ => None,
        }
    }

    pub async fn active_chain_id(&self) -> Option<u64> {
        match &*self.status.read().await {
            SessionStatus::Paired { chain_id, .. } => Some(*chain_id),
            _ => None,
        }
    }

    pub async fn status(&self) -> SessionStatus {
        self.status.read().await.clone()
    }

    pub async fn set_status(&self, new_status: SessionStatus) {
        *self.status.write().await = new_status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_new_session_is_not_paired() {
        let session = WalletConnectSession::new_disconnected();
        assert!(!session.is_paired().await);
        assert!(session.active_address().await.is_none());
    }

    #[tokio::test]
    async fn test_session_status_when_not_paired() {
        let session = WalletConnectSession::new_disconnected();
        let status = session.status().await;
        assert!(matches!(status, SessionStatus::Disconnected));
    }

    #[tokio::test]
    async fn test_initiate_pairing_fails_without_project_id() {
        let session = WalletConnectSession::new_disconnected();
        let result = session.initiate_pairing(1).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("WALLETCONNECT_PROJECT_ID"),
            "Expected NotConfigured error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_new_with_project_id() {
        let session = WalletConnectSession::new(Some("test-id".into()));
        assert!(!session.is_paired().await);
        assert_eq!(session.status().await, SessionStatus::Disconnected);
    }

    #[tokio::test]
    async fn test_poll_event_returns_none_when_no_client() {
        let session = WalletConnectSession::new_disconnected();
        let event = session.poll_event().await;
        assert!(event.is_none());
    }

    #[tokio::test]
    async fn test_set_and_read_status() {
        let session = WalletConnectSession::new_disconnected();
        session
            .set_status(SessionStatus::Paired {
                address: "0xabc".into(),
                chain_id: 137,
            })
            .await;
        assert!(session.is_paired().await);
        assert_eq!(session.active_address().await, Some("0xabc".into()));
        assert_eq!(session.active_chain_id().await, Some(137));
    }
}
