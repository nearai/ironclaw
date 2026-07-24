//! Generic post-bind DM-target provisioning (extension-runtime §5.5).
//!
//! After the generic channel-identity hook binds a proven vendor identity,
//! the caller's personal direct conversation is opened through the
//! extension's own channel adapter and persisted in the generic DM-target
//! store, so the outbound-target surface can offer "DM me" without any
//! vendor code in the host. The adapter interprets the direct-conversation
//! target query grammar `im:{external_actor_id}` (the same `list_targets`
//! convention the retired lane used); adapters without target listing
//! simply provision nothing.
//!
//! Provisioning is fire-and-forget and never fails the callback that already
//! bound the identity. OAuth continuation can publish activation just after
//! the bind, so an inactive snapshot waits on the generic host's publication
//! signal and retries against each new generation for a bounded interval.

use std::{sync::Arc, time::Duration};

use ironclaw_host_api::{ChannelIdentityPostBind, ChannelIdentityPostBindFactory, UserId};
use ironclaw_product::ChannelDeliveryResolver;
use ironclaw_product::TargetQuery;

use ironclaw_extension_host::{FilesystemChannelDmTargetStore, dm_target_payload};

const ACTIVATION_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
enum DmTargetProvisioningError {
    #[error("channel extension is not active in the snapshot")]
    ExtensionInactive,
    #[error("channel target discovery failed: {0}")]
    TargetDiscovery(String),
    #[error("channel DM-target persistence failed: {0}")]
    Persistence(String),
    #[error("timed out waiting for channel extension activation")]
    ActivationTimeout,
    #[error("channel extension activation publisher stopped")]
    ActivationPublisherStopped,
}

/// The direct-conversation target query grammar adapters interpret.
fn direct_conversation_query(external_actor_id: &str) -> String {
    format!("im:{external_actor_id}")
}

/// Builds one generic post-bind provisioner per discovered channel
/// extension — registered on the identity-binding config by composition
/// wiring.
pub(crate) struct ChannelDmTargetProvisioning {
    delivery: Arc<dyn ChannelDeliveryResolver>,
    store: Arc<FilesystemChannelDmTargetStore>,
    snapshot_updates: tokio::sync::watch::Receiver<u64>,
}

impl ChannelDmTargetProvisioning {
    pub(crate) fn new(
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

impl ChannelIdentityPostBind for ChannelDmTargetPostBind {
    fn provision_after_bind(&self, user_id: UserId, external_actor_id: &str) {
        let extension_id = self.extension_id.clone();
        let delivery = Arc::clone(&self.delivery);
        let store = Arc::clone(&self.store);
        let snapshot_updates = self.snapshot_updates.clone();
        let external_actor_id = external_actor_id.to_string();
        tokio::spawn(async move {
            match provision_dm_target_after_bind(
                &extension_id,
                &delivery,
                &store,
                &user_id,
                &external_actor_id,
                snapshot_updates,
            )
            .await
            {
                Ok(true) => tracing::debug!(
                    extension_id,
                    "channel DM target provisioned after identity bind"
                ),
                Ok(false) => tracing::debug!(
                    extension_id,
                    "channel DM target not provisioned (adapter offers no direct target)"
                ),
                Err(reason) => tracing::warn!(
                    extension_id,
                    %reason,
                    "channel DM-target provisioning failed after identity bind"
                ),
            }
        });
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
/// the generic record. `Ok(false)` when the adapter does not support target
/// listing or returns no candidate.
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
    let candidates = match channel
        .adapter
        .list_targets(
            TargetQuery {
                extension_id: channel.extension_id.clone(),
                installation_id: channel.installation_id.clone(),
                query: Some(direct_conversation_query(external_actor_id)),
                limit: 1,
            },
            channel.egress.as_ref(),
        )
        .await
    {
        Ok(candidates) => candidates,
        Err(ironclaw_product::ChannelError::Unsupported) => return Ok(false),
        Err(error) => {
            return Err(DmTargetProvisioningError::TargetDiscovery(
                error.to_string(),
            ));
        }
    };
    let Some(candidate) = candidates.first() else {
        return Ok(false);
    };
    store
        .upsert(
            extension_id,
            user_id,
            external_actor_id.to_string(),
            dm_target_payload(
                candidate.conversation.space_id(),
                candidate.conversation.conversation_id(),
            ),
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
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
        TenantId,
    };
    use ironclaw_product::ResolvedChannelDelivery;
    use ironclaw_product::{
        ChannelAdapter, ChannelError, DeliveryReport, ExternalConversationRef, InboundOutcome,
        OutboundEnvelope, TargetCandidate, VerifiedInbound,
    };

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

    /// Adapter fake: records the target query and serves one DM candidate
    /// (or `Unsupported`).
    struct RecordingAdapter {
        queries: Mutex<Vec<Option<String>>>,
        candidate: Option<TargetCandidate>,
        unsupported: bool,
    }

    impl RecordingAdapter {
        fn with_candidate(space_id: Option<&str>, conversation_id: &str) -> Self {
            Self {
                queries: Mutex::new(Vec::new()),
                candidate: Some(TargetCandidate {
                    conversation: ExternalConversationRef::new(
                        space_id,
                        conversation_id,
                        None,
                        None,
                    )
                    .expect("conversation ref"),
                    display_name: "Direct message".to_string(),
                }),
                unsupported: false,
            }
        }

        fn unsupported() -> Self {
            Self {
                queries: Mutex::new(Vec::new()),
                candidate: None,
                unsupported: true,
            }
        }
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
            unreachable!("DM provisioning tests never parse inbound requests")
        }

        async fn deliver(
            &self,
            _envelope: OutboundEnvelope,
            _egress: &dyn RestrictedEgress,
        ) -> Result<DeliveryReport, ChannelError> {
            unreachable!("DM provisioning tests never deliver")
        }

        async fn list_targets(
            &self,
            query: TargetQuery,
            _egress: &dyn RestrictedEgress,
        ) -> Result<Vec<TargetCandidate>, ChannelError> {
            self.queries
                .lock()
                .expect("queries lock")
                .push(query.query.clone());
            if self.unsupported {
                return Err(ChannelError::Unsupported);
            }
            Ok(self.candidate.clone().into_iter().collect())
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
                extension_id: extension_id.to_string(),
                installation_id: "vendorx-install-1".to_string(),
                adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
                egress: Arc::new(NoopEgress),
            })
        }
    }

    struct EventuallyActiveDeliveryResolver {
        active: Arc<AtomicBool>,
        adapter: Arc<RecordingAdapter>,
    }

    impl ChannelDeliveryResolver for EventuallyActiveDeliveryResolver {
        fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
            if extension_id != "vendorx" || !self.active.load(Ordering::SeqCst) {
                return None;
            }
            Some(ResolvedChannelDelivery {
                extension_id: extension_id.to_string(),
                installation_id: "vendorx-install-1".to_string(),
                adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
                egress: Arc::new(NoopEgress),
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
            adapter.queries.lock().expect("queries lock").clone(),
            vec![Some("im:U777".to_string())],
            "the adapter receives the direct-conversation target query"
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
