//! Resolver trait for converting `CapabilityInputRef` handles into sanitized
//! JSON the hook framework can hand to predicate evaluation.
//!
//! The hook crate intentionally does not know how to dereference a
//! [`CapabilityInputRef`] — that knowledge belongs to the production host
//! (which has workspace / store access). The middleware accepts an
//! `Arc<dyn CapabilityInputResolver>` and consults it before each invocation;
//! when no resolver is configured, the bundled
//! [`NullCapabilityInputResolver`] returns `None`, causing
//! [`crate::points::SanitizedArguments::unresolved`] to be threaded through.
//!
//! Predicate evaluation that requires argument contents (currently
//! `ValueOrRateBound::NumericSum`) is responsible for failing closed in the
//! unresolved case.

use async_trait::async_trait;
use ironclaw_host_api::ExtensionId;
use ironclaw_turns::run_profile::CapabilityInvocation;

/// Resolves a [`CapabilityInvocation`]'s input ref to a sanitized JSON view.
///
/// Implementations should return:
///
/// - `Some(value)` when the ref was resolved and the JSON-shaped payload is
///   safe to hand to hook predicates (already free of secrets / handle
///   pointers / etc. — the framework will further bound size and depth).
/// - `None` when resolution is unavailable, fails, or the result is
///   unsafe to expose. The hook framework treats `None` as
///   "unresolved" — predicate evaluators that depend on argument
///   contents must fail closed in this case.
#[async_trait]
pub trait CapabilityInputResolver: Send + Sync {
    async fn resolve(&self, invocation: &CapabilityInvocation) -> Option<serde_json::Value>;

    /// Cheap streaming size probe consulted by the middleware before
    /// [`Self::resolve`] when a hook would actually read the input.
    /// Implementations backed by a workspace blob or other handle-shaped
    /// source should report the underlying byte length here without
    /// materializing the value; sources that already hold the value
    /// in memory may return `None`.
    ///
    /// When the returned size exceeds the middleware's
    /// `MAX_PREDICATE_INPUT_BYTES` cap, the middleware fails closed
    /// (treats the input as unresolved) and skips [`Self::resolve`]
    /// entirely — protecting hosts from fatal blob materialization for
    /// inputs that predicates were not going to read anyway, and
    /// bounding the cost when predicates do need to inspect them.
    ///
    /// Default returns `None` (unknown). The middleware applies a
    /// post-materialization byte check as a defense-in-depth backstop
    /// when the size hint is unavailable.
    async fn size_hint(&self, _invocation: &CapabilityInvocation) -> Option<u64> {
        None
    }
}

/// Default resolver that never resolves arguments. Used when middleware
/// composers haven't wired in a production resolver yet.
pub struct NullCapabilityInputResolver;

#[async_trait]
impl CapabilityInputResolver for NullCapabilityInputResolver {
    async fn resolve(&self, _invocation: &CapabilityInvocation) -> Option<serde_json::Value> {
        None
    }
}

/// Resolves a capability id to the extension that provides it, when known.
///
/// The hook crate cannot know which capabilities are owned by which
/// extensions — that knowledge lives in the host's capability registry. The
/// middleware accepts an `Arc<dyn CapabilityProviderResolver>` and consults
/// it on every invocation; the resolved provider is threaded into
/// [`crate::points::BeforeCapabilityHookContext::provider`] and the dispatcher
/// uses it to enforce manifest-declared hook scope.
///
/// Implementations should return:
///
/// - `Some(ext)` when the capability is known to be provided by extension
///   `ext`.
/// - `None` when the provider is unknown or the capability is host-internal
///   (e.g., a Builtin capability with no `ExtensionId`). Hooks with scope
///   [`crate::registry::HookBindingScope::OwnCapabilities`] will NOT fire
///   against such invocations — the conservative default.
#[async_trait]
pub trait CapabilityProviderResolver: Send + Sync {
    async fn provider_for(&self, capability_id: &str) -> Option<ExtensionId>;
}

/// Default provider resolver that never resolves a provider. Used when the
/// middleware composer hasn't wired in a production resolver. With this
/// resolver in place, `OwnCapabilities`-scoped hooks effectively never fire,
/// which is the conservative default until the host can answer the question.
pub struct NullCapabilityProviderResolver;

#[async_trait]
impl CapabilityProviderResolver for NullCapabilityProviderResolver {
    async fn provider_for(&self, _capability_id: &str) -> Option<ExtensionId> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::CapabilityId;
    use ironclaw_turns::run_profile::{CapabilityInputRef, CapabilitySurfaceVersion};

    #[tokio::test]
    async fn null_resolver_returns_none() {
        let resolver = NullCapabilityInputResolver;
        let invocation = CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("v1").expect("ok"),
            capability_id: CapabilityId::new("cap.x").expect("ok"),
            input_ref: CapabilityInputRef::new("input:x").expect("ok"),
            approval_resume: None,
            auth_resume: None,
        };
        assert!(resolver.resolve(&invocation).await.is_none());
    }

    #[tokio::test]
    async fn null_provider_resolver_returns_none() {
        let resolver = NullCapabilityProviderResolver;
        assert!(resolver.provider_for("cap.x").await.is_none());
    }
}
