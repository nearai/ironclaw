//! Durable Telegram host state over one erased scoped filesystem.
//!
//! Setup, pairing, identity bindings, and DM targets are one Telegram-owned
//! persistence domain. Services call the inherent methods on this concrete
//! state instead of introducing same-crate store traits with one production
//! implementation.

mod bindings;
mod dm_targets;
mod pairing;
mod records;
mod setup;

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, FilesystemError, RecordVersion, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use serde::{Serialize, de::DeserializeOwned};

/// One concrete Telegram persistence owner. The root backend is erased before
/// entering this domain so service types never become generic over storage.
#[derive(Clone)]
pub struct FilesystemTelegramHostState {
    pub(super) filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    pub(super) scope: ResourceScope,
}

impl std::fmt::Debug for FilesystemTelegramHostState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemTelegramHostState")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl FilesystemTelegramHostState {
    pub fn new(
        filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            filesystem,
            scope: ResourceScope {
                tenant_id,
                user_id,
                agent_id: Some(agent_id),
                project_id,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
        }
    }

    pub(super) async fn read_record<T>(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<(T, RecordVersion)>, FilesystemError>
    where
        T: DeserializeOwned,
    {
        ironclaw_channel_host::host_state_records::read_json_record(
            &self.filesystem,
            &self.scope,
            path,
            "Telegram host-state",
        )
        .await
    }

    pub(super) async fn write_record<T>(
        &self,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError>
    where
        T: Serialize,
    {
        ironclaw_channel_host::host_state_records::write_json_record(
            &self.filesystem,
            &self.scope,
            path,
            value,
            cas,
            "Telegram host-state",
        )
        .await
    }

    pub(super) async fn delete_record(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        self.filesystem.delete(&self.scope, path).await
    }
}

pub use records::TELEGRAM_INSTALLATION_SETUP_PATH;
