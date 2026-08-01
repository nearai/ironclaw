//! Production role resolution for channel-command admission: verified inbound
//! actor → bound IronClaw user (channel identity binding) → active-account
//! admin-boundary role (admin-users directory).

use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{TenantId, UserId},
    user_identity::{RebornUserIdentityLookup, installation_scoped_provider_user_id},
};
use ironclaw_product::{
    AdminUserError, AdminUserRole, AdminUserService, AdminUserStatus, CommandActorRoleResolver,
    ProductCommandContext,
};
use ironclaw_product_contracts::surface::ProductSurfaceError;
use std::sync::Arc;

/// Resolves the channel-command actor's admin-boundary role: an OAuth/pairing
/// identity binding maps the verified inbound actor to a bound IronClaw user
/// (when this extension has one — see [`Self::new`]'s `identity_lookup`), and
/// the admin-users directory maps that user to an active-account role.
pub struct ChannelActorRoleResolver {
    provider: String,
    identity_lookup: Option<Arc<dyn RebornUserIdentityLookup>>,
    admin_users: Arc<dyn AdminUserService>,
    tenant: TenantId,
    operator_user_id: UserId,
}

impl ChannelActorRoleResolver {
    pub fn new(
        provider: String,
        identity_lookup: Option<Arc<dyn RebornUserIdentityLookup>>,
        admin_users: Arc<dyn AdminUserService>,
        tenant: TenantId,
        operator_user_id: UserId,
    ) -> Self {
        Self {
            provider,
            identity_lookup,
            admin_users,
            tenant,
            operator_user_id,
        }
    }

    fn unavailable() -> ProductSurfaceError {
        ProductSurfaceError::from_status(
            ironclaw_product_contracts::surface::ProductSurfaceErrorCode::Unavailable,
            503,
            true,
        )
    }
}

#[async_trait]
impl CommandActorRoleResolver for ChannelActorRoleResolver {
    async fn actor_role(
        &self,
        context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError> {
        let user_id = match &self.identity_lookup {
            Some(lookup) => match lookup
                .resolve_user_identity(
                    &self.provider,
                    &installation_scoped_provider_user_id(
                        &context.installation_id,
                        context.external_actor_ref.id(),
                    ),
                )
                .await
            {
                Ok(Some(user_id)) => user_id,
                Ok(None) => return Ok(None),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        provider = %self.provider,
                        "channel-command role resolver: identity lookup failed"
                    );
                    return Err(Self::unavailable());
                }
            },
            // Composition paths without the durable identity store run under
            // the operator-actor policy: the operator IS the actor.
            None => self.operator_user_id.clone(),
        };
        match self.admin_users.get_user(&self.tenant, &user_id).await {
            Ok(Some(record)) if record.status == AdminUserStatus::Active => Ok(Some(record.role)),
            // Implicit-owner rule, keyed on bound-user identity: when the
            // resolved actor IS the operator (`user_id == operator_user_id`)
            // and the directory has no record at all, treat it as Owner. A
            // persisted record of ANY status (including Suspended) still
            // governs — this arm only fires on "no record", never overriding
            // the arm above.
            //
            // This deliberately differs from the WebUI's
            // `RebornServices::authorize_admin`, whose `caller.operator_config`
            // flag short-circuits to `Ok(())` before ever calling `get_user` —
            // it has no record-governs behavior at all. A suspended operator
            // record therefore denies through this resolver but still admits
            // through the WebUI door. Tracked asymmetry: issue #6877.
            Ok(None) if user_id == self.operator_user_id => Ok(Some(AdminUserRole::Owner)),
            Ok(_) => Ok(None),
            Err(AdminUserError::Unavailable) => Err(Self::unavailable()),
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "channel-command role resolver: admin-users lookup failed"
                );
                Err(ProductSurfaceError::from_status(
                    ironclaw_product_contracts::surface::ProductSurfaceErrorCode::Internal,
                    500,
                    false,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::external::{
        ExternalActorRef, ExternalConversationRef, ExternalEventId,
    };
    use ironclaw_host_api::product_adapter::{
        AdapterInstallationId, ProductAdapterId, ProtocolAuthEvidence,
    };
    use ironclaw_host_api::user_identity::RebornUserIdentityLookupError;
    use ironclaw_product::{
        ActionFingerprintKey, AdminCreateUserFields, AdminCreatedUser, AdminUserRecord,
        AdminUserSecretMeta, AuthRequirement, InboundCommandPayload, ProductActionId,
        ProductTriggerReason, SourceBindingKey,
    };
    use ironclaw_product_contracts::inbound::{
        ParsedProductInbound, ProductInboundEnvelope, ProductInboundPayload, TrustedInboundContext,
    };
    use secrecy::SecretString;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// The installation id `sample_context` bakes into every context it
    /// builds. Binding keys must be scoped to this SAME value
    /// (`installation_scoped_provider_user_id`) — a shared helper rather than
    /// a second hardcoded literal keeps the two from drifting apart.
    fn test_installation_id() -> AdapterInstallationId {
        AdapterInstallationId::new("install_alpha").expect("valid installation")
    }

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("valid user")
    }

    fn sample_context(actor_id: &str) -> ProductCommandContext {
        let adapter_id = ProductAdapterId::new("test_adapter").expect("valid adapter");
        let installation_id = test_installation_id();
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            installation_id.as_str(),
        );
        let trusted = TrustedInboundContext::from_verified_evidence(
            adapter_id,
            installation_id,
            chrono::Utc::now(),
            &evidence,
        )
        .expect("verified");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new("evt:role-resolver").expect("valid event"),
            ExternalActorRef::new("test", actor_id, Option::<String>::None).expect("valid actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation"),
            ProductInboundPayload::Command(
                InboundCommandPayload::new("model", "", ProductTriggerReason::DirectChat)
                    .expect("valid command"),
            ),
        )
        .expect("parsed");
        let envelope =
            ProductInboundEnvelope::from_trusted_parse(trusted, parsed).expect("envelope");
        let source_binding_key =
            SourceBindingKey::new(envelope.source_binding_key()).expect("valid binding key");
        let fingerprint = ActionFingerprintKey::new(
            envelope.adapter_id().clone(),
            envelope.installation_id().clone(),
            envelope.external_actor_ref().clone(),
            source_binding_key,
            envelope.external_event_id().clone(),
        );
        ProductCommandContext::from_envelope(&envelope, ProductActionId::new(), fingerprint)
            .expect("context")
    }

    /// Bindings are keyed by the SCOPED provider-user id
    /// (`installation_scoped_provider_user_id`), matching every real
    /// producer (`channel_identity_binding.rs`, `channel_pairing.rs`) and the
    /// production `ProviderIdentityActorResolver` reader — never a raw actor
    /// id. `calls()` additionally records every `(provider,
    /// provider_user_id)` pair the resolver actually looked up, so a test can
    /// pin the exact key format `actor_role` sends rather than trust that
    /// seeding and lookup happen to agree.
    struct FakeLookup {
        bindings: std::collections::HashMap<String, UserId>,
        fail: bool,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl FakeLookup {
        fn new(bindings: std::collections::HashMap<String, UserId>, fail: bool) -> Self {
            Self {
                bindings,
                fail,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl RebornUserIdentityLookup for FakeLookup {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((provider.to_string(), provider_user_id.to_string()));
            if self.fail {
                return Err(RebornUserIdentityLookupError::Backend(
                    "fake lookup unavailable".to_string(),
                ));
            }
            if provider != "test-provider" {
                return Ok(None);
            }
            Ok(self.bindings.get(provider_user_id).cloned())
        }

        async fn user_has_provider_binding(
            &self,
            _provider: &str,
            _user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Ok(false)
        }
    }

    /// `fail`, when set, is the exact error `get_user` returns — distinct
    /// from a plain bool so tests can pin behavior for a specific
    /// `AdminUserError` variant (e.g. `Internal` vs `Unavailable`) rather than
    /// only "some failure occurred".
    struct FakeAdminUsers {
        roles: Mutex<std::collections::HashMap<String, (AdminUserRole, AdminUserStatus)>>,
        fail: Option<AdminUserError>,
    }

    #[async_trait]
    impl AdminUserService for FakeAdminUsers {
        async fn list_users(
            &self,
            _tenant: &TenantId,
            _status: Option<AdminUserStatus>,
            _after: Option<&UserId>,
            _limit: usize,
        ) -> Result<Vec<AdminUserRecord>, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn get_user(
            &self,
            _tenant: &TenantId,
            user_id: &UserId,
        ) -> Result<Option<AdminUserRecord>, AdminUserError> {
            if let Some(error) = self.fail {
                return Err(error);
            }
            let roles = self.roles.lock().expect("lock");
            Ok(roles
                .get(user_id.as_str())
                .map(|(role, status)| AdminUserRecord {
                    user_id: user_id.clone(),
                    email: None,
                    display_name: None,
                    status: *status,
                    role: *role,
                    created_at: String::new(),
                    updated_at: String::new(),
                    created_by: None,
                    last_login_at: None,
                    metadata: BTreeMap::new(),
                }))
        }

        async fn create_user(
            &self,
            _tenant: &TenantId,
            _actor: &UserId,
            _fields: AdminCreateUserFields,
        ) -> Result<AdminCreatedUser, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn update_profile(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
            _display_name: Option<String>,
            _metadata: Option<BTreeMap<String, String>>,
        ) -> Result<AdminUserRecord, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn set_status(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
            _status: AdminUserStatus,
        ) -> Result<AdminUserRecord, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn set_role(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
            _role: AdminUserRole,
        ) -> Result<AdminUserRecord, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn delete_user(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
        ) -> Result<(), AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn count_active_admins(&self, _tenant: &TenantId) -> Result<usize, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn list_secrets(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
        ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn put_secret(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
            _handle: ironclaw_host_api::ids::SecretHandle,
            _material: SecretString,
        ) -> Result<AdminUserSecretMeta, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn delete_secret(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
            _handle: ironclaw_host_api::ids::SecretHandle,
        ) -> Result<bool, AdminUserError> {
            Err(AdminUserError::Internal)
        }
    }

    #[tokio::test]
    async fn unbound_actor_resolves_to_no_role() {
        let lookup = Arc::new(FakeLookup::new(std::collections::HashMap::new(), false));
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let role = resolver
            .actor_role(&sample_context("unbound-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, None);
    }

    #[tokio::test]
    async fn suspended_admin_account_resolves_to_no_role() {
        let bound_user = user("user-1");
        let mut bindings = std::collections::HashMap::new();
        // Keyed by the SCOPED provider-user id, matching the real write path
        // (`channel_identity_binding.rs`, `channel_pairing.rs`) and the fixed
        // `actor_role` read path — never the raw actor id (regression guard
        // for the unscoped-lookup bug: an unscoped key would make this
        // binding unreachable in production and this test pass vacuously).
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "suspended-actor"),
            bound_user.clone(),
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            bound_user.as_str().to_string(),
            (AdminUserRole::Owner, AdminUserStatus::Suspended),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let role = resolver
            .actor_role(&sample_context("suspended-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, None);
    }

    #[tokio::test]
    async fn active_admin_account_resolves_its_role() {
        let bound_user = user("user-2");
        let mut bindings = std::collections::HashMap::new();
        // Same scoped-key regression guard as above.
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "admin-actor"),
            bound_user.clone(),
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            bound_user.as_str().to_string(),
            (AdminUserRole::Admin, AdminUserStatus::Active),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup.clone()),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let role = resolver
            .actor_role(&sample_context("admin-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, Some(AdminUserRole::Admin));
        // Pin the exact key format `actor_role` sends the identity lookup:
        // the literal is independent of `installation_scoped_provider_user_id`
        // so this fails if the resolver ever regresses back to an unscoped
        // (or differently-scoped) actor id, even if some future change
        // altered the helper itself.
        assert_eq!(
            lookup.calls(),
            vec![(
                "test-provider".to_string(),
                "install_alpha:admin-actor".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn missing_identity_lookup_falls_back_to_operator_actor_policy() {
        let operator = user("operator-b");
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            operator.as_str().to_string(),
            (AdminUserRole::Owner, AdminUserStatus::Active),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            None,
            admin_users,
            tenant("tenant-a"),
            operator.clone(),
        );

        let role = resolver
            .actor_role(&sample_context("whatever-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, Some(AdminUserRole::Owner));
    }

    /// Compound highest-risk lane: composition without a durable identity
    /// store runs under the operator-actor policy (`identity_lookup: None`,
    /// so EVERY actor resolves to `operator_user_id`), combined with no
    /// directory record at all. This pins today's deliberate behavior so a
    /// future change to the fallback lane can't silently alter it — see
    /// issue #6877 for the tracked asymmetry with the WebUI's
    /// `authorize_admin`.
    #[tokio::test]
    async fn operator_fallback_lane_without_directory_record_is_implicit_owner() {
        let operator = user("operator-c");
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            None,
            admin_users,
            tenant("tenant-a"),
            operator,
        );

        let role = resolver
            .actor_role(&sample_context("whatever-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, Some(AdminUserRole::Owner));
    }

    #[tokio::test]
    async fn identity_lookup_failure_is_a_retryable_error() {
        let lookup = Arc::new(FakeLookup::new(std::collections::HashMap::new(), true));
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let error = resolver
            .actor_role(&sample_context("actor"))
            .await
            .expect_err("lookup failure must be retryable, not a silent role");

        assert!(error.retryable);
    }

    #[tokio::test]
    async fn admin_users_unavailable_is_a_retryable_error() {
        let bound_user = user("user-3");
        let mut bindings = std::collections::HashMap::new();
        // Scoped key: this test only exercises the AdminUserService failure
        // path if the identity lookup ABOVE it actually hits.
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "actor"),
            bound_user,
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: Some(AdminUserError::Unavailable),
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let error = resolver
            .actor_role(&sample_context("actor"))
            .await
            .expect_err("admin-users unavailability must be retryable, not a silent role");

        assert!(error.retryable);
    }

    /// The env-bearer operator has no admin-directory record — `get_user`
    /// legitimately returns `Ok(None)` for it — but it is still the
    /// deployment's implicit owner, mirroring `RebornServices::authorize_admin`'s
    /// `caller.operator_config` bypass. Without this rule the operator is
    /// permanently denied channel admin commands (PR-1 final review finding).
    #[tokio::test]
    async fn operator_bound_actor_without_directory_record_is_implicit_owner() {
        let operator = user("operator-a");
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "operator-actor"),
            operator.clone(),
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            operator,
        );

        let role = resolver
            .actor_role(&sample_context("operator-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, Some(AdminUserRole::Owner));
    }

    /// A persisted record for the operator's bound user still governs even
    /// though the implicit-owner rule would otherwise apply: the record is
    /// `Some` (not `None`), so the operator match arm never fires and the
    /// Suspended status denies exactly like any other suspended admin.
    #[tokio::test]
    async fn operator_bound_actor_with_suspended_record_is_not_admin() {
        let operator = user("operator-a");
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "operator-actor"),
            operator.clone(),
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            operator.as_str().to_string(),
            (AdminUserRole::Owner, AdminUserStatus::Suspended),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            operator,
        );

        let role = resolver
            .actor_role(&sample_context("operator-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, None);
    }

    /// The implicit-owner rule is keyed on identity (bound user ==
    /// `operator_user_id`), not merely "no record exists". A distinct,
    /// non-operator user with no directory record must still resolve to no
    /// role.
    #[tokio::test]
    async fn non_operator_bound_actor_without_record_stays_none() {
        let bound_user = user("user-4");
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "plain-actor"),
            bound_user,
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let role = resolver
            .actor_role(&sample_context("plain-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, None);
    }

    /// PR-1-deferred branch: an `Internal` (non-`Unavailable`) admin-users
    /// failure must still map to a non-retryable error, not be silently
    /// treated as "no role".
    #[tokio::test]
    async fn admin_users_internal_error_is_not_retryable() {
        let bound_user = user("user-5");
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "actor"),
            bound_user,
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: Some(AdminUserError::Internal),
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let error = resolver
            .actor_role(&sample_context("actor"))
            .await
            .expect_err("internal admin-users failure must not be silently treated as a role");

        assert!(!error.retryable);
    }

    /// PR-1-deferred branch: an Active `Member` record resolves to
    /// `Some(Member)`, not just the `Owner`/`Admin` cases the other tests
    /// exercise.
    #[tokio::test]
    async fn bound_actor_with_member_record_resolves_member_role() {
        let bound_user = user("user-6");
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            installation_scoped_provider_user_id(&test_installation_id(), "member-actor"),
            bound_user.clone(),
        );
        let lookup = Arc::new(FakeLookup::new(bindings, false));
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            bound_user.as_str().to_string(),
            (AdminUserRole::Member, AdminUserStatus::Active),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: None,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let role = resolver
            .actor_role(&sample_context("member-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, Some(AdminUserRole::Member));
    }
}
