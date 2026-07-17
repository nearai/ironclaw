//! Host API service bindings resolved for one invocation.
//!
//! Capability manifests remain the declaration layer for required host APIs.
//! This module contains the concrete binding layer: after policy/planning and
//! run-profile resolution approve an invocation, composition supplies these
//! services to runtime adapters. First-party handlers consume the Rust traits
//! directly; Script, WASM, MCP, and command-backed adapters should adapt the same
//! bindings into their runtime-specific host APIs rather than resolve placement
//! independently.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_events::AuditSink;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, EventRecord, FileStat, FilesystemError,
    FilesystemOperation, Filter, IndexSpec, Page, RecordVersion, RootFilesystem, SeqNo, StorageTxn,
    VersionedEntry,
};
use ironclaw_host_api::{
    MountPermissions, MountView, ResourceScope, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, ScopedPath,
    VirtualPath,
    runtime_policy::{
        DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind, SecretMode,
    },
};
use ironclaw_secrets::SecretStore;
use thiserror::Error;

use crate::{
    ExecutionPlan, PostEditCheckConfig, PostEditCheckService, RuntimeProcessPort,
    RuntimeSecretMaterialStager,
};

/// Concrete host API bindings for an already-authorized invocation.
///
/// This type is intentionally runtime-agnostic. It represents the approved
/// host API services for a run profile, not a new capability taxonomy.
#[derive(Clone)]
#[non_exhaustive]
pub struct InvocationServices {
    pub filesystem: Arc<dyn RootFilesystem>,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub tool_call_http_egress: Option<Arc<dyn ToolCallHttpEgress>>,
    /// One-shot stager for host-held/host-minted credential material.
    ///
    /// First-party handlers that already hold a trusted secret (e.g. a
    /// host-minted bearer token) stage it through this port and add a
    /// matching `StagedObligation` credential injection, so the secret flows
    /// through the egress credential-injection path instead of being written
    /// into raw request headers. Present only when the invocation may perform
    /// network egress.
    pub runtime_secret_material_stager: Option<RuntimeSecretMaterialStager>,
    pub process: Arc<dyn RuntimeProcessPort>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
    pub audit_sink: Option<Arc<dyn AuditSink>>,
    pub unsafe_raw_diagnostics_allowed: bool,
    /// Operator-configured post-edit check appended to successful
    /// `builtin.write_file` / `builtin.apply_patch` output, bundled with the
    /// process port that runs it. Resolved once per invocation; `None` keeps
    /// the feature off for this invocation. The edit plans declare no process
    /// effect, so the resolver — the only layer that inspects process backends
    /// — selects the port matching the plan's process backend (tenant sandbox
    /// under `HostedMultiTenant`, local host under `LocalSingleUser`) so the
    /// check runs in the same isolation boundary a declared process effect
    /// would, rather than escaping onto the shared provider host. `None`
    /// whenever no backend can run it in isolation. See [`PostEditCheckService`].
    pub post_edit_check: Option<PostEditCheckService>,
}

impl fmt::Debug for InvocationServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationServices")
            .field("filesystem", &"[REDACTED]")
            .field(
                "runtime_http_egress",
                &self.runtime_http_egress.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "tool_call_http_egress",
                &self.tool_call_http_egress.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "runtime_secret_material_stager",
                &self
                    .runtime_secret_material_stager
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field("process", &"[REDACTED]")
            .field(
                "secret_store",
                &self.secret_store.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "audit_sink",
                &self.audit_sink.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "unsafe_raw_diagnostics_allowed",
                &self.unsafe_raw_diagnostics_allowed,
            )
            .field(
                "post_edit_check",
                &self.post_edit_check.as_ref().map(|_| "[CONFIGURED]"),
            )
            .finish()
    }
}

/// HTTP egress port for host-owned tool calls that need bounded
/// model-visible output shaping.
///
/// This port is intentionally host-runtime-local. Shared runtime HTTP callers
/// use [`RuntimeHttpEgress`] and keep strict response-limit behavior; first-party
/// tool handlers can use this narrower port when they need sanitized partial
/// output for model context.
#[async_trait]
pub trait ToolCallHttpEgress: Send + Sync {
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>;
}

/// Inputs used to bind an approved execution plan to concrete host services.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct InvocationServicesResolutionRequest<'a> {
    pub plan: &'a ExecutionPlan,
    pub scope: &'a ResourceScope,
    pub mounts: Option<&'a MountView>,
}

/// Resolves concrete host API services for one planned invocation.
///
/// Resolver implementations are the only layer that should inspect backend
/// kinds. Tool handlers and runtime adapters consume the returned services and
/// must not decide local-vs-sandbox placement themselves.
pub trait InvocationServicesResolver: Send + Sync {
    fn resolve(
        &self,
        request: InvocationServicesResolutionRequest<'_>,
    ) -> Result<InvocationServices, InvocationServicesError>;
}

/// Stable redacted service-resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum InvocationServicesError {
    #[error("filesystem backend {backend:?} is not supported by this invocation services resolver")]
    UnsupportedFilesystemBackend { backend: FilesystemBackendKind },
    #[error("process backend {backend:?} is not supported by this invocation services resolver")]
    UnsupportedProcessBackend { backend: ProcessBackendKind },
    #[error("network mode {mode:?} is not supported by this invocation services resolver")]
    UnsupportedNetworkMode { mode: NetworkMode },
    #[error("secret mode {mode:?} is not supported by this invocation services resolver")]
    UnsupportedSecretMode { mode: SecretMode },
    #[error("capability requires secret access but no secret store is configured")]
    SecretAccessRequired,
}

impl InvocationServicesError {
    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        match self {
            Self::UnsupportedFilesystemBackend { .. } => RuntimeDispatchErrorKind::FilesystemDenied,
            Self::UnsupportedProcessBackend { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
            Self::UnsupportedNetworkMode { .. } => RuntimeDispatchErrorKind::NetworkDenied,
            Self::UnsupportedSecretMode { .. } => RuntimeDispatchErrorKind::SecretDenied,
            Self::SecretAccessRequired => RuntimeDispatchErrorKind::SecretDenied,
        }
    }
}

/// Local-host implementation for plans whose required backends are local.
#[derive(Clone)]
pub struct LocalInvocationServicesResolver {
    filesystem: Arc<dyn RootFilesystem>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    tool_call_http_egress: Option<Arc<dyn ToolCallHttpEgress>>,
    runtime_secret_material_stager: Option<RuntimeSecretMaterialStager>,
    process: Arc<dyn RuntimeProcessPort>,
    tenant_sandbox_process: Option<Arc<dyn RuntimeProcessPort>>,
    secret_store: Option<Arc<dyn SecretStore>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    post_edit_check: Option<PostEditCheckConfig>,
}

impl LocalInvocationServicesResolver {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
        process: Arc<dyn RuntimeProcessPort>,
        secret_store: Option<Arc<dyn SecretStore>>,
    ) -> Self {
        Self {
            filesystem,
            runtime_http_egress,
            tool_call_http_egress: None,
            runtime_secret_material_stager: None,
            process,
            tenant_sandbox_process: None,
            secret_store,
            audit_sink: None,
            post_edit_check: None,
        }
    }

    /// Supplies the one-shot host secret-material stager so first-party
    /// handlers can deliver host-held/host-minted credentials through the
    /// egress credential-injection path. Must wrap the same
    /// `RuntimeSecretInjectionStore` the configured `runtime_http_egress`
    /// reads from, or staged material will not be found at injection time.
    pub fn with_runtime_secret_material_stager(
        mut self,
        stager: Option<RuntimeSecretMaterialStager>,
    ) -> Self {
        self.runtime_secret_material_stager = stager;
        self
    }

    pub fn with_tool_call_http_egress(
        mut self,
        tool_call_http_egress: Option<Arc<dyn ToolCallHttpEgress>>,
    ) -> Self {
        self.tool_call_http_egress = tool_call_http_egress;
        self
    }

    pub fn with_tenant_sandbox_process_port(
        mut self,
        process: Arc<dyn RuntimeProcessPort>,
    ) -> Self {
        self.tenant_sandbox_process = Some(process);
        self
    }

    pub fn with_audit_sink(mut self, audit_sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(audit_sink);
        self
    }

    pub fn with_post_edit_check(mut self, post_edit_check: PostEditCheckConfig) -> Self {
        self.post_edit_check = Some(post_edit_check);
        self
    }
}

impl InvocationServicesResolver for LocalInvocationServicesResolver {
    fn resolve(
        &self,
        request: InvocationServicesResolutionRequest<'_>,
    ) -> Result<InvocationServices, InvocationServicesError> {
        let plan = request.plan;
        let filesystem = self.filesystem_for_plan(plan, request.mounts)?;
        let process = if plan.requires_process {
            match plan.process_backend {
                ProcessBackendKind::LocalHost if local_host_process_execution_permitted(plan) => {
                    Arc::clone(&self.process)
                }
                ProcessBackendKind::TenantSandbox => self.tenant_sandbox_process.clone().ok_or(
                    InvocationServicesError::UnsupportedProcessBackend {
                        backend: plan.process_backend,
                    },
                )?,
                _ => {
                    return Err(InvocationServicesError::UnsupportedProcessBackend {
                        backend: plan.process_backend,
                    });
                }
            }
        } else {
            Arc::clone(&self.process)
        };
        validate_network_plan(plan)?;
        if plan.requires_network && self.runtime_http_egress.is_none() {
            return Err(InvocationServicesError::UnsupportedNetworkMode {
                mode: plan.network_mode,
            });
        }
        validate_secret_plan(plan)?;
        if plan.requires_secret && self.secret_store.is_none() {
            return Err(InvocationServicesError::SecretAccessRequired);
        }
        Ok(InvocationServices {
            filesystem,
            runtime_http_egress: plan
                .requires_network
                .then(|| self.runtime_http_egress.clone())
                .flatten(),
            tool_call_http_egress: plan
                .requires_network
                .then(|| self.tool_call_http_egress.clone())
                .flatten(),
            runtime_secret_material_stager: plan
                .requires_network
                .then(|| self.runtime_secret_material_stager.clone())
                .flatten(),
            process,
            secret_store: if plan.requires_secret {
                self.secret_store.clone()
            } else {
                None
            },
            audit_sink: self.audit_sink.clone(),
            unsafe_raw_diagnostics_allowed: crate::local_runtime_allows_unsafe_raw_http_diagnostics(
                plan.deployment,
                plan.resolved_profile,
            ),
            // The post-edit check spawns an operator command, but the edit
            // plans it rides declare no process effect, so `process` above is
            // the deployment-blind local port. Bundle the check with the port
            // that matches the plan's process backend instead, so it runs in
            // the tenant sandbox under hosted multi-tenant rather than escaping
            // onto the shared provider host; disabled when no backend can run
            // it in isolation.
            post_edit_check: self.post_edit_check.clone().and_then(|config| {
                self.post_edit_check_process(plan)
                    .map(|process| PostEditCheckService { config, process })
            }),
        })
    }
}

/// Whether the plan's process policy resolves process execution to the local
/// host port for this resolver — the condition under which `resolve` hands
/// `self.process` to a process-requiring plan such as `builtin.shell`.
fn local_host_process_execution_permitted(plan: &ExecutionPlan) -> bool {
    matches!(plan.process_backend, ProcessBackendKind::LocalHost)
        && matches!(plan.deployment, DeploymentMode::LocalSingleUser)
}

impl LocalInvocationServicesResolver {
    /// Select the process port the post-edit check must run through, matching
    /// the plan's process-isolation boundary: the local host port only when
    /// local host execution is permitted (`LocalSingleUser` + `LocalHost`), the
    /// tenant sandbox port under a `TenantSandbox` backend. Returns `None`
    /// (check disabled for this invocation) when no backend can run it in
    /// isolation — this mirrors how `resolve` binds `process` for a declared
    /// process effect, but fails soft because the check is advisory rather than
    /// erroring like a process-requiring plan would.
    fn post_edit_check_process(&self, plan: &ExecutionPlan) -> Option<Arc<dyn RuntimeProcessPort>> {
        match plan.process_backend {
            ProcessBackendKind::LocalHost if local_host_process_execution_permitted(plan) => {
                Some(Arc::clone(&self.process))
            }
            ProcessBackendKind::TenantSandbox => self.tenant_sandbox_process.clone(),
            _ => None,
        }
    }

    fn filesystem_for_plan(
        &self,
        plan: &ExecutionPlan,
        mounts: Option<&MountView>,
    ) -> Result<Arc<dyn RootFilesystem>, InvocationServicesError> {
        if !plan.requires_filesystem {
            return Ok(Arc::new(MountScopedRootFilesystem::new(
                Arc::clone(&self.filesystem),
                MountView::default(),
            )));
        }
        match plan.filesystem_backend {
            FilesystemBackendKind::HostWorkspace | FilesystemBackendKind::HostWorkspaceAndHome
                if matches!(plan.deployment, DeploymentMode::LocalSingleUser) =>
            {
                Ok(Arc::clone(&self.filesystem))
            }
            FilesystemBackendKind::ScopedVirtual => {
                let mounts =
                    mounts.ok_or(InvocationServicesError::UnsupportedFilesystemBackend {
                        backend: plan.filesystem_backend,
                    })?;
                Ok(Arc::new(MountScopedRootFilesystem::new(
                    Arc::clone(&self.filesystem),
                    mounts.clone(),
                )))
            }
            _ => Err(InvocationServicesError::UnsupportedFilesystemBackend {
                backend: plan.filesystem_backend,
            }),
        }
    }
}

struct MountScopedRootFilesystem {
    root: Arc<dyn RootFilesystem>,
    mounts: MountView,
}

impl MountScopedRootFilesystem {
    fn new(root: Arc<dyn RootFilesystem>, mounts: MountView) -> Self {
        Self { root, mounts }
    }

    fn resolve(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let Some(grant) = self
            .mounts
            .mounts
            .iter()
            .filter(|grant| virtual_path_in_mount(&grant.target, path))
            .max_by_key(|grant| grant.target.as_str().len())
        else {
            return Err(permission_denied(path, operation));
        };
        if !operation_allowed(&grant.permissions, operation) {
            return Err(permission_denied(path, operation));
        }
        Ok(path.clone())
    }
}

fn permission_denied(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    match ScopedPath::new(path.as_str().to_string())
        .or_else(|_| ScopedPath::new("/unauthorized".to_string()))
    {
        Ok(path) => FilesystemError::PermissionDenied { path, operation },
        Err(error) => FilesystemError::Contract(error),
    }
}

fn virtual_path_in_mount(mount_root: &VirtualPath, path: &VirtualPath) -> bool {
    let mount_root = mount_root.as_str();
    let path = path.as_str();
    mount_root == "/"
        || path == mount_root
        || path
            .strip_prefix(mount_root)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn operation_allowed(permissions: &MountPermissions, operation: FilesystemOperation) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile
        | FilesystemOperation::AppendFile
        | FilesystemOperation::CreateDirAll
        | FilesystemOperation::EnsureIndex
        | FilesystemOperation::BeginTxn
        | FilesystemOperation::Append
        | FilesystemOperation::ReserveSeq => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::MountLocal | FilesystemOperation::Connect => false,
        FilesystemOperation::Query => permissions.read && permissions.list,
        FilesystemOperation::Tail | FilesystemOperation::HeadSeq => permissions.read,
    }
}

#[async_trait]
impl RootFilesystem for MountScopedRootFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.root.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::WriteFile)?;
        self.root.put(&path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::ReadFile)?;
        self.root.get(&path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::ListDir)?;
        self.root.list_dir(&path).await
    }

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::ListDir)?;
        self.root.list_dir_bounded(&path, max_entries).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Query)?;
        self.root.query(&path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::EnsureIndex)?;
        self.root.ensure_index(&path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Stat)?;
        self.root.stat(&path).await
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::ReadFile)?;
        self.root.read_file_bounded(&path, max_bytes).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Delete)?;
        self.root.delete(&path).await
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        // Review fix (PR #5749, round 3): this mount-scoping wrapper advertises
        // the inner backend's capabilities verbatim (see `capabilities` above),
        // so a mount that declares CAS delete must actually serve
        // `delete_if_version` after the same permission resolution as `delete`,
        // rather than falling through to the trait default `Unsupported`.
        let path = self.resolve(path, FilesystemOperation::Delete)?;
        self.root.delete_if_version(&path, expected_version).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::BeginTxn)?;
        self.root.begin(&path).await
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Append)?;
        self.root.append(&path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Append)?;
        self.root.append_batch(&path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Tail)?;
        self.root.tail(&path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::Tail)?;
        self.root.tail_bounded(&path, from, max_records).await
    }

    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::HeadSeq)?;
        self.root.head_seq(&path, from).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::ReadFile)?;
        self.root.read_file(&path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::WriteFile)?;
        self.root.write_file(&path, bytes).await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::AppendFile)?;
        self.root.append_file(&path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let path = self.resolve(path, FilesystemOperation::CreateDirAll)?;
        self.root.create_dir_all(&path).await
    }
}

fn validate_network_plan(plan: &ExecutionPlan) -> Result<(), InvocationServicesError> {
    if !plan.requires_network {
        return Ok(());
    }
    match plan.network_mode {
        NetworkMode::Brokered => Ok(()),
        NetworkMode::Allowlist
            if matches!(
                plan.deployment,
                DeploymentMode::HostedMultiTenant | DeploymentMode::EnterpriseDedicated
            ) =>
        {
            Ok(())
        }
        NetworkMode::DirectLogged | NetworkMode::Direct
            if matches!(plan.deployment, DeploymentMode::LocalSingleUser) =>
        {
            Ok(())
        }
        _ => Err(InvocationServicesError::UnsupportedNetworkMode {
            mode: plan.network_mode,
        }),
    }
}

fn validate_secret_plan(plan: &ExecutionPlan) -> Result<(), InvocationServicesError> {
    if !plan.requires_secret {
        return Ok(());
    }
    match plan.secret_mode {
        SecretMode::BrokeredHandles | SecretMode::TenantBroker | SecretMode::OrgBroker => Ok(()),
        SecretMode::ScrubbedEnv | SecretMode::InheritedEnv
            if matches!(plan.deployment, DeploymentMode::LocalSingleUser) =>
        {
            Ok(())
        }
        _ => Err(InvocationServicesError::UnsupportedSecretMode {
            mode: plan.secret_mode,
        }),
    }
}

#[cfg(test)]
mod tests;
