//! Canonical-identity backing for the channel-actor identity seams.
//!
//! [`RebornUserIdentityLookup`] (Slack actor resolution) and
//! [`RebornUserIdentityBindingStore`] (Slack personal binding) are
//! deliberately unbacked trait seams; this adapter backs BOTH with the one
//! canonical Reborn identity store (`ironclaw_reborn_identity`), so channel
//! actors resolve through the SAME store as WebUI OAuth login instead of a
//! parallel one (issue #4381).
//!
//! Production `serve` currently routes Slack identity through
//! `FilesystemSlackHostState` (the personal-binding host state); migrating
//! that durable binding/lookup onto this canonical adapter is a follow-up.
//! This adapter and its tests lock the canonical-backed seam in the meantime.
//!
//! Channel actors are `channel_actor`-surface external identities and are
//! **link-only**: [`resolve_user_identity`](RebornUserIdentityLookup::resolve_user_identity)
//! never mints (an unbound actor returns `None` and the caller fails
//! closed), and [`bind_user_identity`](RebornUserIdentityBindingStore::bind_user_identity)
//! links a proven actor to an already-authenticated Reborn `UserId`. This
//! is intentionally distinct from the WebUI OAuth `resolve_or_create`
//! (mint) path — a Slack actor must not auto-provision a Reborn account.
//!
//! The host tenant is captured at construction: one `serve` instance
//! serves one installation tenant, and the Slack seams carry no tenant of
//! their own. The Slack `provider_user_id` is already installation-scoped,
//! so it is the external subject and no separate instance id is needed.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn_identity::{ExternalIdentityKey, RebornIdentityResolver, SurfaceKind};

use crate::slack_actor_identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
};

/// Backs the channel-actor identity seams with the canonical Reborn
/// identity store.
pub struct CanonicalRebornUserIdentityStore {
    resolver: Arc<dyn RebornIdentityResolver>,
    tenant_id: TenantId,
}

impl CanonicalRebornUserIdentityStore {
    pub fn new(resolver: Arc<dyn RebornIdentityResolver>, tenant_id: TenantId) -> Self {
        Self {
            resolver,
            tenant_id,
        }
    }

    fn key<'a>(&'a self, provider: &'a str, provider_user_id: &'a str) -> ExternalIdentityKey<'a> {
        ExternalIdentityKey {
            tenant_id: &self.tenant_id,
            surface_kind: SurfaceKind::ChannelActor,
            provider_kind: provider,
            // The Slack provider_user_id is already installation-scoped, so
            // it is the subject and no separate instance id is needed.
            provider_instance_id: None,
            external_subject_id: provider_user_id,
        }
    }
}

#[async_trait]
impl RebornUserIdentityLookup for CanonicalRebornUserIdentityStore {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        self.resolver
            .lookup(self.key(provider, provider_user_id))
            .await
            .map_err(|err| RebornUserIdentityLookupError::Backend(err.to_string()))
    }
}

#[async_trait]
impl RebornUserIdentityBindingStore for CanonicalRebornUserIdentityStore {
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        self.resolver
            .bind(
                self.key(binding.provider.as_str(), binding.provider_user_id.as_str()),
                &binding.user_id,
            )
            .await
            .map_err(|err| RebornUserIdentityBindingError::Backend(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack_personal_binding::{RebornIdentityProviderId, RebornIdentityProviderUserId};
    use ironclaw_reborn_identity::RebornLibSqlIdentityStore;

    async fn store() -> CanonicalRebornUserIdentityStore {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.keep().join("reborn-local-dev.db");
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("open libsql"),
        );
        let resolver = Arc::new(
            RebornLibSqlIdentityStore::open(db)
                .await
                .expect("open store"),
        );
        CanonicalRebornUserIdentityStore::new(resolver, TenantId::new("tenant-a").expect("tenant"))
    }

    fn binding(provider_user_id: &str, user_id: &str) -> RebornUserIdentityBinding {
        RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").expect("provider"),
            provider_user_id: RebornIdentityProviderUserId::new(provider_user_id).expect("subject"),
            user_id: UserId::new(user_id).expect("user"),
        }
    }

    #[tokio::test]
    async fn unbound_actor_resolves_to_none() {
        let store = store().await;
        let resolved = store
            .resolve_user_identity("slack", "inst-1::U-unbound")
            .await
            .expect("lookup");
        assert!(
            resolved.is_none(),
            "an unbound Slack actor must fail closed (None), never auto-provision"
        );
    }

    #[tokio::test]
    async fn bound_actor_resolves_through_canonical_store() {
        let store = store().await;
        store
            .bind_user_identity(binding("inst-1::U-1", "reborn-user-7"))
            .await
            .expect("bind");
        let resolved = store
            .resolve_user_identity("slack", "inst-1::U-1")
            .await
            .expect("lookup");
        assert_eq!(
            resolved.as_ref().map(|u| u.as_str()),
            Some("reborn-user-7"),
            "a bound Slack actor resolves to its canonical UserId through the shared store"
        );
    }
}
