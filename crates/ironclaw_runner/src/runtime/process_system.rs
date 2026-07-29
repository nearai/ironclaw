use std::sync::Arc;

use ironclaw_processes::{
    ProcessCheckpointPort, ProcessDependencyPort, ProcessInputPort, ProcessJournalCommitObserver,
    ProcessJournalObserverRegistry, ProcessJournalSource, ProcessJournalStoreError,
    ProcessRuntimePort, ProcessSubmissionPort, ProcessTransitionPort,
};
use ironclaw_turns::ProcessJournalStoreTurnAdapter;

#[derive(Clone)]
pub struct ProcessRuntimeSystem {
    runtime: Arc<dyn ProcessRuntimePort>,
    submission: Arc<dyn ProcessSubmissionPort<Error = ProcessJournalStoreError>>,
    dependencies: Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>>,
    adapter: Arc<ProcessJournalStoreTurnAdapter>,
    observers: Arc<dyn ProcessJournalObserverRegistry>,
}

impl ProcessRuntimeSystem {
    pub fn from_process_journal_store<F>(
        store: Arc<ironclaw_processes::ProcessJournalStore<F>>,
    ) -> Self
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        let adapter = Arc::new(ProcessJournalStoreTurnAdapter::new(
            Arc::clone(&store) as Arc<dyn ProcessRuntimePort>
        ));
        Self {
            runtime: Arc::clone(&store) as Arc<dyn ProcessRuntimePort>,
            submission: Arc::clone(&store)
                as Arc<dyn ProcessSubmissionPort<Error = ProcessJournalStoreError>>,
            dependencies: Arc::clone(&store)
                as Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>>,
            adapter,
            observers: store as Arc<dyn ProcessJournalObserverRegistry>,
        }
    }

    pub fn runtime(&self) -> Arc<dyn ProcessRuntimePort> {
        Arc::clone(&self.runtime)
    }

    pub fn in_memory_ephemeral() -> Result<Self, String> {
        let mounts = ironclaw_host_api::MountView::new(vec![ironclaw_host_api::MountGrant::new(
            ironclaw_host_api::MountAlias::new("/processes")
                .map_err(|error| format!("process mount alias: {error}"))?,
            ironclaw_host_api::VirtualPath::new("/engine/processes")
                .map_err(|error| format!("process mount path: {error}"))?,
            ironclaw_host_api::MountPermissions::read_write_list_delete(),
        )])
        .map_err(|error| format!("process mount view: {error}"))?;
        let filesystem = Arc::new(ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
            Arc::new(ironclaw_filesystem::InMemoryBackend::new()),
            mounts,
        ));
        Ok(Self::from_process_journal_store(Arc::new(
            ironclaw_processes::ProcessJournalStore::new(filesystem),
        )))
    }

    pub fn submission(&self) -> Arc<dyn ProcessSubmissionPort<Error = ProcessJournalStoreError>> {
        Arc::clone(&self.submission)
    }

    pub fn dependencies(&self) -> Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>> {
        Arc::clone(&self.dependencies)
    }

    pub fn transitions(&self) -> Arc<dyn ProcessTransitionPort<Error = ironclaw_turns::TurnError>> {
        Arc::clone(&self.adapter)
            as Arc<dyn ProcessTransitionPort<Error = ironclaw_turns::TurnError>>
    }

    pub fn journal(&self) -> Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>> {
        Arc::clone(&self.adapter)
            as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>
    }

    pub fn controls(
        &self,
    ) -> Arc<dyn ironclaw_processes::ProcessControlPort<Error = ironclaw_turns::TurnError>> {
        Arc::clone(&self.adapter)
            as Arc<dyn ironclaw_processes::ProcessControlPort<Error = ironclaw_turns::TurnError>>
    }

    pub fn lifecycle(
        &self,
    ) -> Arc<dyn ironclaw_processes::ProcessLifecycleLookupSource<Error = ironclaw_turns::TurnError>>
    {
        Arc::clone(&self.adapter)
            as Arc<
                dyn ironclaw_processes::ProcessLifecycleLookupSource<
                        Error = ironclaw_turns::TurnError,
                    >,
            >
    }

    pub fn gates(
        &self,
    ) -> Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = ironclaw_turns::TurnError>>
    {
        Arc::clone(&self.adapter)
            as Arc<
                dyn ironclaw_processes::ProcessGateQuerySource<Error = ironclaw_turns::TurnError>,
            >
    }

    pub fn checkpoints(&self) -> Arc<dyn ProcessCheckpointPort<Error = ironclaw_turns::TurnError>> {
        Arc::clone(&self.adapter)
            as Arc<dyn ProcessCheckpointPort<Error = ironclaw_turns::TurnError>>
    }

    pub fn inputs(&self) -> Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>> {
        Arc::clone(&self.adapter) as Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>>
    }

    pub fn agent_turn_runtime(&self) -> ironclaw_turns::AgentTurnProcessRuntime {
        ironclaw_turns::AgentTurnProcessRuntime::from_process_adapter(Arc::clone(&self.adapter))
    }

    pub fn subscribe_process_observer(
        &self,
        observer: Arc<dyn ProcessJournalCommitObserver>,
    ) -> Result<(), String> {
        self.observers.subscribe_process_observer(observer)
    }
}
