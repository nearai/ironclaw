use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::AdapterInstallationId;
use async_trait::async_trait;
use ironclaw_conversations::{
    AdapterInstallationId as ConversationInstallationId, AdapterKind,
    RebornFilesystemConversationServices,
};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ExtensionId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    TenantId, VirtualPath,
};

use crate::{IdempotencyLedger, RebornFilesystemIdempotencyLedger};
use ironclaw_host_api::ProductSurfaceCaller;

const CHANNEL_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const CHANNEL_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;

/// Extension-keyed durable roots for the product-owned channel workflow
/// state. This layout is private so ingress and disconnect cannot diverge.
#[derive(Debug, Clone)]
struct ChannelWorkflowStorageRoots {
    idempotency: VirtualPath,
    conversations: VirtualPath,
}

/// Derive the default per-tenant, per-extension storage roots.
fn channel_workflow_storage_roots(
    tenant_id: &TenantId,
    extension_id: &ExtensionId,
) -> Result<ChannelWorkflowStorageRoots, ChannelWorkflowStateError> {
    let tenant = tenant_path_segment(tenant_id.as_str());
    let base = format!(
        "/tenants/{tenant}/shared/channel-extensions/{}",
        extension_id.as_str()
    );
    Ok(ChannelWorkflowStorageRoots {
        idempotency: VirtualPath::new(format!("{base}/product-workflow/idempotency"))
            .map_err(|error| ChannelWorkflowStateError::InvalidStorageRoot(error.to_string()))?,
        conversations: VirtualPath::new(format!("{base}/conversations"))
            .map_err(|error| ChannelWorkflowStateError::InvalidStorageRoot(error.to_string()))?,
    })
}

/// Durable services shared by inbound channel admission and disconnect
/// cleanup for one extension.
pub struct ChannelWorkflowState {
    pub ledger: Arc<dyn IdempotencyLedger>,
    pub conversations: Arc<RebornFilesystemConversationServices>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelWorkflowStateError {
    #[error("invalid channel workflow storage root: {0}")]
    InvalidStorageRoot(String),
    #[error("invalid channel workflow mount: {0}")]
    InvalidMount(String),
    #[error("durable channel conversation store unavailable: {0}")]
    ConversationStoreUnavailable(String),
    #[error("invalid channel conversation identity: {0}")]
    InvalidConversationIdentity(String),
    #[error("channel conversation cleanup failed: {0}")]
    ConversationCleanup(String),
}

/// Concrete product-owned durable channel workflow state service.
///
/// Composition supplies only the root filesystem. This service owns the
/// storage layout, fixed mounts, conversation identity conversion, and
/// per-user/per-installation cleanup semantics.
pub struct ChannelWorkflowStateService {
    filesystem: Arc<dyn RootFilesystem>,
}

impl ChannelWorkflowStateService {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self { filesystem }
    }

    /// Build the canonical durable workflow state for one extension.
    ///
    /// The tenant comes from the same typed scope used by the ledger, and the
    /// owner derives both roots. Callers cannot supply a parallel layout.
    pub async fn build_for_extension(
        &self,
        extension_id: &ExtensionId,
        ledger_scope: ResourceScope,
    ) -> Result<ChannelWorkflowState, ChannelWorkflowStateError> {
        let roots = channel_workflow_storage_roots(&ledger_scope.tenant_id, extension_id)?;
        self.build_at_roots(&roots, ledger_scope).await
    }

    async fn build_at_roots(
        &self,
        roots: &ChannelWorkflowStorageRoots,
        ledger_scope: ResourceScope,
    ) -> Result<ChannelWorkflowState, ChannelWorkflowStateError> {
        let mount = |alias: &str,
                     target: &VirtualPath|
         -> Result<MountGrant, ChannelWorkflowStateError> {
            Ok(MountGrant::new(
                MountAlias::new(alias)
                    .map_err(|error| ChannelWorkflowStateError::InvalidMount(error.to_string()))?,
                target.clone(),
                MountPermissions::read_write_list_delete(),
            ))
        };
        let view = MountView::new(vec![
            mount("/engine/product_workflow/idempotency", &roots.idempotency)?,
            mount("/conversations", &roots.conversations)?,
        ])
        .map_err(|error| ChannelWorkflowStateError::InvalidMount(error.to_string()))?;
        let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&self.filesystem),
            view,
        ));
        let settled_limit = NonZeroUsize::new(CHANNEL_IDEMPOTENCY_LEDGER_SETTLED_LIMIT)
            .ok_or_else(|| ChannelWorkflowStateError::InvalidMount("zero settled limit".into()))?;
        let prune_interval = NonZeroUsize::new(CHANNEL_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL)
            .ok_or_else(|| ChannelWorkflowStateError::InvalidMount("zero prune interval".into()))?;
        let ledger = RebornFilesystemIdempotencyLedger::new(Arc::clone(&scoped), ledger_scope)
            .with_settled_entry_limit(settled_limit)
            .with_settled_prune_interval(prune_interval);
        let conversations = RebornFilesystemConversationServices::new(scoped)
            .await
            .map_err(|error| {
                ChannelWorkflowStateError::ConversationStoreUnavailable(error.to_string())
            })?;
        Ok(ChannelWorkflowState {
            ledger: Arc::new(ledger),
            conversations: Arc::new(conversations),
        })
    }

    /// Remove only the caller-owned conversation bindings for this channel,
    /// narrowed to the installation when one is available.
    pub async fn cleanup_conversation_bindings(
        &self,
        caller: &ProductSurfaceCaller,
        extension_id: &ExtensionId,
        installation_id: Option<&AdapterInstallationId>,
    ) -> Result<(), ChannelWorkflowStateError> {
        let state = self
            .build_for_extension(
                extension_id,
                ResourceScope {
                    tenant_id: caller.tenant_id.clone(),
                    user_id: caller.user_id.clone(),
                    agent_id: caller.agent_id.clone(),
                    project_id: caller.project_id.clone(),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
            )
            .await?;
        let adapter_kind = AdapterKind::new(extension_id.as_str()).map_err(|error| {
            ChannelWorkflowStateError::InvalidConversationIdentity(error.to_string())
        })?;
        let installation_id = installation_id
            .map(|installation_id| ConversationInstallationId::new(installation_id.as_str()))
            .transpose()
            .map_err(|error| {
                ChannelWorkflowStateError::InvalidConversationIdentity(error.to_string())
            })?;
        state
            .conversations
            .unpair_external_actors_owned_by(
                &caller.tenant_id,
                &adapter_kind,
                installation_id.as_ref(),
                &caller.user_id,
            )
            .await
            .map_err(|error| ChannelWorkflowStateError::ConversationCleanup(error.to_string()))?;
        Ok(())
    }
}

fn tenant_path_segment(value: &str) -> &str {
    if value == ironclaw_host_api::SYSTEM_RESERVED_ID {
        "__system__"
    } else {
        value
    }
}

/// Ordered disconnect actions supplied by the caller-facing adapter.
///
/// The product owner fixes the transaction order while composition adapts
/// its provider/auth ports without introducing another runtime-polymorphic
/// production service.
#[async_trait]
pub trait ChannelDisconnectActions: Send + Sync {
    type Error;

    async fn disconnect_pairing(&self) -> Result<(), Self::Error>;
    async fn revoke_personal_credentials(&self) -> Result<(), Self::Error>;
    async fn cleanup_vendor_state(&self) -> Result<(), Self::Error>;
    async fn cleanup_conversation_bindings(&self) -> Result<(), Self::Error>;
    async fn delete_identity_bindings(&self) -> Result<(), Self::Error>;
}

/// Run disconnect cleanup with identity deletion as the final commit point.
pub async fn disconnect_channel_in_order<A>(actions: &A) -> Result<(), A::Error>
where
    A: ChannelDisconnectActions,
{
    actions.disconnect_pairing().await?;
    actions.revoke_personal_credentials().await?;
    actions.cleanup_vendor_state().await?;
    actions.cleanup_conversation_bindings().await?;
    actions.delete_identity_bindings().await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_filesystem::{
        FaultInjecting, FilesystemOperation, InMemoryBackend, RootFilesystem,
    };
    use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, TenantId, UserId};

    use super::{
        ChannelDisconnectActions, ChannelWorkflowStateService, disconnect_channel_in_order,
    };

    struct RecordingActions {
        calls: Mutex<Vec<&'static str>>,
        fail_at: Option<&'static str>,
    }

    impl RecordingActions {
        fn new(fail_at: Option<&'static str>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_at,
            }
        }

        fn record(&self, step: &'static str) -> Result<(), &'static str> {
            self.calls.lock().expect("calls").push(step);
            if self.fail_at == Some(step) {
                Err(step)
            } else {
                Ok(())
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().expect("calls").clone()
        }
    }

    #[async_trait]
    impl ChannelDisconnectActions for RecordingActions {
        type Error = &'static str;

        async fn disconnect_pairing(&self) -> Result<(), Self::Error> {
            self.record("pairing")
        }

        async fn revoke_personal_credentials(&self) -> Result<(), Self::Error> {
            self.record("credentials")
        }

        async fn cleanup_vendor_state(&self) -> Result<(), Self::Error> {
            self.record("vendor")
        }

        async fn cleanup_conversation_bindings(&self) -> Result<(), Self::Error> {
            self.record("conversations")
        }

        async fn delete_identity_bindings(&self) -> Result<(), Self::Error> {
            self.record("identity")
        }
    }

    #[tokio::test]
    async fn disconnect_sequence_keeps_identity_as_the_commit_point() {
        let actions = RecordingActions::new(None);

        disconnect_channel_in_order(&actions)
            .await
            .expect("disconnect");

        assert_eq!(
            actions.calls(),
            vec![
                "pairing",
                "credentials",
                "vendor",
                "conversations",
                "identity",
            ]
        );
    }

    #[tokio::test]
    async fn disconnect_sequence_stops_before_identity_when_conversation_cleanup_fails() {
        let actions = RecordingActions::new(Some("conversations"));

        let error = disconnect_channel_in_order(&actions)
            .await
            .expect_err("conversation cleanup failure");

        assert_eq!(error, "conversations");
        assert_eq!(
            actions.calls(),
            vec!["pairing", "credentials", "vendor", "conversations"]
        );
    }

    #[tokio::test]
    async fn workflow_state_service_derives_the_canonical_extension_root() {
        let filesystem = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
        let root: Arc<dyn RootFilesystem> = filesystem.clone();
        let service = ChannelWorkflowStateService::new(root);
        let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
        let extension_id = ExtensionId::new("vendorx").expect("extension");
        let mut scope = ResourceScope::local_default(
            UserId::new("user:alice").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        scope.tenant_id = tenant_id;

        service
            .build_for_extension(&extension_id, scope)
            .await
            .expect("build canonical workflow state");

        assert_eq!(
            filesystem.recorded_paths(FilesystemOperation::ReadFile),
            vec![
                ironclaw_host_api::VirtualPath::new(
                    "/tenants/tenant:alpha/shared/channel-extensions/vendorx/conversations/state.json",
                )
                .expect("canonical path"),
            ],
            "callers cannot select a root that diverges from disconnect cleanup"
        );
    }
}
