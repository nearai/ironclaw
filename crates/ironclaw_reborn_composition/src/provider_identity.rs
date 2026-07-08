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

use std::sync::Arc;

use ironclaw_conversations::ExternalActorBindingEpoch;
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductWorkflowError,
    ResolvedProductActorUser,
};
use thiserror::Error;

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

    async fn resolve_user_identity_with_binding_epoch(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<(UserId, Option<ExternalActorBindingEpoch>)>, RebornUserIdentityLookupError>
    {
        self.resolve_user_identity(provider, provider_user_id)
            .await
            .map(|resolved| resolved.map(|user_id| (user_id, None)))
    }

    async fn user_identity_binding_epoch_is_current(
        &self,
        provider: &str,
        provider_user_id: &str,
        expected_user_id: &UserId,
        expected_epoch: &ExternalActorBindingEpoch,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        Ok(self
            .resolve_user_identity_with_binding_epoch(provider, provider_user_id)
            .await?
            .is_some_and(|(user_id, epoch)| {
                user_id == *expected_user_id && epoch.as_ref() == Some(expected_epoch)
            }))
    }

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
        }
    }
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
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
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
        self.lookup
            .resolve_user_identity_with_binding_epoch(&self.provider, &provider_user_id)
            .await
            .map(|resolved| {
                resolved.map(|(user_id, binding_epoch)| match binding_epoch {
                    Some(binding_epoch) => {
                        ResolvedProductActorUser::with_binding_epoch(user_id, binding_epoch)
                    }
                    None => ResolvedProductActorUser::new(user_id),
                })
            })
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            })
    }

    async fn resolved_product_actor_user_is_current(
        &self,
        request: &ProductActorUserResolutionRequest,
        expected: &ResolvedProductActorUser,
    ) -> Result<bool, ProductWorkflowError> {
        if request.adapter_id.as_str() != self.adapter_id
            || request.external_actor_ref.kind() != self.actor_kind
        {
            return Ok(false);
        }
        let Some(expected_epoch) = expected.binding_epoch.as_ref() else {
            return Ok(self
                .resolve_product_actor_user(request.clone())
                .await?
                .as_ref()
                == Some(expected));
        };
        let provider_user_id = installation_scoped_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        self.lookup
            .user_identity_binding_epoch_is_current(
                &self.provider,
                &provider_user_id,
                &expected.user_id,
                expected_epoch,
            )
            .await
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            })
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

pub(crate) fn parse_installation_scoped_provider_user_id(
    provider_user_id: &str,
) -> Option<(AdapterInstallationId, &str)> {
    let (installation_id, slack_user_id) = provider_user_id.rsplit_once(':')?;
    if slack_user_id.is_empty() {
        return None;
    }
    Some((
        AdapterInstallationId::new(installation_id.to_string()).ok()?,
        slack_user_id,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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

        assert_eq!(
            resolved,
            Some(ResolvedProductActorUser::new(user("user:alice")))
        );
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
    async fn provider_identity_resolver_rereads_after_revocation() {
        let installation_id = installation("install-alpha");
        let provider_user_id = installation_scoped_provider_user_id(&installation_id, "U123");
        let lookup = Arc::new(RecordingLookup::new([(
            provider_user_id.clone(),
            user("user:alice"),
        )]));
        let resolver = resolver(lookup.clone());
        let request = request("slack_v2", installation_id, "slack_user", "U123");

        let first = resolver
            .resolve_product_actor_user(request.clone())
            .await
            .expect("first resolution succeeds");
        lookup.remove_binding(&provider_user_id);
        let second = resolver
            .resolve_product_actor_user(request)
            .await
            .expect("second resolution succeeds");

        assert_eq!(
            first,
            Some(ResolvedProductActorUser::new(user("user:alice")))
        );
        assert_eq!(second, None);
        assert_eq!(
            lookup.calls(),
            vec![
                ("slack".to_string(), "install-alpha:U123".to_string()),
                ("slack".to_string(), "install-alpha:U123".to_string()),
            ],
            "Slack identity resolution must observe a freshly revoked binding on the next message"
        );
    }

    #[tokio::test]
    async fn slack_actor_epoch_recheck_avoids_a_second_canonical_identity_read() {
        let lookup = Arc::new(RecordingLookup::new([(
            "install-alpha:U123".to_string(),
            user("user:alice"),
        )]));
        let resolver = resolver(lookup.clone());
        let request = request(
            "slack_v2",
            installation("install-alpha"),
            "slack_user",
            "U123",
        );
        let expected = ResolvedProductActorUser::with_binding_epoch(
            user("user:alice"),
            ExternalActorBindingEpoch::new("epoch-1").expect("epoch"),
        );

        assert!(
            resolver
                .resolved_product_actor_user_is_current(&request, &expected)
                .await
                .expect("epoch validation")
        );
        assert!(
            lookup.calls().is_empty(),
            "generation recheck must validate owner authority without rereading the identity record"
        );
        assert_eq!(lookup.epoch_check_calls(), 1);
    }

    #[test]
    fn installation_scoped_provider_user_id_parser_is_reversible_for_delimited_installation_ids() {
        let installation_id = installation("org:install-alpha");
        let provider_user_id = installation_scoped_provider_user_id(&installation_id, "U123");

        let (parsed_installation_id, slack_user_id) =
            parse_installation_scoped_provider_user_id(&provider_user_id)
                .expect("provider user id parses");

        assert_eq!(parsed_installation_id, installation_id);
        assert_eq!(slack_user_id, "U123");
    }

    #[test]
    fn installation_scoped_provider_user_id_parser_rejects_malformed_values() {
        assert_eq!(parse_installation_scoped_provider_user_id("U123"), None);
        assert_eq!(
            parse_installation_scoped_provider_user_id("install-alpha:"),
            None
        );
        assert_eq!(parse_installation_scoped_provider_user_id(":U123"), None);
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
        bindings: std::sync::Mutex<HashMap<String, UserId>>,
        calls: std::sync::Mutex<Vec<(String, String)>>,
        epoch_check_calls: std::sync::Mutex<usize>,
    }

    impl RecordingLookup {
        fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
            Self {
                bindings: std::sync::Mutex::new(bindings.into_iter().collect()),
                calls: std::sync::Mutex::default(),
                epoch_check_calls: std::sync::Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }

        fn remove_binding(&self, provider_user_id: &str) {
            self.bindings
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(provider_user_id);
        }

        fn epoch_check_calls(&self) -> usize {
            *self
                .epoch_check_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
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
            Ok(self
                .bindings
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(provider_user_id)
                .cloned())
        }

        async fn user_has_provider_binding(
            &self,
            _provider: &str,
            user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Ok(self
                .bindings
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .values()
                .any(|bound| bound == user_id))
        }

        async fn user_identity_binding_epoch_is_current(
            &self,
            _provider: &str,
            _provider_user_id: &str,
            _expected_user_id: &UserId,
            _expected_epoch: &ExternalActorBindingEpoch,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            *self
                .epoch_check_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
            Ok(true)
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
