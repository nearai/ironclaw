//! Provider-identity → Reborn user resolution for channel surfaces.
//!
//! Composition explicitly constructs one product-blind [`ProductActorUserResolver`]
//! per channel surface from its adapter id, external actor kind, and identity
//! provider. The resolver maps
//! `(provider, installation-scoped external actor id) → UserId` against the
//! host-owned identity binding store. Adapters extract protocol-shaped
//! external refs and stop there; resolution, binding, and scoping stay
//! host-owned and product-blind.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_conversations::ExternalActorBindingEpoch;
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductWorkflowError,
    ResolvedProductActorUser,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum RebornUserIdentityLookupError {
    #[error("reborn user identity backend unavailable: {0}")]
    Backend(String),
    #[error("stored user identity is invalid: {0}")]
    InvalidUserId(String),
}

#[async_trait::async_trait]
pub(crate) trait RebornUserIdentityLookup: Send + Sync {
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

    /// Whether a binding record exists even when it is revoked or its epoch is
    /// stale. Compatibility fallback must consult record presence so an older
    /// key can never resurrect authority shadowed by a canonical record.
    async fn provider_user_identity_record_exists(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<bool, RebornUserIdentityLookupError>;

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
pub(crate) struct ProviderIdentityActorResolver {
    provider: String,
    adapter_id: String,
    actor_kind: String,
    lookup: Arc<dyn RebornUserIdentityLookup>,
}

impl ProviderIdentityActorResolver {
    pub(crate) fn new(
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
        let canonical = self
            .lookup
            .resolve_user_identity_with_binding_epoch(&self.provider, &provider_user_id)
            .await
            .map_err(binding_resolution_failed)?;
        let resolved = if canonical.is_some() {
            canonical
        } else if let Some(legacy_provider_user_id) = legacy_installation_scoped_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        ) {
            if self
                .lookup
                .provider_user_identity_record_exists(&self.provider, &provider_user_id)
                .await
                .map_err(binding_resolution_failed)?
            {
                None
            } else {
                let legacy = self
                    .lookup
                    .resolve_user_identity_with_binding_epoch(
                        &self.provider,
                        &legacy_provider_user_id,
                    )
                    .await
                    .map_err(binding_resolution_failed)?;
                if legacy.is_none() {
                    None
                } else {
                    // Close the new-write race around compatibility lookup:
                    // canonical authority wins if it appeared while the legacy
                    // record was being read, including a revoked/stale record.
                    let canonical_retry = self
                        .lookup
                        .resolve_user_identity_with_binding_epoch(&self.provider, &provider_user_id)
                        .await
                        .map_err(binding_resolution_failed)?;
                    if canonical_retry.is_some() {
                        canonical_retry
                    } else if self
                        .lookup
                        .provider_user_identity_record_exists(&self.provider, &provider_user_id)
                        .await
                        .map_err(binding_resolution_failed)?
                    {
                        None
                    } else {
                        legacy
                    }
                }
            }
        } else {
            None
        };
        Ok(
            resolved.map(|(user_id, binding_epoch)| match binding_epoch {
                Some(binding_epoch) => {
                    ResolvedProductActorUser::with_binding_epoch(user_id, binding_epoch)
                }
                None => ResolvedProductActorUser::new(user_id),
            }),
        )
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
        if self
            .lookup
            .user_identity_binding_epoch_is_current(
                &self.provider,
                &provider_user_id,
                &expected.user_id,
                expected_epoch,
            )
            .await
            .map_err(binding_resolution_failed)?
        {
            return Ok(true);
        }
        let Some(legacy_provider_user_id) = legacy_installation_scoped_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        ) else {
            return Ok(false);
        };
        if self
            .lookup
            .provider_user_identity_record_exists(&self.provider, &provider_user_id)
            .await
            .map_err(binding_resolution_failed)?
        {
            return Ok(false);
        }
        if !self
            .lookup
            .user_identity_binding_epoch_is_current(
                &self.provider,
                &legacy_provider_user_id,
                &expected.user_id,
                expected_epoch,
            )
            .await
            .map_err(binding_resolution_failed)?
        {
            return Ok(false);
        }
        Ok(!self
            .lookup
            .provider_user_identity_record_exists(&self.provider, &provider_user_id)
            .await
            .map_err(binding_resolution_failed)?)
    }
}

fn binding_resolution_failed(error: RebornUserIdentityLookupError) -> ProductWorkflowError {
    ProductWorkflowError::BindingResolutionFailed {
        reason: error.to_string(),
    }
}

const CANONICAL_PROVIDER_USER_ID_VERSION: &str = "ic1";
const MAX_INSTALLATION_ID_BYTES: usize = 256;
const MAX_EXTERNAL_ACTOR_ID_BYTES: usize = 512;
const MAX_INSTALLATION_LENGTH_DIGITS: usize = 3;
const MAX_CANONICAL_PROVIDER_USER_ID_BYTES: usize = 1_034;
const MAX_LEGACY_PROVIDER_USER_ID_BYTES: usize =
    MAX_INSTALLATION_ID_BYTES + 1 + MAX_EXTERNAL_ACTOR_ID_BYTES;

/// Installation-scoped composite key for a provider identity binding: the
/// same external user id under two adapter installations is two bindings.
pub(crate) fn installation_scoped_provider_user_id(
    installation_id: &AdapterInstallationId,
    external_actor_id: &str,
) -> String {
    format!(
        "{CANONICAL_PROVIDER_USER_ID_VERSION}.{}.{}.{}",
        installation_id.as_str().len(),
        URL_SAFE_NO_PAD.encode(installation_id.as_str()),
        URL_SAFE_NO_PAD.encode(external_actor_id)
    )
}

pub(crate) fn installation_scoped_provider_user_id_prefix(
    installation_id: &AdapterInstallationId,
) -> String {
    format!(
        "{CANONICAL_PROVIDER_USER_ID_VERSION}.{}.{}.",
        installation_id.as_str().len(),
        URL_SAFE_NO_PAD.encode(installation_id.as_str())
    )
}

pub(crate) fn legacy_installation_scoped_provider_user_id(
    installation_id: &AdapterInstallationId,
    external_actor_id: &str,
) -> Option<String> {
    if installation_id.as_str().contains(':')
        || !external_actor_id_is_safe_legacy(external_actor_id)
    {
        return None;
    }
    Some(format!("{}:{external_actor_id}", installation_id.as_str()))
}

pub(crate) fn legacy_installation_scoped_provider_user_id_prefix(
    installation_id: &AdapterInstallationId,
) -> Option<String> {
    (!installation_id.as_str().contains(':')).then(|| format!("{}:", installation_id.as_str()))
}

pub(crate) fn parse_installation_scoped_provider_user_id(
    provider_user_id: &str,
) -> Option<(AdapterInstallationId, String)> {
    if provider_user_id.len() > MAX_CANONICAL_PROVIDER_USER_ID_BYTES {
        return None;
    }
    let mut components = provider_user_id.split('.');
    if components.next()? != CANONICAL_PROVIDER_USER_ID_VERSION {
        return None;
    }
    let declared_length = components.next()?;
    if declared_length.is_empty() || declared_length.len() > MAX_INSTALLATION_LENGTH_DIGITS {
        return None;
    }
    let declared_length = declared_length.parse::<usize>().ok()?;
    if !(1..=MAX_INSTALLATION_ID_BYTES).contains(&declared_length) {
        return None;
    }
    let encoded_installation_id = components.next()?;
    let encoded_external_actor_id = components.next()?;
    if components.next().is_some() || encoded_external_actor_id.is_empty() {
        return None;
    }
    let installation_bytes = URL_SAFE_NO_PAD.decode(encoded_installation_id).ok()?;
    if installation_bytes.len() != declared_length {
        return None;
    }
    let actor_bytes = URL_SAFE_NO_PAD.decode(encoded_external_actor_id).ok()?;
    if actor_bytes.is_empty() || actor_bytes.len() > MAX_EXTERNAL_ACTOR_ID_BYTES {
        return None;
    }
    let installation_id =
        AdapterInstallationId::new(String::from_utf8(installation_bytes).ok()?).ok()?;
    let external_actor_id = String::from_utf8(actor_bytes).ok()?;
    if !external_actor_id_is_valid(&external_actor_id)
        || installation_scoped_provider_user_id(&installation_id, &external_actor_id)
            != provider_user_id
    {
        return None;
    }
    Some((installation_id, external_actor_id))
}

pub(crate) fn parse_legacy_installation_scoped_provider_user_id(
    provider_user_id: &str,
) -> Option<(AdapterInstallationId, String)> {
    if provider_user_id.len() > MAX_LEGACY_PROVIDER_USER_ID_BYTES {
        return None;
    }
    let (installation_id, external_actor_id) = provider_user_id.split_once(':')?;
    if installation_id.contains(':')
        || external_actor_id.contains(':')
        || !external_actor_id_is_valid(external_actor_id)
    {
        return None;
    }
    Some((
        AdapterInstallationId::new(installation_id.to_string()).ok()?,
        external_actor_id.to_string(),
    ))
}

pub(crate) fn parse_any_installation_scoped_provider_user_id(
    provider_user_id: &str,
) -> Option<(ProviderIdentityKeyGeneration, AdapterInstallationId, String)> {
    if let Some((installation_id, actor_id)) =
        parse_installation_scoped_provider_user_id(provider_user_id)
    {
        return Some((
            ProviderIdentityKeyGeneration::Canonical,
            installation_id,
            actor_id,
        ));
    }
    parse_legacy_installation_scoped_provider_user_id(provider_user_id).map(
        |(installation_id, actor_id)| {
            (
                ProviderIdentityKeyGeneration::Legacy,
                installation_id,
                actor_id,
            )
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderIdentityKeyGeneration {
    Canonical,
    Legacy,
}

pub(crate) fn installation_scoped_provider_user_id_matches_prefix(
    provider_user_id: &str,
    provider_user_id_prefix: &str,
) -> bool {
    let Some(expected_installation_id) =
        parse_installation_scoped_provider_user_id_prefix(provider_user_id_prefix)
    else {
        return false;
    };
    parse_any_installation_scoped_provider_user_id(provider_user_id)
        .is_some_and(|(_, installation_id, _)| installation_id == expected_installation_id)
}

fn parse_installation_scoped_provider_user_id_prefix(
    provider_user_id_prefix: &str,
) -> Option<AdapterInstallationId> {
    if let Some(canonical_without_actor) = provider_user_id_prefix.strip_suffix('.') {
        let candidate = format!("{canonical_without_actor}.YQ");
        if let Some((installation_id, actor_id)) =
            parse_installation_scoped_provider_user_id(&candidate)
            && actor_id == "a"
        {
            return Some(installation_id);
        }
    }
    let installation_id = provider_user_id_prefix.strip_suffix(':')?;
    if installation_id.contains(':') {
        return None;
    }
    let installation_id = AdapterInstallationId::new(installation_id.to_string()).ok()?;
    (legacy_installation_scoped_provider_user_id_prefix(&installation_id).as_deref()
        == Some(provider_user_id_prefix))
    .then_some(installation_id)
}

fn external_actor_id_is_safe_legacy(external_actor_id: &str) -> bool {
    !external_actor_id.contains(':') && external_actor_id_is_valid(external_actor_id)
}

pub(crate) fn external_actor_id_is_valid(external_actor_id: &str) -> bool {
    !external_actor_id.is_empty()
        && external_actor_id.len() <= MAX_EXTERNAL_ACTOR_ID_BYTES
        && !external_actor_id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

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
            vec![(
                "slack".to_string(),
                installation_scoped_provider_user_id(&installation("install-alpha"), "U123")
            )]
        );
    }

    #[tokio::test]
    async fn resolver_prefers_canonical_binding_over_legacy_binding() {
        let installation_id = installation("install-alpha");
        let canonical = installation_scoped_provider_user_id(&installation_id, "U123");
        let legacy = legacy_installation_scoped_provider_user_id(&installation_id, "U123")
            .expect("legacy key is safe");
        let lookup = Arc::new(RecordingLookup::new([
            (canonical.clone(), user("user:alice")),
            (legacy, user("user:mallory")),
        ]));
        let resolver = resolver(lookup.clone());

        let resolved = resolver
            .resolve_product_actor_user(request("slack_v2", installation_id, "slack_user", "U123"))
            .await
            .expect("resolution succeeds");

        assert_eq!(
            resolved.map(|resolved| resolved.user_id),
            Some(user("user:alice"))
        );
        assert_eq!(lookup.calls(), vec![("slack".to_string(), canonical)]);
    }

    #[tokio::test]
    async fn resolver_falls_back_to_unambiguous_legacy_binding_only_when_canonical_is_absent() {
        let installation_id = installation("install-alpha");
        let canonical = installation_scoped_provider_user_id(&installation_id, "U123");
        let legacy = legacy_installation_scoped_provider_user_id(&installation_id, "U123")
            .expect("legacy key is safe");
        let lookup = Arc::new(RecordingLookup::new([(legacy.clone(), user("user:alice"))]));
        let resolver = resolver(lookup.clone());

        let resolved = resolver
            .resolve_product_actor_user(request("slack_v2", installation_id, "slack_user", "U123"))
            .await
            .expect("resolution succeeds");

        assert_eq!(
            resolved.map(|resolved| resolved.user_id),
            Some(user("user:alice"))
        );
        assert_eq!(
            lookup.calls(),
            vec![
                ("slack".to_string(), canonical.clone()),
                ("slack".to_string(), legacy),
                ("slack".to_string(), canonical),
            ]
        );
    }

    #[tokio::test]
    async fn resolver_does_not_fall_back_after_canonical_revocation() {
        let installation_id = installation("install-alpha");
        let canonical = installation_scoped_provider_user_id(&installation_id, "U123");
        let legacy = legacy_installation_scoped_provider_user_id(&installation_id, "U123")
            .expect("legacy key is safe");
        let lookup = Arc::new(RecordingLookup::new([
            (canonical.clone(), user("user:alice")),
            (legacy, user("user:mallory")),
        ]));
        lookup.revoke_binding(&canonical);
        let resolver = resolver(lookup.clone());

        let resolved = resolver
            .resolve_product_actor_user(request("slack_v2", installation_id, "slack_user", "U123"))
            .await
            .expect("resolution succeeds");

        assert_eq!(resolved, None);
        assert_eq!(lookup.calls(), vec![("slack".to_string(), canonical)]);
    }

    #[tokio::test]
    async fn resolver_never_falls_back_for_ambiguous_legacy_components() {
        for (installation_id, actor_id) in [(installation("a:b"), "c"), (installation("a"), "b:c")]
        {
            let ambiguous_legacy = format!("{}:{actor_id}", installation_id.as_str());
            let canonical = installation_scoped_provider_user_id(&installation_id, actor_id);
            let lookup = Arc::new(RecordingLookup::new([(
                ambiguous_legacy,
                user("user:mallory"),
            )]));
            let resolver = resolver(lookup.clone());

            assert_eq!(
                resolver
                    .resolve_product_actor_user(request(
                        "slack_v2",
                        installation_id,
                        "slack_user",
                        actor_id,
                    ))
                    .await
                    .expect("resolution succeeds"),
                None
            );
            assert_eq!(lookup.calls(), vec![("slack".to_string(), canonical)]);
        }
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
        lookup.revoke_binding(&provider_user_id);
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
                ("slack".to_string(), provider_user_id.clone()),
                ("slack".to_string(), provider_user_id),
            ],
            "Slack identity resolution must observe a freshly revoked binding on the next message"
        );
    }

    #[tokio::test]
    async fn slack_actor_epoch_recheck_avoids_a_second_canonical_identity_read() {
        let lookup = Arc::new(RecordingLookup::new([(
            installation_scoped_provider_user_id(&installation("install-alpha"), "U123"),
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
    fn installation_scoped_provider_user_id_avoids_delimiter_collision() {
        let left = installation_scoped_provider_user_id(&installation("a:b"), "c");
        let right = installation_scoped_provider_user_id(&installation("a"), "b:c");

        assert_ne!(left, right);
    }

    #[test]
    fn installation_scoped_provider_user_id_uses_installation_byte_length() {
        let provider_user_id =
            installation_scoped_provider_user_id(&installation("org:install-alpha"), "U123");

        assert_eq!(provider_user_id, "ic1.17.b3JnOmluc3RhbGwtYWxwaGE.VTEyMw");
        assert!(!provider_user_id.contains(':'));
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
    fn installation_scoped_provider_user_id_parser_round_trips_unicode_by_byte_length() {
        let installation_id = installation("組織:導入");
        let provider_user_id = installation_scoped_provider_user_id(&installation_id, "利用者:甲");

        let (parsed_installation_id, actor_id) =
            parse_installation_scoped_provider_user_id(&provider_user_id)
                .expect("provider user id parses");

        assert_eq!(parsed_installation_id, installation_id);
        assert_eq!(actor_id, "利用者:甲");
    }

    #[test]
    fn installation_scoped_provider_user_id_parser_rejects_malformed_values() {
        assert_eq!(parse_installation_scoped_provider_user_id("U123"), None);
        assert_eq!(
            parse_installation_scoped_provider_user_id("install-alpha:"),
            None
        );
        assert_eq!(parse_installation_scoped_provider_user_id(":U123"), None);
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.x.YWJj.VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.4.YWJj.VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.3.YWJj"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.3.YWJj.VTEyMw.extra"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.3.@@@.VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.1.w6k.VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.1._w.VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.0..VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.3.YWJj."),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.3.YWJj=.VTEyMw"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id("ic1.3.YWJj.Cg"),
            None
        );
        assert_eq!(
            parse_installation_scoped_provider_user_id(&format!("ic1.3.YWJj.{}", "YQ".repeat(700))),
            None
        );
    }

    #[test]
    fn safe_legacy_helpers_reject_ambiguous_components_and_parse_exact_keys() {
        let installation_id = installation("install-alpha");
        let key = legacy_installation_scoped_provider_user_id(&installation_id, "U123")
            .expect("legacy key is unambiguous");

        assert_eq!(key, "install-alpha:U123");
        assert_eq!(
            legacy_installation_scoped_provider_user_id_prefix(&installation_id).as_deref(),
            Some("install-alpha:")
        );
        assert_eq!(
            parse_legacy_installation_scoped_provider_user_id(&key),
            Some((installation_id, "U123".to_string()))
        );
        assert_eq!(
            legacy_installation_scoped_provider_user_id(&installation("install:alpha"), "U123"),
            None
        );
        assert_eq!(
            legacy_installation_scoped_provider_user_id(&installation("install-alpha"), "U:123"),
            None
        );
        assert_eq!(
            legacy_installation_scoped_provider_user_id_prefix(&installation("install:alpha")),
            None
        );
        assert_eq!(
            parse_legacy_installation_scoped_provider_user_id("install:alpha:U123"),
            None
        );
        assert_eq!(
            parse_legacy_installation_scoped_provider_user_id("install-alpha:"),
            None
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
        bindings: std::sync::Mutex<HashMap<String, UserId>>,
        records: std::sync::Mutex<HashSet<String>>,
        calls: std::sync::Mutex<Vec<(String, String)>>,
        epoch_check_calls: std::sync::Mutex<usize>,
    }

    impl RecordingLookup {
        fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
            let bindings = bindings.into_iter().collect::<HashMap<_, _>>();
            Self {
                records: std::sync::Mutex::new(bindings.keys().cloned().collect()),
                bindings: std::sync::Mutex::new(bindings),
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

        fn revoke_binding(&self, provider_user_id: &str) {
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

        async fn provider_user_identity_record_exists(
            &self,
            _provider: &str,
            provider_user_id: &str,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Ok(self
                .records
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(provider_user_id))
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

        async fn provider_user_identity_record_exists(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Err(RebornUserIdentityLookupError::Backend("db down".into()))
        }
    }
}
