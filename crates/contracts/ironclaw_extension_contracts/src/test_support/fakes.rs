//! In-memory fakes for the extension-tier egress and delivery ports, used by
//! contract tests and downstream adapter tests.
//!
//! The product-tier half of the original module (projection stream, inbound
//! ack/rejection helpers) moved with its DTOs to
//! `ironclaw_product_contracts::test_support::fakes`; a fake belongs beside the
//! port it implements.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::product_adapter_error::ProtocolHttpEgressError;
use ironclaw_host_api::turn::ReplyTargetBindingRef;

use crate::egress::{
    DeliveryAttemptId, DeliveryStatus, EgressHeader, EgressRequest, EgressResponse,
    OutboundDeliverySink, ProtocolHttpEgress,
};

pub struct FakeOutboundDeliverySink {
    statuses: Mutex<FakeDeliveryState>,
}

#[derive(Default)]
struct FakeDeliveryState {
    order: Vec<DeliveryAttemptId>,
    by_attempt: HashMap<DeliveryAttemptId, DeliveryStatus>,
}

impl FakeOutboundDeliverySink {
    pub fn new() -> Self {
        Self {
            statuses: Mutex::new(FakeDeliveryState::default()),
        }
    }

    pub fn statuses(&self) -> Vec<DeliveryStatus> {
        let state = self.statuses.lock().expect("fake sink lock poisoned"); // safety: test-support fake sink; poisoned mutex means another test already panicked;
        state
            .order
            .iter()
            .filter_map(|attempt| state.by_attempt.get(attempt).cloned())
            .collect()
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
        let mut state = self.statuses.lock().expect("fake sink lock poisoned"); // safety: test-support fake sink; poisoned mutex means another test already panicked;
        let attempt_id = status.attempt_id();
        if !state.by_attempt.contains_key(&attempt_id) {
            state.order.push(attempt_id);
        }
        state.by_attempt.insert(attempt_id, status);
    }
}

#[derive(Clone)]
pub struct RecordedEgressCall {
    pub host: String,
    pub method: String,
    pub path: String,
    pub headers: Vec<EgressHeader>,
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
    programmed_responses:
        HashMap<String, VecDeque<Result<EgressResponse, ProtocolHttpEgressError>>>,
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
        let mut state = self.state.lock().expect("fake egress lock poisoned"); // safety: test-support fake egress; poisoned mutex means another test already panicked;
        state.valid_credential_handles.push(handle.into());
    }

    pub fn program_response(
        &self,
        host: impl Into<String>,
        result: Result<EgressResponse, ProtocolHttpEgressError>,
    ) {
        let mut state = self.state.lock().expect("fake egress lock poisoned"); // safety: test-support fake egress; poisoned mutex means another test already panicked;
        state
            .programmed_responses
            .entry(host.into())
            .or_default()
            .push_back(result);
    }

    pub fn calls(&self) -> Vec<RecordedEgressCall> {
        let state = self.state.lock().expect("fake egress lock poisoned"); // safety: test-support fake egress; poisoned mutex means another test already panicked;
        state.recorded.clone()
    }
}

#[async_trait]
impl ProtocolHttpEgress for FakeProtocolHttpEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        let mut state = self.state.lock().expect("fake egress lock poisoned"); // safety: test-support fake egress; poisoned mutex means another test already panicked;
        let host = request.host().as_str().to_string();
        if !state.declared_hosts.iter().any(|h| h == &host) {
            return Err(ProtocolHttpEgressError::UndeclaredHost { host });
        }
        if let Some(handle) = request.credential_handle()
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
            method: request.method().as_str().to_string(),
            path: request.path().as_str().to_string(),
            headers: request.headers().to_vec(),
            body: request.body().to_vec(),
            credential_handle: request.credential_handle().map(|h| h.as_str().to_string()),
        });
        if let Some(queue) = state.programmed_responses.get_mut(&host)
            && let Some(resp) = queue.pop_front()
        {
            return resp;
        }
        Ok(EgressResponse::new(200, br#"{"ok":true}"#.to_vec()))
    }
}

pub fn fake_reply_target(suffix: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(format!("reply:fake-{suffix}")).expect("valid reply target") // safety: test-support helper prefixes caller suffix into bounded ref
}

/// A recording [`crate::reply::ReplySink`]: captures every reconcile request
/// (so a test can prove which revisions reached the edge, at which cadence
/// point, and with which checkpoint) and answers from a scripted outcome
/// queue, defaulting to `Applied` with a checkpoint that echoes the revision
/// it applied.
///
/// The publication worker, the binding rule, and the coordinator's
/// reply-publication bookkeeping are all tested against it.
pub struct RecordingReplySink {
    requests: Mutex<Vec<crate::reply::ReplyReconcileRequest>>,
    scripted: Mutex<VecDeque<crate::reply::ReplySinkOutcome>>,
    provider_ref_prefix: String,
}

impl Default for RecordingReplySink {
    fn default() -> Self {
        Self::new("provider-ref")
    }
}

impl RecordingReplySink {
    pub fn new(provider_ref_prefix: impl Into<String>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            scripted: Mutex::new(VecDeque::new()),
            provider_ref_prefix: provider_ref_prefix.into(),
        }
    }

    /// Queue outcomes for the next reconcile calls, in order. Once the queue
    /// drains the sink answers `Applied` again.
    pub fn script(&self, outcomes: impl IntoIterator<Item = crate::reply::ReplySinkOutcome>) {
        let mut scripted = self.scripted.lock().expect("fake sink lock poisoned"); // safety: test-support fake; poisoned mutex means another test already panicked;
        scripted.extend(outcomes);
    }

    pub fn requests(&self) -> Vec<crate::reply::ReplyReconcileRequest> {
        self.requests
            .lock()
            .expect("fake sink lock poisoned") // safety: test-support fake; poisoned mutex means another test already panicked;
            .clone()
    }

    /// The revisions that reached this sink, in call order.
    pub fn revisions(&self) -> Vec<u64> {
        self.requests()
            .iter()
            .map(|request| request.revision.revision)
            .collect()
    }

    /// The cadence points that reached this sink, in call order.
    pub fn points(&self) -> Vec<crate::reply::ReplyReconcilePoint> {
        self.requests()
            .iter()
            .map(|request| request.point)
            .collect()
    }
}

#[async_trait]
impl crate::reply::ReplySink for RecordingReplySink {
    async fn reconcile(
        &self,
        request: crate::reply::ReplyReconcileRequest,
        _egress: &dyn crate::tool_adapter::RestrictedEgress,
    ) -> Result<crate::reply::ReplySinkReport, crate::channel_adapter::ChannelError> {
        let revision = request.revision.revision;
        let terminal = request.revision.document.is_terminal();
        self.requests
            .lock()
            .expect("fake sink lock poisoned") // safety: test-support fake; poisoned mutex means another test already panicked;
            .push(request);
        let outcome = self
            .scripted
            .lock()
            .expect("fake sink lock poisoned") // safety: test-support fake; poisoned mutex means another test already panicked;
            .pop_front()
            .unwrap_or(crate::reply::ReplySinkOutcome::Applied);
        // Like a real sink that opened a provider presentation before the
        // provider answered, a non-applied outcome still hands back the
        // checkpoint describing what was started.
        let checkpoint_payload = match &outcome {
            crate::reply::ReplySinkOutcome::Applied => format!("applied:{revision}"),
            crate::reply::ReplySinkOutcome::Retryable { .. } => format!("retryable:{revision}"),
            crate::reply::ReplySinkOutcome::Ambiguous { .. } => format!("ambiguous:{revision}"),
            _ => String::new(),
        };
        let checkpoint = crate::reply::ReplySinkCheckpoint::new(1, checkpoint_payload.clone())
            .expect("checkpoint within bound"); // safety: test-support fake with a fixed small payload.
        let mut evidence = crate::reply::ReplySinkEvidence::default();
        if outcome.is_applied() {
            let reference = crate::reply::ReplyProviderRef::new(format!(
                "{}:{revision}",
                self.provider_ref_prefix
            ))
            .expect("provider ref within bound"); // safety: test-support fake with a fixed small ref.
            evidence
                .provider_refs
                .push(reference)
                .expect("one ref per report is within the bound"); // safety: test-support fake pushes a single ref.
            evidence.read_back_verified = terminal;
        }
        Ok(crate::reply::ReplySinkReport {
            checkpoint: (!checkpoint_payload.is_empty()).then_some(checkpoint),
            outcome,
            evidence,
        })
    }
}
