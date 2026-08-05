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
