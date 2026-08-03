//! Coarse `{tenant, user}` container identity key for the persistent
//! per-user sandbox container model (Phase A). Unlike
//! [`super::scope_key::RebornSandboxScopeKey`] (fine-grained; includes
//! agent/project/thread/invocation), this key derives container name and
//! workspace root from `{tenant_id, user_id}` ONLY — every thread/project/
//! agent for the same user shares one container.
//!
//! **Current vs. planned:** `RebornSandboxScopeKey` remains the key the
//! currently-wired ephemeral exec transport uses for workspace and container
//! naming today (see `sandbox_process.rs`). `RebornSandboxUserKey` is not
//! constructed by any production call site in this PR — it is reserved for
//! the future persistent per-user transport (Task A5's reaper and the
//! exec-based transport's per-user container reuse), which will replace
//! `RebornSandboxScopeKey` for container concerns once wired.

use std::path::{Path, PathBuf};

use ironclaw_host_api::{
    ids::{TenantId, UserId},
    resource::ResourceScope,
};

use crate::sandbox_process::key_codec::{digest_hex, encode_parts};

// Not constructed by any production call site in this PR: the consumer
// (the exec-based transport's per-user container reuse and Task A5's
// reaper) lands in a later PR on top of `exec_transport`, which is out of
// scope here (see this PR's description). `pub`/re-exported at the crate
// root today so a downstream composition PR can wire it in without an API
// change; `#[allow(dead_code)]` keeps this crate's lint gate quiet in the
// meantime rather than adding a fake caller.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct RebornSandboxUserKey {
    digest: String,
}

#[allow(dead_code)]
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
        let raw = encode_parts(&[
            ("tenant", tenant_id.as_str().to_string()),
            ("user", user_id.as_str().to_string()),
        ]);
        Self {
            digest: digest_hex(&raw),
        }
    }

    pub fn workspace_path(&self, root: &Path) -> PathBuf {
        root.join("users").join(&self.digest)
    }

    pub fn container_name(&self) -> String {
        let digest_prefix = self.digest.get(..24).unwrap_or(self.digest.as_str());
        format!("ironclaw-reborn-sandbox-user-{digest_prefix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{AgentId, InvocationId, ProjectId, ThreadId};

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
