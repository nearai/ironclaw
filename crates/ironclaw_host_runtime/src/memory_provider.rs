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

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_memory::{MemoryService, PromptWriteSafetyEventSink};
use ironclaw_memory_native::NativeMemoryService;

use crate::memory_binding::{MemoryBindingPolicy, MemoryProviderBinding};

/// How the document-store profile resolves, before constructing a provider.
enum DocumentStoreResolution {
    /// Build the host-bundled native (filesystem-backed) provider.
    Native,
    /// Hand off to a registered third-party provider with this extension id.
    ThirdParty(String),
    /// Disabled, unbound, or an impossible invalid id — fail closed.
    Unavailable,
}

/// Resolves the memory provider for a profile, honoring the binding policy.
///
/// The resolver is provider-agnostic about third parties: it stores already-
/// constructed `Arc<dyn MemoryService>` instances keyed by extension id and
/// hands the right one back when the binding selects it. The composition layer
/// (which may depend on concrete provider crates such as `ironclaw_memory_mem0`)
/// builds those instances and registers them; this crate never names a concrete
/// third-party provider, so the host runtime stays free of provider deps.
#[derive(Clone, Default)]
pub struct MemoryServiceResolver {
    /// `None` = native for every profile (the default). `Some(policy)` resolves
    /// per the configured `profile_id -> extension_id` bindings.
    policy: Option<MemoryBindingPolicy>,
    /// Registered third-party document-store providers, keyed by extension id.
    /// Empty in the default/native configuration, so behavior is unchanged
    /// unless composition injects a provider (e.g. mem0).
    third_party_document_store: BTreeMap<String, Arc<dyn MemoryService>>,
}

impl std::fmt::Debug for MemoryServiceResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryServiceResolver")
            .field("policy", &self.policy.is_some())
            .field(
                "third_party_document_store_providers",
                &self.third_party_document_store.len(),
            )
            .finish()
    }
}

impl MemoryServiceResolver {
    /// Resolver bound to the native provider for every profile (= `Default`).
    pub fn native() -> Self {
        Self {
            policy: None,
            third_party_document_store: BTreeMap::new(),
        }
    }

    /// Resolver backed by an explicit binding policy.
    pub fn from_policy(policy: MemoryBindingPolicy) -> Self {
        Self {
            policy: Some(policy),
            third_party_document_store: BTreeMap::new(),
        }
    }

    /// Resolver from an optional policy (`None` → native default). Convenience
    /// for composition, which carries the resolved policy as an `Option`.
    pub fn from_optional_policy(policy: Option<MemoryBindingPolicy>) -> Self {
        Self {
            policy,
            third_party_document_store: BTreeMap::new(),
        }
    }

    /// Register a concrete third-party document-store provider for `extension_id`.
    ///
    /// When the resolved `memory.document_store.v1` binding is
    /// `ThirdParty { extension_id }` and an instance is registered here under the
    /// same id, [`resolve_document_store`](Self::resolve_document_store) returns
    /// it; otherwise a third-party binding still fails closed (as before). The
    /// binding policy decides *whether* a third party is permitted (fail-closed,
    /// override-gated in production); this registry decides *which instance*
    /// serves it. Composition is the single caller, so the "which provider, and
    /// is it permitted?" decision stays in one place.
    pub fn with_third_party_document_store_provider(
        mut self,
        extension_id: impl Into<String>,
        provider: Arc<dyn MemoryService>,
    ) -> Self {
        self.third_party_document_store
            .insert(extension_id.into(), provider);
        self
    }

    /// Resolve the document-store provider over a per-invocation filesystem.
    ///
    /// Returns `None` (fail-closed) when the document-store profile is disabled,
    /// or bound to a third party for which no provider instance is registered;
    /// callers surface that as a model-visible error (tools) or degrade to
    /// "profile unknown" (profile reader), never silently falling back to native.
    pub fn resolve_document_store(
        &self,
        filesystem: Arc<dyn RootFilesystem>,
        prompt_write_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    ) -> Option<Arc<dyn MemoryService>> {
        match self.document_store_resolution() {
            DocumentStoreResolution::Native => Some(Arc::new(
                NativeMemoryService::from_filesystem(filesystem, prompt_write_safety_event_sink),
            )),
            // A registered third-party provider (e.g. mem0) backs the binding.
            // Third-party providers are remote/REST-backed and do not consume the
            // per-invocation filesystem; an unregistered third-party id fails closed.
            //
            // SECURITY LIMITATION (tracked in #5264): the prompt-write-safety
            // engine (write-time prompt-injection rejection + per-write audit
            // events) lives inside `NativeMemoryService`, so the
            // `prompt_write_safety_event_sink` is intentionally unused on this arm
            // — a third-party document_store binding does NOT get write-time
            // prompt-write-safety enforcement or audit. This is acceptable for the
            // current off-by-default surface because a third-party provider cannot
            // reach the *trusted* prompt surface (identity files
            // AGENTS.md/SOUL.md/… are native-filesystem-only) and all retrieved
            // content is host-wrapped as untrusted before it can enter a prompt.
            // Hoisting prompt-write-safety host-side (provider-agnostic, before
            // dispatch) is the proper fix and is deferred to #5264.
            DocumentStoreResolution::ThirdParty(extension_id) => self
                .third_party_document_store
                .get(&extension_id)
                .map(Arc::clone),
            DocumentStoreResolution::Unavailable => None,
        }
    }

    /// Resolve how memory binds, before provider construction. Disabled fails
    /// closed (no provider).
    fn document_store_resolution(&self) -> DocumentStoreResolution {
        let Some(policy) = &self.policy else {
            return DocumentStoreResolution::Native;
        };
        match policy.binding() {
            MemoryProviderBinding::Native => DocumentStoreResolution::Native,
            MemoryProviderBinding::ThirdParty { extension_id } => {
                DocumentStoreResolution::ThirdParty(extension_id.as_str().to_string())
            }
            MemoryProviderBinding::Disabled => DocumentStoreResolution::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_binding::{
        MEMORY_DISABLED_BINDING_SENTINEL, MemoryBindingInput, MemoryDeploymentProfile,
    };
    use ironclaw_filesystem::InMemoryBackend;

    fn filesystem() -> Arc<dyn RootFilesystem> {
        Arc::new(InMemoryBackend::new())
    }

    fn policy_with_document_store(extension_id: &str) -> MemoryBindingPolicy {
        MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(extension_id.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev)
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
        let resolver =
            MemoryServiceResolver::from_policy(policy_with_document_store("ironclaw.memory"));
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
    fn third_party_binding_without_registered_provider_resolves_to_none() {
        // Third party is permitted in dev without an override, but when no
        // provider instance is registered for the bound id, it fails closed.
        let resolver =
            MemoryServiceResolver::from_policy(policy_with_document_store("acme.honcho"));
        assert!(
            resolver
                .resolve_document_store(filesystem(), None)
                .is_none()
        );
    }

    /// Minimal in-test third-party provider: a unique `read` marker lets the
    /// test assert the resolver returned *this* instance, not the native one.
    #[derive(Debug)]
    struct MarkerProvider;

    #[async_trait::async_trait]
    impl MemoryService for MarkerProvider {
        async fn read(
            &self,
            _invocation: ironclaw_memory::MemoryInvocation,
            _request: ironclaw_memory::MemoryServiceReadRequest,
        ) -> Result<ironclaw_memory::MemoryServiceReadResponse, ironclaw_memory::MemoryServiceError>
        {
            Ok(ironclaw_memory::MemoryServiceReadResponse {
                path: "marker".to_string(),
                content: "registered-third-party".to_string(),
                word_count: 1,
            })
        }
    }

    fn marker_invocation() -> ironclaw_memory::MemoryInvocation {
        ironclaw_memory::MemoryInvocation {
            scope: ironclaw_host_api::ResourceScope {
                tenant_id: ironclaw_host_api::TenantId::new("tenant-swap").unwrap(),
                user_id: ironclaw_host_api::UserId::new("user-swap").unwrap(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: ironclaw_host_api::InvocationId::new(),
            },
            correlation_id: ironclaw_host_api::CorrelationId::new(),
        }
    }

    #[tokio::test]
    async fn third_party_binding_resolves_to_registered_provider() {
        // The binding policy permits the third party; registering an instance for
        // its id makes the resolver hand that instance back instead of failing
        // closed — this is the provider-swap seam.
        let resolver =
            MemoryServiceResolver::from_policy(policy_with_document_store("acme.honcho"))
                .with_third_party_document_store_provider("acme.honcho", Arc::new(MarkerProvider));
        let provider = resolver
            .resolve_document_store(filesystem(), None)
            .expect("registered third-party provider must resolve");
        let read = provider
            .read(
                marker_invocation(),
                ironclaw_memory::MemoryServiceReadRequest {
                    path: "anything".to_string(),
                },
            )
            .await
            .expect("marker provider read");
        // Proves it is the registered instance, not the native filesystem one.
        assert_eq!(read.content, "registered-third-party");
    }

    #[test]
    fn registered_provider_for_a_different_id_still_fails_closed() {
        // Registration is keyed by extension id: a provider registered under a
        // different id does not satisfy this binding.
        let resolver =
            MemoryServiceResolver::from_policy(policy_with_document_store("acme.honcho"))
                .with_third_party_document_store_provider(
                    "other.provider",
                    Arc::new(MarkerProvider),
                );
        assert!(
            resolver
                .resolve_document_store(filesystem(), None)
                .is_none()
        );
    }
}
