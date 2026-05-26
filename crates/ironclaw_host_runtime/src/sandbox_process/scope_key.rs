use std::path::{Path, PathBuf};

use ironclaw_host_api::ResourceScope;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSandboxScopeKey {
    digest: String,
}

impl RebornSandboxScopeKey {
    pub fn from_scope(scope: &ResourceScope) -> Self {
        let mut raw_parts = vec![
            ("tenant", scope.tenant_id.as_str().to_string()),
            ("user", scope.user_id.as_str().to_string()),
        ];
        raw_parts.push((
            "agent",
            scope
                .agent_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_else(|| "_none".to_string()),
        ));
        if let Some(project_id) = &scope.project_id {
            raw_parts.push(("project", project_id.as_str().to_string()));
        } else if let Some(thread_id) = &scope.thread_id {
            raw_parts.push(("thread", thread_id.as_str().to_string()));
        } else {
            raw_parts.push(("invocation", scope.invocation_id.to_string()));
        }

        let raw = encode_scope_parts(&raw_parts);
        let digest = scope_digest(&raw);
        Self { digest }
    }

    pub fn workspace_path(&self, root: &Path) -> PathBuf {
        root.join("scopes").join(&self.digest)
    }

    pub fn container_name_prefix(&self) -> String {
        format!("ironclaw-reborn-sandbox-{}", &self.digest[..24])
    }
}

fn encode_scope_parts(parts: &[(&str, String)]) -> String {
    let mut encoded = String::new();
    for (kind, value) in parts {
        encoded.push_str(&kind.len().to_string());
        encoded.push(':');
        encoded.push_str(kind);
        encoded.push('=');
        encoded.push_str(&value.len().to_string());
        encoded.push(':');
        encoded.push_str(value);
        encoded.push(';');
    }
    encoded
}

fn scope_digest(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };

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
            project_id: project.map(|value| ProjectId::new(value).unwrap()),
            mission_id: None,
            thread_id: thread.map(|value| ThreadId::new(value).unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn scope_key_isolates_tenants_with_same_user_and_project() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxScopeKey::from_scope(&scope(
            "tenant-a",
            "same-user",
            Some("same-project"),
            None,
        ));
        let right = RebornSandboxScopeKey::from_scope(&scope(
            "tenant-b",
            "same-user",
            Some("same-project"),
            None,
        ));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name_prefix(), right.container_name_prefix());
    }

    #[test]
    fn scope_key_uses_thread_when_project_is_absent() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxScopeKey::from_scope(&scope("tenant", "user", None, Some("a")));
        let right = RebornSandboxScopeKey::from_scope(&scope("tenant", "user", None, Some("b")));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name_prefix(), right.container_name_prefix());
    }

    #[test]
    fn scope_key_does_not_collapse_path_special_characters() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxScopeKey::from_scope(&scope("tenant:a", "user", Some("p"), None));
        let right = RebornSandboxScopeKey::from_scope(&scope("tenant_a", "user", Some("p"), None));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name_prefix(), right.container_name_prefix());
    }
}
