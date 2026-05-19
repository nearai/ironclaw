use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{FilesystemError, LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, HostApiError, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, ScopedPath, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, ConversationBindingService, IdempotencyDecision,
    IdempotencyLedger, ProductInboundAction, ProductWorkflowError, ResolveBindingRequest,
    ResolvedBinding,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;

use super::filesystem::local_filesystem;

#[derive(Debug, Error)]
pub enum RebornProductWorkflowHarnessError {
    #[error("failed to create product workflow harness tempdir: {0}")]
    Tempdir(#[from] std::io::Error),
    #[error("failed to configure local filesystem: {0}")]
    Filesystem(#[from] FilesystemError),
    #[error("invalid mount view: {0}")]
    MountView(#[from] HostApiError),
    #[error("missing agent id in product workflow harness scope")]
    MissingAgentScope,
}

pub struct RebornProductWorkflowHarness {
    pub scope: ResourceScope,
    filesystem: Arc<ScopedFilesystem<LocalFilesystem>>,
    backend: Arc<LocalFilesystem>,
    root: Arc<tempfile::TempDir>,
    idempotency_lock: Arc<Mutex<()>>,
}

impl RebornProductWorkflowHarness {
    pub fn filesystem_temp(
        scope: ResourceScope,
    ) -> Result<Self, RebornProductWorkflowHarnessError> {
        let root = Arc::new(tempfile::tempdir()?);
        let backend = Arc::new(local_filesystem(root.path())?);
        Self::filesystem_shared_backend(scope, backend, root)
    }

    pub fn filesystem_shared_backend(
        scope: ResourceScope,
        backend: Arc<LocalFilesystem>,
        root: Arc<tempfile::TempDir>,
    ) -> Result<Self, RebornProductWorkflowHarnessError> {
        let idempotency_lock = idempotency_lock_for_workflow_root(&root, &scope);
        Self::filesystem_shared_backend_with_lock(scope, backend, root, idempotency_lock)
    }

    fn filesystem_shared_backend_with_lock(
        scope: ResourceScope,
        backend: Arc<LocalFilesystem>,
        root: Arc<tempfile::TempDir>,
        idempotency_lock: Arc<Mutex<()>>,
    ) -> Result<Self, RebornProductWorkflowHarnessError> {
        let filesystem = scoped_product_workflow_fs_at(Arc::clone(&backend), &scope)?;
        Ok(Self {
            scope,
            filesystem,
            backend,
            root,
            idempotency_lock,
        })
    }

    pub fn reopened(&self) -> Result<Self, RebornProductWorkflowHarnessError> {
        Self::filesystem_shared_backend_with_lock(
            self.scope.clone(),
            Arc::clone(&self.backend),
            Arc::clone(&self.root),
            Arc::clone(&self.idempotency_lock),
        )
    }

    pub fn with_scope(
        &self,
        scope: ResourceScope,
    ) -> Result<Self, RebornProductWorkflowHarnessError> {
        Self::filesystem_shared_backend_with_lock(
            scope,
            Arc::clone(&self.backend),
            Arc::clone(&self.root),
            Arc::clone(&self.idempotency_lock),
        )
    }

    pub fn binding_service(
        &self,
    ) -> Result<
        FilesystemConversationBindingService<LocalFilesystem>,
        RebornProductWorkflowHarnessError,
    > {
        let agent_id = self
            .scope
            .agent_id
            .clone()
            .ok_or(RebornProductWorkflowHarnessError::MissingAgentScope)?;
        Ok(FilesystemConversationBindingService::new(
            Arc::clone(&self.filesystem),
            self.scope.clone(),
            agent_id,
            self.scope.project_id.clone(),
        ))
    }

    pub fn idempotency_ledger(&self) -> FilesystemIdempotencyLedger<LocalFilesystem> {
        FilesystemIdempotencyLedger::new_with_lock(
            Arc::clone(&self.filesystem),
            self.scope.clone(),
            Duration::from_secs(60),
            Arc::clone(&self.idempotency_lock),
        )
    }

    pub fn idempotency_ledger_with_ttl(
        &self,
        lease_ttl: Duration,
    ) -> FilesystemIdempotencyLedger<LocalFilesystem> {
        FilesystemIdempotencyLedger::new_with_lock(
            Arc::clone(&self.filesystem),
            self.scope.clone(),
            lease_ttl,
            Arc::clone(&self.idempotency_lock),
        )
    }

    pub async fn corrupt_binding_record_for_test(
        &self,
        request: &ResolveBindingRequest,
        bytes: Vec<u8>,
    ) -> Result<(), ProductWorkflowError> {
        let path = binding_path(&self.scope, request)?;
        self.filesystem
            .write_bytes(&self.scope, &path, bytes)
            .await
            .map_err(|error| fs_error("write malformed product workflow record", error))
    }
}

#[derive(Clone)]
pub struct FilesystemConversationBindingService<F> {
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

impl<F> FilesystemConversationBindingService<F>
where
    F: RootFilesystem,
{
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            filesystem,
            scope,
            agent_id,
            project_id,
        }
    }
}

#[async_trait]
impl<F> ConversationBindingService for FilesystemConversationBindingService<F>
where
    F: RootFilesystem + 'static,
{
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let path = binding_path(&self.scope, &request)?;
        if let Some(stored) =
            read_json::<F, StoredConversationBinding>(&self.filesystem, &self.scope, &path).await?
        {
            return Ok(stored.binding);
        }

        let binding = ResolvedBinding {
            tenant_id: self.scope.tenant_id.clone(),
            user_id: user_id_for_binding(&self.scope.tenant_id, &request)?,
            thread_id: thread_id_for_binding(&self.scope.tenant_id, &request)?,
            agent_id: Some(self.agent_id.clone()),
            project_id: self.project_id.clone(),
        };
        let stored = StoredConversationBinding {
            binding: binding.clone(),
        };
        write_json(&self.filesystem, &self.scope, &path, &stored).await?;
        Ok(binding)
    }
}

#[derive(Clone)]
pub struct FilesystemIdempotencyLedger<F> {
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    lease_ttl: Duration,
    lock: Arc<Mutex<()>>,
}

impl<F> FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem,
{
    #[allow(dead_code)] // Convenience constructor for standalone test-support ledgers.
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        lease_ttl: Duration,
    ) -> Self {
        let lock = idempotency_lock_for_filesystem(&filesystem);
        Self::new_with_lock(filesystem, scope, lease_ttl, lock)
    }

    pub fn new_with_lock(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        lease_ttl: Duration,
        lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            filesystem,
            scope,
            lease_ttl,
            lock,
        }
    }
}

#[async_trait]
impl<F> IdempotencyLedger for FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem + 'static,
{
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let path = ledger_path(&fingerprint)?;
        let _guard = self.lock.lock().await;
        let Some(stored) =
            read_json::<F, StoredIdempotencyAction>(&self.filesystem, &self.scope, &path).await?
        else {
            let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
            let stored = StoredIdempotencyAction::reserved(action.clone(), self.lease_ttl)?;
            write_json(&self.filesystem, &self.scope, &path, &stored).await?;
            return Ok(IdempotencyDecision::New(action));
        };

        if stored.action.phase == ActionPhase::Settled {
            return Ok(IdempotencyDecision::Replay(stored.action));
        }
        if stored.is_expired(Utc::now()) {
            let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
            let replacement = StoredIdempotencyAction::reserved(action.clone(), self.lease_ttl)?;
            write_json(&self.filesystem, &self.scope, &path, &replacement).await?;
            return Ok(IdempotencyDecision::New(action));
        }

        Err(ProductWorkflowError::Transient {
            reason: "idempotency fingerprint already in flight; retry after recovery lease".into(),
        })
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let path = ledger_path(&action.fingerprint)?;
        let _guard = self.lock.lock().await;
        let Some(stored) =
            read_json::<F, StoredIdempotencyAction>(&self.filesystem, &self.scope, &path).await?
        else {
            return Err(ProductWorkflowError::Transient {
                reason: "cannot settle missing idempotency reservation".into(),
            });
        };
        if stored.action.action_id != action.action_id {
            return Err(ProductWorkflowError::Transient {
                reason: "cannot settle stale idempotency reservation".into(),
            });
        }
        write_json(
            &self.filesystem,
            &self.scope,
            &path,
            &StoredIdempotencyAction::settled(action),
        )
        .await
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let path = ledger_path(&action.fingerprint)?;
        let _guard = self.lock.lock().await;
        let Some(stored) =
            read_json::<F, StoredIdempotencyAction>(&self.filesystem, &self.scope, &path).await?
        else {
            return Ok(());
        };
        if stored.action.phase == ActionPhase::Settled
            || stored.action.action_id != action.action_id
        {
            return Ok(());
        }
        match self.filesystem.delete(&self.scope, &path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(fs_error("release idempotency reservation", error)),
        }
    }
}

fn idempotency_lock_for_workflow_root(
    root: &tempfile::TempDir,
    scope: &ResourceScope,
) -> Arc<Mutex<()>> {
    idempotency_lock_for_key(format!(
        "workflow-root:{}:{}",
        root.path().display(),
        product_workflow_mount_target(scope)
    ))
}

fn idempotency_lock_for_filesystem<F>(filesystem: &Arc<ScopedFilesystem<F>>) -> Arc<Mutex<()>> {
    idempotency_lock_for_key(format!("scoped-filesystem:{:p}", Arc::as_ptr(filesystem)))
}

fn idempotency_lock_for_key(key: String) -> Arc<Mutex<()>> {
    static IDEMPOTENCY_LOCKS: OnceLock<StdMutex<HashMap<String, Weak<Mutex<()>>>>> =
        OnceLock::new();

    let mut locks = IDEMPOTENCY_LOCKS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .expect("idempotency lock registry poisoned");
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct StoredConversationBinding {
    binding: ResolvedBinding,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct StoredIdempotencyAction {
    action: ProductInboundAction,
    nonterminal_expires_at: Option<DateTime<Utc>>,
}

impl StoredIdempotencyAction {
    fn reserved(
        action: ProductInboundAction,
        lease_ttl: Duration,
    ) -> Result<Self, ProductWorkflowError> {
        let ttl = chrono::Duration::from_std(lease_ttl).map_err(|error| {
            ProductWorkflowError::Transient {
                reason: format!("invalid idempotency lease ttl: {error}"),
            }
        })?;
        Ok(Self {
            action,
            nonterminal_expires_at: Some(Utc::now() + ttl),
        })
    }

    fn settled(action: ProductInboundAction) -> Self {
        Self {
            action,
            nonterminal_expires_at: None,
        }
    }

    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.nonterminal_expires_at
            .is_some_and(|expires_at| expires_at <= now)
    }
}

async fn read_json<F, T>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
) -> Result<Option<T>, ProductWorkflowError>
where
    F: RootFilesystem,
    T: DeserializeOwned,
{
    let Some(versioned) = filesystem
        .get(scope, path)
        .await
        .map_err(|error| fs_error("read product workflow record", error))?
    else {
        return Ok(None);
    };
    let value = serde_json::from_slice(&versioned.entry.body).map_err(|error| {
        ProductWorkflowError::Transient {
            reason: format!("failed to parse product workflow record: {error}"),
        }
    })?;
    Ok(Some(value))
}

async fn write_json<F, T>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    value: &T,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem,
    T: Serialize,
{
    let body =
        serde_json::to_vec_pretty(value).map_err(|error| ProductWorkflowError::Transient {
            reason: format!("failed to serialize product workflow record: {error}"),
        })?;
    filesystem
        .write_bytes(scope, path, body)
        .await
        .map_err(|error| fs_error("write product workflow record", error))
}

fn binding_path(
    scope: &ResourceScope,
    request: &ResolveBindingRequest,
) -> Result<ScopedPath, ProductWorkflowError> {
    let agent_id =
        scope
            .agent_id
            .as_ref()
            .ok_or_else(|| ProductWorkflowError::BindingResolutionFailed {
                reason: "missing agent id in binding scope".to_string(),
            })?;
    let project_id = scope.project_id.as_ref().map_or("", ProjectId::as_str);
    hashed_scoped_path(
        "/workflow/bindings",
        &[
            agent_id.as_str(),
            project_id,
            request.adapter_id.as_str(),
            request.installation_id.as_str(),
            request.external_actor_ref.kind(),
            request.external_actor_ref.id(),
            &request.external_conversation_ref.conversation_fingerprint(),
        ],
    )
}

fn ledger_path(fingerprint: &ActionFingerprintKey) -> Result<ScopedPath, ProductWorkflowError> {
    hashed_scoped_path(
        "/workflow/idempotency",
        &[
            fingerprint.adapter_id.as_str(),
            fingerprint.installation_id.as_str(),
            fingerprint.source_binding_key.as_str(),
            fingerprint.external_event_id.as_str(),
        ],
    )
}

fn hashed_scoped_path(prefix: &str, parts: &[&str]) -> Result<ScopedPath, ProductWorkflowError> {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    ScopedPath::new(format!("{prefix}/{digest}.json")).map_err(|error| {
        ProductWorkflowError::BindingResolutionFailed {
            reason: format!("invalid product workflow scoped path: {error}"),
        }
    })
}

fn user_id_for_binding(
    tenant_id: &TenantId,
    request: &ResolveBindingRequest,
) -> Result<UserId, ProductWorkflowError> {
    scoped_id(
        "user",
        &[
            tenant_id.as_str(),
            request.installation_id.as_str(),
            request.external_actor_ref.kind(),
            request.external_actor_ref.id(),
        ],
        UserId::new,
    )
}

fn thread_id_for_binding(
    tenant_id: &TenantId,
    request: &ResolveBindingRequest,
) -> Result<ThreadId, ProductWorkflowError> {
    scoped_id(
        "thread",
        &[
            tenant_id.as_str(),
            request.installation_id.as_str(),
            &request.external_conversation_ref.conversation_fingerprint(),
        ],
        ThreadId::new,
    )
}

fn scoped_id<T>(
    prefix: &str,
    parts: &[&str],
    construct: impl FnOnce(String) -> Result<T, HostApiError>,
) -> Result<T, ProductWorkflowError> {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    construct(format!("{prefix}-{}", &digest[..32])).map_err(|error| {
        ProductWorkflowError::BindingResolutionFailed {
            reason: format!("invalid derived {prefix} id: {error}"),
        }
    })
}

fn scoped_product_workflow_fs_at<F>(
    backend: Arc<F>,
    scope: &ResourceScope,
) -> Result<Arc<ScopedFilesystem<F>>, HostApiError>
where
    F: RootFilesystem,
{
    let target = product_workflow_mount_target(scope);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workflow").expect("valid product workflow alias"),
        VirtualPath::new(target).expect("valid product workflow target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)))
}

fn product_workflow_mount_target(scope: &ResourceScope) -> String {
    format!(
        "/engine/tenants/{}/users/{}/product-workflow",
        scope.tenant_id, scope.user_id
    )
}

fn fs_error(operation: &str, error: FilesystemError) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: format!("{operation} failed: {error}"),
    }
}

pub fn resource_scope(
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
) -> ResourceScope {
    ResourceScope {
        tenant_id,
        user_id,
        agent_id: Some(agent_id),
        project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}
