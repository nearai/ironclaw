//! Copying an activated skill's files somewhere a host process can open them.
//!
//! A skill bundle lives in the database (`/tenants/<t>/users/<u>/skills/<name>/…`). `builtin.shell`
//! is a host process, so it can only open real host paths — it can never open a database row. The
//! consequence was measured three times, on three deployment shapes, always the same shape:
//!
//! ```text
//! glob scripts/egfr*        -> found
//! read_file scripts/egfr.py -> 2503 bytes
//! shell: python3 scripts/egfr.py -> No such file or directory
//! shell: python3 -c "<the whole algorithm re-typed inline>"
//! ```
//!
//! The agent can read its own script and cannot run it, so it re-derives the method the skill exists
//! to preserve — which is the precise failure a skill carrying a script is supposed to prevent.
//!
//! Staging closes that by writing the bundle's non-manifest files into the workspace, which IS
//! host-backed wherever a shell exists, and telling the model the path. `SKILL.md` is deliberately
//! not staged: it is already delivered as model context, and a second copy on disk invites edits that
//! discovery never reads.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    path::{ScopedPath, VirtualPath},
    resource::ResourceScope,
};

/// Workspace-relative directory holding staged skill bundles.
///
/// Dot-prefixed and listed in the coding tools' `DEFAULT_EXCLUDED_DIRS`, so staged copies never show
/// up in a workspace `glob`/`list_dir`/`grep` and cannot be mistaken for the user's own files.
pub const STAGED_SKILLS_DIRNAME: &str = ".skills";

/// One file of a bundle, ready to stage.
#[derive(Debug, Clone)]
pub struct StagedBundleFile {
    /// Bundle-relative path, e.g. `scripts/egfr.py`.
    pub relative_path: String,
    pub contents: Vec<u8>,
}

/// Writes an activated bundle's files where a host process can reach them.
///
/// A trait rather than a concrete type for the same reason [`crate::SetupMarkerSource`] is one: the
/// activation path must not know which filesystem backs the workspace, and a deployment with no
/// writable workspace simply supplies nothing.
#[async_trait]
pub trait SkillBundleStager: Send + Sync + std::fmt::Debug {
    /// Stages `files` for `skill_name` and returns the directory the model should run them from.
    ///
    /// Returning `None` means staging did not happen and the caller must not promise a path. This is
    /// never an error the turn should fail on: a skill whose scripts could not be staged is still a
    /// usable skill, whereas a turn that dies because a copy failed is not.
    async fn stage_bundle(
        &self,
        scope: &ResourceScope,
        skill_name: &str,
        files: &[StagedBundleFile],
    ) -> Option<String>;
}

/// Stages into the caller's workspace through a read-write scoped filesystem.
pub struct WorkspaceSkillBundleStager<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    /// The backend-facing root the SHELL's `/workspace` alias resolves to.
    ///
    /// Not the same place this stager writes. Under per-caller scoping the file tools address
    /// `<this>/tenants/<t>/users/<u>` while the shell's alias -- registered once at composition, with
    /// no notion of a caller -- addresses `<this>`. The staged directory has to be expressed against
    /// the shell's root or the model is handed a path that silently resolves somewhere else.
    shell_workspace_root: VirtualPath,
}

impl<F> std::fmt::Debug for WorkspaceSkillBundleStager<F>
where
    F: RootFilesystem + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceSkillBundleStager")
            .finish_non_exhaustive()
    }
}

impl<F> WorkspaceSkillBundleStager<F>
where
    F: RootFilesystem + 'static,
{
    /// Takes a READ-WRITE workspace handle.
    ///
    /// The read-only workspace handle the activation path already holds (it backs setup-marker reads)
    /// fails closed on write, so it cannot be reused here. Composition supplies
    /// `read_write_workspace_filesystem`, which is the documented single owner of that recipe.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, shell_workspace_root: VirtualPath) -> Self {
        Self {
            filesystem,
            shell_workspace_root,
        }
    }

    fn staged_path(skill_name: &str, relative_path: &str) -> Option<ScopedPath> {
        // Both segments are already validated upstream -- `skill_name` by `validate_skill_name` and
        // `relative_path` by `SkillFilePath` -- but this is the boundary where they become a path, so
        // it re-checks rather than trusting the caller.
        if skill_name.is_empty()
            || skill_name.contains('/')
            || skill_name.contains("..")
            || relative_path.starts_with('/')
            || relative_path.split('/').any(|segment| segment == "..")
        {
            return None;
        }
        ScopedPath::new(format!(
            "/workspace/{STAGED_SKILLS_DIRNAME}/{skill_name}/{relative_path}"
        ))
        .ok()
    }
}

#[async_trait]
impl<F> SkillBundleStager for WorkspaceSkillBundleStager<F>
where
    F: RootFilesystem + 'static,
{
    async fn stage_bundle(
        &self,
        scope: &ResourceScope,
        skill_name: &str,
        files: &[StagedBundleFile],
    ) -> Option<String> {
        if files.is_empty() {
            return None;
        }
        let mut staged_any = false;
        for file in files {
            let Some(path) = Self::staged_path(skill_name, &file.relative_path) else {
                tracing::debug!(
                    skill = %skill_name,
                    relative_path = %file.relative_path,
                    "refusing to stage a skill file with an unsafe bundle-relative path"
                );
                continue;
            };
            // Written unconditionally rather than compared first: the write is one round trip, a
            // stat-then-write is two, and a stale staged copy is worse than a redundant write. The
            // filesystem is the authority on whether the bytes changed.
            match self
                .filesystem
                .write_file(scope, &path, &file.contents)
                .await
            {
                Ok(()) => staged_any = true,
                Err(error) => {
                    tracing::debug!(
                        skill = %skill_name,
                        scoped_path = %path,
                        %error,
                        "could not stage a skill bundle file; the skill stays usable without it"
                    );
                }
            }
        }
        if !staged_any {
            return None;
        }
        Some(self.runnable_dir(scope, skill_name))
    }
}

impl<F> WorkspaceSkillBundleStager<F>
where
    F: RootFilesystem + 'static,
{
    /// The path the model should use as a working directory, derived from where the bytes actually
    /// landed rather than assumed from the alias.
    ///
    /// The two are not the same string. `/workspace` means `<root>/tenants/<t>/users/<u>` to the file
    /// tools under per-caller scoping and plain `<root>` to the shell, whose alias is registered once
    /// at composition and knows nothing about callers. Assuming either spelling produces a path that
    /// works for one tool and silently misses for the other -- which is the bug this staging exists to
    /// end, so it is not repeated here.
    ///
    /// Asks the filesystem where the staged directory really is, asks it where the shell's
    /// `/workspace` really is, and expresses one relative to the other. Falls back to the plain
    /// workspace-relative spelling when either is database-backed, which is the case where no process
    /// could run anything anyway.
    fn runnable_dir(&self, scope: &ResourceScope, skill_name: &str) -> String {
        let relative = format!("{STAGED_SKILLS_DIRNAME}/{skill_name}");
        let fallback = format!("/workspace/{relative}");
        let Ok(staged_scoped) = ScopedPath::new(format!("/workspace/{relative}")) else {
            return fallback;
        };
        // Where the bytes really landed, through this caller's own view.
        let Some(staged_host) = self.filesystem.host_path_for(scope, &staged_scoped) else {
            return fallback;
        };
        // Where the shell's `/workspace` really is -- deliberately NOT through this caller's view,
        // which would cancel the per-caller segment and produce a path the shell resolves elsewhere.
        let Some(shell_root_host) = self
            .filesystem
            .host_path_for_virtual(&self.shell_workspace_root)
        else {
            return fallback;
        };
        match staged_host.strip_prefix(&shell_root_host) {
            Ok(tail) => format!("/workspace/{}", tail.to_string_lossy()),
            // Staged outside what the shell calls the workspace: no spelling of `/workspace` reaches
            // it, so promising one would be worse than the plain relative form.
            Err(_) => fallback,
        }
    }
}
