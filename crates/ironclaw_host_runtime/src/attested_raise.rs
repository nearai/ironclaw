//! The composition-owned attested-signing raise hook (attested-signing PR14).
//!
//! The `request_signature` first-party capability does not sign. When it is
//! invoked inside the production host runtime, [`DefaultHostRuntime`] routes the
//! invocation to an injected [`AttestedRaiseHook`] instead of normal dispatch.
//!
//! The hook is the composition seam that keeps all crypto/chain logic OUT of
//! the host-runtime/loop/turns crates:
//!
//! 1. builds the decoded transaction + selects the signing provider;
//! 2. computes the binding `ApprovedTxHash`;
//! 3. persists the authoritative gate binding + seals the one-shot grant;
//! 4. returns [`RuntimeCapabilityOutcome::AttestedSigningRequired`] carrying the
//!    opaque `gate_ref` + `expected_tx_hash`.
//!
//! The trait surface is intentionally crypto-free: the host runtime hands the
//! hook opaque JSON params + the execution context and receives a
//! [`RuntimeCapabilityOutcome`] back. All decode/render/hash/seal logic lives in
//! the concrete implementation in `ironclaw_reborn_composition`.
//!
//! **Fail-closed:** any failure (decode, provider selection, hash, persist,
//! seal) MUST surface as [`RuntimeCapabilityOutcome::Failed`] — never a
//! half-raised gate.

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ExecutionContext};
use serde_json::Value;

use crate::RuntimeCapabilityOutcome;

/// Inputs the raise hook needs to build, bind, and persist an attested gate.
///
/// `input` is the opaque `request_signature` parameter object; the concrete
/// composition hook owns its schema (chain, tx_type, decoded-tx fields or
/// payload, signer/account, provider_hint).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AttestedRaiseRequest {
    /// The capability that triggered the raise (always the `request_signature`
    /// capability id). Carried so the produced outcome can echo it for the
    /// host-runtime's outcome/capability consistency check.
    pub capability_id: CapabilityId,
    /// The already-authorized execution context (tenant/user/scope/run/etc.).
    pub context: ExecutionContext,
    /// Opaque `request_signature` parameters.
    pub input: Value,
}

impl AttestedRaiseRequest {
    /// Construct a raise request.
    pub fn new(capability_id: CapabilityId, context: ExecutionContext, input: Value) -> Self {
        Self {
            capability_id,
            context,
            input,
        }
    }
}

/// The composition-owned attested-signing raise hook.
///
/// Object-safe so the host runtime can hold it as
/// `std::sync::Arc<dyn AttestedRaiseHook>` without naming the composition's
/// concrete crypto/store generics.
#[async_trait]
pub trait AttestedRaiseHook: Send + Sync {
    /// Run the raise ceremony for a `request_signature` invocation.
    ///
    /// On success returns [`RuntimeCapabilityOutcome::AttestedSigningRequired`].
    /// On any failure returns [`RuntimeCapabilityOutcome::Failed`] (fail-closed).
    /// Implementations MUST NOT sign or broadcast.
    async fn raise(&self, request: AttestedRaiseRequest) -> RuntimeCapabilityOutcome;
}
