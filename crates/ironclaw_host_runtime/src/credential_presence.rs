//! Production [`CapabilityCredentialPresence`] composing the pre-flight
//! secret store and the product-auth account resolver behind a short-TTL
//! cache.
//!
//! Extracted from `production.rs` (issue #5416, Phase 2 review Fix D) to keep
//! that file from growing past its already-large size, and to co-locate the
//! presence check next to [`crate::credential_presence_cache`], which it is
//! tightly coupled to.

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityDescriptor, CredentialStageError, ResourceScope};
use ironclaw_secrets::SecretStore;

use crate::{
    credential_presence_cache::{
        CredentialOwnerScope, CredentialPresenceCache, CredentialPresenceKey, CredentialSetupKind,
    },
    obligations::{
        RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver, secret_present,
    },
    production::capability_credential_requirements,
    surface::{CapabilityCredentialPresence, CredentialPresenceStatus},
};

/// Mirrors [`crate::production::DefaultHostRuntime::credential_preflight_check`]'s
/// fail-open discipline: a backend error / indeterminate outcome must never
/// be reported as "missing" (that would burn a false-positive sign-in prompt
/// on the model-visible surface), so only conclusive results are cached and
/// only a conclusive `false` downgrades the surface.
pub(crate) struct ProductionCredentialPresence<'a> {
    pub(crate) secret_store: Option<&'a dyn SecretStore>,
    pub(crate) resolver: Option<&'a dyn RuntimeCredentialAccountResolver>,
    pub(crate) cache: &'a CredentialPresenceCache,
}

#[async_trait]
impl CapabilityCredentialPresence for ProductionCredentialPresence<'_> {
    async fn required_credentials_present(
        &self,
        scope: &ResourceScope,
        descriptor: &CapabilityDescriptor,
    ) -> CredentialPresenceStatus {
        let (required_secrets, credential_requirements) =
            capability_credential_requirements(descriptor);
        if required_secrets.is_empty() && credential_requirements.is_empty() {
            return CredentialPresenceStatus::Present;
        }

        let owner_scope = CredentialOwnerScope::from_scope(scope);
        let mut any_missing = false;
        let mut any_indeterminate = false;

        if !required_secrets.is_empty() {
            match self.secret_store {
                Some(store) => {
                    for handle in &required_secrets {
                        let key =
                            CredentialPresenceKey::Secret(owner_scope.clone(), handle.clone());
                        if let Some(present) = self.cache.get(&key) {
                            any_missing |= !present;
                            continue;
                        }
                        match secret_present(store, scope, handle).await {
                            Ok(present) => {
                                self.cache.insert(key, present);
                                any_missing |= !present;
                            }
                            Err(error) => {
                                tracing::debug!(
                                    secret_handle = handle.as_str(),
                                    error = %error,
                                    "credential presence: secret store metadata query failed; treating as indeterminate"
                                );
                                any_indeterminate = true; // silent-ok: backend error must not report a missing credential; caller stays Available and the dispatch-time obligation check remains the backstop
                            }
                        }
                    }
                }
                None => {
                    any_indeterminate = true; // silent-ok: no secret store wired for this graph; dispatch-time obligation check remains the enforcing backstop
                }
            }
        }

        for requirement in &credential_requirements {
            // Scopes are sorted AND deduplicated before the key is built
            // (mirrors `stable_auth_gate_id`'s scope-sort) so key equality
            // does not depend on manifest declaration order or duplicate
            // scope entries, and — critically — so a requirement's presence
            // answer is never aliased onto a different requirement that
            // shares provider/extension but requests different scopes
            // (#5416 Phase 2 Fix B). The setup kind is part of the key too:
            // `ManualToken` and `OAuth` select accounts differently even for
            // an otherwise-identical requirement (see `CredentialSetupKind`).
            let mut scopes = requirement.provider_scopes.clone();
            scopes.sort();
            scopes.dedup();
            let key = CredentialPresenceKey::ProductAuth(
                owner_scope.clone(),
                requirement.provider.clone(),
                requirement.requester_extension.clone(),
                CredentialSetupKind::from_setup(&requirement.setup),
                scopes,
            );
            if let Some(present) = self.cache.get(&key) {
                any_missing |= !present;
                continue;
            }
            let Some(resolver) = self.resolver else {
                any_indeterminate = true; // silent-ok: no account resolver wired; a missing resolver is a wiring gap, not evidence the user lacks a credential
                continue;
            };
            // `account_configured` is side-effect-free (no token refresh, no
            // network staging) — safe to call on every capability-surface
            // render, unlike `resolve_access_secret` which performs an OAuth
            // refresh round-trip (#5416 Phase 2 Fix A).
            match resolver
                .account_configured(RuntimeCredentialAccountRequest {
                    scope,
                    provider: &requirement.provider,
                    setup: &requirement.setup,
                    provider_scopes: &requirement.provider_scopes,
                    requester_extension: &requirement.requester_extension,
                })
                .await
            {
                Ok(present) => {
                    self.cache.insert(key, present);
                    any_missing |= !present;
                }
                Err(CredentialStageError::Backend) => {
                    any_indeterminate = true; // silent-ok: internal staging failure is not attributable to the user's credentials
                }
                Err(CredentialStageError::AuthRequired) => {
                    // `account_configured`'s contract folds "no configured
                    // account" into `Ok(false)`; this arm only exists for
                    // match exhaustiveness. Treat defensively as
                    // indeterminate (fail open) rather than caching a
                    // possibly-wrong "missing" answer if a future impl
                    // violates the contract.
                    any_indeterminate = true; // silent-ok: contract-violation guard, not evidence of a missing user credential
                }
            }
        }

        if any_missing {
            CredentialPresenceStatus::Missing
        } else if any_indeterminate {
            CredentialPresenceStatus::Indeterminate
        } else {
            CredentialPresenceStatus::Present
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obligations::RuntimeCredentialAccessSecret;
    use ironclaw_host_api::{
        CapabilityId, EffectKind, ExtensionId, InvocationId, NetworkTargetPattern, PermissionMode,
        RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
        RuntimeCredentialRequirement, RuntimeCredentialRequirementSource, RuntimeCredentialTarget,
        RuntimeKind, SecretHandle, TrustClass, UserId,
    };

    /// A capability descriptor requiring a single product-auth credential
    /// scoped to `scope`, owned by the `gmail` extension against provider
    /// `google` — mirrors the real gmail manifest shape (multiple
    /// capabilities under the same provider/extension with different
    /// `provider_scopes`) used to reproduce the #5416 Phase 2 Fix B cache
    /// aliasing bug.
    fn descriptor_with_product_auth_scope(id: &str, scope: &str) -> CapabilityDescriptor {
        descriptor_with_product_auth_scopes(id, &[scope])
    }

    /// Same as [`descriptor_with_product_auth_scope`] but accepts the full
    /// `provider_scopes` list verbatim (including literal duplicates), so
    /// tests can pin the cache key's canonicalization behavior.
    fn descriptor_with_product_auth_scopes(id: &str, scopes: &[&str]) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::new(id).unwrap(),
            provider: ExtensionId::new("gmail").unwrap(),
            runtime: RuntimeKind::Wasm,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test capability".to_string(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectKind::DispatchCapability],
            default_permission: PermissionMode::Allow,
            runtime_credentials: vec![RuntimeCredentialRequirement {
                handle: SecretHandle::new("google_oauth_token").unwrap(),
                source: RuntimeCredentialRequirementSource::ProductAuthAccount {
                    provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
                    setup: RuntimeCredentialAccountSetup::ManualToken,
                },
                provider_scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
                audience: NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "gmail.googleapis.com".to_string(),
                    port: None,
                },
                target: RuntimeCredentialTarget::Header {
                    name: "Authorization".to_string(),
                    prefix: None,
                },
                required: true,
            }],
            resource_profile: None,
        }
    }

    fn test_resource_scope() -> ResourceScope {
        ResourceScope::local_default(UserId::new("user").unwrap(), InvocationId::new()).unwrap()
    }

    /// Resolver whose presence answer depends on the requested provider
    /// scopes — used to prove `ProductionCredentialPresence` does not alias
    /// one scope's cached answer onto a different scope under the same
    /// `(provider, requester_extension)`.
    #[derive(Debug)]
    struct ScopeGatedAccountResolver {
        present_scope: String,
    }

    #[async_trait]
    impl RuntimeCredentialAccountResolver for ScopeGatedAccountResolver {
        async fn resolve_access_secret(
            &self,
            request: RuntimeCredentialAccountRequest<'_>,
        ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
            if request.provider_scopes.contains(&self.present_scope) {
                Ok(RuntimeCredentialAccessSecret {
                    scope: request.scope.clone(),
                    handle: SecretHandle::new("google_oauth_token").unwrap(),
                })
            } else {
                Err(CredentialStageError::AuthRequired)
            }
        }

        async fn account_configured(
            &self,
            request: RuntimeCredentialAccountRequest<'_>,
        ) -> Result<bool, CredentialStageError> {
            Ok(request.provider_scopes.contains(&self.present_scope))
        }
    }

    /// Resolver that always reports the account as configured, counting how
    /// many times `account_configured` is actually invoked — used to prove a
    /// second lookup hit the cache instead of the backend.
    #[derive(Debug, Default)]
    struct CountingAccountResolver {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl RuntimeCredentialAccountResolver for CountingAccountResolver {
        async fn resolve_access_secret(
            &self,
            request: RuntimeCredentialAccountRequest<'_>,
        ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
            Ok(RuntimeCredentialAccessSecret {
                scope: request.scope.clone(),
                handle: SecretHandle::new("google_oauth_token").unwrap(),
            })
        }

        async fn account_configured(
            &self,
            _request: RuntimeCredentialAccountRequest<'_>,
        ) -> Result<bool, CredentialStageError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(true)
        }
    }

    /// #5416 Phase 2 Fix B (BLOCKER) regression: two capabilities that share
    /// `(provider, requester_extension)` but require different
    /// `provider_scopes` (the real gmail manifest shape — gmail.readonly /
    /// gmail.send / gmail.modify all under `provider="google"`,
    /// `requester_extension="gmail"`) must get independently correct presence
    /// answers within the same render. Before the fix, the cache key omitted
    /// scopes, so checking the readonly capability first would cache `true`
    /// under a key the send capability's lookup also hits — reporting the
    /// send capability wrongly `Available` (or vice versa, depending on
    /// check order).
    #[tokio::test]
    async fn required_credentials_present_does_not_alias_across_different_scopes() {
        let cache = CredentialPresenceCache::new();
        let resolver = ScopeGatedAccountResolver {
            present_scope: "gmail.readonly".to_string(),
        };
        let presence = ProductionCredentialPresence {
            secret_store: None,
            resolver: Some(&resolver),
            cache: &cache,
        };
        let scope = test_resource_scope();
        let readonly = descriptor_with_product_auth_scope("gmail.readonly_cap", "gmail.readonly");
        let send = descriptor_with_product_auth_scope("gmail.send_cap", "gmail.send");

        let readonly_present = presence
            .required_credentials_present(&scope, &readonly)
            .await;
        let send_present = presence.required_credentials_present(&scope, &send).await;

        assert_eq!(readonly_present, CredentialPresenceStatus::Present);
        assert_eq!(
            send_present,
            CredentialPresenceStatus::Missing,
            "the send-scope capability must not inherit the readonly capability's \
             cached presence answer"
        );
    }

    /// PR #5528 review regression: two requirements for the SAME scope, one
    /// declared once and one declared with a literal duplicate, must resolve
    /// through the identical cache key. Before the fix, `scopes.sort()` ran
    /// without a following `.dedup()`, so `["gmail.readonly"]` and
    /// `["gmail.readonly", "gmail.readonly"]` hashed to different keys —
    /// wasting a cache slot and a resolver round trip on a manifest-declared
    /// duplicate that carries no additional meaning.
    #[tokio::test]
    async fn required_credentials_present_key_is_canonical_across_duplicate_scope_declarations() {
        let cache = CredentialPresenceCache::new();
        let resolver = CountingAccountResolver::default();
        let presence = ProductionCredentialPresence {
            secret_store: None,
            resolver: Some(&resolver),
            cache: &cache,
        };
        let scope = test_resource_scope();
        let single = descriptor_with_product_auth_scope("gmail.single_cap", "gmail.readonly");
        let duplicated = descriptor_with_product_auth_scopes(
            "gmail.duplicated_cap",
            &["gmail.readonly", "gmail.readonly"],
        );

        let single_present = presence.required_credentials_present(&scope, &single).await;
        let duplicated_present = presence
            .required_credentials_present(&scope, &duplicated)
            .await;

        assert_eq!(single_present, CredentialPresenceStatus::Present);
        assert_eq!(duplicated_present, CredentialPresenceStatus::Present);
        assert_eq!(
            resolver.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a manifest-declared duplicate scope must not miss the cache entry the \
             canonical (deduplicated) scope list already populated"
        );
    }
}
