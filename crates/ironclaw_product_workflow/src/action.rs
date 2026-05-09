//! Durable inbound action ledger for idempotent product workflow dispatch.
//!
//! A [`ProductInboundAction`] represents a single mutating action accepted by the
//! workflow facade. It is keyed by tenant + installation + external event fingerprint
//! so that retried/duplicated webhook deliveries are idempotent.

use chrono::{DateTime, Utc};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalEventId, ProductAdapterId, ProductInboundAck,
    ProductInboundPayload,
};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a product inbound action ledger entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProductActionId(Uuid);

impl ProductActionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ProductActionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ProductActionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Composite deduplication key for inbound actions. Two envelopes with the same
/// fingerprint are considered duplicates and the second will replay the first
/// outcome.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionFingerprintKey {
    pub adapter_id: String,
    pub installation_id: String,
    pub source_binding_key: String,
    pub external_event_id: String,
}

impl ActionFingerprintKey {
    pub fn new(
        adapter_id: &ProductAdapterId,
        installation_id: &AdapterInstallationId,
        source_binding_key: &str,
        external_event_id: &ExternalEventId,
    ) -> Self {
        Self {
            adapter_id: adapter_id.as_str().to_string(),
            installation_id: installation_id.as_str().to_string(),
            source_binding_key: source_binding_key.to_string(),
            external_event_id: external_event_id.as_str().to_string(),
        }
    }
}

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
    Command { command: String },
    ApprovalResolution { gate_ref: String },
    AuthResolution { auth_request_ref: String },
    ProjectionSubscription,
    LinkedThreadAction { action_id: String },
    NoOp,
}

impl ActionDispatchKind {
    /// Derive the dispatch kind from a product inbound payload.
    pub fn from_payload(payload: &ProductInboundPayload) -> Self {
        match payload {
            ProductInboundPayload::UserMessage(_) => Self::UserMessageTurn {
                run_id: TurnRunId::new(),
            },
            ProductInboundPayload::Command(cmd) => Self::Command {
                command: cmd.command.clone(),
            },
            ProductInboundPayload::ApprovalResolution(res) => Self::ApprovalResolution {
                gate_ref: res.gate_ref.clone(),
            },
            ProductInboundPayload::AuthResolution(res) => Self::AuthResolution {
                auth_request_ref: res.auth_request_ref.clone(),
            },
            ProductInboundPayload::SubscriptionRequest(_) => Self::ProjectionSubscription,
            ProductInboundPayload::LinkedThreadAction(lta) => Self::LinkedThreadAction {
                action_id: lta.action_id.clone(),
            },
            ProductInboundPayload::NoOp => Self::NoOp,
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
