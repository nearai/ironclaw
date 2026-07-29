//! Production role resolution for channel-command admission: verified inbound
//! actor → bound IronClaw user (channel identity binding) → active-account
//! admin-boundary role (admin-users directory).

use async_trait::async_trait;
use ironclaw_host_api::{ProductSurfaceError, RebornUserIdentityLookup, TenantId, UserId};
use ironclaw_product::{
    AdminUserError, AdminUserRole, AdminUserService, AdminUserStatus, CommandActorRoleResolver,
    ProductCommandContext,
};
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
            ironclaw_host_api::ProductSurfaceErrorCode::Unavailable,
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
                .resolve_user_identity(&self.provider, context.external_actor_ref.id())
                .await
            {
                Ok(Some(user_id)) => user_id,
                Ok(None) => return Ok(None),
                Err(_) => return Err(Self::unavailable()),
            },
            // Composition paths without the durable identity store run under
            // the operator-actor policy: the operator IS the actor.
            None => self.operator_user_id.clone(),
        };
        match self.admin_users.get_user(&self.tenant, &user_id).await {
            Ok(Some(record)) if record.status == AdminUserStatus::Active => Ok(Some(record.role)),
            Ok(_) => Ok(None),
            Err(AdminUserError::Unavailable) => Err(Self::unavailable()),
            Err(_) => Err(ProductSurfaceError::from_status(
                ironclaw_host_api::ProductSurfaceErrorCode::Internal,
                500,
                false,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
        ParsedProductInbound, ProductAdapterId, ProductInboundEnvelope, ProductInboundPayload,
        ProtocolAuthEvidence, RebornUserIdentityLookupError, TrustedInboundContext,
    };
    use ironclaw_product::{
        ActionFingerprintKey, AdminCreateUserFields, AdminCreatedUser, AdminUserRecord,
        AdminUserSecretMeta, AuthRequirement, InboundCommandPayload, ProductActionId,
        ProductTriggerReason, SourceBindingKey,
    };
    use secrecy::SecretString;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("valid user")
    }

    fn sample_context(actor_id: &str) -> ProductCommandContext {
        let adapter_id = ProductAdapterId::new("test_adapter").expect("valid adapter");
        let installation_id =
            AdapterInstallationId::new("install_alpha").expect("valid installation");
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

    struct FakeLookup {
        bindings: std::collections::HashMap<String, UserId>,
        fail: bool,
    }

    #[async_trait]
    impl RebornUserIdentityLookup for FakeLookup {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
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

    struct FakeAdminUsers {
        roles: Mutex<std::collections::HashMap<String, (AdminUserRole, AdminUserStatus)>>,
        fail: bool,
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
            if self.fail {
                return Err(AdminUserError::Unavailable);
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
            _handle: ironclaw_host_api::SecretHandle,
            _material: SecretString,
        ) -> Result<AdminUserSecretMeta, AdminUserError> {
            Err(AdminUserError::Internal)
        }

        async fn delete_secret(
            &self,
            _tenant: &TenantId,
            _user_id: &UserId,
            _handle: ironclaw_host_api::SecretHandle,
        ) -> Result<bool, AdminUserError> {
            Err(AdminUserError::Internal)
        }
    }

    #[tokio::test]
    async fn unbound_actor_resolves_to_no_role() {
        let lookup = Arc::new(FakeLookup {
            bindings: std::collections::HashMap::new(),
            fail: false,
        });
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: false,
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
        bindings.insert("suspended-actor".to_string(), bound_user.clone());
        let lookup = Arc::new(FakeLookup {
            bindings,
            fail: false,
        });
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            bound_user.as_str().to_string(),
            (AdminUserRole::Owner, AdminUserStatus::Suspended),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: false,
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
        bindings.insert("admin-actor".to_string(), bound_user.clone());
        let lookup = Arc::new(FakeLookup {
            bindings,
            fail: false,
        });
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            bound_user.as_str().to_string(),
            (AdminUserRole::Admin, AdminUserStatus::Active),
        );
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(roles),
            fail: false,
        });
        let resolver = ChannelActorRoleResolver::new(
            "test-provider".to_string(),
            Some(lookup),
            admin_users,
            tenant("tenant-a"),
            user("operator-a"),
        );

        let role = resolver
            .actor_role(&sample_context("admin-actor"))
            .await
            .expect("resolves");

        assert_eq!(role, Some(AdminUserRole::Admin));
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
            fail: false,
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

    #[tokio::test]
    async fn identity_lookup_failure_is_a_retryable_error() {
        let lookup = Arc::new(FakeLookup {
            bindings: std::collections::HashMap::new(),
            fail: true,
        });
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: false,
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
        bindings.insert("actor".to_string(), bound_user);
        let lookup = Arc::new(FakeLookup {
            bindings,
            fail: false,
        });
        let admin_users = Arc::new(FakeAdminUsers {
            roles: Mutex::new(std::collections::HashMap::new()),
            fail: true,
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
}
