//! The one fire-time trigger-access checker that is assembly, not policy.
//!
//! The check contract (`TriggerFireAccessCheck`/`Decision`/`Error`/`Checker`),
//! the exact-scope comparison, the static-owner grant, and the OR-combinator
//! moved to `ironclaw_triggers::fire_access` with CHECKLIST WS6 — they are
//! decisions about this host's own trigger records and carry no backend.
//!
//! What stays is the checker that is a *lookup against a backend composition
//! selects*: tenant membership resolved at fire time from the canonical
//! identity directory the SSO login path populates. It renders the same denial
//! reason and applies the same exact-scope rule as its siblings by calling the
//! trigger crate's [`trigger_fire_access_denied`] / [`trigger_fire_scope_matches`]
//! rather than restating either — a second copy of the deny string is exactly
//! how two checkers drift apart.
//!
//! The composition build still selects this one from `TriggerFireAccessPolicy`,
//! which stays here: §6.10.1's Keeps list names deployment config-as-data as
//! composition's charter.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId};
use ironclaw_triggers::{
    TriggerFireAccessCheck, TriggerFireAccessChecker, TriggerFireAccessDecision,
    TriggerFireAccessError, trigger_fire_access_denied, trigger_fire_scope_matches,
};

/// Any active member of the host tenant may fire triggers for one exact scope —
/// the SSO/WebUI deployment. Membership is resolved at fire time from the
/// canonical identity directory (the `StoredUser` records SSO login persists),
/// so a suspended, wrong-tenant, or unknown creator is denied. A directory
/// backend error surfaces as retryable `Unavailable`, never a hard denial.
pub(crate) struct IdentityMembershipTriggerFireChecker {
    directory: Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>,
    tenant_id: TenantId,
    agent: AgentId,
    project: Option<ProjectId>,
}

impl IdentityMembershipTriggerFireChecker {
    pub(crate) fn new(
        directory: Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>,
        tenant_id: TenantId,
        agent: AgentId,
        project: Option<ProjectId>,
    ) -> Self {
        Self {
            directory,
            tenant_id,
            agent,
            project,
        }
    }
}

#[async_trait]
impl TriggerFireAccessChecker for IdentityMembershipTriggerFireChecker {
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
        if !trigger_fire_scope_matches(&request, &self.agent, &self.project) {
            return Ok(trigger_fire_access_denied());
        }
        let user = self
            .directory
            .get_user(&request.creator_user_id)
            .await
            .map_err(|error| TriggerFireAccessError::Unavailable {
                reason: error.to_string(),
            })?;
        // Active member of THIS tenant. A record with no persisted tenant is
        // treated as belonging to the requested tenant (single-tenant
        // back-compat, matching `RebornUserDirectory` enumeration).
        let allowed = user.is_some_and(|user| {
            user.status == ironclaw_reborn_identity::RebornUserStatus::Active
                // `is_none_or` (stable since Rust 1.82) is within MSRV — this
                // workspace is edition 2024 (Rust ≥ 1.85) and clippy enforces it
                // over `map_or(true, …)`.
                && user
                    .tenant_id
                    .as_ref()
                    .is_none_or(|tenant| tenant == &self.tenant_id)
        });
        Ok(if allowed {
            TriggerFireAccessDecision::Allowed
        } else {
            trigger_fire_access_denied()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::UserId;

    fn check(creator: &str, agent: Option<&str>, project: Option<&str>) -> TriggerFireAccessCheck {
        TriggerFireAccessCheck {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            creator_user_id: UserId::new(creator).expect("user"),
            agent_id: agent.map(|a| AgentId::new(a).expect("agent")),
            project_id: project.map(|p| ProjectId::new(p).expect("project")),
            trigger_id: ironclaw_triggers::TriggerId::new(),
            fire_slot: chrono::Utc::now(),
        }
    }

    mod identity {
        use super::*;
        use ironclaw_reborn_identity::{
            RebornIdentityError, RebornUser, RebornUserDirectory, RebornUserProfileUpdate,
            RebornUserRole, RebornUserStatus,
        };

        /// Directory double returning one configured user (or none), and a
        /// backend-error mode for the retryable-unavailable path.
        struct FakeDirectory {
            user: Option<RebornUser>,
            fail: bool,
        }

        impl FakeDirectory {
            fn with_user(user: RebornUser) -> Self {
                Self {
                    user: Some(user),
                    fail: false,
                }
            }
            fn empty() -> Self {
                Self {
                    user: None,
                    fail: false,
                }
            }
            fn failing() -> Self {
                Self {
                    user: None,
                    fail: true,
                }
            }
        }

        fn user(status: RebornUserStatus, tenant: Option<&str>) -> RebornUser {
            RebornUser {
                user_id: UserId::new("member").expect("user"),
                email: None,
                display_name: None,
                status,
                role: RebornUserRole::Member,
                created_at: String::new(),
                updated_at: String::new(),
                created_by: None,
                last_login_at: None,
                tenant_id: tenant.map(|t| TenantId::new(t).expect("tenant")),
                metadata: Default::default(),
            }
        }

        #[async_trait]
        impl RebornUserDirectory for FakeDirectory {
            async fn list_users(
                &self,
                _tenant_id: &TenantId,
                _status: Option<RebornUserStatus>,
                _after: Option<&UserId>,
                _limit: usize,
            ) -> Result<Vec<RebornUser>, RebornIdentityError> {
                Ok(self.user.clone().into_iter().collect())
            }
            async fn get_user(
                &self,
                _user_id: &UserId,
            ) -> Result<Option<RebornUser>, RebornIdentityError> {
                if self.fail {
                    return Err(RebornIdentityError::Backend("backend down".to_string()));
                }
                Ok(self.user.clone())
            }
            async fn create_user(
                &self,
                _tenant_id: &TenantId,
                _email: Option<String>,
                _display_name: Option<String>,
                _role: RebornUserRole,
                _created_by: &UserId,
            ) -> Result<RebornUser, RebornIdentityError> {
                unimplemented!("not used")
            }
            async fn update_profile(
                &self,
                _user_id: &UserId,
                _update: RebornUserProfileUpdate,
            ) -> Result<RebornUser, RebornIdentityError> {
                unimplemented!("not used")
            }
            async fn update_status(
                &self,
                _user_id: &UserId,
                _status: RebornUserStatus,
            ) -> Result<RebornUser, RebornIdentityError> {
                unimplemented!("not used")
            }
            async fn update_role(
                &self,
                _user_id: &UserId,
                _role: RebornUserRole,
            ) -> Result<RebornUser, RebornIdentityError> {
                unimplemented!("not used")
            }
            async fn record_last_login(
                &self,
                _user_id: &UserId,
                _at: String,
            ) -> Result<(), RebornIdentityError> {
                unimplemented!("not used")
            }
            async fn delete_user(
                &self,
                _tenant_id: &TenantId,
                _user_id: &UserId,
            ) -> Result<(), RebornIdentityError> {
                unimplemented!("not used")
            }
            async fn count_active_admins(
                &self,
                _tenant_id: &TenantId,
            ) -> Result<usize, RebornIdentityError> {
                unimplemented!("not used")
            }
        }

        fn membership_checker(directory: FakeDirectory) -> IdentityMembershipTriggerFireChecker {
            IdentityMembershipTriggerFireChecker::new(
                Arc::new(directory),
                TenantId::new("tenant").expect("tenant"),
                AgentId::new("agent").expect("agent"),
                Some(ProjectId::new("project").expect("project")),
            )
        }

        #[tokio::test]
        async fn active_member_of_tenant_is_allowed() {
            let decision = membership_checker(FakeDirectory::with_user(user(
                RebornUserStatus::Active,
                Some("tenant"),
            )))
            .check_trigger_fire_access(check("member", Some("agent"), Some("project")))
            .await
            .expect("check");
            assert_eq!(decision, TriggerFireAccessDecision::Allowed);
        }

        #[tokio::test]
        async fn record_without_tenant_is_allowed_single_tenant_backcompat() {
            let decision = membership_checker(FakeDirectory::with_user(user(
                RebornUserStatus::Active,
                None,
            )))
            .check_trigger_fire_access(check("member", Some("agent"), Some("project")))
            .await
            .expect("check");
            assert_eq!(decision, TriggerFireAccessDecision::Allowed);
        }

        #[tokio::test]
        async fn unknown_user_is_denied() {
            let decision = membership_checker(FakeDirectory::empty())
                .check_trigger_fire_access(check("ghost", Some("agent"), Some("project")))
                .await
                .expect("check");
            assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
        }

        #[tokio::test]
        async fn suspended_member_is_denied() {
            // The behavior the old seed-only store lacked: suspension revokes.
            let decision = membership_checker(FakeDirectory::with_user(user(
                RebornUserStatus::Suspended,
                Some("tenant"),
            )))
            .check_trigger_fire_access(check("member", Some("agent"), Some("project")))
            .await
            .expect("check");
            assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
        }

        #[tokio::test]
        async fn wrong_tenant_member_is_denied() {
            let decision = membership_checker(FakeDirectory::with_user(user(
                RebornUserStatus::Active,
                Some("other-tenant"),
            )))
            .check_trigger_fire_access(check("member", Some("agent"), Some("project")))
            .await
            .expect("check");
            assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
        }

        #[tokio::test]
        async fn scope_mismatch_is_denied_without_directory_hit() {
            let decision = membership_checker(FakeDirectory::with_user(user(
                RebornUserStatus::Active,
                Some("tenant"),
            )))
            .check_trigger_fire_access(check("member", Some("other-agent"), Some("project")))
            .await
            .expect("check");
            assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
        }

        #[tokio::test]
        async fn backend_error_is_retryable_unavailable() {
            let error = membership_checker(FakeDirectory::failing())
                .check_trigger_fire_access(check("member", Some("agent"), Some("project")))
                .await
                .expect_err("directory error");
            assert!(matches!(error, TriggerFireAccessError::Unavailable { .. }));
        }

        /// Integration coverage over the REAL identity store the SSO login path
        /// populates (not the fake): a user resolved through `resolve_or_create`
        /// is an allowed trigger-fire member; an unknown user is denied; and
        /// suspending the user revokes access — the behavior the former
        /// seed-only trigger-access store lacked. Crate-tier because the checker
        /// and directory are composition-internal (`pub(crate)`), so an external
        /// `tests/` integration file cannot construct them.
        #[tokio::test]
        async fn real_identity_store_membership_backs_fire_access() {
            use ironclaw_host_api::{
                ids::{AgentId as HostAgentId, UserId as HostUserId},
                mount::{MountGrant, MountPermissions, MountView},
                path::{MountAlias, VirtualPath},
            };
            use ironclaw_reborn_identity::{
                ExternalSubjectId, ProviderKind, RebornIdentityResolver, RebornIdentityStore,
                RebornUserDirectory, RebornUserStatus, ResolveExternalIdentity, SurfaceKind,
            };

            let tenant = TenantId::new("real-tenant").expect("tenant");
            let root = Arc::new(ironclaw_filesystem::InMemoryBackend::default());
            let view = MountView::new(vec![MountGrant::new(
                MountAlias::new("/tenant-shared").expect("alias"),
                VirtualPath::new("/tenants/test/shared").expect("path"),
                MountPermissions::read_write_list_delete(),
            )])
            .expect("view");
            let filesystem = Arc::new(ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
                root, view,
            ));
            let store = Arc::new(RebornIdentityStore::new(
                filesystem,
                tenant.clone(),
                HostUserId::new("runtime-owner").expect("owner"),
                HostAgentId::new("agent").expect("agent"),
                None,
            ));

            // Admit a user exactly as the SSO login path does.
            let resolver: Arc<dyn RebornIdentityResolver> = store.clone();
            let user_id = resolver
                .resolve_or_create(ResolveExternalIdentity {
                    tenant_id: tenant.clone(),
                    surface_kind: SurfaceKind::Oauth,
                    provider_kind: ProviderKind::new("google").expect("provider"),
                    provider_instance_id: None,
                    external_subject_id: ExternalSubjectId::new("subject-1").expect("subject"),
                    email: Some("alice@example.com".to_string()),
                    email_verified: true,
                    display_name: None,
                })
                .await
                .expect("resolve_or_create admits the user");

            let directory: Arc<dyn RebornUserDirectory> = store.clone();
            let checker = IdentityMembershipTriggerFireChecker::new(
                directory.clone(),
                tenant.clone(),
                AgentId::new("agent").expect("agent"),
                None,
            );

            let allowed = checker
                .check_trigger_fire_access(TriggerFireAccessCheck {
                    tenant_id: tenant.clone(),
                    creator_user_id: user_id.clone(),
                    agent_id: Some(AgentId::new("agent").expect("agent")),
                    project_id: None,
                    trigger_id: ironclaw_triggers::TriggerId::new(),
                    fire_slot: chrono::Utc::now(),
                })
                .await
                .expect("check");
            assert_eq!(allowed, TriggerFireAccessDecision::Allowed);

            let unknown = checker
                .check_trigger_fire_access(check("never-logged-in", Some("agent"), None))
                .await
                .expect("check");
            assert!(matches!(unknown, TriggerFireAccessDecision::Denied { .. }));

            // Suspension revokes trigger-fire access (the new, stricter behavior).
            directory
                .update_status(&user_id, RebornUserStatus::Suspended)
                .await
                .expect("suspend");
            let after_suspend = checker
                .check_trigger_fire_access(TriggerFireAccessCheck {
                    tenant_id: tenant,
                    creator_user_id: user_id,
                    agent_id: Some(AgentId::new("agent").expect("agent")),
                    project_id: None,
                    trigger_id: ironclaw_triggers::TriggerId::new(),
                    fire_slot: chrono::Utc::now(),
                })
                .await
                .expect("check");
            assert!(matches!(
                after_suspend,
                TriggerFireAccessDecision::Denied { .. }
            ));
        }
    }
}
