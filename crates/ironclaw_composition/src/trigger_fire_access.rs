//! Fire-time trigger access checkers built from [`TriggerFireAccessPolicy`].
//!
//! These replaced the former `ironclaw_runner::local_trigger_access` shadow
//! store (arch-simplification §4.4). Trigger-fire authorization is no longer a
//! persisted parallel access table: it is either a pure comparison against a
//! config-supplied owner ([`StaticOwnerTriggerFireChecker`]) or a membership
//! lookup against the canonical identity directory the SSO login path already
//! populates ([`IdentityMembershipTriggerFireChecker`]). The composition build
//! selects one from [`TriggerFireAccessPolicy`] when the trigger poller is
//! enabled.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

use crate::runtime_input::{
    TriggerFireAccessCheck, TriggerFireAccessChecker, TriggerFireAccessDecision,
    TriggerFireAccessError,
};

const DENY_REASON: &str = "trigger creator does not have active access for this scope";

/// Does the fire-time check's exact scope match the granted `(agent, project)`
/// grant? Scope is exact — `None` project means "no project", never a wildcard
/// (matches [`TriggerFireAccessCheck`] semantics).
fn scope_matches(
    check: &TriggerFireAccessCheck,
    agent: &AgentId,
    project: &Option<ProjectId>,
) -> bool {
    check.agent_id.as_ref() == Some(agent) && &check.project_id == project
}

fn denied() -> TriggerFireAccessDecision {
    TriggerFireAccessDecision::Denied {
        reason: DENY_REASON.to_string(),
    }
}

/// A single configured owner may fire triggers for one exact scope — the
/// env-token `serve` and CLI `run` owner grant. Pure comparison, no I/O.
///
/// The `tenant_id` bound is load-bearing: the due-trigger repository is global,
/// so a fire-time check that matched only owner + scope could authorize a
/// foreign tenant's trigger whose creator id happened to equal this owner. The
/// former store keyed every row on tenant; this preserves that.
pub(crate) struct StaticOwnerTriggerFireChecker {
    tenant_id: TenantId,
    owner: UserId,
    agent: AgentId,
    project: Option<ProjectId>,
}

impl StaticOwnerTriggerFireChecker {
    pub(crate) fn new(
        tenant_id: TenantId,
        owner: UserId,
        agent: AgentId,
        project: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            owner,
            agent,
            project,
        }
    }
}

#[async_trait]
impl TriggerFireAccessChecker for StaticOwnerTriggerFireChecker {
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
        let allowed = request.tenant_id == self.tenant_id
            && request.creator_user_id == self.owner
            && scope_matches(&request, &self.agent, &self.project);
        Ok(if allowed {
            TriggerFireAccessDecision::Allowed
        } else {
            denied()
        })
    }
}

/// Any active member of the host tenant may fire triggers for one exact scope —
/// the SSO/WebUI deployment. Membership is resolved at fire time from the
/// canonical identity directory (the `StoredUser` records SSO login persists),
/// so a suspended, wrong-tenant, or unknown creator is denied. A directory
/// backend error surfaces as retryable `Unavailable`, never a hard denial.
pub(crate) struct IdentityMembershipTriggerFireChecker {
    directory: Arc<dyn ironclaw_identity::IronClawUserDirectory>,
    tenant_id: TenantId,
    agent: AgentId,
    project: Option<ProjectId>,
}

impl IdentityMembershipTriggerFireChecker {
    pub(crate) fn new(
        directory: Arc<dyn ironclaw_identity::IronClawUserDirectory>,
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
        if !scope_matches(&request, &self.agent, &self.project) {
            return Ok(denied());
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
        // back-compat, matching `IronClawUserDirectory` enumeration).
        let allowed = user.is_some_and(|user| {
            user.status == ironclaw_identity::IronClawUserStatus::Active
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
            denied()
        })
    }
}

/// OR-combines several checkers: `Allowed` if any grant allows; otherwise
/// `Unavailable` if any grant's backend was unavailable (retryable, so a
/// transient identity-store fault is not a hard denial); otherwise `Denied`.
pub(crate) struct CompositeTriggerFireChecker {
    checkers: Vec<Arc<dyn TriggerFireAccessChecker>>,
}

impl CompositeTriggerFireChecker {
    pub(crate) fn new(checkers: Vec<Arc<dyn TriggerFireAccessChecker>>) -> Self {
        Self { checkers }
    }
}

#[async_trait]
impl TriggerFireAccessChecker for CompositeTriggerFireChecker {
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
        // Split so the last checker takes `request` by move — no redundant
        // final clone (the common case is a single StaticOwner + SsoMembership
        // pair, so this saves one clone per fire).
        let Some((last, rest)) = self.checkers.split_last() else {
            return Ok(denied());
        };
        let mut unavailable: Option<TriggerFireAccessError> = None;
        for checker in rest {
            match checker.check_trigger_fire_access(request.clone()).await {
                Ok(TriggerFireAccessDecision::Allowed) => {
                    return Ok(TriggerFireAccessDecision::Allowed);
                }
                Ok(TriggerFireAccessDecision::Denied { .. }) => {}
                Err(error) => unavailable = Some(error),
            }
        }
        match last.check_trigger_fire_access(request).await {
            Ok(TriggerFireAccessDecision::Allowed) => Ok(TriggerFireAccessDecision::Allowed),
            Ok(TriggerFireAccessDecision::Denied { .. }) => match unavailable {
                Some(error) => Err(error),
                None => Ok(denied()),
            },
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn static_checker() -> StaticOwnerTriggerFireChecker {
        StaticOwnerTriggerFireChecker::new(
            TenantId::new("tenant").expect("tenant"),
            UserId::new("owner").expect("user"),
            AgentId::new("agent").expect("agent"),
            Some(ProjectId::new("project").expect("project")),
        )
    }

    #[tokio::test]
    async fn static_owner_allows_exact_owner_and_scope() {
        let decision = static_checker()
            .check_trigger_fire_access(check("owner", Some("agent"), Some("project")))
            .await
            .expect("check");
        assert_eq!(decision, TriggerFireAccessDecision::Allowed);
    }

    #[tokio::test]
    async fn static_owner_denies_non_owner() {
        let decision = static_checker()
            .check_trigger_fire_access(check("intruder", Some("agent"), Some("project")))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn static_owner_denies_scope_mismatch() {
        // Right owner, wrong project scope.
        let decision = static_checker()
            .check_trigger_fire_access(check("owner", Some("agent"), Some("other")))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
        // Right owner, missing project where one was granted.
        let decision = static_checker()
            .check_trigger_fire_access(check("owner", Some("agent"), None))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn static_owner_denies_foreign_tenant() {
        // The due-trigger repository is global: a foreign tenant's trigger with
        // a matching owner id + scope must NOT be authorized (regression guard).
        let foreign = TriggerFireAccessCheck {
            tenant_id: TenantId::new("other-tenant").expect("tenant"),
            creator_user_id: UserId::new("owner").expect("user"),
            agent_id: Some(AgentId::new("agent").expect("agent")),
            project_id: Some(ProjectId::new("project").expect("project")),
            trigger_id: ironclaw_triggers::TriggerId::new(),
            fire_slot: chrono::Utc::now(),
        };
        let decision = static_checker()
            .check_trigger_fire_access(foreign)
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn composite_allows_if_any_grant_allows() {
        // Two static owners; only the second matches the creator.
        let checkers: Vec<Arc<dyn TriggerFireAccessChecker>> = vec![
            Arc::new(StaticOwnerTriggerFireChecker::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new("owner-a").expect("user"),
                AgentId::new("agent").expect("agent"),
                Some(ProjectId::new("project").expect("project")),
            )),
            Arc::new(StaticOwnerTriggerFireChecker::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new("owner-b").expect("user"),
                AgentId::new("agent").expect("agent"),
                Some(ProjectId::new("project").expect("project")),
            )),
        ];
        let composite = CompositeTriggerFireChecker::new(checkers);
        let decision = composite
            .check_trigger_fire_access(check("owner-b", Some("agent"), Some("project")))
            .await
            .expect("check");
        assert_eq!(decision, TriggerFireAccessDecision::Allowed);
    }

    #[tokio::test]
    async fn composite_denies_if_no_grant_allows() {
        let checkers: Vec<Arc<dyn TriggerFireAccessChecker>> =
            vec![Arc::new(StaticOwnerTriggerFireChecker::new(
                TenantId::new("tenant").expect("tenant"),
                UserId::new("owner-a").expect("user"),
                AgentId::new("agent").expect("agent"),
                None,
            ))];
        let composite = CompositeTriggerFireChecker::new(checkers);
        let decision = composite
            .check_trigger_fire_access(check("stranger", Some("agent"), None))
            .await
            .expect("check");
        assert!(matches!(decision, TriggerFireAccessDecision::Denied { .. }));
    }

    mod identity {
        use super::*;
        use ironclaw_identity::{
            IronClawIdentityError, IronClawUser, IronClawUserDirectory, IronClawUserProfileUpdate,
            IronClawUserRole, IronClawUserStatus,
        };

        /// Directory double returning one configured user (or none), and a
        /// backend-error mode for the retryable-unavailable path.
        struct FakeDirectory {
            user: Option<IronClawUser>,
            fail: bool,
        }

        impl FakeDirectory {
            fn with_user(user: IronClawUser) -> Self {
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

        fn user(status: IronClawUserStatus, tenant: Option<&str>) -> IronClawUser {
            IronClawUser {
                user_id: UserId::new("member").expect("user"),
                email: None,
                display_name: None,
                status,
                role: IronClawUserRole::Member,
                created_at: String::new(),
                updated_at: String::new(),
                created_by: None,
                last_login_at: None,
                tenant_id: tenant.map(|t| TenantId::new(t).expect("tenant")),
                metadata: Default::default(),
            }
        }

        #[async_trait]
        impl IronClawUserDirectory for FakeDirectory {
            async fn list_users(
                &self,
                _tenant_id: &TenantId,
                _status: Option<IronClawUserStatus>,
                _after: Option<&UserId>,
                _limit: usize,
            ) -> Result<Vec<IronClawUser>, IronClawIdentityError> {
                Ok(self.user.clone().into_iter().collect())
            }
            async fn get_user(
                &self,
                _user_id: &UserId,
            ) -> Result<Option<IronClawUser>, IronClawIdentityError> {
                if self.fail {
                    return Err(IronClawIdentityError::Backend("backend down".to_string()));
                }
                Ok(self.user.clone())
            }
            async fn create_user(
                &self,
                _tenant_id: &TenantId,
                _email: Option<String>,
                _display_name: Option<String>,
                _role: IronClawUserRole,
                _created_by: &UserId,
            ) -> Result<IronClawUser, IronClawIdentityError> {
                unimplemented!("not used")
            }
            async fn update_profile(
                &self,
                _user_id: &UserId,
                _update: IronClawUserProfileUpdate,
            ) -> Result<IronClawUser, IronClawIdentityError> {
                unimplemented!("not used")
            }
            async fn update_status(
                &self,
                _user_id: &UserId,
                _status: IronClawUserStatus,
            ) -> Result<IronClawUser, IronClawIdentityError> {
                unimplemented!("not used")
            }
            async fn update_role(
                &self,
                _user_id: &UserId,
                _role: IronClawUserRole,
            ) -> Result<IronClawUser, IronClawIdentityError> {
                unimplemented!("not used")
            }
            async fn record_last_login(
                &self,
                _user_id: &UserId,
                _at: String,
            ) -> Result<(), IronClawIdentityError> {
                unimplemented!("not used")
            }
            async fn delete_user(
                &self,
                _tenant_id: &TenantId,
                _user_id: &UserId,
            ) -> Result<(), IronClawIdentityError> {
                unimplemented!("not used")
            }
            async fn count_active_admins(
                &self,
                _tenant_id: &TenantId,
            ) -> Result<usize, IronClawIdentityError> {
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
                IronClawUserStatus::Active,
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
                IronClawUserStatus::Active,
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
                IronClawUserStatus::Suspended,
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
                IronClawUserStatus::Active,
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
                IronClawUserStatus::Active,
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
                AgentId as HostAgentId, MountAlias, MountGrant, MountPermissions, MountView,
                UserId as HostUserId, VirtualPath,
            };
            use ironclaw_identity::{
                ExternalSubjectId, FilesystemIronClawIdentityStore, IronClawIdentityResolver,
                IronClawUserDirectory, IronClawUserStatus, ProviderKind, ResolveExternalIdentity,
                SurfaceKind,
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
            let store = Arc::new(FilesystemIronClawIdentityStore::new(
                filesystem,
                tenant.clone(),
                HostUserId::new("runtime-owner").expect("owner"),
                HostAgentId::new("agent").expect("agent"),
                None,
            ));

            // Admit a user exactly as the SSO login path does.
            let resolver: Arc<dyn IronClawIdentityResolver> = store.clone();
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

            let directory: Arc<dyn IronClawUserDirectory> = store.clone();
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
                .update_status(&user_id, IronClawUserStatus::Suspended)
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
