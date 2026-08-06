use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    dispatch::{
        CapabilityDisplayOutputPreview, DispatchInputIssue, DispatchInputIssueCode,
        RuntimeDispatchErrorKind,
    },
    error::HostApiError,
    ids::CapabilityId,
    resource::ResourceUsage,
};
use ironclaw_outbound::{
    DeliveryFailureKind, ModelChannelDelivery, ModelChannelDeliveryError,
    ModelChannelDeliveryRequest, OutboundDeliveryTargetId,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{first_party_capability_manifest, resource_profile};

pub const OUTBOUND_DELIVER_CAPABILITY_ID: &str = "builtin.outbound_deliver";

const DESCRIPTION: &str = "Deliver content to one connected channel destination from the assistant (bot) identity. Use this when the user wants content on another surface (\"send me X on my other channel\", a routine prompt's delivery step, a registered shared channel). target_id must be an exact id from builtin__outbound_delivery_targets_list; call once per destination. The result carries provider message references as delivery evidence — report outcomes honestly from it. Your final reply still lands in this conversation automatically: never deliver to the conversation you are replying in, and never use an integration send-message tool (which acts as the user) to deliver your own output.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundDeliverInput {
    target_id: OutboundDeliveryTargetId,
    content: String,
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        OUTBOUND_DELIVER_CAPABILITY_ID,
        DESCRIPTION,
        vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    delivery: Arc<dyn ModelChannelDelivery>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(OUTBOUND_DELIVER_CAPABILITY_ID)?,
        Arc::new(OutboundDeliverHandler { delivery }),
    );
    Ok(())
}

struct OutboundDeliverHandler {
    delivery: Arc<dyn ModelChannelDelivery>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for OutboundDeliverHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let input: OutboundDeliverInput =
            serde_json::from_value(request.input).map_err(|_| super::input_error())?;
        let run_id = request.run_id.ok_or_else(|| {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "explicit channel delivery requires an active run",
            )
        })?;
        let actor = request.authenticated_actor_user_id.ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
        })?;
        let evidence = self
            .delivery
            .deliver_for_model(ModelChannelDeliveryRequest {
                scope: request.scope,
                run_id,
                authenticated_actor_user_id: actor,
                target_id: input.target_id,
                content: input.content,
            })
            .await
            .map_err(map_delivery_error)?;
        let display_name = evidence.target.display_name.to_string();
        Ok(FirstPartyCapabilityResult::new(
            json!({
                "delivered": true,
                "target_id": evidence.target.target_id.as_str(),
                "channel": evidence.target.channel.as_str(),
                "display_name": evidence.target.display_name.as_str(),
                "provider_message_refs": evidence.provider_message_refs,
                "durably_recorded": evidence.durably_recorded,
            }),
            ResourceUsage::default(),
        )
        .with_display_preview(Some(CapabilityDisplayOutputPreview {
            output_summary: Some(delivered_summary(&display_name, evidence.durably_recorded)),
            output_preview: delivered_summary(&display_name, evidence.durably_recorded),
            output_kind: "text".to_string(),
            subtitle: None,
            truncated: false,
        })))
    }
}

/// `durably_recorded: false` means the provider accepted the send but the
/// confirmation row did not commit — delivered, with the weaker claim shown.
fn delivered_summary(display_name: &str, durably_recorded: bool) -> String {
    if durably_recorded {
        format!("Delivered to {display_name}")
    } else {
        format!("Delivered to {display_name} (confirmation record pending)")
    }
}

fn map_delivery_error(error: ModelChannelDeliveryError) -> FirstPartyCapabilityError {
    match error {
        ModelChannelDeliveryError::TargetUnavailable => invalid_target_input(),
        ModelChannelDeliveryError::OriginConversationTarget => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::PolicyDenied,
                "target is this conversation - your reply already lands here",
            )
        }
        ModelChannelDeliveryError::DeliveryCapExceeded => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "the per-run outbound delivery cap was reached",
            )
        }
        ModelChannelDeliveryError::ContentTooLarge => FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::OperationFailed,
            "delivery content exceeds the allowed size",
        ),
        ModelChannelDeliveryError::Rejected => FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::OperationFailed,
            "outbound policy rejected this delivery",
        ),
        ModelChannelDeliveryError::Failed { kind } => FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::OperationFailed,
            delivery_failure_summary(kind),
        ),
        ModelChannelDeliveryError::AccessDenied => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
        }
        ModelChannelDeliveryError::Unavailable | ModelChannelDeliveryError::Internal => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
    }
}

/// Host-authored, per-kind safe summary for a delivered-but-failed attempt
/// (spec §5): the model-visible `Failed` result must carry the sanitized
/// `DeliveryFailureKind`, not a single fixed sentence that discards it. The
/// match is exhaustive over fixed literals — never `{kind:?}`/`Display`
/// interpolation of the enum itself — so every arm is reviewed and known to
/// satisfy `LoopSafeSummary` (no `/ < > [ ] { } \``), and adding a new
/// `DeliveryFailureKind` variant is a compile error here instead of a silent
/// stringly-typed leak.
fn delivery_failure_summary(kind: DeliveryFailureKind) -> &'static str {
    match kind {
        DeliveryFailureKind::AuthorizationRevoked => {
            "the delivery attempt failed: authorization_revoked"
        }
        DeliveryFailureKind::TransientValidatorError => {
            "the delivery attempt failed: transient_validator_error"
        }
        DeliveryFailureKind::TransportUnavailable => {
            "the delivery attempt failed: transport_unavailable"
        }
        DeliveryFailureKind::RateLimited => "the delivery attempt failed: rate_limited",
        DeliveryFailureKind::Rejected => "the delivery attempt failed: rejected",
        DeliveryFailureKind::Unknown => "the delivery attempt failed: unknown",
    }
}

fn invalid_target_input() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "outbound delivery target input failed validation",
        vec![
            DispatchInputIssue::new("target_id", DispatchInputIssueCode::InvalidValue).expected(
                "an exact outbound delivery target id returned by builtin__outbound_delivery_targets_list",
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        dispatch::DispatchFailureDetail,
        ids::{InvocationId, RunId, TenantId, ThreadId, UserId},
        resource::{ResourceEstimate, ResourceScope},
    };
    use ironclaw_outbound::{DeliveryFailureKind, ModelChannelDeliveryEvidence};
    use ironclaw_outbound::{OutboundDeliveryTargetId, OutboundDeliveryTargetSummary};
    use serde_json::{Value, json};

    use crate::{FirstPartyCapabilityRequest, HostProcessPort, InvocationServices};

    use super::*;

    enum FakeOutcome {
        Ok(ModelChannelDeliveryEvidence),
        Err(ModelChannelDeliveryError),
    }

    struct FakeDelivery {
        outcome: FakeOutcome,
        seen: Mutex<Vec<ModelChannelDeliveryRequest>>,
    }

    impl FakeDelivery {
        fn new(outcome: FakeOutcome) -> Self {
            Self {
                outcome,
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ModelChannelDelivery for FakeDelivery {
        async fn deliver_for_model(
            &self,
            request: ModelChannelDeliveryRequest,
        ) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError> {
            self.seen
                .lock()
                .expect("fake delivery lock should not be poisoned")
                .push(request);
            match &self.outcome {
                FakeOutcome::Ok(evidence) => Ok(evidence.clone()),
                FakeOutcome::Err(error) => Err(*error),
            }
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-outbound-deliver").unwrap(),
            user_id: UserId::new("user-outbound-deliver").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: Some(ThreadId::new("thread-outbound-deliver").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_target() -> OutboundDeliveryTargetSummary {
        OutboundDeliveryTargetSummary::new(
            OutboundDeliveryTargetId::new("slack:C123").unwrap(),
            "slack",
            "Team Updates",
            None,
        )
        .unwrap()
    }

    fn sample_request(
        input: Value,
        run_id: Option<RunId>,
        actor: Option<UserId>,
    ) -> FirstPartyCapabilityRequest {
        FirstPartyCapabilityRequest {
            capability_id: CapabilityId::new(OUTBOUND_DELIVER_CAPABILITY_ID).unwrap(),
            scope: sample_scope(),
            authenticated_actor_user_id: actor,
            run_id,
            origin: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            services: InvocationServices {
                filesystem: Arc::new(InMemoryBackend::new()),
                runtime_http_egress: None,
                tool_call_http_egress: None,
                runtime_secret_material_stager: None,
                process: Arc::new(HostProcessPort::new()),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
                post_edit_check: None,
            },
            input,
        }
    }

    fn actor() -> UserId {
        UserId::new("user-outbound-deliver").unwrap()
    }

    #[tokio::test]
    async fn success_builds_output_and_display_preview_from_port_evidence() {
        let evidence = ModelChannelDeliveryEvidence {
            target: sample_target(),
            provider_message_refs: vec!["1721.045".to_string()],
            durably_recorded: true,
        };
        let delivery = Arc::new(FakeDelivery::new(FakeOutcome::Ok(evidence)));
        let handler = OutboundDeliverHandler {
            delivery: delivery.clone(),
        };
        let run_id = RunId::new();
        let request = sample_request(
            json!({"target_id": "slack:C123", "content": "hello team"}),
            Some(run_id),
            Some(actor()),
        );

        let result = handler
            .dispatch(request)
            .await
            .expect("delivery should succeed");

        assert_eq!(result.output["delivered"], json!(true));
        assert_eq!(result.output["target_id"], json!("slack:C123"));
        assert_eq!(result.output["channel"], json!("slack"));
        assert_eq!(result.output["display_name"], json!("Team Updates"));
        assert_eq!(result.output["provider_message_refs"], json!(["1721.045"]));

        let preview = result
            .display_preview
            .expect("display preview should be set");
        assert_eq!(
            preview.output_summary.as_deref(),
            Some("Delivered to Team Updates")
        );
        assert!(!preview.truncated);

        let seen = delivery.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].run_id, run_id);
        assert_eq!(seen[0].authenticated_actor_user_id, actor());
        assert_eq!(seen[0].content, "hello team");
        assert_eq!(seen[0].target_id.as_str(), "slack:C123");
    }

    #[tokio::test]
    async fn missing_run_id_is_operation_failed_with_fixed_summary() {
        let delivery = Arc::new(FakeDelivery::new(FakeOutcome::Err(
            ModelChannelDeliveryError::Internal,
        )));
        let handler = OutboundDeliverHandler {
            delivery: delivery.clone(),
        };
        let request = sample_request(
            json!({"target_id": "slack:C123", "content": "hello"}),
            None,
            Some(actor()),
        );

        let error = handler
            .dispatch(request)
            .await
            .expect_err("missing run_id must fail");

        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::OperationFailed)
        );
        assert_eq!(
            error.safe_summary(),
            Some("explicit channel delivery requires an active run")
        );
        assert!(
            delivery.seen.lock().unwrap().is_empty(),
            "the port must not be called without an active run"
        );
    }

    #[tokio::test]
    async fn missing_actor_is_policy_denied() {
        let delivery = Arc::new(FakeDelivery::new(FakeOutcome::Err(
            ModelChannelDeliveryError::Internal,
        )));
        let handler = OutboundDeliverHandler {
            delivery: delivery.clone(),
        };
        let request = sample_request(
            json!({"target_id": "slack:C123", "content": "hello"}),
            Some(RunId::new()),
            None,
        );

        let error = handler
            .dispatch(request)
            .await
            .expect_err("missing actor must fail");

        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::PolicyDenied));
        assert!(
            delivery.seen.lock().unwrap().is_empty(),
            "the port must not be called without an authenticated actor"
        );
    }

    #[tokio::test]
    async fn rejects_unknown_field_before_port_dispatch() {
        let delivery = Arc::new(FakeDelivery::new(FakeOutcome::Err(
            ModelChannelDeliveryError::Internal,
        )));
        let handler = OutboundDeliverHandler {
            delivery: delivery.clone(),
        };
        let request = sample_request(
            json!({"target_id": "slack:C123", "content": "hello", "extra": true}),
            Some(RunId::new()),
            Some(actor()),
        );

        let error = handler.dispatch(request).await.unwrap_err();

        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
        assert!(
            delivery.seen.lock().unwrap().is_empty(),
            "rejected input must not reach the port"
        );
    }

    #[tokio::test]
    async fn maps_target_unavailable_to_invalid_target_id_issue() {
        let delivery = Arc::new(FakeDelivery::new(FakeOutcome::Err(
            ModelChannelDeliveryError::TargetUnavailable,
        )));
        let handler = OutboundDeliverHandler { delivery };
        let request = sample_request(
            json!({"target_id": "slack:C123", "content": "hello"}),
            Some(RunId::new()),
            Some(actor()),
        );

        let error = handler
            .dispatch(request)
            .await
            .expect_err("target unavailable must fail");

        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
        let FirstPartyCapabilityError::Dispatch { detail, .. } = &error else {
            panic!("expected Dispatch variant");
        };
        let Some(DispatchFailureDetail::InvalidInput { issues }) = detail.as_deref() else {
            panic!("expected InvalidInput detail, got {detail:?}");
        };
        assert!(issues.iter().any(|issue| issue.path == "target_id"));
    }

    #[tokio::test]
    async fn maps_remaining_port_errors_to_dispatch_kinds() {
        let cases: Vec<(
            ModelChannelDeliveryError,
            RuntimeDispatchErrorKind,
            Option<&str>,
        )> = vec![
            (
                ModelChannelDeliveryError::OriginConversationTarget,
                RuntimeDispatchErrorKind::PolicyDenied,
                Some("target is this conversation - your reply already lands here"),
            ),
            (
                ModelChannelDeliveryError::DeliveryCapExceeded,
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the per-run outbound delivery cap was reached"),
            ),
            (
                ModelChannelDeliveryError::ContentTooLarge,
                RuntimeDispatchErrorKind::OperationFailed,
                Some("delivery content exceeds the allowed size"),
            ),
            (
                ModelChannelDeliveryError::Rejected,
                RuntimeDispatchErrorKind::OperationFailed,
                Some("outbound policy rejected this delivery"),
            ),
            (
                ModelChannelDeliveryError::Failed {
                    kind: DeliveryFailureKind::Unknown,
                },
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the delivery attempt failed: unknown"),
            ),
            (
                ModelChannelDeliveryError::Failed {
                    kind: DeliveryFailureKind::Rejected,
                },
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the delivery attempt failed: rejected"),
            ),
            (
                ModelChannelDeliveryError::Failed {
                    kind: DeliveryFailureKind::TransportUnavailable,
                },
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the delivery attempt failed: transport_unavailable"),
            ),
            (
                ModelChannelDeliveryError::Failed {
                    kind: DeliveryFailureKind::AuthorizationRevoked,
                },
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the delivery attempt failed: authorization_revoked"),
            ),
            (
                ModelChannelDeliveryError::Failed {
                    kind: DeliveryFailureKind::TransientValidatorError,
                },
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the delivery attempt failed: transient_validator_error"),
            ),
            (
                ModelChannelDeliveryError::Failed {
                    kind: DeliveryFailureKind::RateLimited,
                },
                RuntimeDispatchErrorKind::OperationFailed,
                Some("the delivery attempt failed: rate_limited"),
            ),
            (
                ModelChannelDeliveryError::AccessDenied,
                RuntimeDispatchErrorKind::PolicyDenied,
                None,
            ),
            (
                ModelChannelDeliveryError::Unavailable,
                RuntimeDispatchErrorKind::Backend,
                None,
            ),
            (
                ModelChannelDeliveryError::Internal,
                RuntimeDispatchErrorKind::Backend,
                None,
            ),
        ];

        for (port_error, expected_kind, expected_summary) in cases {
            let delivery = Arc::new(FakeDelivery::new(FakeOutcome::Err(port_error)));
            let handler = OutboundDeliverHandler { delivery };
            let request = sample_request(
                json!({"target_id": "slack:C123", "content": "hello"}),
                Some(RunId::new()),
                Some(actor()),
            );

            let error = handler.dispatch(request).await.unwrap_err();

            assert_eq!(error.kind(), Some(expected_kind), "{port_error:?}");
            if let Some(summary) = expected_summary {
                assert_eq!(error.safe_summary(), Some(summary), "{port_error:?}");
            }
        }
    }
}
