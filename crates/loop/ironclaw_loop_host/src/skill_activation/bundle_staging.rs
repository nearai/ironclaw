//! Copying an activated skill's files somewhere a host process can open them.
//!
//! A bundle lives in the database, and `builtin.shell` is a host process that can only open real
//! host paths. So an agent could read its own script and not run it, and re-derived the method the
//! skill existed to preserve. Staging writes the bundle's non-manifest files into the workspace,
//! which is host-backed wherever a shell exists, and tells the model the path. `SKILL.md` is not
//! staged: it already arrives as model context, and a second copy invites edits discovery never
//! reads.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};

/// Workspace-relative directory holding staged skill bundles.
///
/// Dot-prefixed so it reads as machine-managed, but deliberately NOT in the coding tools'
/// `DEFAULT_EXCLUDED_DIRS`: excluding it hides staged files from `glob`/`grep`/`list_dir`, and a
/// model that looks for its skill's script instead of using the injected path would not find it.
pub(crate) const STAGED_SKILLS_DIRNAME: &str = ".skills";

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
    /// `None` means staging did not happen, so the caller must not promise a path. Never fatal: a
    /// skill without its scripts is still usable; a turn that dies over a failed copy is not.
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
    /// Takes a READ-WRITE handle: the activation path's own workspace handle is read-only (it backs
    /// setup-marker reads) and fails closed on write.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
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
            // Unconditional: one round trip beats stat-then-write, and a stale copy is worse than
            // a redundant one.
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
    /// The working directory the model runs this skill's commands from.
    ///
    /// Plainly `/workspace/.skills/<name>`, correct only because the shell and the file tools now
    /// resolve `/workspace` to the same directory. Deriving it instead (measuring the staged dir
    /// against the shell's root) emitted a doubled per-caller segment that both tools resolved
    /// twice, so every command failed with a spawn error that read like a missing interpreter.
    fn runnable_dir(&self, _scope: &ResourceScope, skill_name: &str) -> String {
        format!("/workspace/{STAGED_SKILLS_DIRNAME}/{skill_name}")
    }
}

#[cfg(test)]
mod staging_tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    /// The path handed to the model must be the plain workspace spelling; a per-caller segment here
    /// is applied a second time by both the shell and the file tools, and the doubled directory
    /// does not exist.
    #[test]
    fn the_runnable_directory_is_the_plain_workspace_spelling() {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("alias"),
            VirtualPath::new("/projects/workspace").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::default()),
            view,
        ));
        let stager = WorkspaceSkillBundleStager::new(filesystem);
        let scope = ironclaw_host_api::resource::ResourceScope::local_default(
            UserId::new("ada").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");

        assert_eq!(
            stager.runnable_dir(&scope, "egfr-calc"),
            "/workspace/.skills/egfr-calc",
            "the model must be handed the same spelling both the shell and the file tools resolve; \
             any per-caller segment here is applied a second time by both and the directory does not \
             exist"
        );
    }

    fn test_stager() -> (
        WorkspaceSkillBundleStager<InMemoryBackend>,
        ironclaw_host_api::resource::ResourceScope,
    ) {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("alias"),
            VirtualPath::new("/projects/workspace").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::default()),
            view,
        ));
        let scope = ironclaw_host_api::resource::ResourceScope::local_default(
            UserId::new("ada").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        (WorkspaceSkillBundleStager::new(filesystem), scope)
    }

    fn file(relative_path: &str) -> StagedBundleFile {
        StagedBundleFile {
            relative_path: relative_path.to_string(),
            contents: b"print('hi')\n".to_vec(),
        }
    }

    /// Staging writes a bundle into the workspace, so the path it builds is the boundary that keeps
    /// a model-authored bundle from writing anywhere it likes. Every entry here escapes the skill's
    /// own directory, and the upstream validators are NOT what is under test -- this is the last
    /// check before a name becomes a path.
    #[test]
    fn a_bundle_path_that_escapes_the_skill_directory_is_refused() {
        for (skill_name, relative_path, escape) in [
            (
                "../elsewhere",
                "run.py",
                "a parent segment in the skill name",
            ),
            ("nested/skill", "run.py", "a separator in the skill name"),
            ("", "run.py", "an empty skill name"),
            (
                "egfr",
                "../../etc/passwd",
                "a parent segment in the file path",
            ),
            (
                "egfr",
                "scripts/../../../run.py",
                "a parent segment mid-path",
            ),
            ("egfr", "/etc/passwd", "an absolute file path"),
        ] {
            assert!(
                WorkspaceSkillBundleStager::<InMemoryBackend>::staged_path(
                    skill_name,
                    relative_path
                )
                .is_none(),
                "staging must refuse {escape} ({skill_name:?}, {relative_path:?}); it would place a \
                 model-authored file outside the skill's own directory"
            );
        }
    }

    /// The safe shape still resolves, so the guard above is not vacuously rejecting everything.
    #[test]
    fn a_bundle_path_inside_the_skill_directory_resolves() {
        let path = WorkspaceSkillBundleStager::<InMemoryBackend>::staged_path(
            "egfr-calc",
            "scripts/egfr.py",
        )
        .expect("a bundle-relative path inside the skill directory must resolve");
        assert_eq!(
            path.as_str(),
            "/workspace/.skills/egfr-calc/scripts/egfr.py"
        );
    }

    /// A bundle whose files ALL fail to stage must report nothing rather than hand the model a
    /// directory that does not exist. Staging is best-effort by design -- a skill that activates
    /// without its scripts is still useful, a turn that dies because a copy failed is not -- and
    /// `None` is what makes the caller fall back to the unstaged note.
    #[tokio::test]
    async fn a_bundle_with_no_stageable_file_reports_no_directory() {
        let (stager, scope) = test_stager();
        assert_eq!(
            stager.stage_bundle(&scope, "egfr", &[]).await,
            None,
            "an empty bundle has nothing to run, so there is no workdir to advertise"
        );
        assert_eq!(
            stager
                .stage_bundle(&scope, "egfr", &[file("../escape.py")])
                .await,
            None,
            "every entry was refused, so no directory was created and none may be advertised"
        );
    }

    /// One unsafe entry must not cost the whole bundle: the safe files still stage, the model still
    /// gets its workdir, and only the offending entry is dropped.
    #[tokio::test]
    async fn an_unsafe_entry_is_dropped_without_losing_the_rest_of_the_bundle() {
        let (stager, scope) = test_stager();
        let staged = stager
            .stage_bundle(
                &scope,
                "egfr",
                &[file("../escape.py"), file("scripts/egfr.py")],
            )
            .await;
        assert_eq!(
            staged.as_deref(),
            Some("/workspace/.skills/egfr"),
            "the safe file staged, so the skill is runnable and its workdir must be advertised"
        );
        let written = stager
            .filesystem
            .read_file(
                &scope,
                &ScopedPath::new("/workspace/.skills/egfr/scripts/egfr.py").expect("path"),
            )
            .await
            .expect("the safe bundle file must be readable at the advertised workdir");
        assert_eq!(written, b"print('hi')\n".to_vec());
        assert!(
            stager
                .filesystem
                .read_file(
                    &scope,
                    &ScopedPath::new("/workspace/.skills/escape.py").expect("path"),
                )
                .await
                .is_err(),
            "the refused entry must not have been written anywhere"
        );
    }
}
