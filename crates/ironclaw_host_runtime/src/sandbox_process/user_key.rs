//! Coarse `{tenant, user}` container identity key for the persistent
//! per-user sandbox container model (Phase A). Unlike
//! [`super::scope_key::RebornSandboxScopeKey`] (fine-grained; includes
//! agent/project/thread/invocation and is used for nothing
//! container-related after this phase), this key derives container name
//! and workspace root from `{tenant_id, user_id}` ONLY — every
//! thread/project/agent for the same user shares one container.

use std::path::{Path, PathBuf};

use ironclaw_host_api::{ResourceScope, TenantId, UserId};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RebornSandboxUserKey {
    digest: String,
}

impl RebornSandboxUserKey {
    pub fn from_scope(scope: &ResourceScope) -> Self {
        Self::from_tenant_user(&scope.tenant_id, &scope.user_id)
    }

    /// Scope-free constructor: builds the same digest `from_scope` would,
    /// from just the `{tenant_id, user_id}` pair. This is what Task A5's
    /// reaper needs — a `ContainerSummary`'s `ironclaw.tenant`/
    /// `ironclaw.user` labels are exactly a `{TenantId, UserId}` pair, not
    /// a reconstructable `ResourceScope` (no agent/project/thread/
    /// invocation survive on a label). One formula, two entry points.
    pub fn from_tenant_user(tenant_id: &TenantId, user_id: &UserId) -> Self {
        let raw = format!(
            "tenant:{}:{}|user:{}:{}",
            tenant_id.as_str().len(),
            tenant_id.as_str(),
            user_id.as_str().len(),
            user_id.as_str(),
        );
        Self {
            digest: hex::encode(Sha256::digest(raw.as_bytes())),
        }
    }

    pub fn workspace_path(&self, root: &Path) -> PathBuf {
        root.join("users").join(&self.digest)
    }

    pub fn container_name(&self) -> String {
        format!("ironclaw-reborn-sandbox-user-{}", &self.digest[..24])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ThreadId};

    fn scope(
        tenant: &str,
        user: &str,
        project: Option<&str>,
        thread: Option<&str>,
    ) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: project.map(|v| ProjectId::new(v).unwrap()),
            mission_id: None,
            thread_id: thread.map(|v| ThreadId::new(v).unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn one_container_key_per_user_regardless_of_project_or_thread() {
        let root = Path::new("/tmp/reborn-sandbox");
        let a = RebornSandboxUserKey::from_scope(&scope("t", "u", Some("proj-a"), None));
        let b =
            RebornSandboxUserKey::from_scope(&scope("t", "u", Some("proj-b"), Some("thread-x")));

        assert_eq!(a.workspace_path(root), b.workspace_path(root));
        assert_eq!(a.container_name(), b.container_name());
    }

    #[test]
    fn key_isolates_tenants_with_same_user() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxUserKey::from_scope(&scope("tenant-a", "same-user", None, None));
        let right = RebornSandboxUserKey::from_scope(&scope("tenant-b", "same-user", None, None));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name(), right.container_name());
    }

    #[test]
    fn key_isolates_users_within_same_tenant() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxUserKey::from_scope(&scope("tenant", "user-a", None, None));
        let right = RebornSandboxUserKey::from_scope(&scope("tenant", "user-b", None, None));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
    }

    #[test]
    fn length_prefixing_prevents_boundary_collision() {
        // Without a length-prefixed encoding, tenant="a", user="b:c" and
        // tenant="a:b", user="c" would hash identically after naive
        // concatenation. Regression for that class of collision.
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxUserKey::from_scope(&scope("a", "b:c", None, None));
        let right = RebornSandboxUserKey::from_scope(&scope("a:b", "c", None, None));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
    }

    #[test]
    fn from_tenant_user_matches_from_scope_for_the_same_pair() {
        // Task A5's reaper only ever has `{tenant, user}` label strings to
        // work with (no agent/project/thread/invocation survive on a
        // Docker label) — this constructor must produce the exact same
        // digest `from_scope` would, or the reaper's key would never match
        // the activity registry's key for a container it just listed.
        let root = Path::new("/tmp/reborn-sandbox");
        let via_scope =
            RebornSandboxUserKey::from_scope(&scope("t", "u", Some("proj-a"), Some("thread-x")));
        let via_pair = RebornSandboxUserKey::from_tenant_user(
            &TenantId::new("t").unwrap(),
            &UserId::new("u").unwrap(),
        );

        assert_eq!(
            via_scope.workspace_path(root),
            via_pair.workspace_path(root)
        );
        assert_eq!(via_scope.container_name(), via_pair.container_name());
    }
}
