use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, TenantId, UserId,
    VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntimeHttpEgressPort, HostRuntimeServices,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::{InMemoryProcessResultStore, InMemoryProcessStore, ProcessServices};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::InMemorySecretStore;

use crate::bot_api::HostEgressTelegramBotApi;
use crate::state::FilesystemTelegramHostState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedBotApiCall {
    GetMe,
    SetWebhook { url: String, secret: String },
    DeleteWebhook,
    SendMessage { chat_id: i64, text: String },
}

#[derive(Clone)]
pub(crate) struct RecordingBotApi {
    client: Arc<HostEgressTelegramBotApi>,
    network: RecordingTelegramNetwork,
}

impl std::fmt::Debug for RecordingBotApi {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecordingBotApi")
            .finish_non_exhaustive()
    }
}

impl Default for RecordingBotApi {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingBotApi {
    pub(crate) fn new() -> Self {
        let network = RecordingTelegramNetwork::default();
        let client = HostEgressTelegramBotApi::arced(
            host_egress_port(network.clone()),
            ResourceScope::system(),
        );
        Self { client, network }
    }

    pub(crate) fn client(&self) -> Arc<HostEgressTelegramBotApi> {
        Arc::clone(&self.client)
    }

    pub(crate) fn calls(&self) -> Vec<RecordedBotApiCall> {
        self.network.calls()
    }

    pub(crate) fn sends(&self) -> Vec<(i64, String)> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                RecordedBotApiCall::SendMessage { chat_id, text } => Some((chat_id, text)),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn set_bot_identity(&self, id: i64, username: &str) {
        self.network.set_bot_identity(id, username);
    }

    pub(crate) fn reject_get_me(&self, status: u16) {
        self.network.set_get_me_response(provider_rejection(status));
    }

    pub(crate) fn malformed_get_me_response(&self) {
        self.network
            .set_get_me_response(network_response(200, b"not-json".to_vec()));
    }

    pub(crate) fn reject_set_webhook(&self, status: u16) {
        self.network
            .set_set_webhook_response(provider_rejection(status));
    }

    pub(crate) fn fail_sends(&self) {
        self.network.fail_sends.store(true, Ordering::SeqCst);
    }

    pub(crate) fn succeed_sends(&self) {
        self.network.fail_sends.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct RecordingTelegramNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
    get_me_response: Arc<Mutex<NetworkHttpResponse>>,
    set_webhook_response: Arc<Mutex<NetworkHttpResponse>>,
    fail_sends: Arc<AtomicBool>,
}

impl Default for RecordingTelegramNetwork {
    fn default() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            get_me_response: Arc::new(Mutex::new(bot_identity_response(4242, "ironclaw_qa_bot"))),
            set_webhook_response: Arc::new(Mutex::new(ok_response())),
            fail_sends: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl RecordingTelegramNetwork {
    fn set_bot_identity(&self, id: i64, username: &str) {
        *lock(&self.get_me_response) = bot_identity_response(id, username);
    }

    fn set_get_me_response(&self, response: NetworkHttpResponse) {
        *lock(&self.get_me_response) = response;
    }

    fn set_set_webhook_response(&self, response: NetworkHttpResponse) {
        *lock(&self.set_webhook_response) = response;
    }

    fn calls(&self) -> Vec<RecordedBotApiCall> {
        lock(&self.requests)
            .iter()
            .filter_map(recorded_call)
            .collect()
    }
}

#[async_trait]
impl NetworkHttpEgress for RecordingTelegramNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let response = if request.url.ends_with("/getMe") {
            lock(&self.get_me_response).clone()
        } else if request.url.ends_with("/setWebhook") {
            lock(&self.set_webhook_response).clone()
        } else if request.url.ends_with("/sendMessage") && self.fail_sends.load(Ordering::SeqCst) {
            provider_rejection(403)
        } else {
            ok_response()
        };
        lock(&self.requests).push(request);
        Ok(response)
    }
}

fn recorded_call(request: &NetworkHttpRequest) -> Option<RecordedBotApiCall> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).ok()?;
    if request.url.ends_with("/getMe") {
        Some(RecordedBotApiCall::GetMe)
    } else if request.url.ends_with("/setWebhook") {
        Some(RecordedBotApiCall::SetWebhook {
            url: body.get("url")?.as_str()?.to_string(),
            secret: body.get("secret_token")?.as_str()?.to_string(),
        })
    } else if request.url.ends_with("/deleteWebhook") {
        Some(RecordedBotApiCall::DeleteWebhook)
    } else if request.url.ends_with("/sendMessage") {
        Some(RecordedBotApiCall::SendMessage {
            chat_id: body.get("chat_id")?.as_i64()?,
            text: body.get("text")?.as_str()?.to_string(),
        })
    } else {
        None
    }
}

fn bot_identity_response(id: i64, username: &str) -> NetworkHttpResponse {
    network_response(
        200,
        serde_json::json!({
            "ok": true,
            "result": { "id": id, "username": username }
        })
        .to_string()
        .into_bytes(),
    )
}

fn ok_response() -> NetworkHttpResponse {
    network_response(200, br#"{"ok":true,"result":true}"#.to_vec())
}

fn provider_rejection(status: u16) -> NetworkHttpResponse {
    network_response(
        status,
        serde_json::json!({ "ok": false, "description": "test provider rejection" })
            .to_string()
            .into_bytes(),
    )
}

fn network_response(status: u16, body: Vec<u8>) -> NetworkHttpResponse {
    NetworkHttpResponse {
        status,
        headers: Vec::new(),
        usage: NetworkUsage {
            request_bytes: 0,
            response_bytes: body.len() as u64,
            resolved_ip: None,
        },
        body,
    }
}

fn host_egress_port(network: impl NetworkHttpEgress + 'static) -> HostRuntimeHttpEgressPort {
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::<InMemoryProcessStore, InMemoryProcessResultStore>::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .try_with_host_http_egress(network)
    .expect("host HTTP egress should wire");
    services
        .host_runtime_http_egress_port()
        .expect("host runtime HTTP egress port should be configured")
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn telegram_state() -> Arc<FilesystemTelegramHostState> {
    let root: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::default());
    telegram_state_with_root(root)
}

pub(crate) fn fault_injected_telegram_state() -> (
    Arc<FilesystemTelegramHostState>,
    Arc<FaultInjectingFilesystem>,
) {
    let filesystem = Arc::new(FaultInjectingFilesystem::new(Arc::new(
        InMemoryBackend::default(),
    )));
    let root: Arc<dyn RootFilesystem> = filesystem.clone();
    (telegram_state_with_root(root), filesystem)
}

pub(crate) fn telegram_state_with_root(
    root: Arc<dyn RootFilesystem>,
) -> Arc<FilesystemTelegramHostState> {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("mount alias"),
        VirtualPath::new("/tenants/tenant-alpha/shared").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped: Arc<ScopedFilesystem<dyn RootFilesystem>> =
        Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    Arc::new(FilesystemTelegramHostState::new(
        scoped,
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("operator").expect("user"),
        AgentId::new("agent-alpha").expect("agent"),
        None,
    ))
}

/// Filesystem-level fault controller used instead of parallel Telegram store
/// fakes. Every non-faulted operation delegates to the production in-memory
/// backend, so tests still exercise real record paths, JSON, locks, and CAS.
pub(crate) struct FaultInjectingFilesystem {
    inner: Arc<dyn RootFilesystem>,
    fail_reads: AtomicBool,
    fail_writes: AtomicBool,
    fail_deletes: AtomicBool,
    fail_versioned_writes: AtomicBool,
    next_read_barrier: std::sync::Mutex<Option<(usize, Arc<tokio::sync::Barrier>)>>,
}

impl std::fmt::Debug for FaultInjectingFilesystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FaultInjectingFilesystem")
            .finish_non_exhaustive()
    }
}

impl FaultInjectingFilesystem {
    pub(crate) fn new(inner: Arc<dyn RootFilesystem>) -> Self {
        Self {
            inner,
            fail_reads: AtomicBool::new(false),
            fail_writes: AtomicBool::new(false),
            fail_deletes: AtomicBool::new(false),
            fail_versioned_writes: AtomicBool::new(false),
            next_read_barrier: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn fail_reads(&self) {
        self.fail_reads.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_writes(&self) {
        self.fail_writes.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_deletes(&self) {
        self.fail_deletes.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_versioned_writes(&self) {
        self.fail_versioned_writes.store(true, Ordering::SeqCst);
    }

    pub(crate) fn hold_next_reads_at(&self, read_count: usize, barrier: Arc<tokio::sync::Barrier>) {
        let mut slot = match self.next_read_barrier.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some((read_count, barrier));
    }

    fn injected(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
        FilesystemError::Backend {
            path: path.clone(),
            operation,
            reason: "test-injected filesystem failure".to_string(),
        }
    }
}

#[async_trait]
impl RootFilesystem for FaultInjectingFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if self.fail_writes.load(Ordering::SeqCst)
            || (matches!(cas, CasExpectation::Version(_))
                && self.fail_versioned_writes.load(Ordering::SeqCst))
        {
            return Err(Self::injected(path, FilesystemOperation::WriteFile));
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(Self::injected(path, FilesystemOperation::ReadFile));
        }
        let barrier = {
            let mut slot = match self.next_read_barrier.lock() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some((remaining, barrier)) = slot.as_mut() {
                let barrier = Arc::clone(barrier);
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    *slot = None;
                }
                Some(barrier)
            } else {
                None
            }
        };
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if self.fail_deletes.load(Ordering::SeqCst) {
            return Err(Self::injected(path, FilesystemOperation::Delete));
        }
        self.inner.delete(path).await
    }
}
