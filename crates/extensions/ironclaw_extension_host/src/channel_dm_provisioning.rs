//! Generic post-bind DM-target provisioning (extension-runtime §5.5).
//!
//! After the generic channel-identity hook binds a proven vendor identity,
//! the caller's personal direct conversation is opened through the
//! extension's own channel adapter and persisted in the generic DM-target
//! store, so the outbound-target surface can offer "DM me" without any
//! vendor code in the host. The adapter receives a typed, proven external
//! actor id and may return that actor's direct conversation; adapters without
//! direct-target provisioning simply provision nothing.
//!
//! Provisioning is awaited after OAuth completion and never changes the
//! already-committed callback result. OAuth continuation can publish
//! activation just after the bind, so an inactive snapshot waits on the
//! generic host's publication signal and retries against each new generation
//! for a bounded interval.

use std::{sync::Arc, time::Duration};

use ironclaw_host_api::ids::UserId;

use ironclaw_extension_contracts::channel_identity::{
    ChannelIdentityPostBind, ChannelIdentityPostBindFactory,
};
use ironclaw_extension_contracts::{
    channel_adapter::DirectTargetProvisionRequest, external::ExternalActorId,
};
use ironclaw_product_contracts::delivery::ChannelDeliveryResolver;

use ironclaw_extension_host::{FilesystemChannelDmTargetStore, dm_target_payload};

const ACTIVATION_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
enum DmTargetProvisioningError {
    #[error("channel extension is not active in the snapshot")]
    ExtensionInactive,
    #[error("channel direct-target provisioning failed: {0}")]
    DirectTargetProvisioning(String),
    #[error("channel DM-target persistence failed: {0}")]
    Persistence(String),
    #[error("timed out waiting for channel extension activation")]
    ActivationTimeout,
    #[error("channel extension activation publisher stopped")]
    ActivationPublisherStopped,
}

/// Builds one generic post-bind provisioner per discovered channel
/// extension — registered on the identity-binding config by composition
/// wiring.
pub struct ChannelDmTargetProvisioning {
    delivery: Arc<dyn ChannelDeliveryResolver>,
    store: Arc<FilesystemChannelDmTargetStore>,
    snapshot_updates: tokio::sync::watch::Receiver<u64>,
}

impl ChannelDmTargetProvisioning {
    pub fn new(
        delivery: Arc<dyn ChannelDeliveryResolver>,
        store: Arc<FilesystemChannelDmTargetStore>,
        snapshot_updates: tokio::sync::watch::Receiver<u64>,
    ) -> Self {
        Self {
            delivery,
            store,
            snapshot_updates,
        }
    }
}

impl ChannelIdentityPostBindFactory for ChannelDmTargetProvisioning {
    fn post_bind_for_extension(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn ChannelIdentityPostBind>> {
        Some(Arc::new(ChannelDmTargetPostBind {
            extension_id: extension_id.to_string(),
            delivery: Arc::clone(&self.delivery),
            store: Arc::clone(&self.store),
            snapshot_updates: self.snapshot_updates.clone(),
        }))
    }
}

/// One extension's post-bind hook: open the DM in the background.
struct ChannelDmTargetPostBind {
    extension_id: String,
    delivery: Arc<dyn ChannelDeliveryResolver>,
    store: Arc<FilesystemChannelDmTargetStore>,
    snapshot_updates: tokio::sync::watch::Receiver<u64>,
}

#[async_trait::async_trait]
impl ChannelIdentityPostBind for ChannelDmTargetPostBind {
    async fn provision_after_bind(&self, user_id: UserId, external_actor_id: &str) {
        match provision_dm_target_after_bind(
            &self.extension_id,
            &self.delivery,
            &self.store,
            &user_id,
            external_actor_id,
            self.snapshot_updates.clone(),
        )
        .await
        {
            Ok(true) => tracing::debug!(
                extension_id = self.extension_id,
                "channel DM target provisioned after identity bind"
            ),
            Ok(false) => tracing::debug!(
                extension_id = self.extension_id,
                "channel DM target not provisioned (adapter offers no direct target)"
            ),
            Err(reason) => tracing::warn!(
                extension_id = self.extension_id,
                %reason,
                "channel DM-target provisioning failed after identity bind"
            ),
        }
    }
}

async fn provision_dm_target_after_bind(
    extension_id: &str,
    delivery: &Arc<dyn ChannelDeliveryResolver>,
    store: &Arc<FilesystemChannelDmTargetStore>,
    user_id: &UserId,
    external_actor_id: &str,
    mut snapshot_updates: tokio::sync::watch::Receiver<u64>,
) -> Result<bool, DmTargetProvisioningError> {
    let deadline = tokio::time::Instant::now() + ACTIVATION_PUBLICATION_TIMEOUT;
    loop {
        match provision_dm_target(extension_id, delivery, store, user_id, external_actor_id).await {
            Err(DmTargetProvisioningError::ExtensionInactive) => {
                match tokio::time::timeout_at(deadline, snapshot_updates.changed()).await {
                    Ok(Ok(())) => continue,
                    Ok(Err(_)) => {
                        return Err(DmTargetProvisioningError::ActivationPublisherStopped);
                    }
                    Err(_) => return Err(DmTargetProvisioningError::ActivationTimeout),
                }
            }
            result => return result,
        }
    }
}

/// The provisioning body (separable for tests): resolve the active channel
/// delivery, ask the adapter for the caller's direct conversation, persist
/// the generic record. `Ok(false)` when the adapter does not support direct
/// target provisioning or returns no conversation.
async fn provision_dm_target(
    extension_id: &str,
    delivery: &Arc<dyn ChannelDeliveryResolver>,
    store: &Arc<FilesystemChannelDmTargetStore>,
    user_id: &UserId,
    external_actor_id: &str,
) -> Result<bool, DmTargetProvisioningError> {
    let Some(channel) = delivery.resolve_channel_delivery(extension_id) else {
        return Err(DmTargetProvisioningError::ExtensionInactive);
    };
    // Direct-target provisioning is a delivery-half capability. A channel with no
    // delivery half cannot provision a DM target, which is the same "nothing
    // to do here" answer its `Unsupported` arm gives below — not an error.
    let Some(delivery_half) = channel.delivery.as_ref() else {
        return Ok(false);
    };
    let actor_id = ExternalActorId::new(external_actor_id)
        .map_err(|error| DmTargetProvisioningError::DirectTargetProvisioning(error.to_string()))?;
    let conversation = match delivery_half
        .provision_direct_target(
            DirectTargetProvisionRequest { actor_id },
            channel.egress.as_ref(),
        )
        .await
    {
        Ok(conversation) => conversation,
        Err(ironclaw_extension_contracts::channel_adapter::ChannelError::Unsupported) => {
            return Ok(false);
        }
        Err(error) => {
            return Err(DmTargetProvisioningError::DirectTargetProvisioning(
                error.to_string(),
            ));
        }
    };
    let Some(conversation) = conversation else {
        return Ok(false);
    };
    store
        .upsert(
            extension_id,
            user_id,
            external_actor_id.to_string(),
            dm_target_payload(conversation.space_id(), conversation.conversation_id()),
        )
        .await
        .map_err(|error| DmTargetProvisioningError::Persistence(error.to_string()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use async_trait::async_trait;
    use ironclaw_extension_contracts::channel_adapter::{
        ChannelDelivery, DirectTargetProvisionRequest,
    };
    use ironclaw_extension_contracts::channel_adapter::{
        ChannelError, DeliveryReport, OutboundEnvelope,
    };
    use ironclaw_extension_contracts::external::{ExternalActorId, ExternalConversationRef};
    use ironclaw_extension_contracts::tool_adapter::{
        RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ids::ExtensionId;
    use ironclaw_host_api::ids::TenantId;
    use ironclaw_host_api::product_adapter::AdapterInstallationId;
    use ironclaw_product_contracts::delivery::ResolvedChannelDelivery;

    use super::*;

    struct NoopEgress;

    #[async_trait]
    impl RestrictedEgress for NoopEgress {
        async fn send(
            &self,
            _request: RestrictedEgressRequest,
        ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            unreachable!("DM provisioning tests never reach the network")
        }
    }

    /// Adapter fake: records the typed actor and serves one direct conversation
    /// (or `Unsupported`).
    struct RecordingAdapter {
        actors: Mutex<Vec<ExternalActorId>>,
        conversation: Option<ExternalConversationRef>,
        unsupported: bool,
    }

    impl RecordingAdapter {
        fn with_candidate(space_id: Option<&str>, conversation_id: &str) -> Self {
            Self {
                actors: Mutex::new(Vec::new()),
                conversation: Some(
                    ExternalConversationRef::new(space_id, conversation_id, None, None)
                        .expect("conversation ref"),
                ),
                unsupported: false,
            }
        }

        fn unsupported() -> Self {
            Self {
                actors: Mutex::new(Vec::new()),
                conversation: None,
                unsupported: true,
            }
        }
    }

    #[async_trait]
    impl ChannelDelivery for RecordingAdapter {
        async fn deliver(
            &self,
            _envelope: OutboundEnvelope,
            _egress: &dyn RestrictedEgress,
        ) -> Result<DeliveryReport, ChannelError> {
            unreachable!("DM provisioning tests never deliver")
        }

        async fn provision_direct_target(
            &self,
            request: DirectTargetProvisionRequest,
            _egress: &dyn RestrictedEgress,
        ) -> Result<Option<ExternalConversationRef>, ChannelError> {
            self.actors
                .lock()
                .expect("actors lock")
                .push(request.actor_id);
            if self.unsupported {
                return Err(ChannelError::Unsupported);
            }
            Ok(self.conversation.clone())
        }
    }

    struct StaticDeliveryResolver {
        adapter: Arc<RecordingAdapter>,
    }

    impl ChannelDeliveryResolver for StaticDeliveryResolver {
        fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
            if extension_id != "vendorx" {
                return None;
            }
            Some(ResolvedChannelDelivery {
                extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
                installation_id: AdapterInstallationId::new("vendorx-install-1")
                    .expect("valid installation id"),
                reply: None,
                delivery: Some(Arc::clone(&self.adapter) as Arc<dyn ChannelDelivery>),
                egress: Arc::new(NoopEgress),
                reply_transport: Some(
                    ironclaw_extension_contracts::channel::ReplyTransport::Message,
                ),
                requires_enrollment: false,
                declared_egress_hosts: Vec::new(),
            })
        }
    }

    struct EventuallyActiveDeliveryResolver {
        active: Arc<AtomicBool>,
        adapter: Arc<RecordingAdapter>,
    }

    struct MissingDeliveryHalfResolver;

    impl ChannelDeliveryResolver for MissingDeliveryHalfResolver {
        fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
            Some(ResolvedChannelDelivery {
                extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
                installation_id: AdapterInstallationId::new("vendorx-install-1")
                    .expect("valid installation id"),
                reply: None,
                delivery: None,
                egress: Arc::new(NoopEgress),
                reply_transport: None,
                requires_enrollment: false,
                declared_egress_hosts: Vec::new(),
            })
        }
    }

    impl ChannelDeliveryResolver for EventuallyActiveDeliveryResolver {
        fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
            if extension_id != "vendorx" || !self.active.load(Ordering::SeqCst) {
                return None;
            }
            Some(ResolvedChannelDelivery {
                extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
                installation_id: AdapterInstallationId::new("vendorx-install-1")
                    .expect("valid installation id"),
                reply: None,
                delivery: Some(Arc::clone(&self.adapter) as Arc<dyn ChannelDelivery>),
                egress: Arc::new(NoopEgress),
                reply_transport: Some(
                    ironclaw_extension_contracts::channel::ReplyTransport::Message,
                ),
                requires_enrollment: false,
                declared_egress_hosts: Vec::new(),
            })
        }
    }

    fn store() -> Arc<FilesystemChannelDmTargetStore> {
        Arc::new(FilesystemChannelDmTargetStore::new(
            Arc::new(InMemoryBackend::new()),
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("operator").expect("user"),
        ))
    }

    #[tokio::test]
    async fn provisioning_opens_the_direct_conversation_and_persists_the_canonical_payload() {
        let adapter = Arc::new(RecordingAdapter::with_candidate(Some("S-9"), "DM-77"));
        let delivery: Arc<dyn ChannelDeliveryResolver> = Arc::new(StaticDeliveryResolver {
            adapter: Arc::clone(&adapter),
        });
        let store = store();
        let user = UserId::new("user-alice").expect("user");

        let provisioned = provision_dm_target("vendorx", &delivery, &store, &user, "U777")
            .await
            .expect("provisioning succeeds");
        assert!(provisioned);
        assert_eq!(
            adapter
                .actors
                .lock()
                .expect("actors lock")
                .iter()
                .map(ExternalActorId::as_str)
                .collect::<Vec<_>>(),
            vec!["U777"],
            "the adapter receives the typed external actor id"
        );
        let record = store
            .load("vendorx", &user)
            .await
            .expect("load")
            .expect("record persisted");
        assert_eq!(record.external_actor_id, "U777");
        assert_eq!(record.target["space_id"], "S-9");
        assert_eq!(record.target["conversation_id"], "DM-77");

        // The factory hands the same behavior to the identity hook.
        let provisioning = ChannelDmTargetProvisioning::new(
            Arc::clone(&delivery),
            Arc::clone(&store),
            tokio::sync::watch::channel(0_u64).1,
        );
        assert!(
            provisioning.post_bind_for_extension("vendorx").is_some(),
            "every discovered extension gets a post-bind provisioner"
        );
    }

    #[tokio::test]
    async fn adapters_without_target_listing_provision_nothing() {
        let adapter = Arc::new(RecordingAdapter::unsupported());
        let delivery: Arc<dyn ChannelDeliveryResolver> = Arc::new(StaticDeliveryResolver {
            adapter: Arc::clone(&adapter),
        });
        let store = store();
        let user = UserId::new("user-alice").expect("user");

        let provisioned = provision_dm_target("vendorx", &delivery, &store, &user, "U777")
            .await
            .expect("unsupported listing is not an error");
        assert!(!provisioned);
        assert!(store.load("vendorx", &user).await.expect("load").is_none());
    }

    #[tokio::test]
    async fn channels_without_a_delivery_half_provision_nothing() {
        let delivery: Arc<dyn ChannelDeliveryResolver> = Arc::new(MissingDeliveryHalfResolver);
        let store = store();
        let user = UserId::new("user-alice").expect("user");

        let provisioned = provision_dm_target("vendorx", &delivery, &store, &user, "U777")
            .await
            .expect("a missing delivery half is not an error");
        assert!(!provisioned);
        assert!(store.load("vendorx", &user).await.expect("load").is_none());
    }

    #[tokio::test]
    async fn inactive_extensions_fail_with_a_reason_and_persist_nothing() {
        let adapter = Arc::new(RecordingAdapter::with_candidate(None, "DM-1"));
        let delivery: Arc<dyn ChannelDeliveryResolver> = Arc::new(StaticDeliveryResolver {
            adapter: Arc::clone(&adapter),
        });
        let store = store();
        let user = UserId::new("user-alice").expect("user");

        let error = provision_dm_target("ghost", &delivery, &store, &user, "U777")
            .await
            .expect_err("inactive extension fails");
        assert!(
            matches!(error, DmTargetProvisioningError::ExtensionInactive),
            "{error}"
        );
        assert!(store.load("ghost", &user).await.expect("load").is_none());
    }

    #[tokio::test]
    async fn post_bind_provisioning_waits_for_extension_activation_publication() {
        let adapter = Arc::new(RecordingAdapter::with_candidate(Some("S-9"), "DM-77"));
        let active = Arc::new(AtomicBool::new(false));
        let delivery: Arc<dyn ChannelDeliveryResolver> =
            Arc::new(EventuallyActiveDeliveryResolver {
                active: Arc::clone(&active),
                adapter: Arc::clone(&adapter),
            });
        let store = store();
        let user = UserId::new("user-alice").expect("user");
        let (snapshot_published, snapshot_updates) = tokio::sync::watch::channel(0_u64);

        let provision = provision_dm_target_after_bind(
            "vendorx",
            &delivery,
            &store,
            &user,
            "U777",
            snapshot_updates,
        );
        let activate = async {
            tokio::task::yield_now().await;
            assert!(
                store
                    .load("vendorx", &user)
                    .await
                    .expect("load before activation")
                    .is_none(),
                "provisioning must wait while the channel extension is inactive"
            );
            active.store(true, Ordering::SeqCst);
            snapshot_published.send_replace(1);
        };

        let (result, ()) = tokio::join!(provision, activate);
        assert!(result.expect("provisioning resumes after activation"));
        assert!(
            store
                .load("vendorx", &user)
                .await
                .expect("load after activation")
                .is_some(),
            "activation publication should unblock and persist the DM target"
        );
    }
}
