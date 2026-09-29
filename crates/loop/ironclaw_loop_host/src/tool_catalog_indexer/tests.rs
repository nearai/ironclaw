use std::collections::BTreeSet;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use ironclaw_host_api::ids::{CapabilityId, ProviderToolName, TenantId, UserId};
use ironclaw_host_api::resolution::{Resolution, ResolutionBatch};
use ironclaw_loop_contracts::{
    CapabilityDescriptorView, CapabilitySurfaceVersion, InMemoryRunProfileResolver,
    LoopCapabilityPort, LoopRequest, LoopRequestBatch, ProviderToolDefinition, ToolRetrievalError,
    ToolRetrievalIndex, VisibleCapabilitySurface,
};
use serde_json::json;

use super::*;

fn definition(capability_id: &str) -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: CapabilityId::new(capability_id).expect("capability id"),
        name: ProviderToolName::new(ProviderToolName::encode_capability_str(capability_id))
            .expect("tool name"),
        description: format!("{capability_id} tool"),
        description_trust: Default::default(),
        parameters: json!({"type": "object", "properties": {}}),
    }
}

/// A catalog of three tools of which two are visible to the run's user.
struct CatalogPort;

#[async_trait]
impl LoopCapabilityPort for CatalogPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(vec![
            definition("mail.send"),
            definition("repo.search"),
            definition("admin.hidden"),
        ])
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            advertised_choice: Default::default(),
            version: CapabilitySurfaceVersion::new("surface:catalog-index").expect("version"),
            descriptors: ["mail.send", "repo.search"]
                .into_iter()
                .map(|id| CapabilityDescriptorView {
                    capability_id: CapabilityId::new(id).expect("capability id"),
                    provider: None,
                    runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                    safe_name: id.to_string(),
                    safe_description: String::new(),
                    description_trust: Default::default(),
                    parameters_schema: json!({}),
                })
                .collect(),
            callable_capability_ids: None,
        })
    }

    async fn invoke_capability(
        &self,
        _request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        unreachable!("catalog indexing never dispatches")
    }

    async fn invoke_capability_batch(
        &self,
        _request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        unreachable!("catalog indexing never dispatches")
    }
}

/// Records the run each port was built for.
#[derive(Default)]
struct RecordingFactory {
    runs: StdMutex<Vec<(TenantId, Option<UserId>)>>,
}

#[async_trait]
impl LoopCapabilityPortFactory for RecordingFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.runs.lock().expect("lock").push((
            run_context.scope.tenant_id.clone(),
            run_context.actor().map(|actor| actor.user_id.clone()),
        ));
        Ok(Arc::new(CatalogPort))
    }
}

/// Records every background indexing request.
#[derive(Debug, Default)]
struct RecordingRetrieval {
    requests: StdMutex<Vec<(ToolCorpusOwner, Vec<String>)>>,
}

impl RecordingRetrieval {
    fn requests(&self) -> Vec<(ToolCorpusOwner, Vec<String>)> {
        self.requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ToolRetrievalProvider for RecordingRetrieval {
    fn ranker_version(&self) -> &str {
        "recording-v1"
    }

    async fn fit(
        &self,
        _definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        unreachable!("catalog indexing never fits")
    }

    fn index_in_background(&self, owner: &ToolCorpusOwner, definitions: &[ProviderToolDefinition]) {
        self.requests.lock().expect("lock").push((
            owner.clone(),
            definitions
                .iter()
                .map(|definition| definition.capability_id.as_str().to_string())
                .collect(),
        ));
    }
}

fn owner(user: &str) -> ToolCorpusOwner {
    ToolCorpusOwner::new(
        TenantId::new("tenant").expect("tenant"),
        UserId::new(user).expect("user"),
    )
}

fn indexer(
    factory: &Arc<RecordingFactory>,
    retrieval: &Arc<RecordingRetrieval>,
    default_owner: Option<ToolCorpusOwner>,
) -> ToolCatalogIndexer {
    ToolCatalogIndexer::new(
        Arc::clone(factory) as Arc<dyn LoopCapabilityPortFactory>,
        Arc::clone(retrieval) as Arc<dyn ToolRetrievalProvider>,
        Arc::new(InMemoryRunProfileResolver::default()),
        None,
        None,
        default_owner,
    )
}

async fn run_for(user: &str) -> LoopRunContext {
    let profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile");
    let scope = TurnScope {
        tenant_id: TenantId::new("tenant").expect("tenant"),
        agent_id: None,
        project_id: None,
        thread_id: ThreadId::new(format!("thread-{user}")).expect("thread"),
        thread_owner: TurnThreadOwner::ActorFallback,
    };
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
        .with_actor(TurnActor::new(UserId::new(user).expect("user")))
}

async fn eventually(what: &str, condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test]
async fn a_users_visible_catalog_is_derived_as_that_user_and_handed_over() {
    let factory = Arc::new(RecordingFactory::default());
    let retrieval = Arc::new(RecordingRetrieval::default());
    let indexer = indexer(&factory, &retrieval, None);

    let handed = indexer.index_owner(&owner("alice")).await.expect("indexes");

    assert_eq!(handed, 2, "only the visible capabilities are indexed");
    assert_eq!(
        *factory.runs.lock().expect("lock"),
        vec![(
            TenantId::new("tenant").expect("tenant"),
            Some(UserId::new("alice").expect("user"))
        )],
        "the catalog is derived for a run of that user"
    );
    assert_eq!(
        retrieval.requests(),
        vec![(
            owner("alice"),
            vec!["mail.send".to_string(), "repo.search".to_string()]
        )]
    );
}

#[tokio::test]
async fn it_indexes_at_start_and_again_after_each_catalog_change() {
    let factory = Arc::new(RecordingFactory::default());
    let retrieval = Arc::new(RecordingRetrieval::default());
    let indexer = Arc::new(indexer(&factory, &retrieval, Some(owner("alice"))));
    let (changes, receiver) = watch::channel(0_u64);

    let handle = Arc::clone(&indexer).spawn(receiver);
    eventually("the startup pass", || retrieval.requests().len() == 1).await;

    // A user who ran a turn is re-indexed on the next change too.
    indexer.note_run(&run_for("bob").await);
    changes.send_replace(1);
    eventually("the pass after a change", || {
        retrieval.requests().len() == 3
    })
    .await;
    let owners: BTreeSet<_> = retrieval
        .requests()
        .into_iter()
        .skip(1)
        .map(|(owner, _)| owner.user_id.as_str().to_string())
        .collect();
    assert_eq!(
        owners,
        BTreeSet::from(["alice".to_string(), "bob".to_string()])
    );

    handle.shutdown().await;
    changes.send_replace(2);
    tokio::time::sleep(TOOL_CATALOG_INDEX_DEBOUNCE * 3).await;
    assert_eq!(
        retrieval.requests().len(),
        3,
        "a stopped indexer does nothing"
    );
}

#[tokio::test]
async fn known_owners_are_bounded_and_most_recent_first() {
    let factory = Arc::new(RecordingFactory::default());
    let retrieval = Arc::new(RecordingRetrieval::default());
    let indexer = indexer(&factory, &retrieval, Some(owner("default")));
    for user in 0..(MAX_INDEXED_OWNERS + 6) {
        indexer.note_run(&run_for(&format!("user-{user}")).await);
    }
    // Noting a known owner again moves it to the front without duplicating.
    indexer.note_run(&run_for("user-10").await);

    let owners = indexer.owners();
    assert_eq!(owners.len(), MAX_INDEXED_OWNERS);
    assert_eq!(owners[0], owner("user-10"));
    assert_eq!(
        owners[1],
        owner(&format!("user-{}", MAX_INDEXED_OWNERS + 5))
    );
    assert!(
        !owners.contains(&owner("default")),
        "the oldest owners fall off"
    );
    assert_eq!(
        owners
            .iter()
            .filter(|known| **known == owner("user-10"))
            .count(),
        1
    );
}
