use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_attachments::InboundAttachment;
use ironclaw_extension_host::egress::{ApprovedChannelEgress, ChannelEgressTransport};
use ironclaw_extension_host::ingress::{
    ExtensionIngressRouter, ExtensionIngressRouterDeps, IngressPortError, IngressRequest,
    IngressRouterConfig, ReplyContextKey, ReplyContextStore, VerificationCandidate,
};
use ironclaw_extension_host::{
    DeploymentChannelBinding, DeploymentChannelRegistry, ExtensionBindings, ExtensionHost,
    ExtensionHostDeps, InstallationRecordStore, RehydratedInstallationRecordStore,
};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod, ProjectId,
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
    TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_product_adapters::{
    AttachmentRef as ChannelAttachmentRef, ChannelAdapter, ChannelError, DeliveryReport,
    ExternalActorRef, ExternalConversationRef, ExternalEventId, InboundOutcome,
    NormalizedInboundMessage, OutboundEnvelope, ProductAdapterId, ProductAttachmentDescriptor,
    ProductAttachmentKind, ProductTriggerReason, VerifiedInbound,
};
use ironclaw_product_workflow::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    InMemoryIdempotencyLedger, InboundAttachmentReader, ProductWorkflowError,
    ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_threads::{
    InMemorySessionThreadService, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion, SubmitTurnRequest,
    SubmitTurnResponse, TurnCoordinator, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope,
    TurnStatus, events::EventCursor,
};

use super::{
    ChannelInboundSinkConfig, ChannelIngressRegistration, ExtensionIngressRegistry,
    GenericChannelInboundSink, StaticIngressSecrets, VerifiedEvidenceMint,
};
use crate::local_dev_mounts::WORKSPACE_ALIAS;
use crate::support::fs::{ProjectScopedAttachmentLander, ProjectScopedAttachmentReader};

const EXTENSION_ID: &str = "attachment-channel";
const INSTALLATION_ID: &str = "attachment-installation";
const SHARED_SECRET: &[u8] = b"attachment-secret";
const VENDOR_HOST: &str = "files.attachment.example";

#[derive(Clone, Copy)]
enum FetchResult {
    Success,
    Failure { retryable: bool },
}

struct JourneyChannelAdapter {
    fetch_count: Arc<AtomicUsize>,
    fetch_results: Mutex<VecDeque<FetchResult>>,
}

impl JourneyChannelAdapter {
    fn new(results: impl IntoIterator<Item = FetchResult>) -> Self {
        Self {
            fetch_count: Arc::new(AtomicUsize::new(0)),
            fetch_results: Mutex::new(results.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ChannelAdapter for JourneyChannelAdapter {
    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        let value: serde_json::Value =
            serde_json::from_slice(request.body).map_err(|error| ChannelError::Parse {
                reason: error.to_string(),
            })?;
        let event_id = value
            .get("event_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChannelError::Parse {
                reason: "missing event id".to_string(),
            })?;
        let descriptor = ProductAttachmentDescriptor::new(
            "provider-file-1",
            "application/pdf",
            Some("report.pdf".to_string()),
            Some(4),
            ProductAttachmentKind::Document,
        )
        .map_err(|error| ChannelError::Parse {
            reason: error.to_string(),
        })?;
        Ok(InboundOutcome::Messages(vec![NormalizedInboundMessage {
            actor: ExternalActorRef::new("attachment_user", "user-1", None::<&str>).map_err(
                |error| ChannelError::Parse {
                    reason: error.to_string(),
                },
            )?,
            conversation: ExternalConversationRef::new(None, "conversation-1", None, None)
                .map_err(|error| ChannelError::Parse {
                    reason: error.to_string(),
                })?,
            event_id: ExternalEventId::new(event_id).map_err(|error| ChannelError::Parse {
                reason: error.to_string(),
            })?,
            text: "review the report".to_string(),
            trigger: ProductTriggerReason::DirectChat,
            attachments: vec![ChannelAttachmentRef {
                descriptor,
                vendor_ref: "opaque-provider-file-1".to_string(),
                mime_hint: Some("application/pdf".to_string()),
            }],
            reply_context: None,
        }]))
    }

    async fn fetch_attachment(
        &self,
        attachment: &ChannelAttachmentRef,
        egress: &dyn RestrictedEgress,
    ) -> Result<InboundAttachment, ChannelError> {
        self.fetch_count.fetch_add(1, Ordering::SeqCst);
        egress
            .send(RestrictedEgressRequest {
                method: NetworkMethod::Post,
                url: format!("https://{VENDOR_HOST}/files/download"),
                headers: Vec::new(),
                body: None,
                credential: None,
                body_credentials: Vec::new(),
            })
            .await
            .map_err(|error| ChannelError::AttachmentTransfer {
                reason: error.to_string(),
                retryable: true,
            })?;
        match self
            .fetch_results
            .lock()
            .expect("fetch result lock")
            .pop_front()
            .expect("scripted fetch result")
        {
            FetchResult::Success => Ok(InboundAttachment {
                id: attachment.descriptor.external_file_id.clone(),
                mime_type: attachment.descriptor.mime_type.clone(),
                filename: attachment.descriptor.filename.clone(),
                bytes: b"DATA".to_vec(),
            }),
            FetchResult::Failure { retryable } => Err(ChannelError::AttachmentTransfer {
                reason: "provider detail must stay host-side".to_string(),
                retryable,
            }),
        }
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

#[derive(Default)]
struct RecordingTransport {
    requests: Mutex<Vec<ApprovedChannelEgress>>,
}

#[async_trait]
impl ChannelEgressTransport for RecordingTransport {
    async fn execute(
        &self,
        approved: ApprovedChannelEgress,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        self.requests
            .lock()
            .expect("transport requests lock")
            .push(approved);
        Ok(RestrictedEgressResponse {
            status: 200,
            body: Vec::new(),
        })
    }
}

struct OrderingPolicy {
    fetch_count: Arc<AtomicUsize>,
    observed_fetch_counts: Mutex<Vec<usize>>,
}

#[async_trait]
impl BeforeInboundPolicy for OrderingPolicy {
    async fn check_user_message(
        &self,
        _request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductWorkflowError> {
        self.observed_fetch_counts
            .lock()
            .expect("policy observations lock")
            .push(self.fetch_count.load(Ordering::SeqCst));
        Ok(BeforeInboundPolicyOutcome::Allow)
    }
}

struct StaticBinding;

#[async_trait]
impl ConversationBindingService for StaticBinding {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Ok(resolved_binding())
    }

    async fn lookup_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Ok(resolved_binding())
    }
}

fn resolved_binding() -> ResolvedBinding {
    ResolvedBinding {
        tenant_id: TenantId::new("tenant-attachment").expect("tenant"),
        actor_user_id: UserId::new("user-attachment").expect("actor user"),
        subject_user_id: Some(UserId::new("user-attachment").expect("subject user")),
        thread_id: ThreadId::new("thread-attachment").expect("thread"),
        agent_id: Some(AgentId::new("agent-attachment").expect("agent")),
        project_id: Some(ProjectId::new("project-attachment").expect("project")),
    }
}

fn thread_scope() -> ThreadScope {
    let binding = resolved_binding();
    ThreadScope {
        tenant_id: binding.tenant_id,
        agent_id: binding.agent_id.expect("agent"),
        project_id: binding.project_id,
        owner_user_id: binding.subject_user_id,
        mission_id: None,
    }
}

#[derive(Default)]
struct AcceptingTurnCoordinator;

#[async_trait]
impl TurnCoordinator for AcceptingTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Ok(SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Completed,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor::default(),
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("attachment journey does not resume")
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        unreachable!("attachment journey does not retry a turn")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("attachment journey does not cancel")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("attachment journey does not inspect run state")
    }
}

#[derive(Default)]
struct NoopReplyContextStore;

#[async_trait]
impl ReplyContextStore for NoopReplyContextStore {
    async fn put(&self, _key: ReplyContextKey, _context: Vec<u8>) -> Result<(), IngressPortError> {
        Ok(())
    }

    async fn get(&self, _key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError> {
        Ok(None)
    }
}

struct JourneyHarness {
    router: ExtensionIngressRouter,
    adapter: Arc<JourneyChannelAdapter>,
    transport: Arc<RecordingTransport>,
    policy: Arc<OrderingPolicy>,
    threads: Arc<InMemorySessionThreadService>,
    workspace: Arc<ScopedFilesystem<InMemoryBackend>>,
    _host: Arc<ExtensionHost>,
}

fn manifest() -> ironclaw_extensions::ResolvedExtensionManifest {
    ironclaw_extension_host::test_support::resolve_manifest_toml(&format!(
        r#"
schema_version = "reborn.extension_manifest.v3"
id = "{EXTENSION_ID}"
name = "Attachment Channel"
version = "0.1.0"
description = "caller-level attachment journey"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/attachment_channel.wasm"

[channel]
id = "messages"
display_name = "Attachment Channel"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 4096

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "attachment_webhook_secret"
header = "X-Attachment-Secret"

[channel.config]
fields = [ {{ handle = "attachment_webhook_secret", label = "Webhook secret", secret = true }} ]

[[channel.egress]]
scheme = "https"
host = "{VENDOR_HOST}"
methods = ["post"]
"#,
    ))
}

fn workspace_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new(WORKSPACE_ALIAS).expect("workspace alias"),
        VirtualPath::new("/projects/workspace").expect("workspace root"),
        MountPermissions::read_write(),
    )])
    .expect("workspace mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        view,
    ))
}

async fn harness(fetch_results: impl IntoIterator<Item = FetchResult>) -> JourneyHarness {
    let adapter = Arc::new(JourneyChannelAdapter::new(fetch_results));
    let adapter_port: Arc<dyn ChannelAdapter> = adapter.clone();
    let transport = Arc::new(RecordingTransport::default());
    let transport_port: Arc<dyn ChannelEgressTransport> = transport.clone();
    let policy = Arc::new(OrderingPolicy {
        fetch_count: Arc::clone(&adapter.fetch_count),
        observed_fetch_counts: Mutex::new(Vec::new()),
    });
    let threads = Arc::new(InMemorySessionThreadService::default());
    let workspace = workspace_filesystem();
    let binding: Arc<dyn ConversationBindingService> = Arc::new(StaticBinding);
    let inbound = Arc::new(
        DefaultInboundTurnService::new(
            Arc::clone(&binding),
            Arc::clone(&threads),
            AcceptingTurnCoordinator,
        )
        .with_inbound_attachments(Arc::new(ProjectScopedAttachmentLander::new(
            Arc::clone(&workspace),
        ))),
    );
    let workflow = Arc::new(
        DefaultProductWorkflow::new(inbound, Arc::new(InMemoryIdempotencyLedger::new()), binding)
            .with_before_inbound_policy(policy.clone()),
    );
    let sink = Arc::new(GenericChannelInboundSink::new(ChannelInboundSinkConfig {
        adapter_id: ProductAdapterId::new(EXTENSION_ID).expect("adapter id"),
        evidence: VerifiedEvidenceMint::SharedSecretHeader {
            header: "X-Attachment-Secret".to_string(),
        },
        classifier: None,
        workflow,
        observer: None,
    }));
    let registry = Arc::new(ExtensionIngressRegistry::default());
    registry.register(
        EXTENSION_ID,
        ChannelIngressRegistration {
            secrets: Arc::new(StaticIngressSecrets::new(vec![VerificationCandidate {
                installation_id: INSTALLATION_ID.to_string(),
                secret: SHARED_SECRET.to_vec(),
            }])),
            sink,
            drain: None,
        },
    );
    let deployment_channels = Arc::new(
        DeploymentChannelRegistry::try_new([DeploymentChannelBinding::new(
            Arc::new(manifest()),
            adapter_port,
        )
        .expect("deployment binding")])
        .expect("deployment channel registry"),
    );
    let host = Arc::new(
        ExtensionHost::new(ExtensionHostDeps {
            store: Arc::new(RehydratedInstallationRecordStore::default())
                as Arc<dyn InstallationRecordStore>,
            loader: Arc::new(ironclaw_extension_host::test_support::FakeLoader {
                bindings: ExtensionBindings {
                    tools: None,
                    channel: None,
                },
                load_calls: Arc::new(AtomicUsize::new(0)),
                fail_load: false,
            }),
            drain: Arc::new(ironclaw_extension_host::test_support::RecordingDrain::default()),
            egress: Arc::new(ironclaw_extension_host::test_support::FakeEgressFactory),
            reserved_capability_ids: Default::default(),
            reserved_ingress_routes: Default::default(),
            hook_deadline: Duration::from_secs(1),
        })
        .await,
    );
    let router = ExtensionIngressRouter::new(
        host.snapshot_watch(),
        ExtensionIngressRouterDeps {
            secrets: registry.clone(),
            sink: registry,
            reply_context: Arc::new(NoopReplyContextStore),
            channel_egress_transport: Some(transport_port),
        },
        IngressRouterConfig::default(),
    )
    .with_deployment_channels(deployment_channels);
    JourneyHarness {
        router,
        adapter,
        transport,
        policy,
        threads,
        workspace,
        _host: host,
    }
}

fn request(event_id: &str) -> IngressRequest {
    IngressRequest {
        method: "POST".to_string(),
        extension_id: EXTENSION_ID.to_string(),
        route_suffix: "events".to_string(),
        headers: vec![("X-Attachment-Secret".to_string(), SHARED_SECRET.to_vec())],
        body: serde_json::json!({ "event_id": event_id })
            .to_string()
            .into_bytes(),
    }
}

#[tokio::test]
async fn router_retry_releases_then_lands_canonically_and_duplicate_has_no_io() {
    let harness = harness([
        FetchResult::Failure { retryable: true },
        FetchResult::Success,
    ])
    .await;

    assert_eq!(
        harness.router.handle(request("retry-event")).await.status,
        503
    );
    assert_eq!(
        harness.router.handle(request("retry-event")).await.status,
        200
    );
    assert_eq!(
        harness.router.handle(request("retry-event")).await.status,
        200
    );

    assert_eq!(harness.adapter.fetch_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        harness
            .policy
            .observed_fetch_counts
            .lock()
            .expect("policy observations lock")
            .as_slice(),
        [0, 1],
        "each fresh ledger attempt must run policy before provider I/O; duplicate replay runs neither",
    );
    {
        let requests = harness
            .transport
            .requests
            .lock()
            .expect("transport requests lock");
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| {
            request.extension_id == EXTENSION_ID
                && request.installation_id == INSTALLATION_ID
                && request.host == VENDOR_HOST
                && request.url == format!("https://{VENDOR_HOST}/files/download")
        }));
    }

    let history = harness
        .threads
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: resolved_binding().thread_id,
        })
        .await
        .expect("accepted thread history");
    assert_eq!(
        history.messages.len(),
        1,
        "retry and duplicate must not reland"
    );
    let storage_key = history.messages[0].attachments[0]
        .storage_key
        .as_deref()
        .expect("canonical workspace ref");
    assert!(storage_key.starts_with("/workspace/attachments/"));
    let reader = ProjectScopedAttachmentReader::new(Arc::clone(&harness.workspace));
    assert_eq!(
        reader
            .read(&thread_scope(), storage_key)
            .await
            .expect("canonical workspace bytes are readable"),
        b"DATA",
    );
}

#[tokio::test]
async fn router_permanent_transfer_failure_settles_and_duplicate_has_no_io() {
    let harness = harness([FetchResult::Failure { retryable: false }]).await;

    assert_eq!(
        harness
            .router
            .handle(request("permanent-event"))
            .await
            .status,
        200,
        "a permanent workflow rejection is durably settled before vendor acknowledgement",
    );
    assert_eq!(
        harness
            .router
            .handle(request("permanent-event"))
            .await
            .status,
        200,
    );

    assert_eq!(harness.adapter.fetch_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness
            .policy
            .observed_fetch_counts
            .lock()
            .expect("policy observations lock")
            .as_slice(),
        [0],
    );
    assert_eq!(
        harness
            .transport
            .requests
            .lock()
            .expect("transport requests lock")
            .len(),
        1,
    );
}
