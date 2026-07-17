//! Telegram external-actor → Reborn user resolution.
//!
//! Mirrors the Slack actor resolver: adapter/actor-kind gated, re-read on
//! every inbound update (revocation is observed immediately), with the
//! binding-epoch fast path for `is_current` rechecks. Provider identity is
//! installation-scoped (`{installation}:{telegram_user_id}` under provider
//! `telegram`), and the installation id embeds the bot id — so rotating the
//! same bot's token preserves pairings while a bot swap orphans them.

use std::sync::Arc;

use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductWorkflowError,
    ResolvedProductActorUser,
};

use ironclaw_channel_host::identity::RebornUserIdentityLookup;

/// Identity provider key for Telegram bindings. Deliberately the bare vendor
/// name (never `telegram_personal`/`telegram_bot` — retired taxonomy).
pub const TELEGRAM_IDENTITY_PROVIDER: &str = "telegram";

/// The host-wired adapter id for the Telegram v2 adapter instance.
pub const TELEGRAM_V2_ADAPTER_ID: &str = "telegram_v2";

#[derive(Clone)]
pub struct TelegramUserIdentityActorResolver {
    lookup: Arc<dyn RebornUserIdentityLookup>,
}

impl TelegramUserIdentityActorResolver {
    pub fn new(lookup: Arc<dyn RebornUserIdentityLookup>) -> Self {
        Self { lookup }
    }
}

impl std::fmt::Debug for TelegramUserIdentityActorResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TelegramUserIdentityActorResolver(..)")
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for TelegramUserIdentityActorResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
        if request.adapter_id.as_str() != TELEGRAM_V2_ADAPTER_ID {
            return Ok(None);
        }
        if request.external_actor_ref.kind()
            != ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND
        {
            return Ok(None);
        }
        let provider_user_id = telegram_user_identity_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        self.lookup
            .resolve_user_identity_with_binding_epoch(TELEGRAM_IDENTITY_PROVIDER, &provider_user_id)
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
        if request.adapter_id.as_str() != TELEGRAM_V2_ADAPTER_ID
            || request.external_actor_ref.kind()
                != ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND
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
        let provider_user_id = telegram_user_identity_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        self.lookup
            .user_identity_binding_epoch_is_current(
                TELEGRAM_IDENTITY_PROVIDER,
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

pub fn telegram_user_identity_provider_user_id(
    installation_id: &AdapterInstallationId,
    telegram_user_id: &str,
) -> String {
    format!("{}:{telegram_user_id}", installation_id.as_str())
}

/// Whether a `{installation}:{telegram_user_id}` provider id belongs to the
/// given installation — exact segment match, never a raw string prefix, so
/// `tg-bot-1` can never bleed into a `tg-bot-10:…` binding.
pub fn provider_user_id_in_installation(
    provider_user_id: &str,
    installation_id: &AdapterInstallationId,
) -> bool {
    installation_segment_matches(provider_user_id, installation_id.as_str())
}

/// Core exact-segment comparison shared by every installation-scoping check:
/// the candidate's `{installation}` segment (before the first `:`) must equal
/// `installation` exactly — never a string prefix.
pub fn installation_segment_matches(provider_user_id: &str, installation: &str) -> bool {
    provider_user_id
        .split_once(':')
        .is_some_and(|(candidate, _)| candidate == installation)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use ironclaw_conversations::ExternalActorBindingEpoch;
    use ironclaw_host_api::UserId;
    use ironclaw_product_adapters::{ExternalActorRef, ProductAdapterId};
    use ironclaw_product_workflow::ProductActorUserResolutionRequest;

    use super::*;

    type BindingsByProviderUser = HashMap<(String, String), (UserId, Option<String>)>;

    /// Epoch-aware lookup fake keyed by `(provider, provider_user_id)`.
    #[derive(Debug, Default)]
    struct EpochLookup {
        bindings: StdMutex<BindingsByProviderUser>,
    }

    impl EpochLookup {
        fn bind(&self, provider: &str, provider_user_id: &str, user: &str, epoch: Option<&str>) {
            self.bindings.lock().expect("lock").insert(
                (provider.to_string(), provider_user_id.to_string()),
                (UserId::new(user).expect("user"), epoch.map(str::to_string)),
            );
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for EpochLookup {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, ironclaw_channel_host::identity::RebornUserIdentityLookupError>
        {
            Ok(self
                .resolve_user_identity_with_binding_epoch(provider, provider_user_id)
                .await?
                .map(|(user_id, _)| user_id))
        }

        async fn resolve_user_identity_with_binding_epoch(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<
            Option<(UserId, Option<ExternalActorBindingEpoch>)>,
            ironclaw_channel_host::identity::RebornUserIdentityLookupError,
        > {
            Ok(self
                .bindings
                .lock()
                .expect("lock")
                .get(&(provider.to_string(), provider_user_id.to_string()))
                .map(|(user_id, epoch)| {
                    (
                        user_id.clone(),
                        epoch.as_deref().map(|epoch| {
                            ExternalActorBindingEpoch::new(epoch).expect("valid epoch")
                        }),
                    )
                }))
        }

        async fn user_has_provider_binding(
            &self,
            provider: &str,
            user_id: &UserId,
        ) -> Result<bool, ironclaw_channel_host::identity::RebornUserIdentityLookupError> {
            Ok(self.bindings.lock().expect("lock").iter().any(
                |((bound_provider, _), (bound, _))| bound_provider == provider && bound == user_id,
            ))
        }
    }

    fn request(
        adapter_id: &str,
        actor_kind: &str,
        telegram_user_id: &str,
    ) -> ProductActorUserResolutionRequest {
        ProductActorUserResolutionRequest {
            adapter_id: ProductAdapterId::new(adapter_id).expect("adapter id"),
            installation_id: AdapterInstallationId::new("tg-bot-4242").expect("installation"),
            external_actor_ref: ExternalActorRef::new(actor_kind, telegram_user_id, None::<String>)
                .expect("actor ref"),
        }
    }

    fn resolver_with(lookup: EpochLookup) -> TelegramUserIdentityActorResolver {
        TelegramUserIdentityActorResolver::new(Arc::new(lookup))
    }

    #[tokio::test]
    async fn resolver_gates_on_adapter_id_and_actor_kind() {
        use ironclaw_product_workflow::ProductActorUserResolver;

        let lookup = EpochLookup::default();
        lookup.bind(TELEGRAM_IDENTITY_PROVIDER, "tg-bot-4242:555", "ben", None);
        let resolver = resolver_with(lookup);

        let resolved = resolver
            .resolve_product_actor_user(request("other_adapter", "telegram_user", "555"))
            .await
            .expect("resolution runs");
        assert!(resolved.is_none(), "foreign adapters must not resolve");

        let resolved = resolver
            .resolve_product_actor_user(request(TELEGRAM_V2_ADAPTER_ID, "slack_user", "555"))
            .await
            .expect("resolution runs");
        assert!(resolved.is_none(), "foreign actor kinds must not resolve");

        assert!(
            !resolver
                .resolved_product_actor_user_is_current(
                    &request("other_adapter", "telegram_user", "555"),
                    &ironclaw_product_workflow::ResolvedProductActorUser::new(
                        UserId::new("ben").expect("user")
                    ),
                )
                .await
                .expect("is_current runs"),
            "gated requests are never current"
        );
    }

    #[tokio::test]
    async fn resolver_maps_installation_scoped_provider_id_and_carries_epoch() {
        use ironclaw_product_workflow::ProductActorUserResolver;

        let lookup = EpochLookup::default();
        lookup.bind(
            TELEGRAM_IDENTITY_PROVIDER,
            "tg-bot-4242:555",
            "ben",
            Some("EPOCH111"),
        );
        let resolver = resolver_with(lookup);

        let resolved = resolver
            .resolve_product_actor_user(request(
                TELEGRAM_V2_ADAPTER_ID,
                ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
                "555",
            ))
            .await
            .expect("resolution runs")
            .expect("bound actor resolves");
        assert_eq!(resolved.user_id.as_str(), "ben");
        assert_eq!(
            resolved
                .binding_epoch
                .as_ref()
                .map(ExternalActorBindingEpoch::as_str),
            Some("EPOCH111")
        );

        let unbound = resolver
            .resolve_product_actor_user(request(
                TELEGRAM_V2_ADAPTER_ID,
                ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
                "999",
            ))
            .await
            .expect("resolution runs");
        assert!(unbound.is_none(), "unknown telegram accounts stay unbound");
    }

    /// The `is_current` recheck is the revocation observation point: a stale
    /// epoch (unpair/re-pair rotated it) must read as not-current so
    /// mid-flight messages fail closed.
    #[tokio::test]
    async fn resolver_is_current_tracks_the_binding_epoch() {
        use ironclaw_product_workflow::{ProductActorUserResolver, ResolvedProductActorUser};

        let lookup = EpochLookup::default();
        lookup.bind(
            TELEGRAM_IDENTITY_PROVIDER,
            "tg-bot-4242:555",
            "ben",
            Some("EPOCH222"),
        );
        let resolver = resolver_with(lookup);
        let request = request(
            TELEGRAM_V2_ADAPTER_ID,
            ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
            "555",
        );

        let current = ResolvedProductActorUser::with_binding_epoch(
            UserId::new("ben").expect("user"),
            ExternalActorBindingEpoch::new("EPOCH222").expect("epoch"),
        );
        assert!(
            resolver
                .resolved_product_actor_user_is_current(&request, &current)
                .await
                .expect("is_current runs")
        );

        let stale = ResolvedProductActorUser::with_binding_epoch(
            UserId::new("ben").expect("user"),
            ExternalActorBindingEpoch::new("EPOCH111").expect("epoch"),
        );
        assert!(
            !resolver
                .resolved_product_actor_user_is_current(&request, &stale)
                .await
                .expect("is_current runs"),
            "a rotated epoch invalidates in-flight resolution"
        );

        let wrong_user = ResolvedProductActorUser::with_binding_epoch(
            UserId::new("illia").expect("user"),
            ExternalActorBindingEpoch::new("EPOCH222").expect("epoch"),
        );
        assert!(
            !resolver
                .resolved_product_actor_user_is_current(&request, &wrong_user)
                .await
                .expect("is_current runs"),
            "a different bound user is never current"
        );
    }

    #[test]
    fn provider_user_id_formatting_and_installation_matching_are_inverse() {
        let installation = AdapterInstallationId::new("tg-bot-1").expect("installation");
        let provider_user_id = telegram_user_identity_provider_user_id(&installation, "555");
        assert_eq!(provider_user_id, "tg-bot-1:555");
        assert!(provider_user_id_in_installation(
            &provider_user_id,
            &installation
        ));
        // Exact segment match: a longer installation id sharing the prefix
        // must not claim the binding, and vice versa.
        assert!(!provider_user_id_in_installation(
            "tg-bot-10:555",
            &installation
        ));
        assert!(!provider_user_id_in_installation(
            &provider_user_id,
            &AdapterInstallationId::new("tg-bot-10").expect("installation")
        ));
    }
}
