use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FaultInjecting, FileStat,
    FilesystemError, InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem,
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
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::FilesystemSecretStore;

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

    pub(crate) fn hold_next_set_webhooks_at(
        &self,
        call_count: usize,
        barrier: Arc<tokio::sync::Barrier>,
    ) {
        *lock(&self.network.set_webhook_barrier) = Some((call_count, barrier));
    }

    pub(crate) fn accept_set_webhook(&self) {
        self.network.set_set_webhook_response(ok_response());
    }

    pub(crate) fn reject_delete_webhook(&self, status: u16) {
        self.network
            .set_delete_webhook_response(provider_rejection(status));
    }

    pub(crate) fn accept_delete_webhook(&self) {
        self.network.set_delete_webhook_response(ok_response());
    }

    pub(crate) fn fail_sends(&self) {
        self.network.fail_sends.store(true, Ordering::SeqCst);
    }

    pub(crate) fn succeed_sends(&self) {
        self.network.fail_sends.store(false, Ordering::SeqCst);
    }
}

type SetWebhookBarrier = Arc<Mutex<Option<(usize, Arc<tokio::sync::Barrier>)>>>;

#[derive(Clone)]
struct RecordingTelegramNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
    get_me_response: Arc<Mutex<NetworkHttpResponse>>,
    set_webhook_response: Arc<Mutex<NetworkHttpResponse>>,
    delete_webhook_response: Arc<Mutex<NetworkHttpResponse>>,
    set_webhook_barrier: SetWebhookBarrier,
    fail_sends: Arc<AtomicBool>,
}

impl Default for RecordingTelegramNetwork {
    fn default() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            get_me_response: Arc::new(Mutex::new(bot_identity_response(4242, "ironclaw_qa_bot"))),
            set_webhook_response: Arc::new(Mutex::new(ok_response())),
            delete_webhook_response: Arc::new(Mutex::new(ok_response())),
            set_webhook_barrier: Arc::new(Mutex::new(None)),
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

    fn set_delete_webhook_response(&self, response: NetworkHttpResponse) {
        *lock(&self.delete_webhook_response) = response;
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
        if request.url.ends_with("/setWebhook") {
            let barrier = {
                let mut slot = lock(&self.set_webhook_barrier);
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
        }
        let response = if request.url.ends_with("/getMe") {
            lock(&self.get_me_response).clone()
        } else if request.url.ends_with("/setWebhook") {
            lock(&self.set_webhook_response).clone()
        } else if request.url.ends_with("/deleteWebhook") {
            lock(&self.delete_webhook_response).clone()
        } else if request.url.ends_with("/sendMessage") && self.fail_sends.load(Ordering::SeqCst) {
            provider_rejection(403)
        } else if request.url.ends_with("/sendMessage") {
            let body: serde_json::Value = serde_json::from_slice(&request.body)
                .expect("recorded sendMessage request must contain valid JSON");
            let chat_id = body
                .get("chat_id")
                .and_then(serde_json::Value::as_i64)
                .expect("recorded sendMessage request must contain an integer chat_id");
            network_response(
                200,
                serde_json::json!({
                    "ok": true,
                    "result": {
                        "message_id": 1,
                        "chat": { "id": chat_id, "type": "private" }
                    }
                })
                .to_string()
                .into_bytes(),
            )
        } else {
            ok_response()
        };
        lock(&self.requests).push(request);
        Ok(response)
    }
}

fn recorded_call(request: &NetworkHttpRequest) -> Option<RecordedBotApiCall> {
    if request.url.ends_with("/getMe") {
        Some(RecordedBotApiCall::GetMe)
    } else if request.url.ends_with("/setWebhook") {
        let body: serde_json::Value = serde_json::from_slice(&request.body)
            .expect("recorded setWebhook request must contain valid JSON");
        Some(RecordedBotApiCall::SetWebhook {
            url: body
                .get("url")
                .and_then(serde_json::Value::as_str)
                .expect("recorded setWebhook request must contain a string url")
                .to_string(),
            secret: body
                .get("secret_token")
                .and_then(serde_json::Value::as_str)
                .expect("recorded setWebhook request must contain a string secret_token")
                .to_string(),
        })
    } else if request.url.ends_with("/deleteWebhook") {
        Some(RecordedBotApiCall::DeleteWebhook)
    } else if request.url.ends_with("/sendMessage") {
        let body: serde_json::Value = serde_json::from_slice(&request.body)
            .expect("recorded sendMessage request must contain valid JSON");
        Some(RecordedBotApiCall::SendMessage {
            chat_id: body
                .get("chat_id")
                .and_then(serde_json::Value::as_i64)
                .expect("recorded sendMessage request must contain an integer chat_id"),
            text: body
                .get("text")
                .and_then(serde_json::Value::as_str)
                .expect("recorded sendMessage request must contain string text")
                .to_string(),
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
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
    )
    .with_secret_store(Arc::new(FilesystemSecretStore::ephemeral()))
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

/// Telegram state over the shared [`FaultInjecting`] backend. Drive the setup
/// writes first, then arm a fault through the returned handle
/// (`backend.add_fault(Fault::on(op).backend(reason))`); the genuine
/// filesystem-backed state store then surfaces the injected `FilesystemError`
/// through its real `FilesystemError -> DomainError` mapping instead of a
/// hand-rolled stand-in.
pub(crate) fn fault_injecting_telegram_state() -> (
    Arc<FilesystemTelegramHostState>,
    Arc<FaultInjecting<InMemoryBackend>>,
) {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::default()));
    let root: Arc<dyn RootFilesystem> = backend.clone();
    (telegram_state_with_root(root), backend)
}

/// Telegram state over the [`ReadBarrierFilesystem`] decorator, which pins a
/// deterministic concurrent read/read/write interleaving for the pairing/
/// rotation concurrency tests. This is a synchronization seam, not a fault, so
/// it deliberately cannot fold into `ironclaw_filesystem::FaultInjecting`
/// (which only injects errors and records ops).
pub(crate) fn read_barrier_telegram_state()
-> (Arc<FilesystemTelegramHostState>, Arc<ReadBarrierFilesystem>) {
    let filesystem = Arc::new(ReadBarrierFilesystem::new(Arc::new(
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

/// Read-barrier `RootFilesystem` decorator: every op delegates to the inner
/// backend, but a `get` armed via [`ReadBarrierFilesystem::hold_next_reads_at`]
/// blocks on a shared barrier *after* the underlying read completes, pinning a
/// deterministic read/read/write interleaving for the pairing/rotation
/// concurrency tests. This is a synchronization primitive, not a fault
/// injector — filesystem-level fault behavior now lives in the shared
/// `ironclaw_filesystem::FaultInjecting` decorator, which deliberately does not
/// (and should not) provide a blocking barrier.
pub(crate) struct ReadBarrierFilesystem {
    inner: Arc<dyn RootFilesystem>,
    next_read_barrier: std::sync::Mutex<Option<(usize, Arc<tokio::sync::Barrier>)>>,
}

impl std::fmt::Debug for ReadBarrierFilesystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadBarrierFilesystem")
            .finish_non_exhaustive()
    }
}

impl ReadBarrierFilesystem {
    pub(crate) fn new(inner: Arc<dyn RootFilesystem>) -> Self {
        Self {
            inner,
            next_read_barrier: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn hold_next_reads_at(&self, read_count: usize, barrier: Arc<tokio::sync::Barrier>) {
        let mut slot = match self.next_read_barrier.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some((read_count, barrier));
    }
}

#[async_trait]
impl RootFilesystem for ReadBarrierFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // Capture the backend snapshot before waiting so concurrent tests pin
        // a true read/read/write interleaving rather than merely releasing two
        // readers whose actual reads can still occur sequentially.
        let result = self.inner.get(path).await;
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
        result
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        self.inner.delete_if_version(path, expected_version).await
    }
}
