//! First-party coding capability handlers.
//!
//! Keep v1-compatible coding families in narrow modules. Host runtime adapts
//! already-authorized capability invocations into [`CodingCapabilityRequest`];
//! this module receives scoped paths and an explicit filesystem handle only.

mod config;
mod diff_preview;
mod file;
mod glob_tool;
mod grep_tool;
mod inputs;
mod patch;
mod paths;
mod state;
mod text;
mod types;

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityId, MountView, ResourceScope, RunId,
    RuntimeDispatchErrorKind,
};
use serde_json::Value;

use crate::latency::{
    FirstPartyToolLatencyFields, FirstPartyToolLatencyMetrics, json_bytes, started_at,
    trace_tool_error, trace_tool_ok,
};

use state::{SharedCodingEditLocks, SharedCodingReadStates};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingCapabilityKind {
    ReadFile,
    WriteFile,
    ListDir,
    Glob,
    Grep,
    ApplyPatch,
}

impl CodingCapabilityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ListDir => "list_dir",
            Self::Glob => "glob",
            Self::Grep => "grep",
            Self::ApplyPatch => "apply_patch",
        }
    }
}

#[derive(Clone)]
pub struct CodingCapabilityRequest<'a> {
    pub(crate) capability_id: &'a CapabilityId,
    pub(crate) kind: CodingCapabilityKind,
    pub(crate) scope: &'a ResourceScope,
    /// Loop turn-run identity; `None` for non-loop callers. Read-before-edit
    /// state is keyed on it so a recorded read never authorizes edits in a
    /// later run.
    pub(crate) run_id: Option<RunId>,
    pub(crate) mounts: Option<&'a MountView>,
    pub(crate) filesystem: Arc<dyn RootFilesystem>,
    pub(crate) input: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingCapabilityOutput {
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
}

impl CodingCapabilityOutput {
    pub fn new(output: Value) -> Self {
        Self {
            output,
            display_preview: None,
        }
    }

    pub fn with_display_preview(
        output: Value,
        display_preview: Option<CapabilityDisplayOutputPreview>,
    ) -> Self {
        Self {
            output,
            display_preview,
        }
    }
}

impl<'a> CodingCapabilityRequest<'a> {
    pub fn new(
        capability_id: &'a CapabilityId,
        kind: CodingCapabilityKind,
        scope: &'a ResourceScope,
        run_id: Option<RunId>,
        mounts: Option<&'a MountView>,
        filesystem: Arc<dyn RootFilesystem>,
        input: &'a Value,
    ) -> Self {
        Self {
            capability_id,
            kind,
            scope,
            run_id,
            mounts,
            filesystem,
            input,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("coding capability dispatch failed: {kind}")]
pub struct CodingCapabilityError {
    kind: RuntimeDispatchErrorKind,
    safe_summary: Option<String>,
}

impl CodingCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self {
            kind,
            safe_summary: None,
        }
    }

    pub fn with_safe_summary(
        kind: RuntimeDispatchErrorKind,
        safe_summary: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            safe_summary: Some(bound_safe_summary(safe_summary.into())),
        }
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn safe_summary(&self) -> Option<&str> {
        self.safe_summary.as_deref()
    }
}

#[derive(Debug, Default)]
pub struct CodingCapabilityState {
    edit_locks: SharedCodingEditLocks,
    read_states: SharedCodingReadStates,
}

impl CodingCapabilityState {
    pub async fn dispatch(
        &self,
        request: &CodingCapabilityRequest<'_>,
    ) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
        dispatch(request, &self.edit_locks, &self.read_states).await
    }
}

async fn dispatch(
    request: &CodingCapabilityRequest<'_>,
    edit_locks: &SharedCodingEditLocks,
    read_states: &SharedCodingReadStates,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    let started_at = started_at();
    let latency_fields = FirstPartyToolLatencyFields::from_input(
        request.capability_id,
        request.scope,
        request.input,
    );
    let result = match request.kind {
        CodingCapabilityKind::ReadFile => file::read_file(request, read_states)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::WriteFile => file::write_file(request, edit_locks, read_states).await,
        CodingCapabilityKind::ListDir => file::list_dir(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::Glob => glob_tool::glob(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::Grep => grep_tool::grep(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::ApplyPatch => {
            file::apply_patch(request, edit_locks, read_states).await
        }
    };
    trace_coding_latency(request, latency_fields.as_ref(), started_at, &result);
    result
}

fn trace_coding_latency(
    request: &CodingCapabilityRequest<'_>,
    fields: Option<&FirstPartyToolLatencyFields>,
    started_at: Option<std::time::Instant>,
    result: &Result<CodingCapabilityOutput, CodingCapabilityError>,
) {
    let output_bytes = result
        .as_ref()
        .ok()
        .map(|output| json_bytes(&output.output))
        .unwrap_or(0);

    match result {
        Ok(_) => trace_tool_ok(
            "first_party_coding_tool",
            request.kind.as_str(),
            fields,
            started_at,
            FirstPartyToolLatencyMetrics {
                output_bytes,
                ..FirstPartyToolLatencyMetrics::default()
            },
        ),
        Err(error) => trace_tool_error(
            "first_party_coding_tool",
            request.kind.as_str(),
            fields,
            started_at,
            error.kind().as_str(),
            FirstPartyToolLatencyMetrics {
                output_bytes,
                ..FirstPartyToolLatencyMetrics::default()
            },
        ),
    }
}

fn input_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn operation_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
}

fn operation_error_with_summary(summary: impl Into<String>) -> CodingCapabilityError {
    CodingCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::OperationFailed, summary)
}

fn bound_safe_summary(summary: String) -> String {
    const MAX_CHARS: usize = 512;
    const ELLIPSIS: &str = "...";
    let summary = summary.trim();
    let mut chars = summary.chars();
    let bounded: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        let truncated_limit = MAX_CHARS - ELLIPSIS.chars().count();
        let bounded: String = bounded.chars().take(truncated_limit).collect();
        format!("{bounded}{ELLIPSIS}")
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{DiskFilesystem, RootFilesystem};
    use ironclaw_host_api::{
        CapabilityId, HostPath, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        ResourceScope, RuntimeDispatchErrorKind, UserId, VirtualPath,
    };
    use ironclaw_turns::run_profile::LoopSafeSummary;
    use serde_json::json;

    #[test]
    fn coding_tools_do_not_select_runtime_backends() {
        let sources = [
            include_str!("file.rs"),
            include_str!("glob_tool.rs"),
            include_str!("grep_tool.rs"),
            include_str!("paths.rs"),
        ];
        for source in sources {
            assert!(!source.contains("ProcessBackendKind"));
            assert!(!source.contains("FilesystemBackendKind"));
        }
    }

    #[test]
    fn safe_summary_bound_includes_ellipsis_in_limit() {
        let summary = super::bound_safe_summary("x".repeat(600));

        assert_eq!(summary.chars().count(), 512);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn safe_summary_bound_leaves_exact_limit_unchanged() {
        let input = "x".repeat(512);

        assert_eq!(super::bound_safe_summary(input.clone()), input);
    }

    #[tokio::test]
    async fn coding_file_tools_treat_bare_workspace_prefix_as_scoped_alias() {
        let temp_root = tempfile::TempDir::new().expect("temp root");
        let mut local_filesystem = DiskFilesystem::new();
        local_filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("virtual path"),
                HostPath::from_path_buf(temp_root.path().to_path_buf()),
            )
            .expect("projects mount");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_filesystem);
        let mounts = workspace_mounts();
        let scope = ResourceScope::local_default(
            UserId::new("workspace-alias-user").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let state = super::CodingCapabilityState::default();
        let write_capability_id = CapabilityId::new("builtin.write_file").expect("capability id");
        let read_capability_id = CapabilityId::new("builtin.read_file").expect("capability id");

        let write_input = json!({
            "path": "workspace/demo/a.txt",
            "content": "hello"
        });
        let write_request = super::CodingCapabilityRequest::new(
            &write_capability_id,
            super::CodingCapabilityKind::WriteFile,
            &scope,
            None,
            Some(&mounts),
            Arc::clone(&filesystem),
            &write_input,
        );
        let write_output = state.dispatch(&write_request).await.expect("write file");

        assert_eq!(
            write_output.output["path"].as_str(),
            Some("/workspace/demo/a.txt")
        );
        let write_preview = write_output
            .display_preview
            .as_ref()
            .expect("write preview");
        assert_eq!(
            write_preview.subtitle.as_deref(),
            Some("/workspace/demo/a.txt")
        );
        assert!(
            write_preview
                .output_preview
                .contains("--- a/workspace/demo/a.txt\n+++ b/workspace/demo/a.txt"),
            "preview should use normalized path, got: {}",
            write_preview.output_preview
        );
        assert_eq!(
            filesystem
                .read_file(
                    &VirtualPath::new("/projects/workspace/demo/a.txt").expect("virtual path")
                )
                .await
                .expect("normalized write path exists"),
            b"hello".to_vec()
        );
        assert!(temp_root.path().join("workspace/demo/a.txt").exists());
        assert!(
            !temp_root
                .path()
                .join("workspace/workspace/demo/a.txt")
                .exists()
        );

        let read_input = json!({ "path": "workspace/demo/a.txt" });
        let read_request = super::CodingCapabilityRequest::new(
            &read_capability_id,
            super::CodingCapabilityKind::ReadFile,
            &scope,
            None,
            Some(&mounts),
            Arc::clone(&filesystem),
            &read_input,
        );
        let read_output = state.dispatch(&read_request).await.expect("read file");

        assert_eq!(
            read_output.output["path"].as_str(),
            Some("/workspace/demo/a.txt")
        );
        assert_eq!(
            read_output.output["content"].as_str(),
            Some("     1│ hello")
        );

        let url_like_input = json!({
            "path": "workspace/http://example.com/a.txt",
            "content": "blocked"
        });
        let url_like_request = super::CodingCapabilityRequest::new(
            &write_capability_id,
            super::CodingCapabilityKind::WriteFile,
            &scope,
            None,
            Some(&mounts),
            Arc::clone(&filesystem),
            &url_like_input,
        );
        let err = state
            .dispatch(&url_like_request)
            .await
            .expect_err("URL-like workspace alias path rejected");

        assert_eq!(err.kind(), RuntimeDispatchErrorKind::InputEncode);
        assert!(
            !temp_root
                .path()
                .join("workspace/http:/example.com/a.txt")
                .exists(),
            "URL-like path must not be normalized into a writable scoped path"
        );

        let reserved_workspace_file_input = json!({
            "path": "workspace//HEARTBEAT.md",
            "content": "blocked"
        });
        let reserved_workspace_file_request = super::CodingCapabilityRequest::new(
            &write_capability_id,
            super::CodingCapabilityKind::WriteFile,
            &scope,
            None,
            Some(&mounts),
            filesystem,
            &reserved_workspace_file_input,
        );
        let err = state
            .dispatch(&reserved_workspace_file_request)
            .await
            .expect_err("empty alias segments preserve reserved workspace file guard");

        assert_eq!(err.kind(), RuntimeDispatchErrorKind::InputEncode);
        assert!(
            !temp_root.path().join("workspace/HEARTBEAT.md").exists(),
            "reserved workspace memory file must not be written through empty alias segments"
        );
    }

    fn workspace_mounts() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("mount alias"),
            VirtualPath::new("/projects/workspace").expect("virtual path"),
            MountPermissions::read_write(),
        )])
        .expect("mount view")
    }

    struct CodingFixture {
        _temp_root: tempfile::TempDir,
        workspace_dir: std::path::PathBuf,
        filesystem: Arc<dyn RootFilesystem>,
        mounts: MountView,
        scope: ResourceScope,
        state: super::CodingCapabilityState,
    }

    impl CodingFixture {
        fn new(user: &str) -> Self {
            let temp_root = tempfile::TempDir::new().expect("temp root");
            let workspace_dir = temp_root.path().join("workspace");
            std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
            let mut local_filesystem = DiskFilesystem::new();
            local_filesystem
                .mount_local(
                    VirtualPath::new("/projects").expect("virtual path"),
                    HostPath::from_path_buf(temp_root.path().to_path_buf()),
                )
                .expect("projects mount");
            let scope = ResourceScope::local_default(
                UserId::new(user).expect("user id"),
                InvocationId::new(),
            )
            .expect("resource scope");
            Self {
                _temp_root: temp_root,
                workspace_dir,
                filesystem: Arc::new(local_filesystem),
                mounts: workspace_mounts(),
                scope,
                state: super::CodingCapabilityState::default(),
            }
        }

        async fn dispatch(
            &self,
            kind: super::CodingCapabilityKind,
            input: serde_json::Value,
        ) -> Result<super::CodingCapabilityOutput, super::CodingCapabilityError> {
            let capability_id =
                CapabilityId::new(format!("builtin.{}", kind.as_str())).expect("capability id");
            let request = super::CodingCapabilityRequest::new(
                &capability_id,
                kind,
                &self.scope,
                None,
                Some(&self.mounts),
                Arc::clone(&self.filesystem),
                &input,
            );
            self.state.dispatch(&request).await
        }
    }

    fn assert_read_before_edit_rejection(err: &super::CodingCapabilityError, file_hint: &str) {
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
        let summary = err
            .safe_summary()
            .expect("read-before-edit rejection must carry a model-visible reason");
        assert!(
            summary.contains(file_hint),
            "summary should name the file, got: {summary}"
        );
        assert!(
            summary.contains("read_file"),
            "summary should tell the model to use read_file, got: {summary}"
        );
    }

    #[tokio::test]
    async fn write_file_requires_reading_existing_files_first() {
        let fixture = CodingFixture::new("read-before-write-user");
        std::fs::write(fixture.workspace_dir.join("existing.txt"), "original").expect("seed file");

        // An existing file that was never read must not be blindly overwritten.
        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/existing.txt", "content": "blind overwrite"}),
            )
            .await
            .expect_err("write to unread existing file must be rejected");
        assert_read_before_edit_rejection(&err, "existing.txt");
        assert_eq!(
            std::fs::read_to_string(fixture.workspace_dir.join("existing.txt"))
                .expect("existing file"),
            "original",
            "rejected write must not touch the file"
        );

        // A brand-new file needs no prior read.
        fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/new.txt", "content": "fresh"}),
            )
            .await
            .expect("write to a new file succeeds without a prior read");

        // Reading the existing file unlocks the write.
        fixture
            .dispatch(
                super::CodingCapabilityKind::ReadFile,
                json!({"path": "/workspace/existing.txt"}),
            )
            .await
            .expect("read file");
        fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/existing.txt", "content": "informed overwrite"}),
            )
            .await
            .expect("write after read succeeds");
        assert_eq!(
            std::fs::read_to_string(fixture.workspace_dir.join("existing.txt"))
                .expect("existing file"),
            "informed overwrite"
        );
    }

    #[tokio::test]
    async fn apply_patch_requires_fresh_read_and_tracks_chained_edits() {
        let fixture = CodingFixture::new("stale-read-user");
        let file = fixture.workspace_dir.join("main.txt");
        std::fs::write(&file, "alpha beta\n").expect("seed file");

        // Unread file: rejected with the read-first recovery message.
        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::ApplyPatch,
                json!({"path": "/workspace/main.txt", "old_string": "alpha", "new_string": "gamma"}),
            )
            .await
            .expect_err("patch on an unread file must be rejected");
        assert_read_before_edit_rejection(&err, "main.txt");

        // read_file → apply_patch succeeds.
        fixture
            .dispatch(
                super::CodingCapabilityKind::ReadFile,
                json!({"path": "/workspace/main.txt"}),
            )
            .await
            .expect("read file");
        fixture
            .dispatch(
                super::CodingCapabilityKind::ApplyPatch,
                json!({"path": "/workspace/main.txt", "old_string": "alpha", "new_string": "gamma"}),
            )
            .await
            .expect("patch after read succeeds");

        // A successful edit refreshes the read state, so chained edits keep working.
        fixture
            .dispatch(
                super::CodingCapabilityKind::ApplyPatch,
                json!({"path": "/workspace/main.txt", "old_string": "beta", "new_string": "delta"}),
            )
            .await
            .expect("chained patch succeeds without an intervening read");
        assert_eq!(
            std::fs::read_to_string(&file).expect("patched file"),
            "gamma delta\n"
        );

        // Out-of-band modification invalidates the recorded read.
        std::fs::write(&file, "rewritten by someone else\n").expect("out-of-band write");
        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::ApplyPatch,
                json!({"path": "/workspace/main.txt", "old_string": "gamma", "new_string": "x"}),
            )
            .await
            .expect_err("patch on an out-of-band-modified file must be rejected");
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
        let summary = err
            .safe_summary()
            .expect("stale-read rejection must carry a model-visible reason");
        assert!(
            summary.contains("main.txt"),
            "summary should name the file, got: {summary}"
        );
        assert!(
            summary.contains("changed since"),
            "summary should say the file changed since the last read, got: {summary}"
        );
        assert!(
            summary.contains("read it again"),
            "summary should tell the model to re-read, got: {summary}"
        );
        assert_eq!(
            std::fs::read_to_string(&file).expect("file after rejected patch"),
            "rewritten by someone else\n",
            "rejected patch must not touch the file"
        );
    }

    #[tokio::test]
    async fn out_of_scope_path_rejection_names_the_path_and_available_roots() {
        // A model that targets a path outside the scoped mounts (the classic
        // failure: absolute paths like /testbed/... from a task description)
        // must learn WHY the call failed and which roots exist — a bare
        // input-encode category leaves it retrying the same call blind.
        //
        // FilesystemDenied maps to a Denied loop outcome, whose ONLY
        // model-visible channel is the safe summary itself. The summary must
        // therefore both pass the strict loop validator (which rejects `/`)
        // AND carry a delimiter-free rendering of the path and roots — a
        // raw-path summary would silently degrade to the generic category
        // sentence at the runtime boundary.
        let temp_root = tempfile::TempDir::new().expect("temp root");
        let mut local_filesystem = DiskFilesystem::new();
        local_filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("virtual path"),
                HostPath::from_path_buf(temp_root.path().to_path_buf()),
            )
            .expect("projects mount");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_filesystem);
        let mounts = workspace_mounts();
        let scope = ResourceScope::local_default(
            UserId::new("out-of-scope-user").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let state = super::CodingCapabilityState::default();
        let read_capability_id = CapabilityId::new("builtin.read_file").expect("capability id");

        let input = json!({ "path": "/testbed/replacer.go" });
        let request = super::CodingCapabilityRequest::new(
            &read_capability_id,
            super::CodingCapabilityKind::ReadFile,
            &scope,
            None,
            Some(&mounts),
            filesystem,
            &input,
        );
        let err = state
            .dispatch(&request)
            .await
            .expect_err("out-of-scope absolute path must be rejected");

        assert_eq!(err.kind(), RuntimeDispatchErrorKind::FilesystemDenied);
        let summary = err
            .safe_summary()
            .expect("rejection must carry a model-visible reason");
        assert!(
            LoopSafeSummary::new(summary.to_string()).is_ok(),
            "summary must survive the strict loop safe-summary validator \
             (otherwise it degrades to the generic category sentence and the \
             model never sees the reason), got: {summary}"
        );
        assert!(
            summary.contains("testbed replacer.go"),
            "summary should name the offending path, got: {summary}"
        );
        assert!(
            summary.contains("workspace"),
            "summary should name the available scoped roots, got: {summary}"
        );
    }

    #[tokio::test]
    async fn read_only_mount_write_rejection_carries_an_actionable_validated_reason() {
        // Writing through a read-only scoped mount must fail with
        // FilesystemDenied AND tell the model which path hit the permission
        // wall — and, as above, the reason must survive the strict loop
        // safe-summary validator because Denied outcomes have no diagnostic
        // detail channel.
        let temp_root = tempfile::TempDir::new().expect("temp root");
        std::fs::create_dir_all(temp_root.path().join("workspace")).expect("workspace dir");
        let mut local_filesystem = DiskFilesystem::new();
        local_filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("virtual path"),
                HostPath::from_path_buf(temp_root.path().to_path_buf()),
            )
            .expect("projects mount");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_filesystem);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("mount alias"),
            VirtualPath::new("/projects/workspace").expect("virtual path"),
            MountPermissions::read_only(),
        )])
        .expect("mount view");
        let scope = ResourceScope::local_default(
            UserId::new("read-only-write-user").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let state = super::CodingCapabilityState::default();
        let write_capability_id = CapabilityId::new("builtin.write_file").expect("capability id");

        let input = json!({ "path": "/workspace/notes.txt", "content": "hello" });
        let request = super::CodingCapabilityRequest::new(
            &write_capability_id,
            super::CodingCapabilityKind::WriteFile,
            &scope,
            None,
            Some(&mounts),
            filesystem,
            &input,
        );
        let err = state
            .dispatch(&request)
            .await
            .expect_err("write through a read-only mount must be rejected");

        assert_eq!(err.kind(), RuntimeDispatchErrorKind::FilesystemDenied);
        let summary = err
            .safe_summary()
            .expect("permission rejection must carry a model-visible reason");
        assert!(
            LoopSafeSummary::new(summary.to_string()).is_ok(),
            "summary must survive the strict loop safe-summary validator, got: {summary}"
        );
        assert!(
            summary.contains("workspace notes.txt"),
            "summary should name the denied path, got: {summary}"
        );
        assert!(
            summary.contains("does not permit"),
            "summary should say the mount refused the operation, got: {summary}"
        );
    }
}
