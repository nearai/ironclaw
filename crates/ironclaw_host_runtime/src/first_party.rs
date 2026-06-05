//! Host-owned FirstParty capability handler registry.
//!
//! First-party handlers are registered by host composition, not by extension
//! manifests. They receive already-authorized, scoped dispatch input from the
//! Reborn runtime adapter path and return normalized JSON output plus resource
//! usage. Authority decisions remain in `CapabilityHost`/authorization and the
//! runtime-policy/planning layers.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityId, MountView, ResourceEstimate, ResourceScope,
    ResourceUsage, RuntimeCredentialAuthRequirement, RuntimeDispatchErrorKind, SecretHandle,
};
use serde_json::Value;

use crate::InvocationServices;

/// Already-authorized first-party capability dispatch input.
///
/// This is host-composed first-party surface, so the struct is `non_exhaustive`:
/// external crates may inspect fields in custom handlers but must not construct
/// it with a struct literal.
#[derive(Clone)]
#[non_exhaustive]
pub struct FirstPartyCapabilityRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub services: InvocationServices,
    pub input: Value,
}

impl fmt::Debug for FirstPartyCapabilityRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FirstPartyCapabilityRequest")
            .field("capability_id", &self.capability_id)
            .field("scope", &self.scope)
            .field("estimate", &self.estimate)
            .field("mounts", &self.mounts)
            .field("services", &self.services)
            .field("input", &self.input)
            .finish()
    }
}

impl PartialEq for FirstPartyCapabilityRequest {
    fn eq(&self, other: &Self) -> bool {
        self.capability_id == other.capability_id
            && self.scope == other.scope
            && self.estimate == other.estimate
            && self.mounts == other.mounts
            && self.input == other.input
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FirstPartyCapabilityRequest {
    #[doc(hidden)]
    pub fn request_for_test(
        capability_id: CapabilityId,
        scope: ResourceScope,
        input: Value,
        runtime_http_egress: Option<Arc<dyn ironclaw_host_api::RuntimeHttpEgress>>,
    ) -> Self {
        Self {
            capability_id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            services: InvocationServices {
                filesystem: Arc::new(ironclaw_filesystem::InMemoryBackend::new()),
                runtime_http_egress,
                tool_call_http_egress: None,
                process: Arc::new(crate::LocalHostProcessPort::new()),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
            },
            input,
        }
    }
}

/// Normalized first-party capability output before resource reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FirstPartyCapabilityResult {
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
}

impl FirstPartyCapabilityResult {
    pub fn new(output: Value, usage: ResourceUsage) -> Self {
        Self {
            output,
            display_preview: None,
            usage,
        }
    }

    pub fn with_display_preview(
        mut self,
        display_preview: Option<CapabilityDisplayOutputPreview>,
    ) -> Self {
        self.display_preview = display_preview;
        self
    }
}

/// Stable redacted first-party handler failure.
///
/// Two distinct outcomes are modelled as enum variants rather than optional
/// fields so that the compiler enforces exhaustive handling and the `Display`
/// impl produces a meaningful message for both paths.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FirstPartyCapabilityError {
    /// Runtime dispatch failed for a non-auth reason.
    #[error("first-party capability dispatch failed: {kind}")]
    Dispatch {
        kind: RuntimeDispatchErrorKind,
        safe_summary: Option<String>,
        usage: Option<ResourceUsage>,
    },
    /// Dispatch was blocked because a staged credential is missing or expired.
    #[error("first-party capability requires authentication")]
    AuthRequired {
        /// Specific secret handles the runtime auth gate must prompt for.
        /// May be empty when the obligation layer does not know which handle failed.
        required_secrets: Vec<SecretHandle>,
        /// Structured credential requirements the runtime auth gate can turn into OAuth.
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
        usage: Option<ResourceUsage>,
    },
}

impl FirstPartyCapabilityError {
    /// Construct a dispatch failure with no auth context.
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self::Dispatch {
            kind,
            safe_summary: None,
            usage: None,
        }
    }

    pub fn with_safe_summary(
        kind: RuntimeDispatchErrorKind,
        safe_summary: impl Into<String>,
    ) -> Self {
        Self::Dispatch {
            kind,
            safe_summary: Some(safe_summary.into()),
            usage: None,
        }
    }

    /// Construct an auth-required failure with no specific secret handles.
    pub fn auth_required() -> Self {
        Self::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
            usage: None,
        }
    }

    /// Construct an auth-required failure naming the handles to re-authorize.
    pub fn auth_required_with(required_secrets: Vec<SecretHandle>) -> Self {
        Self::auth_required_with_context(required_secrets, Vec::new())
    }

    /// Construct an auth-required failure naming both secret handles and OAuth requirements.
    pub fn auth_required_with_context(
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> Self {
        Self::AuthRequired {
            required_secrets,
            credential_requirements,
            usage: None,
        }
    }

    /// Construct an auth-required failure naming the OAuth credential requirements.
    pub fn auth_required_for_credentials(
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> Self {
        Self::auth_required_with_context(Vec::new(), credential_requirements)
    }

    /// Attach resource usage. Builder-style for use in handler return expressions.
    pub fn with_usage(self, usage: ResourceUsage) -> Self {
        match self {
            Self::Dispatch {
                kind, safe_summary, ..
            } => Self::Dispatch {
                kind,
                safe_summary,
                usage: Some(usage),
            },
            Self::AuthRequired {
                required_secrets,
                credential_requirements,
                ..
            } => Self::AuthRequired {
                required_secrets,
                credential_requirements,
                usage: Some(usage),
            },
        }
    }

    /// Runtime dispatch error kind. Returns `None` for `AuthRequired` variants.
    pub fn kind(&self) -> Option<RuntimeDispatchErrorKind> {
        match self {
            Self::Dispatch { kind, .. } => Some(*kind),
            Self::AuthRequired { .. } => None,
        }
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        match self {
            Self::Dispatch { usage, .. } | Self::AuthRequired { usage, .. } => usage.as_ref(),
        }
    }

    pub fn safe_summary(&self) -> Option<&str> {
        match self {
            Self::Dispatch { safe_summary, .. } => safe_summary.as_deref(),
            Self::AuthRequired { .. } => None,
        }
    }

    /// Secret handles required for re-authentication, if this is an auth failure.
    pub fn required_secrets(&self) -> Option<&Vec<SecretHandle>> {
        match self {
            Self::AuthRequired {
                required_secrets, ..
            } => Some(required_secrets),
            Self::Dispatch { .. } => None,
        }
    }

    /// Structured credential requirements, if this is an auth failure.
    pub fn credential_requirements(&self) -> Option<&Vec<RuntimeCredentialAuthRequirement>> {
        match self {
            Self::AuthRequired {
                credential_requirements,
                ..
            } => Some(credential_requirements),
            Self::Dispatch { .. } => None,
        }
    }

    pub fn is_auth_required(&self) -> bool {
        matches!(self, Self::AuthRequired { .. })
    }
}

/// Host-owned first-party capability implementation.
#[async_trait]
pub trait FirstPartyCapabilityHandler: Send + Sync {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError>;
}

/// Host-owned registry keyed by declared [`CapabilityId`].
#[derive(Clone, Default)]
pub struct FirstPartyCapabilityRegistry {
    handlers: HashMap<CapabilityId, Arc<dyn FirstPartyCapabilityHandler>>,
}

impl FirstPartyCapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_handler<T>(mut self, capability_id: CapabilityId, handler: Arc<T>) -> Self
    where
        T: FirstPartyCapabilityHandler + 'static,
    {
        self.insert_handler(capability_id, handler);
        self
    }

    pub fn insert_handler<T>(&mut self, capability_id: CapabilityId, handler: Arc<T>)
    where
        T: FirstPartyCapabilityHandler + 'static,
    {
        let handler: Arc<dyn FirstPartyCapabilityHandler> = handler;
        self.handlers.insert(capability_id, handler);
    }

    pub fn get(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<Arc<dyn FirstPartyCapabilityHandler>> {
        self.handlers.get(capability_id).cloned()
    }

    pub fn contains_handler(&self, capability_id: &CapabilityId) -> bool {
        self.handlers.contains_key(capability_id)
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{ResourceUsage, SecretHandle};

    #[test]
    fn first_party_capability_error_kind_returns_none_for_auth_required() {
        // kind() must return None for both auth_required() and auth_required_with().
        assert_eq!(FirstPartyCapabilityError::auth_required().kind(), None);
        let handle = SecretHandle::new("google-access-token").unwrap();
        assert_eq!(
            FirstPartyCapabilityError::auth_required_with(vec![handle.clone()]).kind(),
            None
        );
        assert!(FirstPartyCapabilityError::auth_required().is_auth_required());
    }

    #[test]
    fn first_party_capability_error_with_usage_preserves_required_secrets() {
        let handle = SecretHandle::new("google-access-token").unwrap();
        let usage = ResourceUsage {
            network_egress_bytes: 42,
            ..ResourceUsage::default()
        };
        let error = FirstPartyCapabilityError::auth_required_with(vec![handle.clone()])
            .with_usage(usage.clone());

        assert!(error.is_auth_required());
        assert_eq!(
            error.kind(),
            None,
            "kind() must remain None for AuthRequired after with_usage"
        );
        assert_eq!(
            error.required_secrets(),
            Some(&vec![handle]),
            "required_secrets must survive with_usage"
        );
        assert_eq!(
            error.usage().map(|u| u.network_egress_bytes),
            Some(42),
            "usage must be set"
        );
    }

    #[test]
    fn first_party_capability_error_with_usage_on_dispatch_variant() {
        use ironclaw_host_api::RuntimeDispatchErrorKind;
        let error = FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend).with_usage(
            ResourceUsage {
                network_egress_bytes: 10,
                ..ResourceUsage::default()
            },
        );
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::Backend));
        assert_eq!(error.required_secrets(), None);
        assert!(!error.is_auth_required());
    }
}
