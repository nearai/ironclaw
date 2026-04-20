//! Per-project host workspace directory resolution.
//!
//! Each engine v2 project gets a real directory on the host filesystem at
//! `~/.ironclaw/projects/<user_id>/<project_id>/`. That's the directory the
//! user can see, edit, and back up. It's also the bind-mount source for the
//! per-project sandbox container's `/project/` mount.
//!
//! [`Project::workspace_path`] can override the default; otherwise the helpers
//! in this module compute and create the standard path. The engine crate
//! intentionally doesn't know about `~/.ironclaw` — that's a host-side concept
//! kept here so the engine stays portable.

use std::io;
use std::path::{Component, Path, PathBuf};

use ironclaw_engine::Project;

use crate::bootstrap::ironclaw_base_dir;

/// Subdirectory under [`ironclaw_base_dir`] that holds per-project workspaces.
pub const PROJECTS_SUBDIR: &str = "projects";

/// Resolve the host-filesystem workspace path for a project.
///
/// If the project has an explicit `workspace_path` override, that is returned
/// verbatim. Otherwise the default is
/// `~/.ironclaw/projects/<user_id>/<project_id>/` — namespaced by user so
/// multi-tenant deployments never collide on disk.
pub fn project_workspace_path(project: &Project) -> PathBuf {
    if let Some(ref explicit) = project.workspace_path {
        return explicit.clone();
    }
    default_project_workspace_path(&project.user_id, project.id.0)
}

/// Compute the default host workspace path for a project id, ignoring any
/// override on the [`Project`] record. Namespaced by `user_id` for
/// multi-tenant safety.
pub fn default_project_workspace_path(user_id: &str, project_id: uuid::Uuid) -> PathBuf {
    ironclaw_base_dir()
        .join(PROJECTS_SUBDIR)
        .join(sanitize_path_component(user_id))
        .join(project_id.to_string())
}

/// Sanitize a string for use as a single path component.
///
/// Rejects `..`, `/`, and `\` so a malicious `user_id` like `../../etc`
/// cannot escape the projects directory via `PathBuf::join`.
fn sanitize_path_component(s: &str) -> String {
    if s.is_empty() {
        // Empty input would produce an empty hex string, which is a no-op
        // in `PathBuf::join` and would drop the tenant namespace.
        return "_anonymous".to_string();
    }
    let p = Path::new(s);
    let safe = p.components().all(|c| matches!(c, Component::Normal(_)));
    if safe {
        s.to_string()
    } else {
        // Fall back to a hex-encoded representation that is always a
        // single safe component.
        hex::encode(s.as_bytes())
    }
}

/// Create the project workspace directory if it does not exist, returning the
/// resolved path. Idempotent. On Unix the directory is created with mode 0700
/// so secrets accidentally written into the workspace are not world-readable.
pub fn ensure_project_workspace_dir(project: &Project) -> io::Result<PathBuf> {
    let path = project_workspace_path(project);
    ensure_dir(&path)?;
    Ok(path)
}

/// Collect directories that need to be created, then create them and
/// tighten permissions on each one we actually created.
fn ensure_dir(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    // Walk upwards to find which ancestors don't exist yet, so we can
    // tighten permissions on all of them (not just the leaf).
    let mut to_tighten: Vec<PathBuf> = Vec::new();
    {
        let mut cur = path.to_path_buf();
        while !cur.exists() {
            to_tighten.push(cur.clone());
            match cur.parent() {
                Some(p) if p != cur => cur = p.to_path_buf(),
                _ => break,
            }
        }
    }
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Tighten every directory we just created. Without this,
        // intermediates like `projects/` and `projects/<user_id>/`
        // inherit umask defaults (potentially 0o755), letting other
        // host users traverse the directory tree.
        for dir in &to_tighten {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_engine::Project;

    #[test]
    fn override_path_is_returned_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new("u", "test", "").with_workspace_path(dir.path().to_path_buf());
        assert_eq!(project_workspace_path(&project), dir.path());
    }

    #[test]
    fn default_path_is_namespaced_by_user_and_project() {
        let project = Project::new("alice", "test", "");
        let path = project_workspace_path(&project);
        let base = ironclaw_base_dir();
        assert!(path.starts_with(&base));
        // Path is `<base>/projects/alice/<project_id>`
        let relative = path.strip_prefix(&base).unwrap();
        let components: Vec<_> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        assert_eq!(components[0], "projects");
        assert_eq!(components[1], "alice");
        assert_eq!(components[2], project.id.0.to_string());
    }

    #[test]
    fn empty_user_id_does_not_drop_namespace() {
        let id = uuid::Uuid::new_v4();
        let path = default_project_workspace_path("", id);
        let base = ironclaw_base_dir().join(PROJECTS_SUBDIR);
        // Must have a non-empty component between projects/ and <project_id>.
        let relative = path.strip_prefix(&base).unwrap();
        let components: Vec<_> = relative.components().collect();
        assert!(
            components.len() >= 2,
            "empty user_id must still produce a namespace component, got {path:?}"
        );
    }

    #[test]
    fn adversarial_user_id_does_not_escape_projects_dir() {
        let base = ironclaw_base_dir().join(PROJECTS_SUBDIR);
        for adversarial in ["../../etc", "../root", "a/../../b", "foo/bar"] {
            let path = default_project_workspace_path(adversarial, uuid::Uuid::new_v4());
            assert!(
                path.starts_with(&base),
                "user_id={adversarial:?} must stay under {base:?}, got {path:?}"
            );
        }
    }

    #[test]
    fn different_users_get_different_paths() {
        let id = ironclaw_engine::ProjectId::new();
        let p1 = default_project_workspace_path("alice", id.0);
        let p2 = default_project_workspace_path("bob", id.0);
        assert_ne!(p1, p2);
        assert!(p1.to_string_lossy().contains("alice"));
        assert!(p2.to_string_lossy().contains("bob"));
    }

    #[test]
    fn ensure_creates_idempotent() {
        let parent = tempfile::tempdir().unwrap();
        let project =
            Project::new("u", "test", "").with_workspace_path(parent.path().join("proj-x"));

        let p1 = ensure_project_workspace_dir(&project).unwrap();
        assert!(p1.exists());
        // second call: still ok, still exists
        let p2 = ensure_project_workspace_dir(&project).unwrap();
        assert_eq!(p1, p2);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p1).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "workspace dir should be 0700");
        }
    }

    #[test]
    fn ensure_tightens_intermediate_dirs() {
        let parent = tempfile::tempdir().unwrap();
        // Multi-level: parent/a/b/c — all three should get 0o700.
        let project = Project::new("u", "test", "")
            .with_workspace_path(parent.path().join("a").join("b").join("c"));

        let p = ensure_project_workspace_dir(&project).unwrap();
        assert!(p.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["a", "a/b", "a/b/c"] {
                let dir = parent.path().join(name);
                let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
                assert_eq!(
                    mode, 0o700,
                    "intermediate dir '{name}' should be 0o700, got {mode:o}"
                );
            }
        }
    }
}
