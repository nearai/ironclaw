// --- Inline-attachment landing (vision, #4644) ---

use super::*;
use ironclaw_loop_host::RejectingInputEnqueue;

use ironclaw_attachments::{AttachmentCleanupReport, ProjectScopedAttachmentLander};
use ironclaw_extension_contracts::external::ExternalEventId;
use ironclaw_filesystem::{
    Fault, FaultInjecting, FilesystemError, FilesystemOperation, InMemoryBackend, ScopedFilesystem,
};
use ironclaw_host_api::product_adapter::auth::{AuthRequirement, ProtocolAuthEvidence};
use ironclaw_host_api::{
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};
use ironclaw_product_contracts::inbound::{
    ParsedProductInbound, ProductInboundEnvelope, ProductInboundPayload, TrustedInboundContext,
};
use ironclaw_product_contracts::surface::ProductSurfaceError;
use ironclaw_threads::{
    AttachmentKind, AttachmentRef, FilesystemSessionThreadService, InMemorySessionThreadService,
};

use ironclaw_product_contracts::binding::ResolveBindingRequest;
use ironclaw_product_contracts::error::ProductOperationFailure;

struct LandingBindingStub;

#[async_trait]
impl ProductBindingResolver for LandingBindingStub {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        Ok(ResolvedBinding {
            tenant_id: tenant_id(),
            actor_user_id: user_id(),
            thread_id: thread_id(),
            agent_id: Some(AgentId::new("agent:alpha").unwrap()),
            project_id: None,
            source_binding_ref: SourceBindingRef::new("source:alpha").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:alpha").unwrap(),
        })
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
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
    user_message_envelope_with_descriptors("evt:image-1", Vec::new())
}

fn user_message_envelope_with_descriptors(
    event_id: &str,
    descriptors: Vec<ProductAttachmentDescriptor>,
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
                descriptors,
                ProductTriggerReason::DirectChat,
            )
            .expect("payload"),
        ),
    )
    .expect("parsed inbound");
    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

fn attachment_descriptor(id: &str, size_bytes: Option<u64>) -> ProductAttachmentDescriptor {
    ProductAttachmentDescriptor::new(
        id,
        "image/png",
        Some(format!("{id}.png")),
        size_bytes,
        ProductAttachmentKind::Image,
    )
    .expect("attachment descriptor")
}

fn complete_attachment(
    descriptor: &ProductAttachmentDescriptor,
    bytes: Vec<u8>,
) -> InboundAttachment {
    InboundAttachment {
        id: descriptor.external_file_id.clone(),
        mime_type: descriptor.mime_type.clone(),
        filename: descriptor.filename.clone(),
        bytes,
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
async fn policy_rewrite_can_filter_complete_attachments_and_keeps_exact_bytes() {
    let first = attachment_descriptor("first", Some(1));
    let second = attachment_descriptor("second", Some(1));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_descriptors(
        "evt:rewrite-filter",
        vec![first.clone(), second.clone()],
    );

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &ScriptedAttachmentPolicy(rewritten_payload(vec![second.clone()])),
            vec![
                complete_attachment(&first, vec![1]),
                complete_attachment(&second, vec![2]),
            ],
        )
        .await;

    assert!(matches!(
        result,
        Ok(InboundUserMessageDispatch::Accepted(_))
    ));
    let landed = lander.landed.lock().expect("landed lock");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].id, "second");
    assert_eq!(landed[0].bytes, vec![2]);
}

#[tokio::test]
async fn policy_rewrite_can_reorder_complete_attachments_with_descriptors() {
    let first = attachment_descriptor("first", Some(1));
    let second = attachment_descriptor("second", Some(1));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_descriptors(
        "evt:rewrite-reorder",
        vec![first.clone(), second.clone()],
    );

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &ScriptedAttachmentPolicy(rewritten_payload(vec![second.clone(), first.clone()])),
            vec![
                complete_attachment(&first, vec![1]),
                complete_attachment(&second, vec![2]),
            ],
        )
        .await;

    assert!(matches!(
        result,
        Ok(InboundUserMessageDispatch::Accepted(_))
    ));
    let landed = lander.landed.lock().expect("landed lock");
    assert_eq!(
        landed
            .iter()
            .map(|file| file.id.as_str())
            .collect::<Vec<_>>(),
        vec!["second", "first"]
    );
    assert_eq!(
        landed
            .iter()
            .map(|file| file.bytes.clone())
            .collect::<Vec<_>>(),
        vec![vec![2], vec![1]]
    );
}

#[tokio::test]
async fn policy_rewrite_rejects_injected_or_mutated_attachment_descriptors() {
    let original = attachment_descriptor("original", Some(1));
    let injected = attachment_descriptor("injected", Some(1));
    let mut mutated = original.clone();
    mutated.filename = Some("changed.png".to_string());

    for (event_id, rewritten_descriptor) in [
        ("evt:rewrite-injected", injected),
        ("evt:rewrite-mutated", mutated),
    ] {
        let service = DefaultInboundTurnService::new(
            LandingBindingStub,
            Arc::new(InMemorySessionThreadService::default()),
            CapturingTurnCoordinator::default(),
            Arc::new(RejectingInputEnqueue),
        )
        .with_inbound_attachments(Arc::new(CapturingLander::default()));
        let envelope = user_message_envelope_with_descriptors(event_id, vec![original.clone()]);

        let result = service
            .accept_user_message_with_before_policy_and_attachments(
                &envelope,
                &ScriptedAttachmentPolicy(rewritten_payload(vec![rewritten_descriptor])),
                vec![complete_attachment(&original, vec![1])],
            )
            .await;

        assert!(matches!(
            result,
            Err(ProductSurfaceFailure::InboundAttachmentFailed {
                retryable: false,
                ..
            })
        ));
    }
}

struct ScriptedAttachmentPolicy(BeforeInboundPolicyOutcome);

#[async_trait]
impl BeforeInboundPolicy for ScriptedAttachmentPolicy {
    async fn check_user_message(
        &self,
        _request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductSurfaceFailure> {
        Ok(self.0.clone())
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
async fn complete_attachment_is_validated_before_policy_then_lands_once() {
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let descriptor = attachment_descriptor("channel-image-0", Some(bytes.len() as u64));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope =
        user_message_envelope_with_descriptors("evt:channel-allow", vec![descriptor.clone()]);

    let dispatch = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &ScriptedAttachmentPolicy(BeforeInboundPolicyOutcome::Allow),
            vec![complete_attachment(&descriptor, bytes.clone())],
        )
        .await
        .expect("complete attachment turn succeeds");

    assert!(matches!(dispatch, InboundUserMessageDispatch::Accepted(_)));
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
    let descriptor = ProductAttachmentDescriptor::new(
        "channel-text-0",
        "text/plain; charset=utf-8",
        Some("notes.txt".to_string()),
        Some(bytes.len() as u64),
        ProductAttachmentKind::Document,
    )
    .expect("attachment descriptor");
    let attachment = InboundAttachment {
        id: descriptor.external_file_id.clone(),
        mime_type: "text/plain; charset=utf-8".to_string(),
        filename: descriptor.filename.clone(),
        bytes: bytes.clone(),
    };
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope =
        user_message_envelope_with_descriptors("evt:channel-mime-params", vec![descriptor]);

    let dispatch = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![attachment],
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
    let descriptor = ProductAttachmentDescriptor::new(
        "channel-photo-0",
        "image/png",
        None,
        Some(bytes.len() as u64),
        ProductAttachmentKind::Image,
    )
    .expect("attachment descriptor");
    let attachment = InboundAttachment {
        id: descriptor.external_file_id.clone(),
        mime_type: "image/png".to_string(),
        filename: Some("file_15.jpg".to_string()),
        bytes: bytes.clone(),
    };
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_descriptors("evt:channel-photo", vec![descriptor]);

    service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &NoopBeforeInboundPolicy,
            vec![attachment],
        )
        .await
        .expect("complete attachment turn succeeds");

    let landed = lander.landed.lock().expect("landed lock");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].filename.as_deref(), Some("file_15.jpg"));
}

#[tokio::test]
async fn rejected_policy_never_lands_complete_attachment() {
    let descriptor = attachment_descriptor("channel-image-0", None);
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope =
        user_message_envelope_with_descriptors("evt:channel-reject", vec![descriptor.clone()]);

    let dispatch = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &ScriptedAttachmentPolicy(BeforeInboundPolicyOutcome::Reject(
                ProductRejection::permanent(
                    ProductRejectionKind::PolicyDenied,
                    "rejected by test policy",
                ),
            )),
            vec![complete_attachment(&descriptor, vec![1])],
        )
        .await
        .expect("policy rejection is a dispatch outcome");

    assert!(matches!(dispatch, InboundUserMessageDispatch::Rejected(_)));
    assert!(lander.landed.lock().expect("landed lock").is_empty());
}

#[tokio::test]
async fn accepted_message_replay_does_not_reland_complete_attachment() {
    let bytes = vec![0x89, b'P', b'N', b'G'];
    let descriptor = attachment_descriptor("channel-image-0", Some(bytes.len() as u64));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope =
        user_message_envelope_with_descriptors("evt:channel-replay", vec![descriptor.clone()]);

    for _ in 0..2 {
        service
            .accept_user_message_with_before_policy_and_attachments(
                &envelope,
                &NoopBeforeInboundPolicy,
                vec![complete_attachment(&descriptor, bytes.clone())],
            )
            .await
            .expect("delivery or accepted replay succeeds");
    }

    assert_eq!(lander.landed.lock().expect("landed lock").len(), 1);
}

#[tokio::test]
async fn declared_size_disagreement_fails_before_policy_or_landing() {
    let descriptor = attachment_descriptor("channel-image-0", Some(2));
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_descriptors(
        "evt:channel-size-mismatch",
        vec![descriptor.clone()],
    );

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &UnexpectedBeforeInboundPolicy,
            vec![complete_attachment(&descriptor, vec![1])],
        )
        .await;
    let Err(error) = result else {
        panic!("declared-size disagreement must fail");
    };

    assert!(matches!(
        error,
        ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        }
    ));
    assert!(lander.landed.lock().expect("landed lock").is_empty());
}

#[tokio::test]
async fn missing_complete_bytes_fail_closed_without_landing() {
    let descriptor = attachment_descriptor("channel-image-0", None);
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope = user_message_envelope_with_descriptors("evt:channel-missing", vec![descriptor]);

    let result = service
        .accept_user_message_with_before_policy(&envelope, &UnexpectedBeforeInboundPolicy)
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
async fn fetched_id_mismatch_fails_before_policy_or_landing() {
    let descriptor = attachment_descriptor("channel-image-0", None);
    let lander = Arc::new(CapturingLander::default());
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(lander.clone());
    let envelope =
        user_message_envelope_with_descriptors("evt:channel-id-mismatch", vec![descriptor]);

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &UnexpectedBeforeInboundPolicy,
            vec![InboundAttachment {
                id: "different-image".to_string(),
                mime_type: "image/png".to_string(),
                filename: Some("channel-image-0.png".to_string()),
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
async fn count_limit_fails_before_policy() {
    let descriptors = (0..=DEFAULT_ATTACHMENT_BUDGETS.max_count)
        .map(|index| attachment_descriptor(&format!("image-{index}"), Some(1)))
        .collect::<Vec<_>>();
    let attachments = descriptors
        .iter()
        .map(|descriptor| complete_attachment(descriptor, vec![1]))
        .collect();
    let service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(Arc::new(CapturingLander::default()));
    let envelope = user_message_envelope_with_descriptors("evt:too-many", descriptors);

    let result = service
        .accept_user_message_with_before_policy_and_attachments(
            &envelope,
            &UnexpectedBeforeInboundPolicy,
            attachments,
        )
        .await;

    assert!(matches!(
        result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
}

#[tokio::test]
async fn actual_per_file_and_total_limits_fail_without_landing() {
    let per_file_descriptor = attachment_descriptor("too-large", None);
    let per_file_lander = Arc::new(CapturingLander::default());
    let per_file_service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(per_file_lander.clone());
    let per_file_envelope = user_message_envelope_with_descriptors(
        "evt:actual-file-too-large",
        vec![per_file_descriptor.clone()],
    );

    let per_file_result = per_file_service
        .accept_user_message_with_before_policy_and_attachments(
            &per_file_envelope,
            &UnexpectedBeforeInboundPolicy,
            vec![complete_attachment(
                &per_file_descriptor,
                vec![0; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1],
            )],
        )
        .await;
    assert!(matches!(
        per_file_result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
    assert!(
        per_file_lander
            .landed
            .lock()
            .expect("landed lock")
            .is_empty()
    );

    let total_descriptors = (0..3)
        .map(|index| attachment_descriptor(&format!("total-{index}"), None))
        .collect::<Vec<_>>();
    let total_attachments = total_descriptors
        .iter()
        .map(|descriptor| complete_attachment(descriptor, vec![0; 4 * 1024 * 1024]))
        .collect();
    let total_lander = Arc::new(CapturingLander::default());
    let total_service = DefaultInboundTurnService::new(
        LandingBindingStub,
        Arc::new(InMemorySessionThreadService::default()),
        CapturingTurnCoordinator::default(),
        Arc::new(RejectingInputEnqueue),
    )
    .with_inbound_attachments(total_lander.clone());
    let total_envelope =
        user_message_envelope_with_descriptors("evt:actual-total-too-large", total_descriptors);

    let total_result = total_service
        .accept_user_message_with_before_policy_and_attachments(
            &total_envelope,
            &UnexpectedBeforeInboundPolicy,
            total_attachments,
        )
        .await;
    assert!(matches!(
        total_result,
        Err(ProductSurfaceFailure::InboundAttachmentFailed {
            retryable: false,
            ..
        })
    ));
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

    let bytes = vec![0x89, b'P', b'N', b'G'];
    let descriptor = attachment_descriptor("openai-image-0", Some(bytes.len() as u64));
    let envelope = user_message_envelope_with_descriptors("evt:image-1", vec![descriptor.clone()]);
    let attachment = InboundAttachment {
        id: descriptor.external_file_id,
        mime_type: "image/png".to_string(),
        filename: descriptor.filename,
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

    let descriptor = ProductAttachmentDescriptor::new(
        "rollback-image",
        "image/png",
        Some("rollback.png".to_string()),
        Some(4),
        ProductAttachmentKind::Image,
    )
    .expect("rollback descriptor");
    let envelope = user_message_envelope_with_descriptors("evt:rollback", vec![descriptor.clone()]);
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
                id: descriptor.external_file_id,
                mime_type: "image/png".to_string(),
                filename: descriptor.filename,
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

    let descriptor = attachment_descriptor("openai-image-0", Some(4));
    let envelope =
        user_message_envelope_with_descriptors("evt:image-no-lander", vec![descriptor.clone()]);
    let attachment = InboundAttachment {
        id: descriptor.external_file_id,
        mime_type: "image/png".to_string(),
        filename: descriptor.filename,
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
            Err(ProductSurfaceFailure::AttachmentLanderUnavailable)
        ),
        "a missing lander must reject the turn (503, never settling the \
         idempotency reservation so a retry can succeed once wired), never \
         silently drop the attachment"
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
