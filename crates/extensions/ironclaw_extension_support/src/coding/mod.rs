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
    dispatch::{CapabilityDisplayOutputPreview, RuntimeDispatchErrorKind},
    ids::{CapabilityId, RunId},
    mount::MountView,
    resource::ResourceScope,
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
    // `output_bytes` feeds nothing but the latency trace, and both `trace_tool_*`
    // return immediately on `None` fields — so measuring first paid a full
    // serialization pass over every successful `read_file` / `write_file` /
    // `apply_patch` / `list_dir` / `grep` result on every deployment that has
    // not turned the `ironclaw_latency` TRACE target on, which is all of them
    // by default. `ironclaw_observability`'s charter is zero-cost-when-off; the
    // trace was, this field was not (#7103). The two neighbouring constructors
    // in `latency.rs` already check before measuring; this now matches them.
    //
    // Not the same as `web_access.rs` / `gsuite/handlers.rs`, which also call
    // `json_bytes` unconditionally: there the value feeds
    // `ResourceUsage::set_output_bytes`, i.e. resource accounting, which must
    // happen whether or not anyone is tracing. Those are correct as written.
    if fields.is_none() {
        return;
    }
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

    use ironclaw_filesystem::{
        DiskFilesystem, Fault, FaultInjecting, FaultKind, FilesystemError, FilesystemOperation,
        RootFilesystem,
    };
    use ironclaw_host_api::{
        dispatch::RuntimeDispatchErrorKind,
        error::HostApiError,
        ids::{CapabilityId, InvocationId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{HostPath, MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use ironclaw_loop_contracts::LoopSafeSummary;
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

    /// #7103: `output_bytes` feeds only the latency trace, and `trace_tool_ok` /
    /// `trace_tool_error` both return immediately when latency tracing is off —
    /// but the measurement ran first, so every successful coding-tool call paid
    /// a full serialization pass over its output on deployments that never
    /// enabled the `ironclaw_latency` TRACE target.
    ///
    /// Driven through `CodingCapabilityState::dispatch`, the public entry point,
    /// rather than `trace_coding_latency` directly: the guard is only worth
    /// anything if the real dispatch path honours it, and `dispatch` also builds
    /// the latency fields that decide whether it should.
    ///
    /// The counter's own liveness is asserted in the same test — a probe that
    /// never increments would report "no serialization" forever.
    #[tokio::test]
    async fn coding_dispatch_does_not_measure_output_bytes_when_latency_tracing_is_off() {
        use crate::latency::JSON_BYTES_CALLS;

        assert!(
            !ironclaw_observability::live_latency_enabled(),
            "this test asserts the tracing-off path; a subscriber enabling the \
             `ironclaw_latency` TRACE target would make it pass vacuously"
        );

        let temp_root = tempfile::TempDir::new().expect("temp root");
        std::fs::create_dir_all(temp_root.path().join("workspace")).expect("workspace dir");
        std::fs::write(
            temp_root.path().join("workspace/big.txt"),
            "some text worth not serializing\n".repeat(512),
        )
        .expect("seed a payload worth not serializing");
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
            UserId::new("latency-off-user").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let state = super::CodingCapabilityState::default();
        let capability_id = CapabilityId::new("builtin.read_file").expect("capability id");
        let input = json!({ "path": "/workspace/big.txt" });
        let request = super::CodingCapabilityRequest::new(
            &capability_id,
            super::CodingCapabilityKind::ReadFile,
            &scope,
            None,
            Some(&mounts),
            Arc::clone(&filesystem),
            &input,
        );

        JSON_BYTES_CALLS.with(|calls| calls.set(0));
        let output = state.dispatch(&request).await.expect("read file");
        assert!(
            output.output["content"]
                .as_str()
                .is_some_and(|content| { content.len() > 1024 }),
            "the dispatch must actually have produced a large output, or there \
             would be nothing worth not serializing"
        );

        assert_eq!(
            JSON_BYTES_CALLS.with(std::cell::Cell::get),
            0,
            "with latency tracing off, dispatch must not serialize the tool \
             output to count its bytes"
        );

        // The probe is live: the same helper still counts when it is called.
        crate::latency::json_bytes(&output.output);
        assert_eq!(
            JSON_BYTES_CALLS.with(std::cell::Cell::get),
            1,
            "the call counter itself must work, or the assertion above proves \
             nothing"
        );
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
            Self::with_filesystem(user, |filesystem| Arc::new(filesystem))
        }

        fn with_faults(user: &str, faults: Vec<Fault>) -> Self {
            Self::with_filesystem(user, move |filesystem| {
                let filesystem = FaultInjecting::new(filesystem);
                for fault in faults {
                    filesystem.add_fault(fault);
                }
                Arc::new(filesystem)
            })
        }

        fn with_filesystem(
            user: &str,
            wrap: impl FnOnce(DiskFilesystem) -> Arc<dyn RootFilesystem>,
        ) -> Self {
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
            let filesystem = wrap(local_filesystem);
            let scope = ResourceScope::local_default(
                UserId::new(user).expect("user id"),
                InvocationId::new(),
            )
            .expect("resource scope");
            Self {
                _temp_root: temp_root,
                workspace_dir,
                filesystem,
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

    #[test]
    fn filesystem_summaries_preserve_error_class_without_backend_details() {
        let virtual_path =
            VirtualPath::new("/projects/private/backend/secret.rs").expect("virtual path");
        let cases = [
            (
                FilesystemError::Unsupported {
                    path: virtual_path.clone(),
                    operation: FilesystemOperation::ReadFile,
                },
                RuntimeDispatchErrorKind::FilesystemDenied,
                "permission denied or unsupported path",
            ),
            (
                FilesystemError::Backend {
                    path: virtual_path.clone(),
                    operation: FilesystemOperation::ReadFile,
                    reason: "raw backend detail /host/secret".to_string(),
                },
                RuntimeDispatchErrorKind::Backend,
                "filesystem backend error",
            ),
            (
                FilesystemError::Contract(HostApiError::InvalidPath {
                    value: "raw/invalid/path".to_string(),
                    reason: "backend parser detail".to_string(),
                }),
                RuntimeDispatchErrorKind::InputEncode,
                "invalid path",
            ),
            (
                FilesystemError::BackendBusy {
                    path: virtual_path,
                    operation: FilesystemOperation::ReadFile,
                },
                RuntimeDispatchErrorKind::FilesystemDenied,
                "filesystem error",
            ),
        ];

        for (error, expected_kind, expected_reason) in cases {
            let mapped = super::paths::filesystem_error_with_summary(
                "grep",
                "/workspace\\nested/source.rs",
                error,
            );
            assert_eq!(mapped.kind(), expected_kind);
            let summary = mapped.safe_summary().expect("model-visible summary");
            assert_eq!(
                summary,
                format!("grep failed for path workspace nested source.rs: {expected_reason}")
            );
            assert!(LoopSafeSummary::new(summary.to_string()).is_ok());
            assert!(!summary.contains("backend detail") && !summary.contains("host secret"));
        }
    }

    #[tokio::test]
    async fn grep_missing_root_and_explicit_read_failure_are_actionable() {
        let missing_fixture = CodingFixture::new("grep-missing-root-user");
        let missing = missing_fixture
            .dispatch(
                super::CodingCapabilityKind::Grep,
                json!({"path": "/workspace/missing.rs", "pattern": "needle"}),
            )
            .await
            .expect_err("missing grep root must fail");
        assert_eq!(missing.kind(), RuntimeDispatchErrorKind::OperationFailed);
        assert_eq!(
            missing.safe_summary(),
            Some("grep failed for path workspace missing.rs: file not found")
        );

        let read_failure_fixture = CodingFixture::with_faults(
            "grep-explicit-read-user",
            vec![
                Fault::on(FilesystemOperation::ReadFile)
                    .path("gone.rs")
                    .returning(FaultKind::NotFound),
            ],
        );
        std::fs::write(
            read_failure_fixture.workspace_dir.join("gone.rs"),
            "needle\n",
        )
        .expect("seed explicit file");
        let disappeared = read_failure_fixture
            .dispatch(
                super::CodingCapabilityKind::Grep,
                json!({"path": "/workspace/gone.rs", "pattern": "needle"}),
            )
            .await
            .expect_err("disappearing explicit file must fail");
        assert_eq!(
            disappeared.safe_summary(),
            Some("grep failed for path workspace gone.rs: file not found")
        );
    }

    #[tokio::test]
    async fn grep_directory_failures_are_reported_for_every_output_mode() {
        let fixture = CodingFixture::with_faults(
            "grep-partial-scan-user",
            vec![
                Fault::on(FilesystemOperation::Stat)
                    .path("stat-fail.rs")
                    .backend("stat backend detail /host/stat"),
                Fault::on(FilesystemOperation::ReadFile)
                    .path("read-fail.rs")
                    .backend("read backend detail /host/read"),
            ],
        );
        for name in ["ok.rs", "stat-fail.rs", "read-fail.rs"] {
            std::fs::write(fixture.workspace_dir.join(name), "needle\n").expect("seed file");
        }

        for output_mode in ["files_with_matches", "count", "content"] {
            let output = fixture
                .dispatch(
                    super::CodingCapabilityKind::Grep,
                    json!({
                        "path": "/workspace",
                        "pattern": "needle",
                        "output_mode": output_mode,
                    }),
                )
                .await
                .expect("partial directory scan remains successful")
                .output;

            assert_eq!(output["incomplete"], json!(true));
            assert_eq!(output["skipped_file_count"], json!(2));
            let skipped = output["skipped_files"]
                .as_array()
                .expect("skipped file diagnostics");
            assert!(skipped.contains(&json!({"file": "stat-fail.rs", "operation": "stat"})));
            assert!(skipped.contains(&json!({
                "file": "read-fail.rs",
                "operation": "read_file"
            })));
            assert!(
                !output.to_string().contains("backend detail")
                    && !output.to_string().contains("host stat")
                    && !output.to_string().contains("host read")
            );
        }
    }

    #[tokio::test]
    async fn grep_bounds_skipped_file_diagnostics_in_the_owning_crate() {
        let fixture = CodingFixture::with_faults(
            "grep-skipped-cap-user",
            vec![
                Fault::on(FilesystemOperation::Stat)
                    .path("skip-")
                    .backend("injected backend detail /host/secret"),
            ],
        );
        for index in 0..25 {
            std::fs::write(
                fixture.workspace_dir.join(format!("skip-{index:02}.rs")),
                "needle\n",
            )
            .expect("seed skipped file");
        }

        let output = fixture
            .dispatch(
                super::CodingCapabilityKind::Grep,
                json!({"path": "/workspace", "pattern": "needle"}),
            )
            .await
            .expect("bounded partial scan remains successful")
            .output;

        assert_eq!(output["incomplete"], json!(true));
        assert_eq!(output["skipped_file_count"], json!(25));
        assert_eq!(
            output["skipped_files"]
                .as_array()
                .expect("skipped file diagnostics")
                .len(),
            20
        );
        assert!(!output.to_string().contains("backend detail"));
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

    fn assert_binary_document_rejection(err: &super::CodingCapabilityError, file_hint: &str) {
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
        let summary = err
            .safe_summary()
            .expect("binary-document rejection must carry a model-visible reason");
        assert!(
            summary.contains(file_hint),
            "summary should name the file, got: {summary}"
        );
        assert!(
            summary.contains("binary documents cannot be edited with text tools"),
            "summary should explain why text editing is unsafe, got: {summary}"
        );
    }

    #[tokio::test]
    async fn write_file_rejects_opaque_document_target_without_touching_bytes() {
        let fixture = CodingFixture::new("opaque-document-user");
        let file = fixture.workspace_dir.join("review.docx");
        let original = b"opaque document bytes";
        std::fs::write(&file, original).expect("seed document");

        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/review.docx", "content": "replacement"}),
            )
            .await
            .expect_err("text write to DOCX must be rejected");

        assert_binary_document_rejection(&err, "review.docx");
        assert_eq!(
            std::fs::read(file).expect("document after rejected write"),
            original
        );
    }

    #[tokio::test]
    async fn extracted_rtf_read_does_not_authorize_raw_overwrite() {
        let fixture = CodingFixture::new("extracted-rtf-user");
        let file = fixture.workspace_dir.join("review.rtf");
        let original = br"{\rtf1\ansi Original RTF text}";
        std::fs::write(&file, original).expect("seed RTF document");

        let read = fixture
            .dispatch(
                super::CodingCapabilityKind::ReadFile,
                json!({"path": "/workspace/review.rtf"}),
            )
            .await
            .expect("read extracted RTF text");
        assert!(
            read.output["content"]
                .as_str()
                .expect("read content")
                .contains("Original RTF text")
        );

        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/review.rtf", "content": "replacement"}),
            )
            .await
            .expect_err("extracted text must not authorize a raw overwrite");

        assert_binary_document_rejection(&err, "review.rtf");
        assert_eq!(
            std::fs::read(file).expect("RTF after rejected write"),
            original
        );
    }

    #[tokio::test]
    async fn write_file_rejects_existing_pdf_but_allows_new_pdf() {
        let fixture = CodingFixture::new("pdf-write-user");
        let existing = fixture.workspace_dir.join("existing.pdf");
        let original = b"%PDF-1.4 existing\n";
        std::fs::write(&existing, original).expect("seed PDF");

        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/existing.pdf", "content": "replacement"}),
            )
            .await
            .expect_err("existing PDF overwrite must be rejected");
        assert_binary_document_rejection(&err, "existing.pdf");
        assert_eq!(
            std::fs::read(existing).expect("PDF after rejected write"),
            original
        );

        fixture
            .dispatch(
                super::CodingCapabilityKind::WriteFile,
                json!({"path": "/workspace/new.pdf", "content": "%PDF-1.4 new\n"}),
            )
            .await
            .expect("new PDF creation must remain supported");
        assert_eq!(
            std::fs::read(fixture.workspace_dir.join("new.pdf")).expect("new PDF"),
            b"%PDF-1.4 new\n"
        );
    }

    #[tokio::test]
    async fn apply_patch_rejects_probe_clean_document_path_after_full_read() {
        let fixture = CodingFixture::new("pdf-patch-user");
        let file = fixture.workspace_dir.join("review.pdf");
        let original = b"alpha beta\n";
        std::fs::write(&file, original).expect("seed probe-clean PDF path");

        fixture
            .dispatch(
                super::CodingCapabilityKind::ReadFile,
                json!({"path": "/workspace/review.pdf"}),
            )
            .await
            .expect("read probe-clean PDF path as raw text");

        let err = fixture
            .dispatch(
                super::CodingCapabilityKind::ApplyPatch,
                json!({"path": "/workspace/review.pdf", "old_string": "alpha", "new_string": "gamma"}),
            )
            .await
            .expect_err("text patch to a PDF path must be rejected");

        assert_binary_document_rejection(&err, "review.pdf");
        assert_eq!(
            std::fs::read(file).expect("PDF after rejected patch"),
            original
        );
    }

    #[tokio::test]
    async fn read_file_falls_back_to_legacy_document_extraction() {
        let fixture = CodingFixture::new("legacy-document-user");
        let file = fixture.workspace_dir.join("legacy.doc");
        let mut original = b"Legacy document text".to_vec();
        original.extend_from_slice(&[0; 32]);
        std::fs::write(file, original).expect("seed legacy document");

        let read = fixture
            .dispatch(
                super::CodingCapabilityKind::ReadFile,
                json!({"path": "/workspace/legacy.doc"}),
            )
            .await
            .expect("legacy document extraction fallback succeeds");

        assert!(
            read.output["content"]
                .as_str()
                .expect("read content")
                .contains("Legacy document text")
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
