//! Provider-identity → Reborn user resolution for channel surfaces.
//!
//! One generic, manifest-parameterized [`ProductActorUserResolver`]: the
//! channel surface supplies the adapter id and external actor kind, the auth
//! surface supplies the provider id, and the resolver maps
//! `(provider, installation-scoped external actor id) → UserId` against the
//! host-owned identity binding store. Adapters extract protocol-shaped
//! external refs and stop there; resolution, binding, and scoping stay
//! host-owned and product-blind — a new channel gets identity binding by
//! declaring surfaces, not by writing a resolver.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductWorkflowError,
};
use thiserror::Error;

const PROVIDER_IDENTITY_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum RebornUserIdentityLookupError {
    #[error("reborn user identity backend unavailable: {0}")]
    Backend(String),
    #[error("stored user identity is invalid: {0}")]
    InvalidUserId(String),
}

#[async_trait::async_trait]
pub trait RebornUserIdentityLookup: Send + Sync {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError>;

    /// Whether the given IronClaw user has any binding for `provider` — the
    /// reverse of [`Self::resolve_user_identity`]. Used to tell whether the
    /// calling user has personally connected a channel.
    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError>;

    /// Like [`Self::user_has_provider_binding`], but only counts bindings
    /// whose provider user id starts with `provider_user_id_prefix` (the
    /// installation-scoped composite key prefix). Backends that cannot
    /// enumerate bindings report unavailability instead of guessing.
    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if provider_user_id_prefix.is_none() {
            return self.user_has_provider_binding(provider, user_id).await;
        }
        Err(RebornUserIdentityLookupError::Backend(
            "scoped provider binding lookup is unavailable".to_string(),
        ))
    }
}

/// The generic actor→user resolver for a channel surface.
///
/// Parameterized entirely by data (`provider`, `adapter_id`, `actor_kind`) so
/// per-channel resolver implementations are structurally unnecessary. Requests
/// for a different adapter or actor kind resolve to `None` so multiple
/// channel surfaces can stack their resolvers.
#[derive(Clone)]
pub struct ProviderIdentityActorResolver {
    provider: String,
    adapter_id: String,
    actor_kind: String,
    lookup: Arc<dyn RebornUserIdentityLookup>,
    resolved_user_cache: Arc<Mutex<HashMap<String, CachedProviderIdentity>>>,
    cache_ttl: Duration,
}

impl ProviderIdentityActorResolver {
    pub fn new(
        provider: impl Into<String>,
        adapter_id: impl Into<String>,
        actor_kind: impl Into<String>,
        lookup: Arc<dyn RebornUserIdentityLookup>,
    ) -> Self {
        Self {
            provider: provider.into(),
            adapter_id: adapter_id.into(),
            actor_kind: actor_kind.into(),
            lookup,
            resolved_user_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl: PROVIDER_IDENTITY_CACHE_TTL,
        }
    }

    fn cached_user(&self, provider_user_id: &str) -> Result<Option<UserId>, ProductWorkflowError> {
        let mut cache = self.resolved_user_cache.lock().map_err(|_| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: "provider identity cache lock poisoned".into(),
            }
        })?;
        let Some(cached) = cache.get(provider_user_id) else {
            return Ok(None);
        };
        if cached.expires_at <= Instant::now() {
            cache.remove(provider_user_id);
            return Ok(None);
        }
        Ok(Some(cached.user_id.clone()))
    }

    fn cache_user(
        &self,
        provider_user_id: String,
        user_id: UserId,
    ) -> Result<(), ProductWorkflowError> {
        self.resolved_user_cache
            .lock()
            .map_err(|_| ProductWorkflowError::BindingResolutionFailed {
                reason: "provider identity cache lock poisoned".into(),
            })?
            .insert(
                provider_user_id,
                CachedProviderIdentity {
                    user_id,
                    expires_at: Instant::now() + self.cache_ttl,
                },
            );
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CachedProviderIdentity {
    user_id: UserId,
    expires_at: Instant,
}

impl std::fmt::Debug for ProviderIdentityActorResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderIdentityActorResolver")
            .field("provider", &self.provider)
            .field("adapter_id", &self.adapter_id)
            .field("actor_kind", &self.actor_kind)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for ProviderIdentityActorResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.adapter_id.as_str() != self.adapter_id {
            return Ok(None);
        }
        if request.external_actor_ref.kind() != self.actor_kind {
            return Ok(None);
        }
        let provider_user_id = installation_scoped_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        if let Some(user_id) = self.cached_user(&provider_user_id)? {
            return Ok(Some(user_id));
        }
        let resolved = self
            .lookup
            .resolve_user_identity(&self.provider, &provider_user_id)
            .await
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            })?;
        if let Some(user_id) = resolved.as_ref() {
            self.cache_user(provider_user_id, user_id.clone())?;
        }
        Ok(resolved)
    }
}

/// Installation-scoped composite key for a provider identity binding: the
/// same external user id under two adapter installations is two bindings.
pub fn installation_scoped_provider_user_id(
    installation_id: &AdapterInstallationId,
    external_actor_id: &str,
) -> String {
    format!("{}:{external_actor_id}", installation_id.as_str())
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ProductAdapterId};

    use super::*;

    fn resolver(lookup: Arc<dyn RebornUserIdentityLookup>) -> ProviderIdentityActorResolver {
        ProviderIdentityActorResolver::new("slack", "slack_v2", "slack_user", lookup)
    }

    #[tokio::test]
    async fn resolver_uses_installation_scoped_provider_user_id() {
        let installation_id = installation("install-alpha");
        let lookup = Arc::new(RecordingLookup::new([(
            installation_scoped_provider_user_id(&installation_id, "U123"),
            user("user:alice"),
        )]));
        let resolver = resolver(lookup.clone());

        let resolved = resolver
            .resolve_product_actor_user(request("slack_v2", installation_id, "slack_user", "U123"))
            .await
            .expect("resolution succeeds");

        assert_eq!(resolved, Some(user("user:alice")));
        assert_eq!(
            lookup.calls(),
            vec![("slack".to_string(), "install-alpha:U123".to_string())]
        );
    }

    #[tokio::test]
    async fn resolver_scopes_same_external_user_per_installation() {
        let lookup = Arc::new(RecordingLookup::new([(
            "install-beta:U123".to_string(),
            user("user:bob"),
        )]));
        let resolver = resolver(lookup);

        let resolved = resolver
            .resolve_product_actor_user(request(
                "slack_v2",
                installation("install-alpha"),
                "slack_user",
                "U123",
            ))
            .await
            .expect("resolution succeeds");

        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn resolver_ignores_other_adapters_and_actor_kinds() {
        let lookup = Arc::new(RecordingLookup::new([(
            "install-alpha:U123".to_string(),
            user("user:alice"),
        )]));
        let resolver = resolver(lookup.clone());

        assert_eq!(
            resolver
                .resolve_product_actor_user(request(
                    "telegram_v2",
                    installation("install-alpha"),
                    "slack_user",
                    "U123",
                ))
                .await
                .expect("resolution succeeds"),
            None
        );
        assert_eq!(
            resolver
                .resolve_product_actor_user(request(
                    "slack_v2",
                    installation("install-alpha"),
                    "telegram_user",
                    "U123",
                ))
                .await
                .expect("resolution succeeds"),
            None
        );
        assert!(lookup.calls().is_empty());
    }

    #[tokio::test]
    async fn resolver_propagates_backend_error_as_binding_resolution_failed() {
        let resolver = resolver(Arc::new(FailingLookup));

        let err = resolver
            .resolve_product_actor_user(request(
                "slack_v2",
                installation("install-alpha"),
                "slack_user",
                "U123",
            ))
            .await
            .expect_err("backend error should propagate");

        assert!(matches!(
            err,
            ProductWorkflowError::BindingResolutionFailed { .. }
        ));
    }

    #[tokio::test]
    async fn resolver_caches_positive_user_resolution() {
        let installation_id = installation("install-alpha");
        let lookup = Arc::new(RecordingLookup::new([(
            installation_scoped_provider_user_id(&installation_id, "U123"),
            user("user:alice"),
        )]));
        let resolver = resolver(lookup.clone());
        let request = request("slack_v2", installation_id, "slack_user", "U123");

        let first = resolver
            .resolve_product_actor_user(request.clone())
            .await
            .expect("first resolution succeeds");
        let second = resolver
            .resolve_product_actor_user(request)
            .await
            .expect("second resolution succeeds");

        assert_eq!(first, Some(user("user:alice")));
        assert_eq!(second, Some(user("user:alice")));
        assert_eq!(
            lookup.calls(),
            vec![("slack".to_string(), "install-alpha:U123".to_string())]
        );
    }

    fn request(
        adapter_id: &str,
        installation_id: AdapterInstallationId,
        actor_kind: &str,
        actor_id: &str,
    ) -> ProductActorUserResolutionRequest {
        ProductActorUserResolutionRequest::new(
            ProductAdapterId::new(adapter_id).expect("adapter"),
            installation_id,
            ExternalActorRef::new(actor_kind, actor_id, None::<String>).expect("actor"),
        )
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("installation")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("user")
    }

    #[derive(Debug, Default)]
    struct RecordingLookup {
        bindings: HashMap<String, UserId>,
        calls: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl RecordingLookup {
        fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
            Self {
                bindings: bindings.into_iter().collect(),
                calls: std::sync::Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for RecordingLookup {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((provider.to_string(), provider_user_id.to_string()));
            Ok(self.bindings.get(provider_user_id).cloned())
        }

        async fn user_has_provider_binding(
            &self,
            _provider: &str,
            user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Ok(self.bindings.values().any(|bound| bound == user_id))
        }
    }

    #[derive(Debug)]
    struct FailingLookup;

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for FailingLookup {
        async fn resolve_user_identity(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            Err(RebornUserIdentityLookupError::Backend("db down".into()))
        }

        async fn user_has_provider_binding(
            &self,
            _provider: &str,
            _user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Err(RebornUserIdentityLookupError::Backend("db down".into()))
        }
    }
}
