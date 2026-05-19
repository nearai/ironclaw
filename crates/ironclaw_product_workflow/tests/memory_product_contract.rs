//! Contract tests for the #3287 Reborn memory product facade first slice.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_memory::{
    MemoryAppendDocumentRequest, MemoryBootstrapClearOutcome, MemoryBootstrapClearRequest,
    MemoryDocumentEntry, MemoryDocumentPath, MemoryDocumentRecord, MemoryDocumentScope,
    MemoryDocumentService, MemoryLayerService, MemoryLayerWriteMode, MemoryLayerWriteOutcome,
    MemoryLayerWriteRequest, MemoryListDocumentsRequest, MemoryPatchDocumentOutcome,
    MemoryPatchDocumentRequest, MemoryProductSearchHit, MemoryProductSearchRequest,
    MemoryProfileService, MemoryProfileSyncOutcome, MemoryProfileSyncRequest,
    MemoryPromptWriteSafetyDecision, MemoryPromptWriteSafetyPolicy, MemoryPromptWriteSafetyRequest,
    MemoryReadDocumentRequest, MemorySearchService, MemorySeedService, MemoryServiceError,
    MemoryStatus, MemoryStatusRequest, MemoryTreeRequest, MemoryVersionListRequest,
    MemoryVersionReadRequest, MemoryVersionRecord, MemoryVersionService, MemoryVersionSummary,
    MemoryWriteActor, MemoryWriteDocumentOutcome, MemoryWriteDocumentRequest, MemoryWriteSurface,
};
use ironclaw_product_workflow::{
    MemoryProductError, MemoryProductFacade, MemoryProductReadMode, MemoryProductReadRequest,
    MemoryProductReadResponse, MemoryProductSearchRequest as FacadeSearchRequest,
    MemoryProductServices, MemoryProductWriteMode, MemoryProductWriteRequest,
    MemoryProductWriteTarget,
};

#[derive(Default)]
struct FakeMemoryProductServices {
    reads: Mutex<Vec<MemoryReadDocumentRequest>>,
    writes: Mutex<Vec<MemoryWriteDocumentRequest>>,
    appends: Mutex<Vec<MemoryAppendDocumentRequest>>,
    patches: Mutex<Vec<MemoryPatchDocumentRequest>>,
    lists: Mutex<Vec<MemoryListDocumentsRequest>>,
    trees: Mutex<Vec<MemoryTreeRequest>>,
    statuses: Mutex<Vec<MemoryStatusRequest>>,
    searches: Mutex<Vec<MemoryProductSearchRequest>>,
    layer_writes: Mutex<Vec<MemoryLayerWriteRequest>>,
    version_lists: Mutex<Vec<MemoryVersionListRequest>>,
    version_reads: Mutex<Vec<MemoryVersionReadRequest>>,
    bootstrap_clears: Mutex<Vec<MemoryBootstrapClearRequest>>,
    profile_syncs: Mutex<Vec<MemoryProfileSyncRequest>>,
    prompt_checks: Mutex<Vec<MemoryPromptWriteSafetyRequest>>,
}

impl FakeMemoryProductServices {
    fn new() -> Self {
        Self::default()
    }

    fn prompt_checks(&self) -> Vec<MemoryPromptWriteSafetyRequest> {
        self.prompt_checks.lock().expect("prompt checks").clone()
    }

    fn writes(&self) -> Vec<MemoryWriteDocumentRequest> {
        self.writes.lock().expect("writes").clone()
    }

    fn appends(&self) -> Vec<MemoryAppendDocumentRequest> {
        self.appends.lock().expect("appends").clone()
    }

    fn layer_writes(&self) -> Vec<MemoryLayerWriteRequest> {
        self.layer_writes.lock().expect("layer writes").clone()
    }

    fn searches(&self) -> Vec<MemoryProductSearchRequest> {
        self.searches.lock().expect("searches").clone()
    }

    fn version_lists(&self) -> Vec<MemoryVersionListRequest> {
        self.version_lists.lock().expect("version lists").clone()
    }

    fn bootstrap_clears(&self) -> Vec<MemoryBootstrapClearRequest> {
        self.bootstrap_clears
            .lock()
            .expect("bootstrap clears")
            .clone()
    }

    fn profile_syncs(&self) -> Vec<MemoryProfileSyncRequest> {
        self.profile_syncs.lock().expect("profile syncs").clone()
    }
}

#[async_trait]
impl MemoryDocumentService for FakeMemoryProductServices {
    async fn read_current(
        &self,
        request: MemoryReadDocumentRequest,
    ) -> Result<MemoryDocumentRecord, MemoryServiceError> {
        self.reads.lock().expect("reads").push(request.clone());
        Ok(MemoryDocumentRecord {
            path: request.path,
            content: "current content".to_string(),
            metadata: serde_json::json!({}),
        })
    }

    async fn write(
        &self,
        request: MemoryWriteDocumentRequest,
    ) -> Result<MemoryWriteDocumentOutcome, MemoryServiceError> {
        let relative_path = request.path.relative_path().to_string();
        let content_length = request.content.len();
        self.writes.lock().expect("writes").push(request);
        Ok(MemoryWriteDocumentOutcome {
            relative_path,
            content_length,
        })
    }

    async fn append(
        &self,
        request: MemoryAppendDocumentRequest,
    ) -> Result<MemoryWriteDocumentOutcome, MemoryServiceError> {
        let relative_path = request.path.relative_path().to_string();
        let content_length = request.content.len();
        self.appends.lock().expect("appends").push(request);
        Ok(MemoryWriteDocumentOutcome {
            relative_path,
            content_length,
        })
    }

    async fn patch(
        &self,
        request: MemoryPatchDocumentRequest,
    ) -> Result<MemoryPatchDocumentOutcome, MemoryServiceError> {
        self.patches.lock().expect("patches").push(request);
        Ok(MemoryPatchDocumentOutcome {
            replacements: 1,
            content_length: 12,
        })
    }

    async fn list(
        &self,
        request: MemoryListDocumentsRequest,
    ) -> Result<Vec<MemoryDocumentEntry>, MemoryServiceError> {
        self.lists.lock().expect("lists").push(request);
        Ok(vec![MemoryDocumentEntry {
            relative_path: "MEMORY.md".to_string(),
            is_directory: false,
        }])
    }

    async fn tree(
        &self,
        request: MemoryTreeRequest,
    ) -> Result<Vec<MemoryDocumentEntry>, MemoryServiceError> {
        self.trees.lock().expect("trees").push(request);
        Ok(vec![MemoryDocumentEntry {
            relative_path: "daily".to_string(),
            is_directory: true,
        }])
    }

    async fn status(
        &self,
        request: MemoryStatusRequest,
    ) -> Result<MemoryStatus, MemoryServiceError> {
        self.statuses.lock().expect("statuses").push(request);
        Ok(MemoryStatus {
            document_count: 2,
            indexed_document_count: 1,
        })
    }
}

#[async_trait]
impl MemorySearchService for FakeMemoryProductServices {
    async fn search(
        &self,
        request: MemoryProductSearchRequest,
    ) -> Result<Vec<MemoryProductSearchHit>, MemoryServiceError> {
        self.searches.lock().expect("searches").push(request);
        Ok(vec![MemoryProductSearchHit {
            relative_path: "MEMORY.md".to_string(),
            content: "hit".to_string(),
            score: 0.8,
        }])
    }
}

#[async_trait]
impl MemoryLayerService for FakeMemoryProductServices {
    async fn write_layer(
        &self,
        request: MemoryLayerWriteRequest,
    ) -> Result<MemoryLayerWriteOutcome, MemoryServiceError> {
        let relative_path = request.path.relative_path().to_string();
        let actual_layer = request.layer_name.clone();
        self.layer_writes
            .lock()
            .expect("layer writes")
            .push(request);
        Ok(MemoryLayerWriteOutcome {
            relative_path,
            actual_layer,
            redirected: false,
        })
    }
}

#[async_trait]
impl MemoryVersionService for FakeMemoryProductServices {
    async fn list_versions(
        &self,
        request: MemoryVersionListRequest,
    ) -> Result<Vec<MemoryVersionSummary>, MemoryServiceError> {
        self.version_lists
            .lock()
            .expect("version lists")
            .push(request);
        Ok(vec![MemoryVersionSummary {
            version: 1,
            content_hash: "sha256:test".to_string(),
        }])
    }

    async fn read_version(
        &self,
        request: MemoryVersionReadRequest,
    ) -> Result<MemoryVersionRecord, MemoryServiceError> {
        self.version_reads
            .lock()
            .expect("version reads")
            .push(request.clone());
        Ok(MemoryVersionRecord {
            path: request.path,
            version: request.version,
            content: "old content".to_string(),
            content_hash: "sha256:test".to_string(),
        })
    }
}

#[async_trait]
impl MemorySeedService for FakeMemoryProductServices {
    async fn clear_bootstrap(
        &self,
        request: MemoryBootstrapClearRequest,
    ) -> Result<MemoryBootstrapClearOutcome, MemoryServiceError> {
        self.bootstrap_clears
            .lock()
            .expect("bootstrap clears")
            .push(request);
        Ok(MemoryBootstrapClearOutcome {
            relative_path: "BOOTSTRAP.md".to_string(),
        })
    }
}

#[async_trait]
impl MemoryProfileService for FakeMemoryProductServices {
    async fn sync_profile_documents(
        &self,
        request: MemoryProfileSyncRequest,
    ) -> Result<MemoryProfileSyncOutcome, MemoryServiceError> {
        self.profile_syncs
            .lock()
            .expect("profile syncs")
            .push(request);
        Ok(MemoryProfileSyncOutcome {
            synced_relative_paths: vec![
                "USER.md".to_string(),
                "context/assistant-directives.md".to_string(),
            ],
        })
    }
}

#[async_trait]
impl MemoryPromptWriteSafetyPolicy for FakeMemoryProductServices {
    async fn check_product_write(
        &self,
        request: MemoryPromptWriteSafetyRequest,
    ) -> Result<MemoryPromptWriteSafetyDecision, MemoryServiceError> {
        self.prompt_checks
            .lock()
            .expect("prompt checks")
            .push(request);
        Ok(MemoryPromptWriteSafetyDecision::Allow)
    }
}

fn scope() -> MemoryDocumentScope {
    MemoryDocumentScope::new_with_agent(
        "tenant-alpha",
        "user-alpha",
        Some("agent-alpha"),
        Some("project-alpha"),
    )
    .expect("valid scope")
}

fn custom_path(path: &str) -> MemoryDocumentPath {
    MemoryDocumentPath::new_with_agent(
        "tenant-alpha",
        "user-alpha",
        Some("agent-alpha"),
        Some("project-alpha"),
        path,
    )
    .expect("valid path")
}

fn actor() -> MemoryWriteActor {
    MemoryWriteActor::User {
        user_id: "user-alpha".to_string(),
    }
}

fn build_facade() -> (MemoryProductFacade, Arc<FakeMemoryProductServices>) {
    let fake = Arc::new(FakeMemoryProductServices::new());
    let facade = MemoryProductFacade::new(MemoryProductServices {
        document_service: fake.clone(),
        search_service: fake.clone(),
        layer_service: fake.clone(),
        version_service: fake.clone(),
        seed_service: fake.clone(),
        profile_service: fake.clone(),
        prompt_write_policy: fake.clone(),
    });
    (facade, fake)
}

#[tokio::test]
async fn memory_target_resolves_legacy_path_and_checks_prompt_policy() {
    let (facade, fake) = build_facade();

    let response = facade
        .write(MemoryProductWriteRequest {
            scope: scope(),
            target: MemoryProductWriteTarget::Memory,
            mode: MemoryProductWriteMode::Replace,
            content: "remember this".to_string(),
            metadata: Some(serde_json::json!({ "skip_indexing": true })),
            layer_name: None,
            force: false,
            actor: actor(),
            surface: MemoryWriteSurface::LlmTool,
        })
        .await
        .expect("write succeeds");

    assert_eq!(response.relative_path, "MEMORY.md");
    assert_eq!(response.status, "written");

    let prompt_checks = fake.prompt_checks();
    assert_eq!(prompt_checks.len(), 1);
    assert_eq!(prompt_checks[0].path.relative_path(), "MEMORY.md");
    assert_eq!(prompt_checks[0].protected_path_class.as_str(), "memory_md");
    assert_eq!(
        prompt_checks[0].authority.surface,
        MemoryWriteSurface::LlmTool
    );

    let writes = fake.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].path.relative_path(), "MEMORY.md");
    assert_eq!(writes[0].options.metadata.skip_indexing, Some(true));
    assert_eq!(
        writes[0].options.changed_by.as_deref(),
        Some("user:user-alpha")
    );
}

#[tokio::test]
async fn filesystem_looking_paths_fail_before_service_calls() {
    let (facade, fake) = build_facade();

    let err = facade
        .write(MemoryProductWriteRequest {
            scope: scope(),
            target: MemoryProductWriteTarget::CustomPath("/Users/alice/secrets.md".to_string()),
            mode: MemoryProductWriteMode::Replace,
            content: "nope".to_string(),
            metadata: None,
            layer_name: None,
            force: false,
            actor: actor(),
            surface: MemoryWriteSurface::Cli,
        })
        .await
        .expect_err("absolute host path rejected");

    assert!(matches!(err, MemoryProductError::InvalidRequest { .. }));
    assert!(fake.prompt_checks().is_empty());
    assert!(fake.writes().is_empty());
    assert!(fake.appends().is_empty());
}

#[tokio::test]
async fn malformed_relative_paths_fail_before_service_calls() {
    for path in [
        " notes.md",
        "notes.md ",
        "notes//idea.md",
        "notes/../secret.md",
        "./notes.md",
        "notes/./idea.md",
        "C:/Users/alice/secret.md",
    ] {
        let (facade, fake) = build_facade();

        let err = facade
            .write(MemoryProductWriteRequest {
                scope: scope(),
                target: MemoryProductWriteTarget::CustomPath(path.to_string()),
                mode: MemoryProductWriteMode::Replace,
                content: "nope".to_string(),
                metadata: None,
                layer_name: None,
                force: false,
                actor: actor(),
                surface: MemoryWriteSurface::Cli,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, MemoryProductError::InvalidRequest { .. }));
        assert!(fake.prompt_checks().is_empty());
        assert!(fake.writes().is_empty());
        assert!(fake.appends().is_empty());
    }
}

#[tokio::test]
async fn layer_writes_route_through_layer_service_with_force_and_authority() {
    let (facade, fake) = build_facade();

    let response = facade
        .write(MemoryProductWriteRequest {
            scope: scope(),
            target: MemoryProductWriteTarget::CustomPath("projects/alpha/notes.md".to_string()),
            mode: MemoryProductWriteMode::Append,
            content: "layer note".to_string(),
            metadata: Some(serde_json::json!({ "hygiene": { "enabled": true } })),
            layer_name: Some("shared".to_string()),
            force: true,
            actor: actor(),
            surface: MemoryWriteSurface::Web,
        })
        .await
        .expect("layer write succeeds");

    assert_eq!(response.actual_layer.as_deref(), Some("shared"));
    assert_eq!(response.redirected, Some(false));
    assert!(fake.writes().is_empty());
    assert!(fake.appends().is_empty());

    let layer_writes = fake.layer_writes();
    assert_eq!(layer_writes.len(), 1);
    assert_eq!(
        layer_writes[0].path.relative_path(),
        "projects/alpha/notes.md"
    );
    assert_eq!(layer_writes[0].mode, MemoryLayerWriteMode::Append);
    assert!(layer_writes[0].force);
}

#[tokio::test]
async fn invalid_layer_patch_is_rejected_before_prompt_policy() {
    let (facade, fake) = build_facade();

    let err = facade
        .write(MemoryProductWriteRequest {
            scope: scope(),
            target: MemoryProductWriteTarget::CustomPath("MEMORY.md".to_string()),
            mode: MemoryProductWriteMode::Patch {
                old_string: "old".to_string(),
                new_string: "new".to_string(),
                replace_all: false,
            },
            content: String::new(),
            metadata: None,
            layer_name: Some("shared".to_string()),
            force: false,
            actor: actor(),
            surface: MemoryWriteSurface::Web,
        })
        .await
        .expect_err("layer patch is invalid");

    assert!(matches!(err, MemoryProductError::InvalidRequest { .. }));
    assert!(fake.prompt_checks().is_empty());
    assert!(fake.layer_writes().is_empty());
}

#[tokio::test]
async fn read_version_listing_routes_to_version_service() {
    let (facade, fake) = build_facade();

    let response = facade
        .read(MemoryProductReadRequest {
            scope: scope(),
            relative_path: "daily/2026-05-19.md".to_string(),
            mode: MemoryProductReadMode::ListVersions { limit: 25 },
            primary_scope_only: true,
        })
        .await
        .expect("version list succeeds");

    assert!(matches!(response, MemoryProductReadResponse::Versions(_)));
    let version_lists = fake.version_lists();
    assert_eq!(version_lists.len(), 1);
    assert_eq!(version_lists[0].path.relative_path(), "daily/2026-05-19.md");
    assert_eq!(version_lists[0].limit, 25);
}

#[tokio::test]
async fn identity_prompt_docs_require_primary_scope_reads() {
    let (facade, fake) = build_facade();

    let err = facade
        .read(MemoryProductReadRequest {
            scope: scope(),
            relative_path: "IDENTITY.md".to_string(),
            mode: MemoryProductReadMode::Current,
            primary_scope_only: false,
        })
        .await
        .expect_err("identity docs cannot use secondary scopes");

    assert!(matches!(err, MemoryProductError::InvalidRequest { .. }));
    assert!(fake.version_lists().is_empty());
}

#[tokio::test]
async fn search_excludes_secondary_identity_documents_and_carries_group_context() {
    let (facade, fake) = build_facade();
    let secondary = MemoryDocumentScope::new_with_agent(
        "tenant-alpha",
        "user-beta",
        Some("agent-alpha"),
        Some("project-alpha"),
    )
    .expect("valid secondary scope");

    let hits = facade
        .search(FacadeSearchRequest {
            scope: scope(),
            query: "old decision".to_string(),
            limit: 7,
            secondary_scopes: vec![secondary],
            group_context: Some(ironclaw_memory::MemorySearchGroupContext {
                conversation_id: "group-1".to_string(),
                personal_memory_allowed: false,
            }),
        })
        .await
        .expect("search succeeds");

    assert_eq!(hits.len(), 1);
    let searches = fake.searches();
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].query, "old decision");
    assert_eq!(searches[0].limit, 7);
    assert!(searches[0].exclude_identity_documents_from_secondary);
    assert!(
        !searches[0]
            .group_context
            .as_ref()
            .expect("group context")
            .personal_memory_allowed
    );
}

#[tokio::test]
async fn profile_write_syncs_derived_profile_documents_after_document_write() {
    let (facade, fake) = build_facade();

    let response = facade
        .write(MemoryProductWriteRequest {
            scope: scope(),
            target: MemoryProductWriteTarget::CustomPath("context/profile.json".to_string()),
            mode: MemoryProductWriteMode::Replace,
            content: "{}".to_string(),
            metadata: None,
            layer_name: None,
            force: false,
            actor: actor(),
            surface: MemoryWriteSurface::Web,
        })
        .await
        .expect("profile write succeeds");

    assert_eq!(
        response.synced_relative_paths,
        vec![
            "USER.md".to_string(),
            "context/assistant-directives.md".to_string()
        ]
    );
    let profile_syncs = fake.profile_syncs();
    assert_eq!(profile_syncs.len(), 1);
    assert_eq!(
        profile_syncs[0].profile_path.relative_path(),
        "context/profile.json"
    );
}

#[tokio::test]
async fn bootstrap_target_routes_to_seed_service_not_document_write() {
    let (facade, fake) = build_facade();

    let response = facade
        .write(MemoryProductWriteRequest {
            scope: scope(),
            target: MemoryProductWriteTarget::Bootstrap,
            mode: MemoryProductWriteMode::Replace,
            content: "ignored".to_string(),
            metadata: None,
            layer_name: None,
            force: false,
            actor: actor(),
            surface: MemoryWriteSurface::SetupAdmin,
        })
        .await
        .expect("bootstrap clear succeeds");

    assert_eq!(response.status, "cleared");
    assert_eq!(response.relative_path, "BOOTSTRAP.md");
    assert_eq!(fake.bootstrap_clears().len(), 1);
    assert_eq!(fake.prompt_checks().len(), 1);
    assert!(fake.prompt_checks()[0].content.is_empty());
    assert!(fake.writes().is_empty());
    assert!(fake.appends().is_empty());
}

#[test]
fn memory_service_errors_sanitize_backend_details() {
    let err = MemoryServiceError::new(
        ironclaw_memory::MemoryServiceErrorCode::Unavailable,
        "postgres error: host=/private/tmp/db socket token=abc",
    );

    assert_eq!(err.message(), "memory service operation failed");
}

#[test]
fn fake_path_helper_keeps_tests_on_reborn_scope_shape() {
    let path = custom_path("projects/alpha/notes.md");

    assert_eq!(path.scope().tenant_id(), "tenant-alpha");
    assert_eq!(path.scope().user_id(), "user-alpha");
    assert_eq!(path.scope().agent_id(), Some("agent-alpha"));
    assert_eq!(path.scope().project_id(), Some("project-alpha"));
    assert_eq!(path.relative_path(), "projects/alpha/notes.md");
}
