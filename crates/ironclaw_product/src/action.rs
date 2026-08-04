//! Durable inbound action ledger for idempotent product workflow dispatch.
//!
//! A [`ProductInboundAction`] represents a single mutating action accepted by the
//! workflow service. It is keyed by tenant + installation + external event fingerprint
//! so that retried/duplicated webhook deliveries are idempotent.

use crate::{ProductInboundAck, ProductInboundPayload, ProductRejectionKind};
use chrono::{DateTime, Utc};
use ironclaw_product_contracts::action::{
    ActionFingerprintKey, AuthRequestRef, LinkedThreadActionId, ProductActionId, ProductCommandName,
};
use ironclaw_turns::{LoopGateRef, TurnRunId};
use serde::{Deserialize, Serialize};

use crate::error::ProductSurfaceFailure;

/// Current phase of an inbound action saga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPhase {
    /// Action has been received and fingerprint reserved, but downstream
    /// dispatch has not started.
    Received,
    /// The action has been dispatched to the appropriate downstream service
    /// (turn coordinator, command router, etc.).
    Dispatched,
    /// A durable outcome has been recorded. The action is terminal.
    Settled,
    /// The action was a duplicate of an already-settled action.
    DeduplicatedReplay,
}

/// Which downstream path the action was routed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionDispatchKind {
    UserMessageTurn { run_id: TurnRunId },
    Command { command: ProductCommandName },
    ApprovalResolution { gate_ref: LoopGateRef },
    ScopedApprovalResolution,
    AuthResolution { auth_request_ref: AuthRequestRef },
    ProjectionRead,
    ProjectionSubscription,
    ControlAction,
    LinkedThreadAction { action_id: LinkedThreadActionId },
    Rejected { kind: ProductRejectionKind },
    NoOp,
}

impl ActionDispatchKind {
    /// Derive the dispatch kind from a product inbound payload while preserving
    /// typed internal identifiers after boundary validation.
    pub fn try_from_payload(
        payload: &ProductInboundPayload,
    ) -> Result<Self, ProductSurfaceFailure> {
        match payload {
            ProductInboundPayload::UserMessage(_) => Ok(Self::UserMessageTurn {
                run_id: TurnRunId::new(),
            }),
            ProductInboundPayload::Command(cmd) => Ok(Self::Command {
                command: ProductCommandName::new(cmd.command.clone())
                    .map_err(|reason| ProductSurfaceFailure::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::ApprovalResolution(res) => Ok(Self::ApprovalResolution {
                gate_ref: LoopGateRef::new(res.gate_ref.clone())
                    .map_err(|reason| ProductSurfaceFailure::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::ScopedApprovalResolution(_) => {
                Ok(Self::ScopedApprovalResolution)
            }
            ProductInboundPayload::AuthResolution(res) => Ok(Self::AuthResolution {
                auth_request_ref: AuthRequestRef::new(res.auth_request_ref.clone())
                    .map_err(|reason| ProductSurfaceFailure::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::ProjectionRead(_) => Ok(Self::ProjectionRead),
            ProductInboundPayload::SubscriptionRequest(_) => Ok(Self::ProjectionSubscription),
            ProductInboundPayload::ControlAction(_) => Ok(Self::ControlAction),
            ProductInboundPayload::LinkedThreadAction(lta) => Ok(Self::LinkedThreadAction {
                action_id: LinkedThreadActionId::new(lta.action_id.clone())
                    .map_err(|reason| ProductSurfaceFailure::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::NoOp => Ok(Self::NoOp),
        }
    }
}

/// Durable ledger record for a product inbound action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductInboundAction {
    pub action_id: ProductActionId,
    pub fingerprint: ActionFingerprintKey,
    pub phase: ActionPhase,
    pub dispatch_kind: Option<ActionDispatchKind>,
    pub outcome: Option<ProductInboundAck>,
    pub received_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}

impl ProductInboundAction {
    /// Create a new action record in the `Received` phase.
    pub fn begin(fingerprint: ActionFingerprintKey, received_at: DateTime<Utc>) -> Self {
        Self {
            action_id: ProductActionId::new(),
            fingerprint,
            phase: ActionPhase::Received,
            dispatch_kind: None,
            outcome: None,
            received_at,
            settled_at: None,
        }
    }

    /// Transition to `Dispatched` phase.
    pub fn mark_dispatched(&mut self, dispatch_kind: ActionDispatchKind) {
        self.phase = ActionPhase::Dispatched;
        self.dispatch_kind = Some(dispatch_kind);
    }

    /// Transition to `Settled` phase with a terminal outcome.
    pub fn settle(&mut self, outcome: ProductInboundAck) {
        self.phase = ActionPhase::Settled;
        self.outcome = Some(outcome);
        self.settled_at = Some(Utc::now());
    }

    /// Mark as a deduplicated replay of a prior settled action.
    pub fn mark_deduplicated(&mut self, prior_outcome: ProductInboundAck) {
        self.phase = ActionPhase::DeduplicatedReplay;
        self.outcome = Some(ProductInboundAck::Duplicate {
            prior: Box::new(prior_outcome),
        });
        self.settled_at = Some(Utc::now());
    }

    /// Whether this action has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            ActionPhase::Settled | ActionPhase::DeduplicatedReplay
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{ProductInboundAck, ProductRejection, ProductRejectionKind};
    use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalEventId};
    use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
    use ironclaw_product_contracts::action::SourceBindingKey;

    use super::*;

    fn fingerprint() -> ActionFingerprintKey {
        ActionFingerprintKey::new(
            ProductAdapterId::new("test_adapter").expect("valid adapter"),
            AdapterInstallationId::new("install_alpha").expect("valid installation"),
            ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor"),
            SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
                .expect("valid source binding"),
            ExternalEventId::new("evt:action").expect("valid event"),
        )
    }

    #[test]
    fn inbound_action_tracks_dispatch_settle_and_terminal_state() {
        let mut action = ProductInboundAction::begin(fingerprint(), Utc::now());
        assert_eq!(action.phase, ActionPhase::Received);
        assert!(!action.is_terminal());
        assert!(action.dispatch_kind.is_none());
        assert!(action.outcome.is_none());

        let run_id = TurnRunId::new();
        action.mark_dispatched(ActionDispatchKind::UserMessageTurn { run_id });
        assert_eq!(action.phase, ActionPhase::Dispatched);
        assert_eq!(
            action.dispatch_kind,
            Some(ActionDispatchKind::UserMessageTurn { run_id })
        );
        assert!(!action.is_terminal());

        action.settle(ProductInboundAck::NoOp);
        assert_eq!(action.phase, ActionPhase::Settled);
        assert_eq!(action.outcome, Some(ProductInboundAck::NoOp));
        assert!(action.settled_at.is_some());
        assert!(action.is_terminal());
    }

    #[test]
    fn inbound_action_marks_deduplicated_replay_with_prior_outcome() {
        let mut action = ProductInboundAction::begin(fingerprint(), Utc::now());
        let prior = ProductInboundAck::Rejected(ProductRejection::permanent(
            ProductRejectionKind::PolicyDenied,
            "already rejected",
        ));

        action.mark_deduplicated(prior.clone());

        assert_eq!(action.phase, ActionPhase::DeduplicatedReplay);
        assert_eq!(
            action.outcome,
            Some(ProductInboundAck::Duplicate {
                prior: Box::new(prior)
            })
        );
        assert!(action.settled_at.is_some());
        assert!(action.is_terminal());
    }
}
