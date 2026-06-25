//! Single construction point for the memory provider (issue #3537).
//!
//! Every consumer of memory — the model-facing memory tools, the user-profile
//! reader, and (when wired) the turn-start context retriever — needs an
//! `Arc<dyn MemoryService>` for a given memory capability profile. Before this,
//! each consumer constructed its own provider (`NativeMemoryService::from_filesystem`)
//! and re-checked the binding inline, so the "which provider, and is it
//! permitted?" decision was duplicated across call sites.
//!
//! [`MemoryServiceResolver`] is that decision, in one place: given a profile and
//! the per-invocation inputs (the request filesystem + an optional prompt-write
//! safety sink), it resolves the bound provider or returns `None` (fail-closed)
//! when the profile is disabled or bound to an unimplemented third party. The
//! consumers just call it — none of them constructs a provider or matches on a
//! binding themselves.
//!
//! It wraps an `Option<MemoryBindingPolicy>` where `None` means the
//! behavior-preserving default (native for every profile), so the resolver is
//! `Default`/cheaply-cloneable and the tool structs that hold it do not need a
//! fallible constructor.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::CapabilityProfileId;
use ironclaw_memory::{MemoryService, PromptWriteSafetyEventSink};
use ironclaw_memory_native::NativeMemoryService;

use crate::memory_binding::{MemoryBindingPolicy, MemoryProviderBinding};
use crate::memory_profiles::MEMORY_DOCUMENT_STORE_PROFILE_ID;

/// Resolves the memory provider for a profile, honoring the binding policy.
#[derive(Clone, Default)]
pub struct MemoryServiceResolver {
    /// `None` = native for every profile (the default). `Some(policy)` resolves
    /// per the configured `profile_id -> extension_id` bindings.
    policy: Option<MemoryBindingPolicy>,
}

impl std::fmt::Debug for MemoryServiceResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryServiceResolver")
            .field("policy", &self.policy.is_some())
            .finish()
    }
}

impl MemoryServiceResolver {
    /// Resolver bound to the native provider for every profile (= `Default`).
    pub fn native() -> Self {
        Self { policy: None }
    }

    /// Resolver backed by an explicit binding policy.
    pub fn from_policy(policy: MemoryBindingPolicy) -> Self {
        Self {
            policy: Some(policy),
        }
    }

    /// Resolver from an optional policy (`None` → native default). Convenience
    /// for composition, which carries the resolved policy as an `Option`.
    pub fn from_optional_policy(policy: Option<MemoryBindingPolicy>) -> Self {
        Self { policy }
    }

    /// Resolve the document-store provider over a per-invocation filesystem.
    ///
    /// Returns `None` (fail-closed) when the document-store profile is disabled
    /// or bound to an unimplemented third party; callers surface that as a
    /// model-visible error (tools) or degrade to "profile unknown" (profile
    /// reader), never silently falling back to native.
    pub fn resolve_document_store(
        &self,
        filesystem: Arc<dyn RootFilesystem>,
        prompt_write_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    ) -> Option<Arc<dyn MemoryService>> {
        if self.binds_native(MEMORY_DOCUMENT_STORE_PROFILE_ID) {
            Some(Arc::new(NativeMemoryService::from_filesystem(
                filesystem,
                prompt_write_safety_event_sink,
            )))
        } else {
            None
        }
    }

    /// Whether the profile is bound to the native provider. Unknown/disabled/
    /// third-party (and the impossible invalid-id case) all fail closed.
    fn binds_native(&self, profile_id: &str) -> bool {
        match &self.policy {
            None => true,
            Some(policy) => match CapabilityProfileId::new(profile_id) {
                Ok(id) => matches!(policy.binding_for(&id), Some(MemoryProviderBinding::Native)),
                Err(_) => false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_binding::{
        MEMORY_DISABLED_BINDING_SENTINEL, MemoryBindingInput, MemoryDeploymentProfile,
        MemoryProfileBindingEntry,
    };
    use ironclaw_filesystem::InMemoryBackend;

    fn filesystem() -> Arc<dyn RootFilesystem> {
        Arc::new(InMemoryBackend::new())
    }

    fn policy_with_document_store(extension_id: &str) -> MemoryBindingPolicy {
        MemoryBindingPolicy::resolve(MemoryBindingInput {
            deployment: MemoryDeploymentProfile::LocalDev,
            native_available: true,
            bindings: vec![MemoryProfileBindingEntry {
                profile_id: CapabilityProfileId::new(MEMORY_DOCUMENT_STORE_PROFILE_ID).unwrap(),
                extension_id: extension_id.to_string(),
            }],
            overrides: Vec::new(),
        })
        .expect("policy resolves")
    }

    #[test]
    fn native_resolver_builds_a_document_store_service() {
        let resolver = MemoryServiceResolver::native();
        assert!(
            resolver
                .resolve_document_store(filesystem(), None)
                .is_some()
        );
        // Default == native.
        assert!(
            MemoryServiceResolver::default()
                .resolve_document_store(filesystem(), None)
                .is_some()
        );
    }

    #[test]
    fn explicit_native_policy_builds_a_service() {
        let resolver = MemoryServiceResolver::from_policy(policy_with_document_store(
            "ironclaw.memory.native",
        ));
        assert!(
            resolver
                .resolve_document_store(filesystem(), None)
                .is_some()
        );
    }

    #[test]
    fn disabled_binding_resolves_to_none() {
        let resolver = MemoryServiceResolver::from_policy(policy_with_document_store(
            MEMORY_DISABLED_BINDING_SENTINEL,
        ));
        assert!(
            resolver
                .resolve_document_store(filesystem(), None)
                .is_none()
        );
    }

    #[test]
    fn third_party_binding_resolves_to_none() {
        // Third party is permitted in dev without an override, but no third-party
        // provider is implemented, so it fails closed at construction.
        let resolver =
            MemoryServiceResolver::from_policy(policy_with_document_store("acme.honcho"));
        assert!(
            resolver
                .resolve_document_store(filesystem(), None)
                .is_none()
        );
    }
}
