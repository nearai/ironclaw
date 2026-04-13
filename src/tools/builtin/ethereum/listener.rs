use std::sync::Arc;

use tokio::sync::mpsc;
use walletconnect_client::prelude::Event;

use crate::channels::IncomingMessage;
use crate::tools::callback::ToolCallbackRegistry;

use super::session::{SessionStatus, WalletConnectSession};

/// Background listener that monitors the WalletConnect relay for wallet
/// events (connection, disconnection, account/chain changes, transaction
/// responses) and updates session state accordingly.
pub struct WalletConnectListener {
    session: Arc<WalletConnectSession>,
    #[allow(dead_code)]
    callback_registry: Arc<ToolCallbackRegistry>,
    #[allow(dead_code)]
    inject_tx: mpsc::Sender<IncomingMessage>,
}

impl WalletConnectListener {
    pub fn new(
        session: Arc<WalletConnectSession>,
        callback_registry: Arc<ToolCallbackRegistry>,
        inject_tx: mpsc::Sender<IncomingMessage>,
    ) -> Self {
        Self {
            session,
            callback_registry,
            inject_tx,
        }
    }

    /// Start the background listener. Returns a JoinHandle.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tracing::info!("WalletConnect listener started");

            loop {
                match self.session.poll_event().await {
                    Some(event) => self.handle_event(event).await,
                    None => {
                        // No active client or no event ready — back off briefly.
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        })
    }

    async fn handle_event(&self, event: Event) {
        match event {
            Event::Connected => {
                tracing::info!("WalletConnect: wallet connected");
                self.session.update_from_connected().await;
            }
            Event::Disconnected => {
                tracing::info!("WalletConnect: wallet disconnected");
                self.session.set_status(SessionStatus::Disconnected).await;
            }
            Event::AccountsChanged(accounts) => {
                tracing::info!("WalletConnect: accounts changed: {accounts:?}");
                let chain_id = self.session.active_chain_id().await.unwrap_or(1);
                self.session.update_accounts(chain_id).await;
            }
            Event::ChainIdChanged(new_chain_id) => {
                tracing::info!("WalletConnect: chain changed to {new_chain_id}");
                self.session.update_accounts(new_chain_id).await;
            }
            Event::Broken => {
                tracing::warn!("WalletConnect: connection broken");
                self.session.set_status(SessionStatus::Disconnected).await;
            }
        }
    }
}
