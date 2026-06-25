use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::error::{RebornServicesError, RebornServicesErrorCode};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IronhubRegisterRequest {
    pub uid: String,
    pub aid: String,
    pub ts: u64,
    pub nonce: String,
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IronhubInstallDeliveryRequest {
    pub slug: String,
    pub version: String,
    pub uid: String,
    pub aid: String,
    pub ts: u64,
    pub nonce: String,
    pub artifact_digest: String,
    pub sig: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub private_manifest_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IronhubInstallDeliveryResult {
    pub installed: bool,
    pub slug: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum IronhubLinkError {
    #[error("invalid agent-link signature")]
    InvalidSignature,
    #[error("agent-link timestamp outside the accepted window")]
    StaleTimestamp,
    #[error("ironhub install failed: {reason}")]
    Install { reason: String },
    #[error("ironhub link service is unavailable")]
    Unavailable,
}

#[async_trait]
pub trait IronhubLinkService: Send + Sync {
    async fn register(&self, request: IronhubRegisterRequest) -> Result<(), IronhubLinkError>;

    async fn deliver_install(
        &self,
        request: IronhubInstallDeliveryRequest,
    ) -> Result<IronhubInstallDeliveryResult, IronhubLinkError>;
}

pub(super) fn ironhub_link_unavailable() -> RebornServicesError {
    RebornServicesError::service_unavailable(false)
}

pub(super) fn map_ironhub_link_error(error: IronhubLinkError) -> RebornServicesError {
    match error {
        IronhubLinkError::InvalidSignature | IronhubLinkError::StaleTimestamp => {
            RebornServicesError::from_status(RebornServicesErrorCode::Forbidden, 403, false)
        }
        IronhubLinkError::Install { .. } => RebornServicesError::internal_invariant(),
        IronhubLinkError::Unavailable => RebornServicesError::service_unavailable(false),
    }
}
