// --- Inline-attachment landing (vision, #4644) ---

use super::*;
use ironclaw_loop_host::RejectingInputEnqueue;

use crate::{
    AuthRequirement, ExternalEventId, ParsedProductInbound, ProductInboundEnvelope,
    ProductInboundPayload, ProtocolAuthEvidence, TrustedInboundContext,
};
use ironclaw_attachments::{AttachmentCleanupReport, ProjectScopedAttachmentLander};
use ironclaw_filesystem::{
    Fault, FaultInjecting, FilesystemError, FilesystemOperation, InMemoryBackend, ScopedFilesystem,
};
use ironclaw_host_api::{
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};
use ironclaw_product_contracts::surface::ProductSurfaceError;
use ironclaw_threads::{
    AttachmentKind, AttachmentRef, FilesystemSessionThreadService, InMemorySessionThreadService,
};

use crate::binding::ResolveBindingRequest;

struct LandingBindingStub;

#[async_trait]
impl ConversationBindingService for LandingBindingStub {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductSurfaceFailure> {
        Ok(ResolvedBinding {
            tenant_id: tenant_id(),
            actor_user_id: user_id(),
            subject_user_id: Some(user_id()),
            thread_id: thread_id(),
            agent_id: Some(AgentId::new("agent:alpha").unwrap()),
            project_id: None,
        })
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductSurfaceFailure> {
        self.resolve_binding(request).await
    }
}

#[derive(Default)]
struct CapturingLander {
    landed: Mutex<Vec<InboundAttachment>>,
    cleanup_calls: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl InboundAttachmentLander for CapturingLander {
    async fn land(
        &self,
        _thread_scope: &ThreadScope,
        message_id: &str,
        attachments: Vec<InboundAttachment>,
    ) -> Result<Vec<AttachmentRef>, ProductSurfaceError> {
        let refs = attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| AttachmentRef {
                id: attachment.id.clone(),
                kind: AttachmentKind::Image,
                mime_type: attachment.mime_type.clone(),
                filename: attachment.filename.clone(),
                size_bytes: Some(attachment.bytes.len() as u64),
                storage_key: Some(format!(
                    "/workspace/attachments/test/{message_id}-{index}-img"
                )),
                extracted_text: None,
            })
            .collect();
        self.landed.lock().unwrap().extend(attachments);
        Ok(refs)
    }

    async fn rollback(
        &self,
        _thread_scope: &ThreadScope,
        _attachments: &[AttachmentRef],
    ) -> Result<(), ProductSurfaceError> {
        Ok(())
    }

    async fn cleanup_stale(
        &self,
        _thread_scope: &ThreadScope,
        referenced_storage_keys: &[String],
    ) -> Result<AttachmentCleanupReport, ProductSurfaceError> {
        self.cleanup_calls
            .lock()
            .unwrap()
            .push(referenced_storage_keys.to_vec());
        Ok(AttachmentCleanupReport::default())
    }
}

fn user_message_envelope() -> ProductInboundEnvelope {
    user_message_envelope_with_refs("evt:image-1", Vec::new())
}

fn user_message_envelope_with_refs(
    event_id: &str,
    channel_attachment_refs: Vec<ChannelAttachmentRef>,
) -> ProductInboundEnvelope {
    let installation_id = AdapterInstallationId::new("install_alpha").expect("install");
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        installation_id,
        received_at(),
        &evidence,
    )
    .expect("trusted context");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(event_id).expect("event"),
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "look at this",
                channel_attachment_refs
                    .iter()
                    .map(|source| source.descriptor.clone())
                    .collect(),
                ProductTriggerReason::DirectChat,
            )
            .expect("payload"),
        ),
    )
    .expect("parsed inbound");
    ProductInboundEnvelope::from_trusted_parse(context, parsed)
        .expect("envelope")
        .with_channel_attachment_refs(channel_attachment_refs)
        .expect("matching channel refs")
}

fn channel_attachment_ref(id: &str, size_bytes: Option<u64>) -> ChannelAttachmentRef {
    ChannelAttachmentRef {
        descriptor: ProductAttachmentDescriptor::new(
            id,
            "image/png",
            Some(format!("{id}.png")),
            size_bytes,
            ProductAttachmentKind::Image,
        )
        .expect("attachment descriptor"),
        vendor_ref: format!("vendor:{id}"),
    }
}

struct DenyAllRestrictedEgress;

#[async_trait]
impl RestrictedEgress for DenyAllRestrictedEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

struct FetchingChannelAdapter {
    fetch_count: AtomicUsize,
    fetched_vendor_refs: Mutex<Vec<String>>,
    results: Mutex<VecDeque<Result<InboundAttachment, ChannelError>>>,
}

impl FetchingChannelAdapter {
    fn new(results: impl IntoIterator<Item = Result<InboundAttachment, ChannelError>>) -> Self {
        Self {
            fetch_count: AtomicUsize::new(0),
            fetched_vendor_refs: Mutex::new(Vec::new()),
            results: Mutex::new(results.into_iter().collect()),
        }
    }

    fn fetch_count(&self) -> usize {
        self.fetch_count.load(Ordering::SeqCst)
    }

    fn fetched_vendor_refs(&self) -> Vec<String> {
        self.fetched_vendor_refs
            .lock()
            .expect("fetched vendor refs lock")
            .clone()
    }
}

#[async_trait]
impl ChannelAdapter for FetchingChannelAdapter {
    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        unimplemented!("not used by attachment workflow tests")
    }

    async fn fetch_attachment(
        &self,
        _attachment: &ChannelAttachmentRef,
        _egress: &dyn RestrictedEgress,
    ) -> Result<InboundAttachment, ChannelError> {
        self.fetch_count.fetch_add(1, Ordering::SeqCst);
        self.fetched_vendor_refs
            .lock()
            .expect("fetched vendor refs lock")
            .push(_attachment.vendor_ref.clone());
        self.results
            .lock()
            .expect("fetch results lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(ChannelError::AttachmentTransfer {
                    reason: "no scripted attachment result".to_string(),
                    retryable: false,
                })
            })
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        unimplemented!("not used by attachment workflow tests")
    }
}

fn rewritten_payload(descriptors: Vec<ProductAttachmentDescriptor>) -> BeforeInboundPolicyOutcome {
    BeforeInboundPolicyOutcome::RewriteUserMessage(
        UserMessagePayload::new(
            "policy-rewritten",
            descriptors,
            ProductTriggerReason::DirectChat,
        )
        .expect("rewritten payload"),
    )
}

#[tokio::test]
async fn policy_rewrite_can_filter_channel_attachments_and_keeps_the_exact_source() {
    let first = channel_attachment_ref("first", Some(1));
    let second = channel_attachment_ref("second", Some(1));
    let adapter = Arc::new(FetchingChannelAdapter::new([Ok(InboundAttachment {
        id: second.descriptor.external_file_id.clone(),
        mime_type: second.descriptor.mime_type.clone(),
        filename: second.descriptor.filename.clone(),
        bytes: vec![2],
    })]));
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(Arc::new(CapturingLander::default()));
    let envelope =
        user_message_envelope_with_refs("evt:rewrite-filter", vec![first, second.clone()]);

    let result = accept_channel_message(
        &service,
        &envelope,
        &FetchMustNotPrecedePolicy {
            adapter: adapter.clone(),
            outcome: rewritten_payload(vec![second.descriptor.clone()]),
        },
        adapter.clone(),
    )
    .await;

    assert!(matches!(
        result,
        Ok(InboundUserMessageDispatch::Accepted(_))
    ));
    assert_eq!(adapter.fetched_vendor_refs(), vec![second.vendor_ref]);
}

#[tokio::test]
async fn policy_rewrite_can_reorder_channel_attachments_and_sources_follow_descriptors() {
    let first = channel_attachment_ref("first", Some(1));
    let second = channel_attachment_ref("second", Some(1));
    let adapter = Arc::new(FetchingChannelAdapter::new([
        Ok(InboundAttachment {
            id: second.descriptor.external_file_id.clone(),
            mime_type: second.descriptor.mime_type.clone(),
            filename: second.descriptor.filename.clone(),
            bytes: vec![2],
        }),
        Ok(InboundAttachment {
            id: first.descriptor.external_file_id.clone(),
            mime_type: first.descriptor.mime_type.clone(),
            filename: first.descriptor.filename.clone(),
            bytes: vec![1],
        }),
    ]));
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(Arc::new(CapturingLander::default()));
    let envelope =
        user_message_envelope_with_refs("evt:rewrite-reorder", vec![first.clone(), second.clone()]);

    let result = accept_channel_message(
        &service,
        &envelope,
        &FetchMustNotPrecedePolicy {
            adapter: adapter.clone(),
            outcome: rewritten_payload(vec![second.descriptor.clone(), first.descriptor.clone()]),
        },
        adapter.clone(),
    )
    .await;

    assert!(matches!(
        result,
        Ok(InboundUserMessageDispatch::Accepted(_))
    ));
    assert_eq!(
        adapter.fetched_vendor_refs(),
        vec![second.vendor_ref, first.vendor_ref]
    );
}

#[tokio::test]
async fn policy_rewrite_rejects_injected_or_ambiguous_channel_attachment_sources() {
    let unique = channel_attachment_ref("unique", Some(1));
    let injected = channel_attachment_ref("injected", Some(1));
    let duplicate_a = channel_attachment_ref("duplicate", Some(1));
    let mut duplicate_b = duplicate_a.clone();
    duplicate_b.vendor_ref = "vendor:duplicate-other".to_string();

    let cases = [
        (
            "evt:rewrite-injected",
            vec![unique],
            vec![injected.descriptor],
        ),
        (
            "evt:rewrite-ambiguous",
            vec![duplicate_a.clone(), duplicate_b],
            vec![duplicate_a.descriptor],
        ),
    ];
    for (event_id, sources, rewritten_descriptors) in cases {
        let adapter = Arc::new(FetchingChannelAdapter::new([]));
        let service = DefaultInboundTurnService::new(
            LandingBindingStub,
            Arc::new(InMemorySessionThreadService::default()),
            CapturingTurnCoordinator::default(),
            Arc::new(RejectingInputEnqueue),
        )
        .with_inbound_attachments(Arc::new(CapturingLander::default()));
        let envelope = user_message_envelope_with_refs(event_id, sources);

        let result = accept_channel_message(
            &service,
            &envelope,
            &FetchMustNotPrecedePolicy {
                adapter: adapter.clone(),
                outcome: rewritten_payload(rewritten_descriptors),
            },
            adapter.clone(),
        )
        .await;

        assert!(matches!(
            result,
            Err(ProductSurfaceFailure::TurnSubmissionRejected { .. })
        ));
        assert_eq!(adapter.fetch_count(), 0);
    }
}

type AttachmentTurnService = DefaultInboundTurnService<
    LandingBindingStub,
    Arc<InMemorySessionThreadService>,
    CapturingTurnCoordinator,
>;

async fn accept_channel_message(
    service: &AttachmentTurnService,
    envelope: &ProductInboundEnvelope,
    policy: &dyn BeforeInboundPolicy,
    adapter: Arc<FetchingChannelAdapter>,
) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
    service
        .accept_user_message_with_before_policy_and_channel_transfer(
            envelope,
            policy,
            adapter,
            Arc::new(DenyAllRestrictedEgress),
        )
        .await
}

struct FetchMustNotPrecedePolicy {
    adapter: Arc<FetchingChannelAdapter>,
    outcome: BeforeInboundPolicyOutcome,
}

#[async_trait]
impl BeforeInboundPolicy for FetchMustNotPrecedePolicy {
    async fn check_user_message(
        &self,
        _request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductSurfaceFailure> {
        assert_eq!(
            self.adapter.fetch_count(),
            0,
            "attachment bytes must not be fetched before policy"
        );
        Ok(self.outcome.clone())
    }
}

struct UnexpectedBeforeInboundPolicy;

#[async_trait]
impl BeforeInboundPolicy for UnexpectedBeforeInboundPolicy {
    async fn check_user_message(
        &self,
        _request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductSurfaceFailure> {
        panic!("invalid channel attachment metadata must fail before policy")
    }
}

#[tokio::test]
async fn channel_attachment_fetches_after_policy_then_lands_once() {
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let source = channel_attachment_ref("channel-image-0", Some(bytes.len() as u64));
    let adapter = Arc::new(FetchingChannelAdapter::new([Ok(InboundAttachment {
        id: source.descriptor.external_file_id.clone(),
        mime_type: "image/png".to_string(),
        filename: source.descriptor.filename.clone(),
        bytes: bytes.clone(),
    })]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-allow", vec![source]);

    let dispatch = accept_channel_message(
        &service,
        &envelope,
        &FetchMustNotPrecedePolicy {
            adapter: adapter.clone(),
            outcome: BeforeInboundPolicyOutcome::Allow,
        },
        adapter.clone(),
    )
    .await
    .expect("channel attachment turn succeeds");

    assert!(matches!(dispatch, InboundUserMessageDispatch::Accepted(_)));
    assert_eq!(adapter.fetch_count(), 1);
    let landed = lander.landed.lock().expect("landed lock");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].bytes, bytes);
}

/// Regression: the fetched/declared MIME comparison normalized only the
/// fetched side, so any descriptor whose media type carried a parameter
/// (`text/plain; charset=utf-8` — what Telegram clients routinely report
/// for text documents) failed the equality check and rejected the whole
/// message, caption included, instead of catching a provider mismatch.
#[tokio::test]
async fn declared_mime_parameters_do_not_reject_a_matching_attachment() {
    let bytes = b"hello".to_vec();
    let source = ChannelAttachmentRef {
        descriptor: ProductAttachmentDescriptor::new(
            "channel-text-0",
            "text/plain; charset=utf-8",
            Some("notes.txt".to_string()),
            Some(bytes.len() as u64),
            ProductAttachmentKind::Document,
        )
        .expect("attachment descriptor"),
        vendor_ref: "vendor:channel-text-0".to_string(),
    };
    let adapter = Arc::new(FetchingChannelAdapter::new([Ok(InboundAttachment {
        id: source.descriptor.external_file_id.clone(),
        mime_type: "text/plain; charset=utf-8".to_string(),
        filename: source.descriptor.filename.clone(),
        bytes: bytes.clone(),
    })]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-mime-params", vec![source]);

    let dispatch = accept_channel_message(
        &service,
        &envelope,
        &FetchMustNotPrecedePolicy {
            adapter: adapter.clone(),
            outcome: BeforeInboundPolicyOutcome::Allow,
        },
        adapter.clone(),
    )
    .await
    .expect("a parameterized declared MIME type still admits the message");

    assert!(matches!(dispatch, InboundUserMessageDispatch::Accepted(_)));
    let landed = lander.landed.lock().expect("landed lock");
    assert_eq!(landed.len(), 1);
    // The landed copy carries the canonical form, not the raw parameters.
    assert_eq!(landed[0].mime_type, "text/plain");
}

/// Regression: the descriptor filename was copied over the fetched one
/// unconditionally, discarding the name an adapter recovered for vendor
/// payloads that carry none (Telegram photos, voice notes, stickers).
#[tokio::test]
async fn adapter_recovered_filename_survives_when_the_descriptor_has_none() {
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let source = ChannelAttachmentRef {
        descriptor: ProductAttachmentDescriptor::new(
            "channel-photo-0",
            "image/png",
            None,
            Some(bytes.len() as u64),
            ProductAttachmentKind::Image,
        )
        .expect("attachment descriptor"),
        vendor_ref: "vendor:channel-photo-0".to_string(),
    };
    let adapter = Arc::new(FetchingChannelAdapter::new([Ok(InboundAttachment {
        id: source.descriptor.external_file_id.clone(),
        mime_type: "image/png".to_string(),
        filename: Some("file_15.jpg".to_string()),
        bytes: bytes.clone(),
    })]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-photo", vec![source]);

    accept_channel_message(
        &service,
        &envelope,
        &FetchMustNotPrecedePolicy {
            adapter: adapter.clone(),
            outcome: BeforeInboundPolicyOutcome::Allow,
        },
        adapter.clone(),
    )
    .await
    .expect("channel attachment turn succeeds");

    let landed = lander.landed.lock().expect("landed lock");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].filename.as_deref(), Some("file_15.jpg"));
}

#[tokio::test]
async fn rejected_policy_never_fetches_channel_attachment() {
    let source = channel_attachment_ref("channel-image-0", None);
    let adapter = Arc::new(FetchingChannelAdapter::new([]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-reject", vec![source]);

    let dispatch = accept_channel_message(
        &service,
        &envelope,
        &FetchMustNotPrecedePolicy {
            adapter: adapter.clone(),
            outcome: BeforeInboundPolicyOutcome::Reject(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                "rejected by test policy",
            )),
        },
        adapter.clone(),
    )
    .await
    .expect("policy rejection is a dispatch outcome");

    assert!(matches!(dispatch, InboundUserMessageDispatch::Rejected(_)));
    assert_eq!(adapter.fetch_count(), 0);
    assert!(lander.landed.lock().expect("landed lock").is_empty());
}

#[tokio::test]
async fn accepted_message_replay_does_not_refetch_or_reland_attachment() {
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let source = channel_attachment_ref("channel-image-0", Some(bytes.len() as u64));
    let adapter = Arc::new(FetchingChannelAdapter::new([Ok(InboundAttachment {
        id: source.descriptor.external_file_id.clone(),
        mime_type: "image/png".to_string(),
        filename: source.descriptor.filename.clone(),
        bytes,
    })]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-replay", vec![source]);

    accept_channel_message(
        &service,
        &envelope,
        &NoopBeforeInboundPolicy,
        adapter.clone(),
    )
    .await
    .expect("first delivery succeeds");
    accept_channel_message(
        &service,
        &envelope,
        &NoopBeforeInboundPolicy,
        adapter.clone(),
    )
    .await
    .expect("accepted replay succeeds");

    assert_eq!(adapter.fetch_count(), 1);
    assert_eq!(lander.landed.lock().expect("landed lock").len(), 1);
}

#[tokio::test]
async fn declared_attachment_over_budget_fails_before_fetch_or_landing() {
    let source = channel_attachment_ref(
        "channel-image-0",
        Some(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 + 1),
    );
    let adapter = Arc::new(FetchingChannelAdapter::new([]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-too-large", vec![source]);

    let result = accept_channel_message(
        &service,
        &envelope,
        &NoopBeforeInboundPolicy,
        adapter.clone(),
    )
    .await;
    let Err(error) = result else {
        panic!("declared oversized attachment must fail");
    };

    assert!(matches!(
        error,
        ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        }
    ));
    assert_eq!(adapter.fetch_count(), 0);
    assert!(lander.landed.lock().expect("landed lock").is_empty());
}

#[tokio::test]
async fn retryable_channel_transfer_can_retry_without_duplicate_landing() {
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let source = channel_attachment_ref("channel-image-0", Some(bytes.len() as u64));
    let adapter = Arc::new(FetchingChannelAdapter::new([
        Err(ChannelError::AttachmentTransfer {
            reason: "provider timeout details".to_string(),
            retryable: true,
        }),
        Ok(InboundAttachment {
            id: source.descriptor.external_file_id.clone(),
            mime_type: "image/png".to_string(),
            filename: None,
            bytes,
        }),
    ]));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-retry", vec![source]);

    let first = accept_channel_message(
        &service,
        &envelope,
        &NoopBeforeInboundPolicy,
        adapter.clone(),
    )
    .await;
    assert!(matches!(
        first,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: true,
            ref reason,
        }) if reason == "channel attachment transfer failed"
    ));
    let second = accept_channel_message(
        &service,
        &envelope,
        &NoopBeforeInboundPolicy,
        adapter.clone(),
    )
    .await;

    assert!(matches!(
        second,
        Ok(InboundUserMessageDispatch::Accepted(_))
    ));
    assert_eq!(adapter.fetch_count(), 2);
    assert_eq!(lander.landed.lock().expect("landed lock").len(), 1);
}

#[test]
fn channel_configuration_failure_is_retryable_and_redacted() {
    let failure = channel_attachment_error(ChannelError::Configuration {
        reason: "secret token details".to_string(),
    });

    assert!(matches!(
        failure,
        ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: true,
            ref reason,
        } if reason == "channel attachment transfer failed"
    ));
}

#[tokio::test]
async fn missing_transfer_support_fails_closed_without_landing() {
    let source = channel_attachment_ref("channel-image-0", None);
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-unsupported", vec![source]);

    let result = service
        .accept_user_message_with_before_policy(&envelope, &NoopBeforeInboundPolicy)
        .await;

    assert!(matches!(
        result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
    assert!(lander.landed.lock().expect("landed lock").is_empty());
}

#[tokio::test]
async fn mixed_inline_and_channel_sources_fail_before_fetch_or_landing() {
    let source = channel_attachment_ref("channel-image-0", None);
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_refs("evt:channel-mixed", vec![source]);

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![InboundAttachment {
                id: "inline-image-0".to_string(),
                mime_type: "image/png".to_string(),
                filename: Some("inline.png".to_string()),
                bytes: vec![0x89, b'P', b'N', b'G'],
            }],
        )
        .await;

    assert!(matches!(
        result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
    assert!(lander.landed.lock().expect("landed lock").is_empty());
}

#[tokio::test]
async fn count_and_declared_total_limits_fail_before_fetch() {
    let cases = [
        (
            "evt:channel-too-many",
            (0..=DEFAULT_ATTACHMENT_BUDGETS.max_count)
                .map(|index| channel_attachment_ref(&format!("image-{index}"), Some(1)))
                .collect::<Vec<_>>(),
        ),
        (
            "evt:channel-total-too-large",
            (0..3)
                .map(|index| {
                    channel_attachment_ref(&format!("image-{index}"), Some(4 * 1024 * 1024))
                })
                .collect::<Vec<_>>(),
        ),
    ];

    for (event_id, sources) in cases {
        let adapter = Arc::new(FetchingChannelAdapter::new([]));
        let service = DefaultInboundTurnService::new(
            LandingBindingStub,
            Arc::new(InMemorySessionThreadService::default()),
            CapturingTurnCoordinator::default(),
            Arc::new(RejectingInputEnqueue),
        )
        .with_inbound_attachments(Arc::new(CapturingLander::default()));
        let envelope = user_message_envelope_with_refs(event_id, sources);

        let result = accept_channel_message(
            &service,
            &envelope,
            &UnexpectedBeforeInboundPolicy,
            adapter.clone(),
        )
        .await;

        assert!(matches!(
            result,
            Err(ProductSurfaceFailure::InboundAttachmentFailed {
                retryable: false,
                ..
            })
        ));
        assert_eq!(adapter.fetch_count(), 0);
    }
}

#[tokio::test]
async fn actual_per_file_and_total_limits_fail_without_landing() {
    let per_file_source = channel_attachment_ref("too-large", None);
    let per_file_adapter = Arc::new(FetchingChannelAdapter::new([Ok(InboundAttachment {
        id: per_file_source.descriptor.external_file_id.clone(),
        mime_type: "image/png".to_string(),
        filename: None,
        bytes: vec![0; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1],
    })]));
    let per_file_lander = Arc::new(CapturingLander::default());
    let per_file_service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(per_file_lander.clone());
    let per_file_envelope =
        user_message_envelope_with_refs("evt:actual-file-too-large", vec![per_file_source]);

    let per_file_result = accept_channel_message(
        &per_file_service,
        &per_file_envelope,
        &NoopBeforeInboundPolicy,
        per_file_adapter.clone(),
    )
    .await;
    assert!(matches!(
        per_file_result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
    assert_eq!(per_file_adapter.fetch_count(), 1);
    assert!(
        per_file_lander
            .landed
            .lock()
            .expect("landed lock")
            .is_empty()
    );

    let total_sources = (0..3)
        .map(|index| channel_attachment_ref(&format!("total-{index}"), None))
        .collect::<Vec<_>>();
    let total_results = total_sources.iter().map(|source| {
        Ok(InboundAttachment {
            id: source.descriptor.external_file_id.clone(),
            mime_type: "image/png".to_string(),
            filename: None,
            bytes: vec![0; 4 * 1024 * 1024],
        })
    });
    let total_adapter = Arc::new(FetchingChannelAdapter::new(total_results));
    let total_lander = Arc::new(CapturingLander::default());
    let total_service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(total_lander.clone());
    let total_envelope =
        user_message_envelope_with_refs("evt:actual-total-too-large", total_sources);

    let total_result = accept_channel_message(
        &total_service,
        &total_envelope,
        &NoopBeforeInboundPolicy,
        total_adapter.clone(),
    )
    .await;
    assert!(matches!(
        total_result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
    assert_eq!(total_adapter.fetch_count(), 3);
    assert!(total_lander.landed.lock().expect("landed lock").is_empty());
}

/// Caller-level coverage for the native vision door: a user message carrying
/// host-staged inline bytes must route those bytes through the
/// [`InboundAttachmentLander`] before message acceptance (the bytes never
/// touch the bytes-free product envelope). Mirrors the WebChat landing path.
#[tokio::test]
async fn native_attachment_path_lands_inline_bytes_before_acceptance() {
    let thread_service = std::sync::Arc::new(InMemorySessionThreadService::default());
    let lander = std::sync::Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        thread_service,
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());

    let envelope = user_message_envelope();
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let attachment = InboundAttachment {
        id: "openai-image-0".to_string(),
        mime_type: "image/png".to_string(),
        filename: Some("image-0.png".to_string()),
        bytes: bytes.clone(),
    };

    let dispatch = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![attachment],
        )
        .await
        .expect("accepting a user message with inline attachments succeeds");

    assert!(matches!(dispatch, InboundUserMessageDispatch::Accepted(_)));
    let landed = lander.landed.lock().unwrap();
    assert_eq!(landed.len(), 1, "the inline image is landed exactly once");
    assert_eq!(landed[0].mime_type, "image/png");
    assert_eq!(landed[0].bytes, bytes);
    drop(landed);
    let cleanup_calls = lander.cleanup_calls.lock().unwrap();
    assert_eq!(cleanup_calls.len(), 1);
    assert!(
        cleanup_calls[0]
            .iter()
            .any(|key| key.contains("evt:image-1"))
    );
}

#[tokio::test]
async fn native_attachment_path_rolls_back_landed_batch_when_message_acceptance_fails() {
    let thread_backend = Arc::new(
        FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/messages/")
                .backend("injected message acceptance failure"),
        ),
    );
    let thread_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").unwrap(),
        VirtualPath::new("/tenants/test/users/test/threads").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let thread_service = Arc::new(FilesystemSessionThreadService::new(Arc::new(
        ScopedFilesystem::with_fixed_view(thread_backend, thread_mounts),
    )));

    let attachment_backend = Arc::new(InMemoryBackend::new());
    let attachment_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let attachment_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        attachment_backend,
        attachment_mounts,
    ));
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        thread_service,
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(Arc::new(ProjectScopedAttachmentLander::new(Arc::clone(
        &attachment_filesystem,
    ))));

    let envelope = user_message_envelope_with_refs("evt:rollback", Vec::new());
    let resource_scope = ThreadScope {
        tenant_id: tenant_id(),
        agent_id: AgentId::new("agent:alpha").unwrap(),
        project_id: None,
        owner_user_id: Some(user_id()),
        mission_id: None,
    }
    .to_resource_scope();
    let date_before = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![InboundAttachment {
                id: "rollback-image".to_string(),
                mime_type: "image/png".to_string(),
                filename: Some("rollback.png".to_string()),
                bytes: vec![1, 2, 3, 4],
            }],
        )
        .await;
    let date_after = chrono::Utc::now().format("%Y-%m-%d").to_string();

    assert!(result.is_err(), "message acceptance must fail in this test");
    for date in [date_before, date_after] {
        let batch = ironclaw_attachments::attachment_batch_scoped_path(
            "/workspace",
            &date,
            envelope.external_event_id().as_str(),
        )
        .unwrap();
        assert!(
            matches!(
                attachment_filesystem.stat(&resource_scope, &batch).await,
                Err(FilesystemError::NotFound { .. })
            ),
            "a failed message acceptance must remove the landed attachment batch at {}",
            batch.as_str()
        );
    }
}

/// Without a lander wired, a user message carrying inline bytes must fail
/// closed (rejected), never silently dropping the attachment.
#[tokio::test]
async fn native_attachment_path_without_lander_fails_closed() {
    let thread_service = std::sync::Arc::new(InMemorySessionThreadService::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        thread_service,
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    );

    let envelope = user_message_envelope();
    let attachment = InboundAttachment {
        id: "openai-image-0".to_string(),
        mime_type: "image/png".to_string(),
        filename: Some("image-0.png".to_string()),
        bytes: vec![0x89, b'P', b'N', b'G'],
    };

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![attachment],
        )
        .await;

    assert!(
        matches!(
            result,
            Err(ProductSurfaceFailure::TurnSubmissionRejected { .. })
        ),
        "a missing lander must reject the turn, never silently drop the attachment"
    );
}

/// A turn service that does not override the attachments method, exercising
/// the trait default. Its `accept_user_message_with_before_policy` returns a
/// distinct `Transient` error so a test can tell "the default delegated"
/// (Transient) apart from "the default rejected" (TurnSubmissionRejected).
struct DefaultAttachmentsTurnService;

#[async_trait]
impl InboundTurnService for DefaultAttachmentsTurnService {
    async fn replay_accepted_user_message(
        &self,
        _envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductSurfaceFailure> {
        Ok(None)
    }

    async fn accept_user_message(
        &self,
        _envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure> {
        Err(ProductSurfaceFailure::Transient {
            reason: "delegated".into(),
        })
    }

    async fn accept_user_message_with_before_policy(
        &self,
        _envelope: &ProductInboundEnvelope,
        _before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        Err(ProductSurfaceFailure::Transient {
            reason: "delegated".into(),
        })
    }
}

/// The trait default must reject a turn carrying inline bytes rather than
/// silently dropping them, but still pass an attachment-free turn straight
/// through to the underlying acceptance path.
#[tokio::test]
async fn default_attachments_impl_rejects_bytes_but_passes_empty_through() {
    let service = DefaultAttachmentsTurnService;
    let envelope = user_message_envelope();

    let rejected = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![InboundAttachment {
                id: "openai-image-0".to_string(),
                mime_type: "image/png".to_string(),
                filename: Some("image-0.png".to_string()),
                bytes: vec![0x89, b'P', b'N', b'G'],
            }],
        )
        .await;
    assert!(
        matches!(
            rejected,
            Err(ProductSurfaceFailure::TurnSubmissionRejected { .. })
        ),
        "the default must fail closed on inline bytes, never silently drop them"
    );

    let delegated = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            Vec::new(),
        )
        .await;
    assert!(
        matches!(delegated, Err(ProductSurfaceFailure::Transient { .. })),
        "with no attachments the default must delegate to the normal path"
    );
}
