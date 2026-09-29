//! Index users' tool catalogs when the catalog changes, not in their turns.
//!
//! A ranker that embeds tool documents ([`ToolRetrievalProvider`]) needs a
//! vector per authorized tool before it can rank. Left to the turn, the first
//! turn after a change (a restart, an installed extension, a re-discovered MCP
//! server) pays for embedding every new document. [`ToolCatalogIndexer`] moves
//! that work off the turn: it derives each known user's authorized catalog the
//! way a turn does (the runner's own capability-port stack, minus disclosure,
//! for a synthetic run of that user) and hands it to
//! [`ToolRetrievalProvider::index_in_background`], which embeds only what is
//! missing.
//!
//! # When it runs
//!
//! [`ToolCatalogIndexer::spawn`] runs one pass at once (the startup warm-up)
//! and another after every change signal, debounced so a burst of lifecycle
//! transitions costs one pass. The signal is any `watch` counter the host
//! bumps when the tool catalog changes; composition wires the active
//! extension registry's.
//!
//! # Whose catalogs
//!
//! The runtime's default user, plus the most recent users who ran a turn in
//! this process ([`ToolCatalogIndexer::note_run`]), up to
//! [`MAX_INDEXED_OWNERS`]. A user outside that set is not re-indexed on a
//! change; their next fit embeds what their catalog gained, as before.
//!
//! # Bounds and failure
//!
//! A pass is sequential over owners and only *requests* indexing: the
//! provider bounds the embedding work itself. A user whose catalog cannot be
//! derived is skipped with a `debug!` line. The task logs at `debug!` only.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironclaw_host_api::ids::{AgentId, ProjectId, ThreadId};
use ironclaw_host_api::turn::{TurnActor, TurnId, TurnRunId, TurnScope, TurnThreadOwner};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopRunContext, RunProfileResolutionRequest,
    RunProfileResolver, ToolCorpusOwner, ToolRetrievalProvider, VisibleCapabilityRequest,
};
use tokio::sync::watch;
use tracing::debug;

use crate::capability_port::LoopCapabilityPortFactory;

/// Most users whose catalogs are kept indexed.
pub const MAX_INDEXED_OWNERS: usize = 64;

/// How long a change signal waits for further changes before a pass.
pub const TOOL_CATALOG_INDEX_DEBOUNCE: Duration = Duration::from_millis(250);

const LOG_TARGET: &str = "ironclaw::reborn::tool_catalog_index";
/// Thread id of the synthetic run a catalog is derived for. It is never
/// persisted: deriving a catalog creates no thread, turn, or run record.
const INDEX_THREAD_ID: &str = "tool-catalog-index";

/// The owner of a run's tool catalog: the run's tenant and acting user (its
/// actor, else its explicit thread owner). `None` for a run with neither,
/// whose fits stay owner-blind.
pub fn tool_corpus_owner_for_run(run_context: &LoopRunContext) -> Option<ToolCorpusOwner> {
    let user_id = run_context
        .actor()
        .map(|actor| actor.user_id.clone())
        .or_else(|| run_context.scope.explicit_owner_user_id().cloned())?;
    Some(ToolCorpusOwner::new(
        run_context.scope.tenant_id.clone(),
        user_id,
    ))
}

/// Derives users' authorized tool catalogs and asks the retrieval provider to
/// index them in the background. See the module docs.
pub struct ToolCatalogIndexer {
    catalog: Arc<dyn LoopCapabilityPortFactory>,
    retrieval: Arc<dyn ToolRetrievalProvider>,
    profiles: Arc<dyn RunProfileResolver>,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    owners: Mutex<VecDeque<ToolCorpusOwner>>,
}

impl std::fmt::Debug for ToolCatalogIndexer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolCatalogIndexer")
            .field("ranker_version", &self.retrieval.ranker_version())
            .finish_non_exhaustive()
    }
}

impl ToolCatalogIndexer {
    /// `catalog` must build the same port stack a turn's disclosure layer
    /// wraps, so the documents indexed are the documents a turn fits.
    /// `agent_id`/`project_id` are the runtime's default run axes;
    /// `default_owner`, when known, is indexed from the first pass.
    pub fn new(
        catalog: Arc<dyn LoopCapabilityPortFactory>,
        retrieval: Arc<dyn ToolRetrievalProvider>,
        profiles: Arc<dyn RunProfileResolver>,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        default_owner: Option<ToolCorpusOwner>,
    ) -> Self {
        Self {
            catalog,
            retrieval,
            profiles,
            agent_id,
            project_id,
            owners: Mutex::new(default_owner.into_iter().collect()),
        }
    }

    /// Remember the owner of a run's catalog, most recent first, so later
    /// catalog changes re-index it.
    pub fn note_run(&self, run_context: &LoopRunContext) {
        let Some(owner) = tool_corpus_owner_for_run(run_context) else {
            return;
        };
        let Ok(mut owners) = self.owners.lock() else {
            return;
        };
        if owners.front() == Some(&owner) {
            return;
        }
        owners.retain(|known| known != &owner);
        owners.push_front(owner);
        owners.truncate(MAX_INDEXED_OWNERS);
    }

    /// The owners a pass indexes, most recent first.
    pub fn owners(&self) -> Vec<ToolCorpusOwner> {
        self.owners
            .lock()
            .map(|owners| owners.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Derive `owner`'s authorized catalog and hand it to the provider.
    /// Returns how many tool definitions were handed over.
    pub async fn index_owner(&self, owner: &ToolCorpusOwner) -> Result<usize, AgentLoopHostError> {
        let profile = self
            .profiles
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    format!("tool catalog indexing could not resolve the run profile: {error}"),
                )
            })?;
        let thread_id = ThreadId::new(INDEX_THREAD_ID).map_err(|error| {
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
        })?;
        let scope = TurnScope {
            tenant_id: owner.tenant_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            thread_id,
            thread_owner: TurnThreadOwner::explicit(Some(owner.user_id.clone())),
        };
        let run_context = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
            .with_actor(TurnActor::new(owner.user_id.clone()));
        let port = self.catalog.create_capability_port(&run_context).await?;
        let surface = port.visible_capabilities(VisibleCapabilityRequest).await?;
        // The same filter the disclosure layer applies before it fits: the
        // definitions of the capabilities visible to this user.
        let visible: BTreeSet<_> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let definitions: Vec<_> = port
            .tool_definitions()?
            .into_iter()
            .filter(|definition| visible.contains(&definition.capability_id))
            .collect();
        self.retrieval.index_in_background(owner, &definitions);
        Ok(definitions.len())
    }

    /// One pass over every known owner.
    pub async fn index_all(&self) {
        for owner in self.owners() {
            match self.index_owner(&owner).await {
                Ok(definitions) => debug!(
                    target: LOG_TARGET,
                    definitions,
                    "requested background indexing of a user's tool catalog"
                ),
                Err(error) => debug!(
                    target: LOG_TARGET,
                    error_kind = ?error.kind,
                    "could not derive a user's tool catalog for indexing"
                ),
            }
        }
    }

    /// Run the startup pass now, then a pass after each change on `changes`
    /// until the handle is shut down or the signal's sender is dropped.
    pub fn spawn(self: Arc<Self>, mut changes: watch::Receiver<u64>) -> ToolCatalogIndexerHandle {
        let task = tokio::spawn(async move {
            changes.borrow_and_update();
            loop {
                self.index_all().await;
                if changes.changed().await.is_err() {
                    return;
                }
                // Let a burst of transitions settle into one pass.
                tokio::time::sleep(TOOL_CATALOG_INDEX_DEBOUNCE).await;
                changes.borrow_and_update();
            }
        });
        ToolCatalogIndexerHandle { task: Some(task) }
    }
}

/// The running indexer task, its one lifecycle owner. [`Self::shutdown`]
/// stops it and waits for it; dropping the handle aborts it. Stopping
/// mid-pass loses nothing: a pass only requests indexing, and the provider
/// owns the embedding work it started.
#[derive(Debug)]
pub struct ToolCatalogIndexerHandle {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ToolCatalogIndexerHandle {
    /// Stop the task and wait for it to end.
    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for ToolCatalogIndexerHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests;
