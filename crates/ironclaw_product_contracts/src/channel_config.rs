//! The per-extension channel-config port (PROPOSAL §6.1.3).
//!
//! `[channel.config]` declares operator-supplied configuration for a channel
//! extension. The product setup service routes submitted values through this
//! port and derives config completeness from the field status it returns. The
//! port implementation is `ironclaw_extension_manager`'s (WS2.4); it wraps the
//! `ChannelConfigService` core that stayed in `ironclaw_extension_host`,
//! because the host owns the durable installation store and the scoped secret
//! store the values land in.
//!
//! The field DTO itself is manifest vocabulary and already lives in
//! [`crate::package_lifecycle::ChannelConfigField`]; this module adds only the
//! port.

use async_trait::async_trait;
use ironclaw_host_api::ids::ExtensionId;

use crate::package_lifecycle::ChannelConfigField;
use crate::surface::ProductSurfaceError;

/// The generic channel-config configure port: per-extension operator config
/// declared by the extension manifest's channel-config fields.
/// `ironclaw_extension_manager` implements it over the host's
/// `ChannelConfigService`, which owns the durable installation store and the
/// scoped secret store; the setup service routes submitted values through it
/// and derives config completeness from the field status.
#[async_trait]
pub trait ChannelConfigProductService: Send + Sync {
    /// Per-field presence for the extension's declared channel config.
    /// Empty when the extension declares none (or is not installed yet).
    async fn field_status(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ChannelConfigField>, ProductSurfaceError>;

    /// Validate submitted `(handle, value)` pairs against the installed
    /// manifest's declared fields and persist them (non-secret values
    /// durably per installation, secret values into the scoped secret
    /// store). Saving while the extension is active re-runs its activation
    /// with the new values.
    async fn save_values(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), ProductSurfaceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_obj_safe!(ChannelConfigProductService);

    struct DeclaresNothing;

    #[async_trait]
    impl ChannelConfigProductService for DeclaresNothing {
        async fn field_status(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<Vec<ChannelConfigField>, ProductSurfaceError> {
            Ok(Vec::new())
        }

        async fn save_values(
            &self,
            _extension_id: &ExtensionId,
            _values: Vec<(String, String)>,
        ) -> Result<(), ProductSurfaceError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn an_extension_declaring_no_config_reports_an_empty_field_set() {
        // Empty is the "nothing to configure" answer the setup view renders
        // from; it must be reachable without an error, or a channel with no
        // `[channel.config]` cannot complete setup.
        let service: std::sync::Arc<dyn ChannelConfigProductService> =
            std::sync::Arc::new(DeclaresNothing);
        let extension_id = ExtensionId::new("slack").expect("valid extension id");
        assert!(
            service
                .field_status(&extension_id)
                .await
                .expect("empty status is not an error")
                .is_empty()
        );
        service
            .save_values(&extension_id, Vec::new())
            .await
            .expect("saving nothing is not an error");
    }
}
