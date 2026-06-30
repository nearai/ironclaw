//! Error types for the hook framework.
//!
//! `HookError` carries dispatcher-visible failure conditions. Sink-internal
//! errors are intentionally not exposed; they are converted into
//! [`crate::failure_policy::FailureCategory`] by the dispatcher.

use thiserror::Error;

use crate::identity::HookId;

/// Errors visible at the boundary between the dispatcher and its callers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HookError {
    /// A registered hook id does not resolve to a loadable binding in the
    /// active registry. Returned by lookup paths only; dispatch never proceeds
    /// against a missing hook silently.
    #[error("hook id `{0}` is not bound in the active registry")]
    UnknownHook(HookId),

    /// The dispatcher rejected a decision the caller attempted to mint outside
    /// the trust-tier permitted for the hook. Should be unreachable when the
    /// sink traits are used correctly; the variant exists so that future
    /// programmatic-hook surfaces (WASM) can return this rather than panic.
    #[error("hook `{hook_id}` attempted a decision its trust class does not permit: {reason}")]
    AttenuationViolation {
        hook_id: HookId,
        reason: SanitizedReason,
    },

    /// The hook protocol was violated (malformed decision, wrong kind for the
    /// point, etc.). Triggers slot poisoning for the rest of the turn run.
    #[error("hook `{hook_id}` violated the dispatch protocol: {reason}")]
    ProtocolViolation {
        hook_id: HookId,
        reason: SanitizedReason,
    },

    /// Registry construction failure — typically a manifest validation error
    /// surfacing at registry assembly time.
    #[error("hook registry construction failed: {0}")]
    RegistryConstruction(String),
}

/// A short, host-redacted explanation safe to surface in audit logs and
/// model-visible decisions. Construction is internal; callers receive these
/// already-sanitized strings via the dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedReason(pub(crate) String);

impl SanitizedReason {
    /// Construct from a static string. The static literal contract is the
    /// caller's promise that the content is safe to emit verbatim.
    pub(crate) fn from_static(text: &'static str) -> Self {
        Self(text.to_string())
    }

    /// Construct from an already-host-sanitized owned string. Reserved for
    /// the predicate evaluator and the WASM-hook sink, which build reason
    /// strings dynamically from manifest-declared static prefixes.
    #[allow(dead_code)]
    pub(crate) fn from_owned(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SanitizedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
