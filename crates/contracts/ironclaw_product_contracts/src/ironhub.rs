use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::descriptors::{EmptyProductCommandInput, ProductSurfaceCommandDescriptor, ProductView};
use crate::product_wire::{RebornIronhubLinkResponse, RebornIronhubLinkSetKeyRequest};
use crate::surface::ProductSurfaceCaller;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IronhubRegisterRequest {
    pub uid: String,
    pub aid: String,
    pub ts: u64,
    pub nonce: String,
    pub sig: String,
}

impl std::fmt::Debug for IronhubRegisterRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IronhubRegisterRequest")
            .field("uid", &self.uid)
            .field("aid", &self.aid)
            .field("ts", &self.ts)
            .field("nonce", &self.nonce)
            .field("sig", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub private_manifest_url: Option<String>,
}

impl std::fmt::Debug for IronhubInstallDeliveryRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IronhubInstallDeliveryRequest")
            .field("slug", &self.slug)
            .field("version", &self.version)
            .field("uid", &self.uid)
            .field("aid", &self.aid)
            .field("ts", &self.ts)
            .field("nonce", &self.nonce)
            .field("artifact_digest", &self.artifact_digest)
            .field("sig", &"<redacted>")
            .field(
                "private_manifest_url",
                &self.private_manifest_url.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[error("agent-link request replayed")]
    Replay,
    #[error("ironhub install failed: {reason}")]
    Install { reason: String },
    #[error("invalid ironhub install request: {reason}")]
    InvalidInput { reason: String },
    #[error("ironhub link service is unavailable")]
    Unavailable,
}

#[async_trait]
pub trait IronhubLinkService: Send + Sync {
    async fn register(&self, request: IronhubRegisterRequest) -> Result<(), IronhubLinkError>;

    async fn deliver_install(
        &self,
        caller: ProductSurfaceCaller,
        request: IronhubInstallDeliveryRequest,
    ) -> Result<IronhubInstallDeliveryResult, IronhubLinkError>;
}

pub const IRONHUB_DELIVER_INSTALL_COMMAND_ID: &str = "ironhub.deliver_install";
pub const IRONHUB_DELIVER_INSTALL_COMMAND: ProductSurfaceCommandDescriptor<
    IronhubInstallDeliveryRequest,
    IronhubInstallDeliveryResult,
> = ProductSurfaceCommandDescriptor::new(IRONHUB_DELIVER_INSTALL_COMMAND_ID);

/// Implemented by `ironclaw_extension_manager`; read live so a key stored
/// after startup reports as stored but not active. Mutations take the caller
/// for audit; the operator gate is the assistant's.
#[async_trait]
pub trait IronhubLinkAdminService: Send + Sync {
    async fn status(&self) -> Result<RebornIronhubLinkResponse, IronhubLinkError>;

    /// The hub mints the key; the agent only accepts it.
    async fn set_shared_key(
        &self,
        caller: ProductSurfaceCaller,
        shared_key: SecretString,
    ) -> Result<RebornIronhubLinkResponse, IronhubLinkError>;

    /// Forgets the stored key. The gateway keeps running on the key it booted
    /// with until the next restart.
    async fn clear_shared_key(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornIronhubLinkResponse, IronhubLinkError>;
}

pub const IRONHUB_LINK_VIEW: ProductView<serde_json::Value, RebornIronhubLinkResponse> =
    ProductView::unpaginated("ironhub_link");

pub const IRONHUB_LINK_SET_KEY_COMMAND_ID: &str = "ironhub.link.set_key";
pub const IRONHUB_LINK_SET_KEY_COMMAND: ProductSurfaceCommandDescriptor<
    RebornIronhubLinkSetKeyRequest,
    RebornIronhubLinkResponse,
> = ProductSurfaceCommandDescriptor::new(IRONHUB_LINK_SET_KEY_COMMAND_ID);

pub const IRONHUB_LINK_CLEAR_KEY_COMMAND_ID: &str = "ironhub.link.clear_key";
pub const IRONHUB_LINK_CLEAR_KEY_COMMAND: ProductSurfaceCommandDescriptor<
    EmptyProductCommandInput,
    RebornIronhubLinkResponse,
> = ProductSurfaceCommandDescriptor::new(IRONHUB_LINK_CLEAR_KEY_COMMAND_ID);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_debug_output_redacts_signatures_and_private_access_tokens() {
        let secret = "private-access-token";
        let request = IronhubInstallDeliveryRequest {
            slug: "private-skill".to_string(),
            version: "1.0.0".to_string(),
            uid: "signed-user".to_string(),
            aid: "signed-agent".to_string(),
            ts: 1_700_000_000,
            nonce: "nonce".to_string(),
            artifact_digest: "sha256:deadbeef".to_string(),
            sig: "hmac-signature".to_string(),
            private_manifest_url: Some(format!(
                "https://catalog.example/private/manifest?token={secret}"
            )),
        };

        let rendered = format!("{request:?}");
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains("hmac-signature"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn register_request_debug_output_redacts_signature() {
        let request = IronhubRegisterRequest {
            uid: "user".to_string(),
            aid: "agent".to_string(),
            ts: 1_700_000_000,
            nonce: "nonce".to_string(),
            sig: "register-secret".to_string(),
        };
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("register-secret"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn install_delivery_rejects_unsigned_unknown_fields() {
        let result = serde_json::from_value::<IronhubInstallDeliveryRequest>(serde_json::json!({
            "slug": "skill",
            "version": "1.0.0",
            "uid": "signed-user",
            "aid": "signed-agent",
            "ts": 1_700_000_000_u64,
            "nonce": "nonce",
            "artifact_digest": "sha256:deadbeef",
            "sig": "signature",
            "kind": "tool"
        }));

        assert!(
            result.is_err(),
            "unknown fields must not create an unsigned install-routing channel"
        );
    }
}
