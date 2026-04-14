#[derive(Debug, thiserror::Error)]
pub enum EthereumError {
    #[error("not paired: no WalletConnect session. Call wallet_pair first")]
    NotPaired,

    #[error("WalletConnect session expired. Call wallet_pair to re-pair")]
    SessionExpired,

    #[error("simulation failed: {reason}")]
    SimulationFailed { reason: String },

    #[error("insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: String, need: String },

    #[error("invalid address: {address}")]
    InvalidAddress { address: String },

    #[error("WalletConnect error: {0}")]
    WalletConnect(String),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("ethereum not configured: {0}")]
    NotConfigured(String),
}

impl From<EthereumError> for crate::tools::tool::ToolError {
    fn from(e: EthereumError) -> Self {
        match &e {
            EthereumError::NotPaired | EthereumError::SessionExpired => {
                crate::tools::tool::ToolError::ExecutionFailed(e.to_string())
            }
            EthereumError::InvalidAddress { .. } | EthereumError::InsufficientBalance { .. } => {
                crate::tools::tool::ToolError::InvalidParameters(e.to_string())
            }
            EthereumError::SimulationFailed { .. }
            | EthereumError::WalletConnect(_)
            | EthereumError::Rpc(_) => {
                crate::tools::tool::ToolError::ExternalService(e.to_string())
            }
            EthereumError::NotConfigured(_) => {
                crate::tools::tool::ToolError::ExecutionFailed(e.to_string())
            }
        }
    }
}
