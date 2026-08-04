//! In-memory fakes for the product-tier projection and inbound DTOs, used by
//! contract tests and downstream consumer tests.
//!
//! The extension-tier half of the original module (egress transport, delivery
//! sink) moved with its ports to
//! `ironclaw_extension_contracts::test_support::fakes`; a fake belongs beside
//! the port it implements.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::ids::SecretHandle;
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use secrecy::{ExposeSecret, SecretString};

use crate::operator_secrets::{OperatorSecretValueStore, OperatorSecretValueStoreError};

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

/// In-memory [`OperatorSecretValueStore`] for consumers that need a working
/// store without a secret substrate.
///
/// It is deliberately *not* a stand-in for the production adapter's substrate
/// behaviour: the lease protocol, the crypto and the backend error mapping all
/// live behind the port and are pinned where they are implemented
/// (`ironclaw_reborn_composition::operator_secret_store`). What this fake is
/// for is driving a caller's own policy and fail-closed paths — including the
/// failing case, which a real store can only be pushed into with an injected
/// backend fault.
pub struct FakeOperatorSecretValueStore {
    values: Mutex<BTreeMap<String, String>>,
    calls: Mutex<BTreeMap<&'static str, usize>>,
    failing_ops: Vec<OperatorSecretValueOp>,
    reason: &'static str,
}

/// Which [`OperatorSecretValueStore`] operation a
/// [`FakeOperatorSecretValueStore`] should fail.
///
/// Per-operation rather than all-or-nothing because the fail-closed paths worth
/// testing are per-operation: "provider delete must not report success when the
/// key delete fails" needs `put` and `contains` to keep working.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperatorSecretValueOp {
    Put,
    Contains,
    Handles,
    Read,
    Delete,
}

impl OperatorSecretValueOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Put => "put",
            Self::Contains => "contains",
            Self::Handles => "handles",
            Self::Read => "read",
            Self::Delete => "delete",
        }
    }
}

impl FakeOperatorSecretValueStore {
    /// A working store.
    pub fn new() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(BTreeMap::new()),
            failing_ops: Vec::new(),
            reason: "BackendUnavailable",
        }
    }

    /// A store whose every operation fails with `reason`.
    ///
    /// For a caller's fail-closed paths: the production adapter reports the
    /// substrate's `stable_reason`, so `reason` should be one of those strings
    /// (`"BackendUnavailable"`, `"MissingCredential"`, …) rather than free text.
    pub fn failing(reason: &'static str) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(BTreeMap::new()),
            failing_ops: vec![
                OperatorSecretValueOp::Put,
                OperatorSecretValueOp::Contains,
                OperatorSecretValueOp::Handles,
                OperatorSecretValueOp::Read,
                OperatorSecretValueOp::Delete,
            ],
            reason,
        }
    }

    /// A store where only `ops` fail, with `reason`; everything else works.
    pub fn failing_on(
        ops: impl IntoIterator<Item = OperatorSecretValueOp>,
        reason: &'static str,
    ) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(BTreeMap::new()),
            failing_ops: ops.into_iter().collect(),
            reason,
        }
    }

    /// How many times `op` has been called.
    ///
    /// Lets a caller assert *how* it reads the store — one batched
    /// [`OperatorSecretValueStore::handles`] versus N per-handle
    /// [`OperatorSecretValueStore::contains`] probes — at the port rather than
    /// by counting a substrate's filesystem operations.
    pub fn call_count(&self, op: OperatorSecretValueOp) -> usize {
        self.calls
            .lock()
            .expect("fake state lock poisoned") // safety: test-support fake state; poisoned mutex means another test already panicked;
            .get(op.as_str())
            .copied()
            .unwrap_or(0)
    }

    fn guard(&self, op: OperatorSecretValueOp) -> Result<(), OperatorSecretValueStoreError> {
        *self
            .calls
            .lock()
            .expect("fake state lock poisoned") // safety: test-support fake state; poisoned mutex means another test already panicked;
            .entry(op.as_str())
            .or_insert(0) += 1;
        if self.failing_ops.contains(&op) {
            return Err(OperatorSecretValueStoreError::new(self.reason));
        }
        Ok(())
    }

    fn values(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.values.lock().expect("fake state lock poisoned") // safety: test-support fake state; poisoned mutex means another test already panicked;
    }
}

impl Default for FakeOperatorSecretValueStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OperatorSecretValueStore for FakeOperatorSecretValueStore {
    async fn put(
        &self,
        handle: &SecretHandle,
        value: SecretString,
    ) -> Result<(), OperatorSecretValueStoreError> {
        self.guard(OperatorSecretValueOp::Put)?;
        self.values().insert(
            handle.as_str().to_string(),
            value.expose_secret().to_string(),
        );
        Ok(())
    }

    async fn contains(&self, handle: &SecretHandle) -> Result<bool, OperatorSecretValueStoreError> {
        self.guard(OperatorSecretValueOp::Contains)?;
        Ok(self.values().contains_key(handle.as_str()))
    }

    async fn handles(&self) -> Result<Vec<SecretHandle>, OperatorSecretValueStoreError> {
        self.guard(OperatorSecretValueOp::Handles)?;
        self.values()
            .keys()
            .map(|handle| {
                SecretHandle::new(handle.clone())
                    .map_err(|_| OperatorSecretValueStoreError::new("MissingCredential"))
            })
            .collect()
    }

    async fn read(
        &self,
        handle: &SecretHandle,
    ) -> Result<Option<SecretString>, OperatorSecretValueStoreError> {
        self.guard(OperatorSecretValueOp::Read)?;
        Ok(self
            .values()
            .get(handle.as_str())
            .map(|value| SecretString::from(value.clone())))
    }

    async fn delete(&self, handle: &SecretHandle) -> Result<bool, OperatorSecretValueStoreError> {
        self.guard(OperatorSecretValueOp::Delete)?;
        Ok(self.values().remove(handle.as_str()).is_some())
    }
}
