//! The operator control plane's secret-value port (PROPOSAL §6.2.2, §8.2,
//! CHECKLIST WS3 — "tighten direct `secrets` consumers").
//!
//! `ironclaw_operator` stores one class of secret: the API-key **values** an
//! operator pastes into the WebChat v2 Inference tab, which are re-injected into
//! the resolved `LlmConfig` at provider-build and reload time. Reaching that
//! storage used to mean holding `ironclaw_secrets` — the substrate that owns
//! CAS one-shot leases, AAD/crypto, and the OS keychain master key — directly
//! from the products tier. §8.2's product row says the products tier loses that
//! edge; this port is the replacement, and PROPOSAL §12.1b is why it lands
//! *before* the edge is removed rather than with it.
//!
//! **The port is narrower than the substrate on purpose, in three ways.**
//!
//! 1. **No lease vocabulary.** `SecretStorePort` exposes a lease/consume CAS
//!    protocol whose whole point is that a staged one-shot credential is
//!    consumed exactly once by a runtime lane. The operator does not stage
//!    anything for a lane — it reads a configuration value back on every
//!    provider reload — so [`OperatorSecretValueStore::read`] is a plain
//!    repeatable read and the lease dance stays behind the implementor. A
//!    products-tier caller cannot reach the staging protocol through this port
//!    at all.
//! 2. **No scope argument.** The implementor fixes the scope. LLM configuration
//!    is operator-wide (a single instance config, not per-user), and with the
//!    scope in the caller's hands nothing stopped a products-tier caller
//!    addressing a *tenant's* secrets through the same handle. Now it cannot
//!    name a scope, so it cannot reach one.
//! 3. **No substrate error detail.** [`OperatorSecretValueStoreError`] carries
//!    only a stable classification string. The substrate's error Display can
//!    name handles and backend detail; that no longer crosses into the product
//!    tier, into an operator log line, or into a `Display` a route might
//!    surface.
//!
//! What the operator keeps is its own policy: which handle a provider id maps to
//! (`llm_provider_<id>_api_key`) and what to do when a read comes back empty.
//! That is naming and fail-closed behaviour, and it belongs with the control
//! plane, not with assembly.

use async_trait::async_trait;
use ironclaw_host_api::ids::SecretHandle;
use secrecy::SecretString;
use thiserror::Error;

/// Operator-scoped storage for secret **values**, as the products tier is
/// allowed to see it.
///
/// The implementor supplies the scope (see the module docs) and owns every
/// substrate concern: leases, crypto, backend selection, error classification.
/// Callers address a value by [`SecretHandle`] and nothing else.
///
/// The production implementation is
/// `ironclaw_composition::RuntimeOperatorSecretValueStore`, over
/// `ironclaw_secrets::SecretStorePort` — assembly is the only layer that may
/// name both sides. `ironclaw_operator::LlmKeyStore` is the sole consumer;
/// `crates/ironclaw_architecture_tests/tests/reborn_operator_port_inversion.rs` pins
/// both facts.
#[async_trait]
pub trait OperatorSecretValueStore: Send + Sync {
    /// Store (or replace) the value under `handle`.
    async fn put(
        &self,
        handle: &SecretHandle,
        value: SecretString,
    ) -> Result<(), OperatorSecretValueStoreError>;

    /// Whether a value exists under `handle`, without revealing it.
    async fn contains(&self, handle: &SecretHandle) -> Result<bool, OperatorSecretValueStoreError>;

    /// Every handle that currently holds a value in the operator scope.
    ///
    /// Used to answer "which providers have a stored key" without reading any
    /// of them. Ordering is unspecified.
    async fn handles(&self) -> Result<Vec<SecretHandle>, OperatorSecretValueStoreError>;

    /// Read the value under `handle`, or `Ok(None)` when nothing is stored.
    ///
    /// **Repeatable.** The operator reads a key on every provider build and
    /// every live reload, so an implementation over a one-shot lease protocol
    /// must leave the underlying secret in place. "Not stored" is `Ok(None)`,
    /// never an error — a provider with no operator-set key is an ordinary
    /// state, and mapping it to an error would make the caller's fail-closed
    /// paths fire on a healthy instance.
    async fn read(
        &self,
        handle: &SecretHandle,
    ) -> Result<Option<SecretString>, OperatorSecretValueStoreError>;

    /// Delete the value under `handle`; returns whether one existed.
    ///
    /// Idempotent: deleting an absent handle is `Ok(false)`, not an error.
    async fn delete(&self, handle: &SecretHandle) -> Result<bool, OperatorSecretValueStoreError>;
}

/// Why an [`OperatorSecretValueStore`] call failed, carrying only a stable
/// classification.
///
/// Deliberately opaque. The substrate behind the port classifies its own
/// failures (`ironclaw_secrets::SecretStoreError::stable_reason`) and the
/// implementor forwards that classification; nothing else crosses. Callers log
/// [`Self::stable_reason`] and fail closed — there is no variant to branch on,
/// because every failure of this port means the same thing to the control
/// plane: the value could not be trusted to be stored, present, or removed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("operator secret store unavailable ({reason})")]
pub struct OperatorSecretValueStoreError {
    reason: &'static str,
}

impl OperatorSecretValueStoreError {
    /// Build an error from a stable, non-secret classification string.
    ///
    /// `reason` is `&'static str` rather than `String` so an implementation
    /// cannot accidentally format a handle, a path, or backend detail into it.
    pub fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    /// The stable classification, safe to log and to compare across releases.
    pub fn stable_reason(&self) -> &'static str {
        self.reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_carries_the_stable_reason_and_nothing_else() {
        let error = OperatorSecretValueStoreError::new("BackendUnavailable");
        assert_eq!(error.stable_reason(), "BackendUnavailable");
        assert_eq!(
            error.to_string(),
            "operator secret store unavailable (BackendUnavailable)"
        );
    }
}
