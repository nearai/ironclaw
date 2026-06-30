//! Reborn-owned Slack actor identity resolution.
//!
//! This module intentionally lives in Reborn composition instead of the legacy
//! `src/pairing` path. It adapts a host-owned integration-user identity lookup
//! to the product workflow's actor-to-user resolver contract.

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
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};
use thiserror::Error;

pub(crate) const SLACK_IDENTITY_PROVIDER: &str = "slack";
const SLACK_IDENTITY_CACHE_TTL: Duration = Duration::from_secs(30);

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
}

#[derive(Clone)]
pub struct SlackUserIdentityActorResolver {
    lookup: Arc<dyn RebornUserIdentityLookup>,
    resolved_user_cache: Arc<Mutex<HashMap<String, CachedSlackUserIdentity>>>,
    cache_ttl: Duration,
}

impl SlackUserIdentityActorResolver {
    pub fn new(lookup: Arc<dyn RebornUserIdentityLookup>) -> Self {
        Self {
            lookup,
            resolved_user_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl: SLACK_IDENTITY_CACHE_TTL,
        }
    }

    fn cached_user(&self, provider_user_id: &str) -> Result<Option<UserId>, ProductWorkflowError> {
        let mut cache = self.resolved_user_cache.lock().map_err(|_| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: "slack actor identity cache lock poisoned".into(),
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
                reason: "slack actor identity cache lock poisoned".into(),
            })?
            .insert(
                provider_user_id,
                CachedSlackUserIdentity {
                    user_id,
                    expires_at: Instant::now() + self.cache_ttl,
                },
            );
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CachedSlackUserIdentity {
    user_id: UserId,
    expires_at: Instant,
}

impl std::fmt::Debug for SlackUserIdentityActorResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SlackUserIdentityActorResolver(..)")
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for SlackUserIdentityActorResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.adapter_id.as_str() != SLACK_V2_ADAPTER_ID {
            return Ok(None);
        }
        if request.external_actor_ref.kind() != SLACK_USER_ACTOR_KIND {
            return Ok(None);
        }
        let provider_user_id = slack_user_identity_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        if let Some(user_id) = self.cached_user(&provider_user_id)? {
            return Ok(Some(user_id));
        }
        let resolved = self
            .lookup
            .resolve_user_identity(SLACK_IDENTITY_PROVIDER, &provider_user_id)
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

pub fn slack_user_identity_provider_user_id(
    installation_id: &AdapterInstallationId,
    slack_user_id: &str,
) -> String {
    format!("{}:{slack_user_id}", installation_id.as_str())
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ProductAdapterId};

    use super::*;

    #[tokio::test]
    async fn slack_actor_identity_resolver_uses_installation_scoped_provider_user_id() {
        let installation_id = installation("install-alpha");
        let lookup = Arc::new(RecordingLookup::new([(
            slack_user_identity_provider_user_id(&installation_id, "U123"),
            user("user:alice"),
        )]));
        let resolver = SlackUserIdentityActorResolver::new(lookup.clone());

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
    async fn slack_actor_identity_resolver_scopes_same_slack_user_per_installation() {
        let lookup = Arc::new(RecordingLookup::new([(
            "install-beta:U123".to_string(),
            user("user:bob"),
        )]));
        let resolver = SlackUserIdentityActorResolver::new(lookup);

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
    async fn slack_actor_identity_resolver_ignores_non_slack_actor_shapes() {
        let lookup = Arc::new(RecordingLookup::new([(
            "install-alpha:U123".to_string(),
            user("user:alice"),
        )]));
        let resolver = SlackUserIdentityActorResolver::new(lookup.clone());

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
    async fn slack_actor_identity_resolver_propagates_backend_error_as_binding_resolution_failed() {
        let resolver = SlackUserIdentityActorResolver::new(Arc::new(FailingLookup));

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
    async fn slack_actor_identity_resolver_caches_positive_user_resolution() {
        let installation_id = installation("install-alpha");
        let lookup = Arc::new(RecordingLookup::new([(
            slack_user_identity_provider_user_id(&installation_id, "U123"),
            user("user:alice"),
        )]));
        let resolver = SlackUserIdentityActorResolver::new(lookup.clone());
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
    }
}
