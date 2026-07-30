//! Process-backed fixtures for cross-crate turn projection tests.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_processes::{
    GetProcessCheckpointRequest, ProcessCheckpointId, ProcessCheckpointPort,
    ProcessCheckpointRecord, ProcessJournalStore, ProcessRuntimePort, ProcessTransitionPort,
    RecordProcessCheckpointRequest,
};

use crate::{
    AgentTurnProcessRuntime, ProcessJournalStoreTurnAdapter, ProcessLoopCheckpointStore, TurnError,
};

#[derive(Clone)]
pub struct InMemoryAgentTurnProcessSystem {
    store: Arc<ProcessJournalStore<InMemoryBackend>>,
    adapter: Arc<ProcessJournalStoreTurnAdapter>,
    runtime: AgentTurnProcessRuntime,
}

impl InMemoryAgentTurnProcessSystem {
    pub fn new() -> Self {
        let store = Arc::new(ProcessJournalStore::new(in_memory_processes_filesystem()));
        let adapter = Arc::new(ProcessJournalStoreTurnAdapter::new(
            Arc::clone(&store) as Arc<dyn ProcessRuntimePort>
        ));
        let runtime = AgentTurnProcessRuntime::from_process_adapter(Arc::clone(&adapter));
        Self {
            store,
            adapter,
            runtime,
        }
    }

    pub fn runtime(&self) -> AgentTurnProcessRuntime {
        self.runtime.clone()
    }

    pub fn transitions(&self) -> Arc<dyn ProcessTransitionPort<Error = TurnError>> {
        Arc::clone(&self.adapter) as Arc<dyn ProcessTransitionPort<Error = TurnError>>
    }

    pub fn store(&self) -> Arc<ProcessJournalStore<InMemoryBackend>> {
        Arc::clone(&self.store)
    }
}

impl Default for InMemoryAgentTurnProcessSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub fn in_memory_agent_turn_process_system() -> InMemoryAgentTurnProcessSystem {
    InMemoryAgentTurnProcessSystem::new()
}

/// Process-backed agent-turn projection fixture.
pub fn in_memory_agent_turn_runtime() -> AgentTurnProcessRuntime {
    in_memory_agent_turn_process_system().runtime()
}

pub fn in_memory_loop_checkpoint_store() -> ProcessLoopCheckpointStore {
    ProcessLoopCheckpointStore::new(Arc::new(InMemoryProcessCheckpointPort::default()))
}

#[derive(Default)]
struct InMemoryProcessCheckpointPort {
    records: Mutex<HashMap<ProcessCheckpointId, ProcessCheckpointRecord>>,
}

#[async_trait]
impl ProcessCheckpointPort for InMemoryProcessCheckpointPort {
    type Error = TurnError;

    async fn record_process_checkpoint(
        &self,
        request: RecordProcessCheckpointRequest,
    ) -> Result<ProcessCheckpointRecord, Self::Error> {
        let record = ProcessCheckpointRecord {
            checkpoint_id: request.checkpoint_id.clone(),
            process_id: request.process_id,
            scope: request.scope,
            state_ref: request.state_ref,
            payload: request.payload,
            created_at: request.created_at,
            metadata: request.metadata,
        };
        self.records
            .lock()
            .map_err(|_| TurnError::Unavailable {
                reason: "process checkpoint fixture lock poisoned".to_string(),
            })?
            .insert(request.checkpoint_id, record.clone());
        Ok(record)
    }

    async fn get_process_checkpoint(
        &self,
        request: GetProcessCheckpointRequest,
    ) -> Result<Option<ProcessCheckpointRecord>, Self::Error> {
        Ok(self
            .records
            .lock()
            .map_err(|_| TurnError::Unavailable {
                reason: "process checkpoint fixture lock poisoned".to_string(),
            })?
            .get(&request.checkpoint_id)
            .filter(|record| {
                record.process_id == request.process_id && record.scope == request.scope
            })
            .cloned())
    }
}

pub fn in_memory_processes_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("processes alias"),
        VirtualPath::new("/engine/processes").expect("processes target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("processes mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}
