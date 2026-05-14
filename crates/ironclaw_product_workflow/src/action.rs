//! Durable inbound action ledger for idempotent product workflow dispatch.
//!
//! A [`ProductInboundAction`] represents a single mutating action accepted by the
//! workflow facade. It is keyed by tenant + installation + external event fingerprint
//! so that retried/duplicated webhook deliveries are idempotent.

use chrono::{DateTime, Utc};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalEventId, ProductAdapterId, ProductInboundAck,
    ProductInboundPayload, ProductRejectionKind,
};
use ironclaw_turns::{LoopGateRef, TurnRunId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ProductWorkflowError;

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

const SOURCE_BINDING_KEY_MAX_BYTES: usize = 2_048;
const PRODUCT_COMMAND_NAME_MAX_BYTES: usize = 256;
const INTERACTION_REF_MAX_BYTES: usize = 512;

fn validate_typed_token(kind: &'static str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{kind} exceeds {max_bytes}-byte limit"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} contains unsupported control characters"));
    }
    Ok(())
}

macro_rules! typed_token {
    ($name:ident, $kind:literal, $max_bytes:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_typed_token($kind, &value, $max_bytes)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

typed_token!(
    SourceBindingKey,
    "source binding key",
    SOURCE_BINDING_KEY_MAX_BYTES
);
typed_token!(
    ProductCommandName,
    "product command name",
    PRODUCT_COMMAND_NAME_MAX_BYTES
);
typed_token!(
    AuthRequestRef,
    "auth request ref",
    INTERACTION_REF_MAX_BYTES
);
typed_token!(
    LinkedThreadActionId,
    "linked thread action id",
    INTERACTION_REF_MAX_BYTES
);

/// Composite deduplication key for inbound actions. Two envelopes with the same
/// fingerprint are considered duplicates and the second will replay the first
/// outcome.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionFingerprintKey {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub source_binding_key: SourceBindingKey,
    pub external_event_id: ExternalEventId,
}

impl ActionFingerprintKey {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        source_binding_key: SourceBindingKey,
        external_event_id: ExternalEventId,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            source_binding_key,
            external_event_id,
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
    Command { command: ProductCommandName },
    ApprovalResolution { gate_ref: LoopGateRef },
    AuthResolution { auth_request_ref: AuthRequestRef },
    ProjectionSubscription,
    LinkedThreadAction { action_id: LinkedThreadActionId },
    Rejected { kind: ProductRejectionKind },
    NoOp,
}

impl ActionDispatchKind {
    /// Derive the dispatch kind from a product inbound payload while preserving
    /// typed internal identifiers after boundary validation.
    pub fn try_from_payload(payload: &ProductInboundPayload) -> Result<Self, ProductWorkflowError> {
        match payload {
            ProductInboundPayload::UserMessage(_) => Ok(Self::UserMessageTurn {
                run_id: TurnRunId::new(),
            }),
            ProductInboundPayload::Command(cmd) => Ok(Self::Command {
                command: ProductCommandName::new(cmd.command.clone())
                    .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::ApprovalResolution(res) => Ok(Self::ApprovalResolution {
                gate_ref: LoopGateRef::new(res.gate_ref.clone())
                    .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::AuthResolution(res) => Ok(Self::AuthResolution {
                auth_request_ref: AuthRequestRef::new(res.auth_request_ref.clone())
                    .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?,
            }),
            ProductInboundPayload::SubscriptionRequest(_) => Ok(Self::ProjectionSubscription),
            ProductInboundPayload::LinkedThreadAction(lta) => Ok(Self::LinkedThreadAction {
                action_id: LinkedThreadActionId::new(lta.action_id.clone())
                    .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?,
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
