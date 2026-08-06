use ironclaw_host_api::turn::ReplyTargetBindingRef;

use crate::{
    CommunicationDeliveryCandidate, CommunicationDeliveryIntent, CommunicationDeliveryResolution,
    CommunicationDeliveryResolutionRequest, OutboundError, RequestedOutboundContext,
    RunNotificationContext, RunNotificationOrigin,
};

/// Deterministic host-owned outbound target selection.
///
/// The engine only chooses a `CommunicationDeliveryCandidate`. It does not
/// validate the target, record attempts, or mutate any inbound / approval /
/// auth / transcript state.
///
/// Every run notification names its own target: a live run replies on its
/// source route, and a background run's notifier resolves the creator's
/// notification channels itself and passes each one as a
/// [`RunNotificationOrigin::RunScopedTarget`]. The engine reads no stored
/// preference at all — there are no per-purpose target slots any more.
pub(crate) struct OutboundResolutionEngine;

impl OutboundResolutionEngine {
    pub(crate) async fn resolve(
        &self,
        request: &CommunicationDeliveryResolutionRequest,
    ) -> Result<CommunicationDeliveryResolution, OutboundError> {
        let CommunicationDeliveryResolutionRequest {
            scope: _,
            actor: _,
            modality: _,
            intent,
        } = request;
        match intent {
            CommunicationDeliveryIntent::RequestedOutbound(context) => {
                Ok(CommunicationDeliveryResolution::candidate(
                    self.candidate_from_requested_outbound(context),
                ))
            }
            CommunicationDeliveryIntent::RunNotification(context) => {
                self.resolve_run_notification_context(context).await
            }
        }
    }

    fn candidate_from_requested_outbound(
        &self,
        context: &RequestedOutboundContext,
    ) -> CommunicationDeliveryCandidate {
        let kind = context.delivery_kind();
        CommunicationDeliveryCandidate {
            target: context.requested_target.clone(),
            kind,
        }
    }

    async fn resolve_run_notification_context(
        &self,
        context: &RunNotificationContext,
    ) -> Result<CommunicationDeliveryResolution, OutboundError> {
        let kind = context.delivery_kind();
        let target: ReplyTargetBindingRef = match &context.origin {
            RunNotificationOrigin::LiveSourceRoute { source_route } => {
                source_route.reply_target_binding_ref.clone()
            }
            RunNotificationOrigin::RunScopedTarget { target } => target.clone(),
            RunNotificationOrigin::SystemEvent { reason } => {
                return Ok(CommunicationDeliveryResolution::NoDelivery { reason: *reason });
            }
        };

        Ok(CommunicationDeliveryResolution::candidate(
            CommunicationDeliveryCandidate { target, kind },
        ))
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnScope};

    use super::*;
    use crate::{
        CommunicationDeliveryKind, CommunicationModality, RequestedOutboundKind,
        RunNotificationEventKind, SourceRouteContext, SystemEventReasonCode,
    };

    #[tokio::test]
    async fn requested_outbound_prefers_the_explicit_target() {
        let engine = OutboundResolutionEngine;
        let request = requested_request("reply:requested");

        let candidate = engine
            .resolve(&request)
            .await
            .expect("requested outbound resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:requested"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn requested_outbound_delivery_status_preserves_the_explicit_target() {
        let engine = OutboundResolutionEngine;
        let request = requested_request_with_kind(
            "reply:delivery-status",
            RequestedOutboundKind::DeliveryStatus,
        );

        let candidate = engine
            .resolve(&request)
            .await
            .expect("requested delivery status resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:delivery-status"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::DeliveryStatus);
    }

    #[tokio::test]
    async fn live_source_route_final_reply_uses_the_source_route() {
        let engine = OutboundResolutionEngine;

        let candidate = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::FinalReplyReady,
                RunNotificationOrigin::LiveSourceRoute {
                    source_route: SourceRouteContext {
                        reply_target_binding_ref: reply_ref("reply:source-route"),
                    },
                },
            ))
            .await
            .expect("live source route final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:source-route"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn live_source_route_approval_needed_uses_the_source_route() {
        let engine = OutboundResolutionEngine;

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::ApprovalNeeded,
            RunNotificationOrigin::LiveSourceRoute {
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:source-route",
            CommunicationDeliveryKind::ApprovalPrompt,
        )
        .await;
    }

    #[tokio::test]
    async fn live_source_route_auth_required_uses_the_source_route() {
        let engine = OutboundResolutionEngine;

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::AuthRequired,
            RunNotificationOrigin::LiveSourceRoute {
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:source-route",
            CommunicationDeliveryKind::AuthPrompt,
        )
        .await;
    }

    #[tokio::test]
    async fn system_event_notifications_are_metadata_only_without_candidate() {
        let engine = OutboundResolutionEngine;

        let resolution = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::ProgressUpdate,
                RunNotificationOrigin::SystemEvent {
                    reason: SystemEventReasonCode::Operator,
                },
            ))
            .await
            .expect("system event resolves as metadata-only");

        assert_eq!(
            resolution,
            CommunicationDeliveryResolution::NoDelivery {
                reason: SystemEventReasonCode::Operator
            }
        );
    }

    /// The surviving per-target origin: a run-scoped binding is used verbatim,
    /// with no preference lookup at all. Every background-run notification and
    /// every explicit model delivery resolves through this arm.
    #[tokio::test]
    async fn run_scoped_target_uses_the_sealed_binding_for_every_event_kind() {
        let engine = OutboundResolutionEngine;

        for (event_kind, expected_kind) in [
            (
                RunNotificationEventKind::ApprovalNeeded,
                CommunicationDeliveryKind::ApprovalPrompt,
            ),
            (
                RunNotificationEventKind::AuthRequired,
                CommunicationDeliveryKind::AuthPrompt,
            ),
            (
                RunNotificationEventKind::RunBlocked,
                CommunicationDeliveryKind::ApprovalPrompt,
            ),
            (
                RunNotificationEventKind::ModelDelivery,
                CommunicationDeliveryKind::ModelDelivery,
            ),
        ] {
            let candidate = engine
                .resolve(&run_notification_request(
                    event_kind,
                    RunNotificationOrigin::RunScopedTarget {
                        target: reply_ref("reply:run-scoped"),
                    },
                ))
                .await
                .expect("run-scoped notification resolves");
            let candidate = expect_candidate(candidate);
            assert_eq!(candidate.target, reply_ref("reply:run-scoped"));
            assert_eq!(candidate.kind, expected_kind);
        }
    }

    fn requested_request(requested_target: &str) -> CommunicationDeliveryResolutionRequest {
        requested_request_with_kind(requested_target, RequestedOutboundKind::ProductMessage)
    }

    fn requested_request_with_kind(
        requested_target: &str,
        requested_kind: RequestedOutboundKind,
    ) -> CommunicationDeliveryResolutionRequest {
        CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor("user-a"),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RequestedOutbound(RequestedOutboundContext {
                requested_target: reply_ref(requested_target),
                requested_kind,
            }),
        }
    }

    fn run_notification_request(
        event_kind: RunNotificationEventKind,
        origin: RunNotificationOrigin,
    ) -> CommunicationDeliveryResolutionRequest {
        CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor("user-a"),
            modality: CommunicationModality::Mixed,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind,
                origin,
            }),
        }
    }

    fn scope() -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
            Some(UserId::new("user-a").expect("valid user")),
        )
    }

    fn actor(user: &str) -> TurnActor {
        TurnActor::new(UserId::new(user).expect("valid user"))
    }

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::new(value).expect("valid thread")
    }

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("valid reply target")
    }

    fn expect_candidate(
        resolution: CommunicationDeliveryResolution,
    ) -> CommunicationDeliveryCandidate {
        match resolution {
            CommunicationDeliveryResolution::Candidate { candidate } => candidate,
            CommunicationDeliveryResolution::NoDelivery { reason } => {
                panic!("expected candidate, got metadata-only resolution for {reason}")
            }
        }
    }

    async fn assert_resolves_to(
        engine: &OutboundResolutionEngine,
        event_kind: RunNotificationEventKind,
        origin: RunNotificationOrigin,
        expected_target: &str,
        expected_kind: CommunicationDeliveryKind,
    ) {
        let candidate = engine
            .resolve(&run_notification_request(event_kind, origin))
            .await
            .expect("run notification resolves");
        let candidate = expect_candidate(candidate);
        assert_eq!(candidate.target, reply_ref(expected_target));
        assert_eq!(candidate.kind, expected_kind);
    }
}
