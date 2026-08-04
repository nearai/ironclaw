//! In-memory fakes for the product-tier projection and inbound DTOs, used by
//! contract tests and downstream consumer tests.
//!
//! The extension-tier half of the original module (egress transport, delivery
//! sink) moved with its ports to
//! `ironclaw_extension_contracts::test_support::fakes`; a fake belongs beside
//! the port it implements.

use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::product_adapter_error::ProductAdapterError;

use crate::inbound::{
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductRejection,
    ProductRejectionKind,
};
use crate::outbound::{ProductOutboundEnvelope, ProjectionCursor};
use crate::projection::{ProjectionStream, ProjectionSubscriptionRequest};

pub struct FakeProjectionStream {
    state: Mutex<
        Vec<(
            Option<ProjectionSubscriptionRequest>,
            ProductOutboundEnvelope,
        )>,
    >,
}

impl FakeProjectionStream {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(Vec::new()),
        }
    }

    /// Wildcard push retained for simple tests.
    pub fn push(&self, envelope: ProductOutboundEnvelope) {
        let mut state = self.state.lock().expect("fake state lock poisoned"); // safety: test-support fake state; poisoned mutex means another test already panicked;
        state.push((None, envelope));
    }

    pub fn push_for_request(
        &self,
        request: ProjectionSubscriptionRequest,
        envelope: ProductOutboundEnvelope,
    ) {
        let mut state = self.state.lock().expect("fake state lock poisoned"); // safety: test-support fake state; poisoned mutex means another test already panicked;
        state.push((Some(request), envelope));
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
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        let mut state = self.state.lock().expect("fake state lock poisoned"); // safety: test-support fake state; poisoned mutex means another test already panicked;
        let mut drained = Vec::new();
        let mut retained = Vec::new();
        for (expected, envelope) in std::mem::take(&mut *state) {
            if expected
                .as_ref()
                .is_none_or(|expected| expected == &request)
            {
                drained.push(envelope);
            } else {
                retained.push((expected, envelope));
            }
        }
        *state = retained;
        Ok(drained)
    }
}

pub fn ensure_durable_outcome(ack: &ProductInboundAck) -> bool {
    ack.is_durable_outcome()
}

pub fn ensure_noop_outcome(ack: &ProductInboundAck) -> bool {
    matches!(ack, ProductInboundAck::NoOp)
}

pub fn assert_no_raw_attachment_bytes(envelopes: &[ProductInboundEnvelope]) {
    for envelope in envelopes {
        if let ProductInboundPayload::UserMessage(payload) = envelope.payload() {
            for attachment in &payload.attachments {
                let json = serde_json::to_value(attachment).expect("serialize"); // safety: attachment descriptor is plain scalar serde;
                let object = json.as_object().expect("attachment object"); // safety: derived Serialize for descriptor struct emits an object;
                if object.contains_key("data") {
                    panic!("attachment must not carry raw bytes"); // safety: test-support assertion helper
                }
                if object.contains_key("source_url") {
                    panic!("attachment must not carry source_url"); // safety: test-support assertion helper
                }
                if object.contains_key("local_path") {
                    panic!("attachment must not carry local_path"); // safety: test-support assertion helper
                }
            }
        }
    }
}

pub fn fake_projection_cursor(suffix: &str) -> ProjectionCursor {
    ProjectionCursor::new(format!("cursor:fake-{suffix}")).expect("valid projection cursor") // safety: test-support helper prefixes caller suffix into bounded cursor
}

pub fn fake_rejection(kind: ProductRejectionKind, reason: &str) -> ProductRejection {
    ProductRejection::permanent(kind, reason)
}
