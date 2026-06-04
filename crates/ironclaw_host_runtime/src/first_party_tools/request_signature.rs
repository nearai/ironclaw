//! The `request_signature` first-party capability (attested-signing PR14).
//!
//! This capability lets the agent request a blockchain signature. It does NOT
//! sign. In the production host runtime, [`crate::DefaultHostRuntime`] routes a
//! `request_signature` invocation to the composition-owned
//! [`crate::AttestedRaiseHook`] instead of this handler: the hook builds the
//! decoded transaction, computes the binding `ApprovedTxHash`, persists the
//! authoritative gate binding, seals the one-shot grant, and returns
//! [`crate::RuntimeCapabilityOutcome::AttestedSigningRequired`].
//!
//! The handler body here is the fail-closed fallback for runtimes that have no
//! raise hook wired (e.g. bare in-memory test harnesses that never compose the
//! attested substrate): it refuses rather than pretending to raise a gate.

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, RuntimeDispatchErrorKind};
use serde_json::Value;

use crate::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, resource_profile};

pub const REQUEST_SIGNATURE_CAPABILITY_ID: &str = "builtin.request_signature";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        REQUEST_SIGNATURE_CAPABILITY_ID,
        "Request an attested blockchain signature for a decoded transaction. \
         Does not sign — raises a human-in-the-loop attested-signing gate.",
        // Dispatching this capability raises a gate; the heavy lifting (decode,
        // bind, seal, initiate) is owned by the composition raise hook, so the
        // declared effect is the dispatch itself.
        vec![EffectKind::DispatchCapability],
        // The raise ceremony is itself the human-in-the-loop boundary, so the
        // capability defaults to asking before it runs.
        PermissionMode::Ask,
        resource_profile(),
    )
}

/// Fail-closed handler: only reachable when no [`crate::AttestedRaiseHook`] is
/// wired. A signing request that cannot raise an attested gate MUST NOT
/// silently succeed — it refuses.
pub(super) fn dispatch(_input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    Err(FirstPartyCapabilityError::new(
        RuntimeDispatchErrorKind::UnsupportedRunner,
    ))
}
