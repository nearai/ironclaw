//! In-memory fakes used by contract tests and downstream adapter tests.
//!
//! These fakes are deliberately small. They DO NOT exercise the kernel,
//! TurnCoordinator, projection-stream service, or any production storage.
//! They exist solely to let adapter implementations validate their parse /
//! render contracts without wiring real Reborn infrastructure.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};

use crate::egress::{
    DeliveryStatus, EgressRequest, EgressResponse, OutboundDeliverySink, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use crate::error::ProductAdapterError;
use crate::external::ExternalEventId;
use crate::inbound::{ProductInboundAck, ProductInboundEnvelope, ProductRejection};
use crate::outbound::{ProductOutboundEnvelope, ProjectionCursor};
use crate::projection::{ProjectionStream, ProjectionSubscriptionRequest};
use crate::workflow::ProductWorkflow;

/// Fake `ProductWorkflow` whose acceptance behavior can be programmed per
/// `external_event_id`. Records every envelope it accepted.
pub struct FakeProductWorkflow {
    state: Mutex<FakeProductWorkflowState>,
}

#[derive(Default)]
struct FakeProductWorkflowState {
    /// Outcome to return for the next call with a given external_event_id.
    /// If the id is not in the map, default to a fresh `Accepted` outcome.
    programmed: HashMap<ExternalEventId, ProductInboundAck>,
    /// Per-event-id durable outcome cache used for dedupe semantics on the
    /// fake. The fake simulates the workflow returning a `Duplicate { prior }`
    /// on the second call with the same external_event_id within the same
    /// installation+adapter.
    outcomes_by_event: HashMap<EventDedupeKey, ProductInboundAck>,
    accepted_envelopes: Vec<ProductInboundEnvelope>,
    /// Optional override: if set, every call returns this outcome and skips
    /// dedupe/programmed lookup. Used for transient-error simulation.
    fail_with: Option<ProductAdapterError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EventDedupeKey {
    adapter_id: String,
    installation_id: String,
    event_id: String,
}

impl FakeProductWorkflow {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeProductWorkflowState::default()),
        }
    }

    /// Program the next outcome for an external_event_id. The first call with
    /// that id returns this outcome; subsequent calls with the same id return
    /// a `Duplicate { prior: <programmed outcome> }`.
    pub fn program_outcome(&self, event_id: ExternalEventId, outcome: ProductInboundAck) {
        let mut state = self.state.lock().expect("fake state lock poisoned");
        state.programmed.insert(event_id, outcome);
    }

    pub fn force_failure(&self, error: ProductAdapterError) {
        let mut state = self.state.lock().expect("fake state lock poisoned");
        state.fail_with = Some(error);
    }

    pub fn accepted_envelopes(&self) -> Vec<ProductInboundEnvelope> {
        let state = self.state.lock().expect("fake state lock poisoned");
        state.accepted_envelopes.clone()
    }

    pub fn accepted_count(&self) -> usize {
        self.accepted_envelopes().len()
    }
}

impl Default for FakeProductWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProductWorkflow for FakeProductWorkflow {
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
        let mut state = self.state.lock().expect("fake state lock poisoned");
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        let key = EventDedupeKey {
            adapter_id: envelope.adapter_id.as_str().to_string(),
            installation_id: envelope.installation_id.as_str().to_string(),
            event_id: envelope.external_event_id.as_str().to_string(),
        };
        if let Some(prior) = state.outcomes_by_event.get(&key).cloned() {
            return Ok(ProductInboundAck::Duplicate {
                prior: Box::new(prior),
            });
        }
        let outcome = state
            .programmed
            .remove(&envelope.external_event_id)
            .unwrap_or_else(|| ProductInboundAck::Accepted {
                accepted_message_ref: format!("msg:{}", envelope.external_event_id),
                submitted_run_id: Some(TurnRunId::new()),
            });
        state.outcomes_by_event.insert(key, outcome.clone());
        state.accepted_envelopes.push(envelope);
        Ok(outcome)
    }
}

/// Fake projection stream — emits envelopes that were `push`'d.
pub struct FakeProjectionStream {
    state: Mutex<Vec<ProductOutboundEnvelope>>,
}

impl FakeProjectionStream {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, envelope: ProductOutboundEnvelope) {
        let mut state = self.state.lock().expect("fake state lock poisoned");
        state.push(envelope);
    }
}

impl Default for FakeProjectionStream {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProjectionStream for FakeProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        let mut state = self.state.lock().expect("fake state lock poisoned");
        let drained = std::mem::take(&mut *state);
        Ok(drained)
    }
}

/// Fake delivery sink — records every status report.
pub struct FakeOutboundDeliverySink {
    statuses: Mutex<Vec<DeliveryStatus>>,
}

impl FakeOutboundDeliverySink {
    pub fn new() -> Self {
        Self {
            statuses: Mutex::new(Vec::new()),
        }
    }

    pub fn statuses(&self) -> Vec<DeliveryStatus> {
        self.statuses
            .lock()
            .expect("fake sink lock poisoned")
            .clone()
    }
}

impl Default for FakeOutboundDeliverySink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OutboundDeliverySink for FakeOutboundDeliverySink {
    async fn record(&self, status: DeliveryStatus) {
        self.statuses
            .lock()
            .expect("fake sink lock poisoned")
            .push(status);
    }
}

/// Records all egress calls and returns programmed responses or errors.
#[derive(Clone)]
pub struct RecordedEgressCall {
    pub host: String,
    pub method: String,
    pub path: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub credential_handle: Option<String>,
}

pub struct FakeProtocolHttpEgress {
    state: Mutex<FakeEgressState>,
}

#[derive(Default)]
struct FakeEgressState {
    declared_hosts: Vec<String>,
    valid_credential_handles: Vec<String>,
    recorded: Vec<RecordedEgressCall>,
    /// Programmed response per host. If none, returns 200 OK with empty body.
    programmed_responses: HashMap<String, Result<EgressResponse, ProtocolHttpEgressError>>,
}

impl FakeProtocolHttpEgress {
    pub fn new(declared_hosts: impl IntoIterator<Item = String>) -> Self {
        Self {
            state: Mutex::new(FakeEgressState {
                declared_hosts: declared_hosts.into_iter().collect(),
                ..Default::default()
            }),
        }
    }

    pub fn allow_credential_handle(&self, handle: impl Into<String>) {
        let mut state = self.state.lock().expect("fake egress lock poisoned");
        state.valid_credential_handles.push(handle.into());
    }

    pub fn program_response(
        &self,
        host: impl Into<String>,
        result: Result<EgressResponse, ProtocolHttpEgressError>,
    ) {
        let mut state = self.state.lock().expect("fake egress lock poisoned");
        state.programmed_responses.insert(host.into(), result);
    }

    pub fn calls(&self) -> Vec<RecordedEgressCall> {
        let state = self.state.lock().expect("fake egress lock poisoned");
        state.recorded.clone()
    }
}

#[async_trait]
impl ProtocolHttpEgress for FakeProtocolHttpEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        let mut state = self.state.lock().expect("fake egress lock poisoned");
        let host = request.host.as_str().to_string();
        if !state.declared_hosts.iter().any(|h| h == &host) {
            return Err(ProtocolHttpEgressError::UndeclaredHost { host });
        }
        if let Some(handle) = &request.credential_handle
            && !state
                .valid_credential_handles
                .iter()
                .any(|h| h == handle.as_str())
        {
            return Err(ProtocolHttpEgressError::UnknownCredentialHandle {
                handle: handle.as_str().to_string(),
            });
        }
        state.recorded.push(RecordedEgressCall {
            host: host.clone(),
            method: request.method.clone(),
            path: request.path.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            credential_handle: request
                .credential_handle
                .as_ref()
                .map(|h| h.as_str().to_string()),
        });
        if let Some(resp) = state.programmed_responses.remove(&host) {
            return resp;
        }
        Ok(EgressResponse {
            status: 200,
            headers: std::collections::BTreeMap::new(),
            body: br#"{"ok":true}"#.to_vec(),
        })
    }
}

/// Convenience helper used by adapter tests to assert that no `submit_turn`
/// went through the workflow when the adapter chose `NoOp`.
pub fn ensure_durable_outcome(ack: &ProductInboundAck) -> bool {
    ack.is_durable_outcome()
}

/// Assertion helper: verify that a fake workflow saw no envelopes containing
/// raw bytes for attachments. This is enforced by the type system today —
/// `ProductAttachmentDescriptor` has no bytes field — but the helper makes the
/// check explicit in tests as a regression guard if the type ever grows one.
pub fn assert_no_raw_attachment_bytes(envelopes: &[ProductInboundEnvelope]) {
    for envelope in envelopes {
        if let crate::inbound::ProductInboundPayload::UserMessage(payload) = &envelope.payload {
            for attachment in &payload.attachments {
                let json = serde_json::to_value(attachment).expect("serialize");
                let object = json.as_object().expect("attachment object");
                assert!(
                    !object.contains_key("data"),
                    "attachment must not carry raw bytes"
                );
                assert!(
                    !object.contains_key("source_url"),
                    "attachment must not carry source_url"
                );
                assert!(
                    !object.contains_key("local_path"),
                    "attachment must not carry local_path"
                );
            }
        }
    }
}

/// Quick reply-target helper for tests.
pub fn fake_reply_target(suffix: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(format!("reply:fake-{suffix}")).expect("valid reply target")
}

/// Quick projection cursor helper for tests.
pub fn fake_projection_cursor(suffix: &str) -> ProjectionCursor {
    ProjectionCursor::new(format!("cursor:fake-{suffix}"))
}

/// Quick rejection helper for tests.
pub fn fake_rejection(
    kind: crate::inbound::ProductRejectionKind,
    reason: &str,
) -> ProductRejection {
    ProductRejection {
        kind,
        reason: reason.to_string(),
    }
}
