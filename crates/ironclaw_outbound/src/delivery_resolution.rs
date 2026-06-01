use crate::ids::{TriggerFireSlot, TriggerOriginRef};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};
use serde::{Deserialize, Serialize};

/// Delivery resolution target categories used by the outbound resolver.
///
/// Translation note: these domain kinds lower into the existing
/// `OutboundPushKind`/`PrepareOutboundDeliveryRequest` path at the outbound
/// policy boundary. `ApprovalPrompt` and `AuthPrompt` stay on the
/// run-notification side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationDeliveryKind {
    FinalReply,
    ProgressUpdate,
    DeliveryStatus,
    ApprovalPrompt,
    AuthPrompt,
}

/// Narrow intent for explicitly requested outbound delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedOutboundKind {
    ProductMessage,
    DeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationDeliveryResolutionRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub modality: CommunicationModality,
    pub intent: CommunicationDeliveryIntent,
}

impl CommunicationDeliveryResolutionRequest {
    pub fn delivery_kind(&self) -> CommunicationDeliveryKind {
        self.intent.delivery_kind()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationDeliveryIntent {
    RequestedOutbound(RequestedOutboundContext),
    RunNotification(RunNotificationContext),
}

impl CommunicationDeliveryIntent {
    pub fn delivery_kind(&self) -> CommunicationDeliveryKind {
        match self {
            Self::RequestedOutbound(context) => context.delivery_kind(),
            Self::RunNotification(context) => context.delivery_kind(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedOutboundContext {
    pub requested_target: ReplyTargetBindingRef,
    pub requested_kind: RequestedOutboundKind,
}

impl RequestedOutboundContext {
    pub fn delivery_kind(&self) -> CommunicationDeliveryKind {
        match self.requested_kind {
            RequestedOutboundKind::ProductMessage => CommunicationDeliveryKind::FinalReply,
            RequestedOutboundKind::DeliveryStatus => CommunicationDeliveryKind::DeliveryStatus,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNotificationContext {
    pub event_kind: RunNotificationEventKind,
    pub origin: RunNotificationOrigin,
}

impl RunNotificationContext {
    pub fn delivery_kind(&self) -> CommunicationDeliveryKind {
        self.event_kind.delivery_kind()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemEventReasonCode {
    Generic,
    Trigger,
    Tool,
    Operator,
}

impl SystemEventReasonCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Trigger => "trigger",
            Self::Tool => "tool",
            Self::Operator => "operator",
        }
    }
}

impl std::fmt::Display for SystemEventReasonCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunNotificationEventKind {
    FinalReplyReady,
    ProgressUpdate,
    ApprovalNeeded,
    AuthRequired,
    RunBlocked,
    DeliveryStatus,
}

impl RunNotificationEventKind {
    pub fn delivery_kind(self) -> CommunicationDeliveryKind {
        match self {
            Self::FinalReplyReady => CommunicationDeliveryKind::FinalReply,
            Self::ProgressUpdate => CommunicationDeliveryKind::ProgressUpdate,
            Self::ApprovalNeeded | Self::RunBlocked => CommunicationDeliveryKind::ApprovalPrompt,
            Self::AuthRequired => CommunicationDeliveryKind::AuthPrompt,
            Self::DeliveryStatus => CommunicationDeliveryKind::DeliveryStatus,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunNotificationOrigin {
    LiveSourceRoute {
        source_route: SourceRouteContext,
    },
    Triggered {
        trigger: TriggerCommunicationContext,
    },
    TriggeredFromSourceRoute {
        trigger: TriggerCommunicationContext,
        source_route: SourceRouteContext,
    },
    SystemEvent {
        reason: SystemEventReasonCode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRouteContext {
    /// Canonical outbound target binding for the source route.
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

/// Non-canonical trigger-origin reference used only inside outbound
/// notification context.
///
/// The canonical trigger identity belongs in `ironclaw_triggers::TriggerId`
/// in PR 9, once the trigger crate exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCommunicationContext {
    pub trigger_origin_ref: TriggerOriginRef,
    pub trigger_source_kind: TriggerSourceKind,
    pub fire_slot: TriggerFireSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSourceKind {
    Schedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationModality {
    Text,
    Voice,
    Image,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeliveryTargetCapabilities {
    pub final_replies: bool,
    pub progress: bool,
    pub gate_prompts: bool,
    pub auth_prompts: bool,
    pub modalities: Vec<CommunicationModality>,
}

/// Candidate produced by the outbound resolution step.
///
/// The candidate is still only a target choice. It lowers into the existing
/// `OutboundPushCandidate` / `PrepareOutboundDeliveryRequest` boundary, where
/// target validation and delivery-attempt recording still live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationDeliveryCandidate {
    pub target: ReplyTargetBindingRef,
    pub kind: CommunicationDeliveryKind,
}

/// Result of resolving a communication request.
///
/// Most intents produce a concrete delivery candidate. Host/system events are
/// metadata-only in P0 unless a caller explicitly requested outbound delivery,
/// so they resolve to `NoDelivery` rather than being treated as invalid input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CommunicationDeliveryResolution {
    Candidate {
        candidate: CommunicationDeliveryCandidate,
    },
    NoDelivery {
        reason: SystemEventReasonCode,
    },
}

impl CommunicationDeliveryResolution {
    pub fn candidate(candidate: CommunicationDeliveryCandidate) -> Self {
        Self::Candidate { candidate }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use serde_json::{from_str, to_string};

    #[test]
    fn communication_delivery_resolution_request_round_trips_requested_outbound() {
        let request = CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor(),
            modality: CommunicationModality::Mixed,
            intent: CommunicationDeliveryIntent::RequestedOutbound(RequestedOutboundContext {
                requested_target: reply_ref("reply:requested"),
                requested_kind: RequestedOutboundKind::ProductMessage,
            }),
        };

        let json = to_string(&request).expect("serialize requested outbound request");
        let decoded: CommunicationDeliveryResolutionRequest =
            from_str(&json).expect("deserialize requested outbound request");
        assert_eq!(decoded, request);
        assert_eq!(
            decoded.delivery_kind(),
            CommunicationDeliveryKind::FinalReply
        );
    }

    #[test]
    fn communication_delivery_resolution_request_round_trips_run_notification() {
        let request = CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::RunBlocked,
                origin: RunNotificationOrigin::TriggeredFromSourceRoute {
                    trigger: trigger_context(),
                    source_route: source_route_context(),
                },
            }),
        };

        let json = to_string(&request).expect("serialize run notification request");
        let decoded: CommunicationDeliveryResolutionRequest =
            from_str(&json).expect("deserialize run notification request");
        assert_eq!(decoded, request);
        assert_eq!(
            decoded.delivery_kind(),
            CommunicationDeliveryKind::ApprovalPrompt
        );
    }

    #[test]
    fn run_notification_origin_round_trips_live_source_route() {
        assert_json_round_trip(RunNotificationOrigin::LiveSourceRoute {
            source_route: source_route_context(),
        });
    }

    #[test]
    fn run_notification_origin_round_trips_triggered() {
        assert_json_round_trip(RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        });
    }

    #[test]
    fn run_notification_origin_round_trips_triggered_from_source_route() {
        assert_json_round_trip(RunNotificationOrigin::TriggeredFromSourceRoute {
            trigger: trigger_context(),
            source_route: source_route_context(),
        });
    }

    #[test]
    fn run_notification_origin_round_trips_system_event() {
        assert_json_round_trip(RunNotificationOrigin::SystemEvent {
            reason: SystemEventReasonCode::Generic,
        });
    }

    #[test]
    fn run_notification_event_kind_delivery_kind_maps_all_variants() {
        assert_eq!(
            RunNotificationEventKind::FinalReplyReady.delivery_kind(),
            CommunicationDeliveryKind::FinalReply
        );
        assert_eq!(
            RunNotificationEventKind::ProgressUpdate.delivery_kind(),
            CommunicationDeliveryKind::ProgressUpdate
        );
        assert_eq!(
            RunNotificationEventKind::ApprovalNeeded.delivery_kind(),
            CommunicationDeliveryKind::ApprovalPrompt
        );
        assert_eq!(
            RunNotificationEventKind::AuthRequired.delivery_kind(),
            CommunicationDeliveryKind::AuthPrompt
        );
        assert_eq!(
            RunNotificationEventKind::RunBlocked.delivery_kind(),
            CommunicationDeliveryKind::ApprovalPrompt
        );
        assert_eq!(
            RunNotificationEventKind::DeliveryStatus.delivery_kind(),
            CommunicationDeliveryKind::DeliveryStatus
        );
    }

    #[test]
    fn outbound_translation_enums_round_trip_all_variants() {
        for value in [
            CommunicationDeliveryKind::FinalReply,
            CommunicationDeliveryKind::ProgressUpdate,
            CommunicationDeliveryKind::DeliveryStatus,
            CommunicationDeliveryKind::ApprovalPrompt,
            CommunicationDeliveryKind::AuthPrompt,
        ] {
            assert_json_round_trip(value);
        }

        for value in [
            RequestedOutboundKind::ProductMessage,
            RequestedOutboundKind::DeliveryStatus,
        ] {
            assert_json_round_trip(value);
        }

        for value in [
            RunNotificationEventKind::FinalReplyReady,
            RunNotificationEventKind::ProgressUpdate,
            RunNotificationEventKind::ApprovalNeeded,
            RunNotificationEventKind::AuthRequired,
            RunNotificationEventKind::RunBlocked,
            RunNotificationEventKind::DeliveryStatus,
        ] {
            assert_json_round_trip(value);
        }

        for value in [
            CommunicationModality::Text,
            CommunicationModality::Voice,
            CommunicationModality::Image,
            CommunicationModality::Mixed,
            CommunicationModality::Unknown,
        ] {
            assert_json_round_trip(value);
        }

        assert_json_round_trip(TriggerSourceKind::Schedule);

        for value in [
            SystemEventReasonCode::Generic,
            SystemEventReasonCode::Trigger,
            SystemEventReasonCode::Tool,
            SystemEventReasonCode::Operator,
        ] {
            assert_json_round_trip(value);
        }
    }

    #[test]
    fn communication_delivery_candidate_round_trips() {
        let candidate = CommunicationDeliveryCandidate {
            target: reply_ref("reply:candidate"),
            kind: CommunicationDeliveryKind::DeliveryStatus,
        };

        let json = to_string(&candidate).expect("serialize delivery candidate");
        let decoded: CommunicationDeliveryCandidate =
            from_str(&json).expect("deserialize delivery candidate");
        assert_eq!(decoded, candidate);
    }

    #[test]
    fn communication_delivery_resolution_serializes_all_variants() {
        assert_json_serializes(CommunicationDeliveryResolution::candidate(
            CommunicationDeliveryCandidate {
                target: reply_ref("reply:candidate"),
                kind: CommunicationDeliveryKind::FinalReply,
            },
        ));
        assert_json_serializes(CommunicationDeliveryResolution::NoDelivery {
            reason: SystemEventReasonCode::Operator,
        });
    }

    #[test]
    fn delivery_target_capabilities_round_trip() {
        let capabilities = DeliveryTargetCapabilities {
            final_replies: true,
            progress: true,
            gate_prompts: false,
            auth_prompts: true,
            modalities: vec![CommunicationModality::Text, CommunicationModality::Mixed],
        };

        let json = to_string(&capabilities).expect("serialize capabilities");
        let decoded: DeliveryTargetCapabilities =
            from_str(&json).expect("deserialize capabilities");
        assert_eq!(decoded, capabilities);
    }

    #[test]
    fn delivery_target_capabilities_default_is_all_false_and_empty_modalities() {
        let capabilities = DeliveryTargetCapabilities::default();

        assert!(!capabilities.final_replies);
        assert!(!capabilities.progress);
        assert!(!capabilities.gate_prompts);
        assert!(!capabilities.auth_prompts);
        assert!(capabilities.modalities.is_empty());
    }

    #[test]
    fn system_event_reason_code_rejects_unknown_variants() {
        assert_json_round_trip(SystemEventReasonCode::Generic);
        assert!(from_str::<SystemEventReasonCode>("\"backend_failure\"").is_err());
    }

    fn scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
        )
    }

    fn actor() -> TurnActor {
        TurnActor::new(UserId::new("user-a").expect("valid user"))
    }

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::new(value).expect("valid thread")
    }

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("valid reply target")
    }

    fn source_route_context() -> SourceRouteContext {
        SourceRouteContext {
            reply_target_binding_ref: reply_ref("reply:source-route"),
        }
    }

    fn trigger_context() -> TriggerCommunicationContext {
        TriggerCommunicationContext {
            trigger_origin_ref: TriggerOriginRef::new("trigger:daily")
                .expect("valid trigger origin ref"),
            trigger_source_kind: TriggerSourceKind::Schedule,
            fire_slot: TriggerFireSlot::new("2026-05-29T09:00:00Z").expect("valid fire slot"),
        }
    }

    fn assert_json_round_trip<T>(value: T)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = to_string(&value).expect("serialize value");
        let decoded: T = from_str(&json).expect("deserialize value");
        assert_eq!(decoded, value);
    }

    fn assert_json_serializes<T>(value: T)
    where
        T: Serialize + std::fmt::Debug,
    {
        to_string(&value).expect("serialize value");
    }
}
