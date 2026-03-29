use std::sync::Arc;

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::tools::callback::ToolCallbackRegistry;

use super::session::WalletConnectSession;

/// Background listener that monitors the WalletConnect relay for wallet
/// responses (transaction approvals/rejections) and resolves them via
/// the callback registry.
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
            tracing::info!("WalletConnect listener started (stub - awaiting library integration)");

            // TODO: Replace with actual WalletConnect relay subscription.
            // The real implementation will:
            // 1. Connect to the WalletConnect relay
            // 2. Subscribe to session events
            // 3. On transaction response:
            //    - Extract correlation_id from the request metadata
            //    - Format result string (tx hash or rejection reason)
            //    - Call self.callback_registry.resolve(correlation_id, result, &self.inject_tx)
            // 4. On session disconnect/expiry:
            //    - Update self.session.set_status(SessionStatus::Expired)
            //    - Reconnect with backoff if appropriate

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if self.session.is_paired().await {
                    tracing::trace!("WalletConnect listener heartbeat - session active");
                }
            }
        })
    }
}
