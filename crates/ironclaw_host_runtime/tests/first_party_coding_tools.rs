use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, LocalFilesystem,
    RootFilesystem,
};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, CapabilitySurfaceVersion, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID,
    HostRuntime, HostRuntimeServices, LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind,
    WRITE_FILE_CAPABILITY_ID, builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn builtin_coding_grep_reports_oversized_explicit_file_as_partial_result() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("huge.txt"),
        vec![b'x'; max_read_size() + 1],
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/huge.txt", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(grepped["files"], json!([]));
    assert_eq!(grepped["count"], json!(0));
    assert_eq!(grepped["truncated"], json!(true));
    assert_eq!(grepped["limit_reason"], json!("file_size_bytes"));
    assert_eq!(grepped["file_bytes"], json!(max_read_size() + 1));
    assert_eq!(grepped["max_file_bytes"], json!(max_read_size()));
}

#[tokio::test]
async fn builtin_coding_grep_skips_oversized_files_like_resilient_v1_search() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ok.rs"), "needle\n").unwrap();
    std::fs::write(
        temp.path().join("huge.txt"),
        vec![b'x'; max_read_size() + 1],
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(grepped["files"], json!(["ok.rs"]));
    assert_eq!(grepped["truncated"], json!(false));
}

#[tokio::test]
async fn builtin_coding_grep_reports_scan_budget_truncation_for_all_output_modes() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..50 {
        std::fs::write(temp.path().join(format!("file-{index:02}.rs")), "needle\n").unwrap();
    }

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(StatLenOverrideFilesystem {
        inner: filesystem,
        suffix: ".rs",
        len: max_read_size() as u64,
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let files = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_aggregate_scan_limit(&files);
    assert_eq!(files["count"], json!(6));
    assert_eq!(files["files"].as_array().unwrap().len(), 6);

    let counts = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle", "output_mode": "count"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_aggregate_scan_limit(&counts);
    assert_eq!(counts["total"], json!(6));
    assert_eq!(counts["counts"].as_array().unwrap().len(), 6);

    let content = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle", "output_mode": "content"}),
        context,
    )
    .await
    .unwrap();
    assert_aggregate_scan_limit(&content);
    assert!(content["content"].as_str().unwrap().contains("needle"));
}

#[tokio::test]
async fn builtin_coding_list_and_grep_skip_entries_when_stat_fails() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ok.rs"), "needle\n").unwrap();
    std::fs::write(temp.path().join("skip.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(StatFailureFilesystem {
        inner: filesystem,
        fail_suffix: "/skip.rs",
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let listed = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(listed["entries"], json!(["ok.rs (7B)"]));

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();
    assert_eq!(grepped["files"], json!(["ok.rs"]));
}

#[tokio::test]
async fn builtin_coding_grep_skips_entries_when_read_fails_during_directory_search() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ok.rs"), "needle\n").unwrap();
    std::fs::write(temp.path().join("skip.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(ReadFailureFilesystem {
        inner: filesystem,
        fail_suffix: "/skip.rs",
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let grepped = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(grepped["files"], json!(["ok.rs"]));
}

#[tokio::test]
async fn builtin_coding_grep_fails_on_explicit_file_read_error() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("fail.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(ReadFailureFilesystem {
        inner: filesystem,
        fail_suffix: "/fail.rs",
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let error = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/fail.rs", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_coding_grep_treats_backend_infrastructure_as_backend_failure() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("fail.rs"), "needle\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(ReadInfrastructureFailureFilesystem {
        inner: filesystem,
        fail_suffix: "/fail.rs",
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let error = invoke_with_context(
        &runtime,
        GREP_CAPABILITY_ID,
        json!({"path": "/workspace/fail.rs", "pattern": "needle"}),
        context,
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
}

#[tokio::test]
async fn builtin_coding_list_fails_when_visited_entry_budget_is_exceeded() {
    let temp = tempfile::tempdir().unwrap();
    let (_filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(ManySkippedEntriesFilesystem);

    let error = invoke_with_context(
        &runtime,
        LIST_DIR_CAPABILITY_ID,
        json!({"path": "/workspace"}),
        execution_context_with_mounts(coding_capability_ids(), mounts),
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Resource);
}

#[tokio::test]
async fn builtin_write_file_returns_unified_diff_display_preview() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {\n    old();\n}\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "content": "fn main() {\n    new();\n}\n"
        }),
        context,
    )
    .await;

    let preview = completed
        .display_preview
        .expect("write_file should attach display preview");
    assert_eq!(preview.output_kind, "unified_diff");
    assert_eq!(preview.subtitle.as_deref(), Some("/workspace/main.rs"));
    assert!(preview.output_preview.contains("--- a/workspace/main.rs"));
    assert!(preview.output_preview.contains("-    old();"));
    assert!(preview.output_preview.contains("+    new();"));
}

#[tokio::test]
async fn builtin_write_file_does_not_read_existing_content_for_write_only_mount() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "secret-old-content\n").unwrap();

    let permissions = MountPermissions {
        read: false,
        write: true,
        delete: false,
        list: false,
        execute: false,
    };
    let (filesystem, mounts) = mounted_filesystem(temp.path(), permissions);
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "content": "replacement\n"
        }),
        context,
    )
    .await;

    assert!(
        completed.display_preview.is_none(),
        "write-only authority must not expose old file contents through a diff preview"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.rs")).unwrap(),
        "replacement\n"
    );
}

#[tokio::test]
async fn builtin_write_file_new_file_returns_additions_only_diff_preview() {
    let temp = tempfile::tempdir().unwrap();
    // No pre-existing file — write_file creates it from scratch.

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({
            "path": "/workspace/new.rs",
            "content": "fn hello() {}\n"
        }),
        context,
    )
    .await;

    let preview = completed
        .display_preview
        .expect("write_file on new file should attach display preview");
    assert_eq!(preview.output_kind, "unified_diff");
    // Additions-only: summary must contain /-0
    let summary = preview.output_summary.as_deref().unwrap_or("");
    assert!(
        summary.contains("/-0"),
        "expected /-0 in summary for new-file write, got: {summary}"
    );
    // No deletion lines in the preview (only additions from the new file).
    let deletion_lines: Vec<_> = preview
        .output_preview
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .collect();
    assert!(
        deletion_lines.is_empty(),
        "unexpected deletion lines in new-file diff: {deletion_lines:?}"
    );
    assert!(
        preview.output_preview.contains("+fn hello() {}"),
        "expected addition line"
    );
}

#[tokio::test]
async fn builtin_apply_patch_returns_unified_diff_display_preview() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {\n    old();\n}\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let completed = invoke_completed_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "old_string": "old();",
            "new_string": "new();"
        }),
        context,
    )
    .await;

    let preview = completed
        .display_preview
        .expect("apply_patch should attach display preview");
    assert_eq!(preview.output_kind, "unified_diff");
    assert_eq!(
        preview.output_summary.as_deref(),
        Some("Edited 1 file: +1/-1")
    );
    assert!(preview.output_preview.contains("-    old();"));
    assert!(preview.output_preview.contains("+    new();"));
}

#[tokio::test]
async fn builtin_apply_patch_failure_reports_path_and_match_count() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {\n    old();\n}\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let failure = invoke_failure_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "old_string": "missing();",
            "new_string": "new();"
        }),
        context,
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    assert_eq!(
        failure.message.as_deref(),
        Some("apply_patch failed for path workspace main.rs: old_string matched 0 times")
    );
}

#[tokio::test]
async fn builtin_apply_patch_rejects_empty_old_string() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let failure = invoke_failure_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "old_string": "",
            "new_string": "new();"
        }),
        context,
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn builtin_apply_patch_ambiguous_match_reports_floor_count_without_full_scan() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.rs"),
        "old();\nold();\nold();\nold();\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let failure = invoke_failure_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "old_string": "old();",
            "new_string": "new();"
        }),
        context,
    )
    .await;

    // The uniqueness scan short-circuits after the second hit, so the count
    // is reported as a floor — not the exact total of 4.
    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    assert_eq!(
        failure.message.as_deref(),
        Some(
            "apply_patch failed for path workspace main.rs: old_string matched 2 or more times; \
set replace_all=true or provide a unique old_string"
        )
    );
}

#[tokio::test]
async fn builtin_apply_patch_replace_all_substitutes_every_occurrence() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "old();\nkeep();\nold();\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs"}),
        context.clone(),
    )
    .await
    .unwrap();

    let value = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.rs",
            "old_string": "old();",
            "new_string": "new();",
            "replace_all": true
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(value["replacements"], json!(2));
    let rewritten = std::fs::read_to_string(temp.path().join("main.rs")).unwrap();
    assert_eq!(rewritten, "new();\nkeep();\nnew();\n");
}

#[tokio::test]
async fn builtin_read_file_failure_reports_missing_path() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let failure = invoke_failure_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/missing.py"}),
        context,
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    assert_eq!(
        failure.message.as_deref(),
        Some("read_file failed for path workspace missing.py: file not found")
    );
}

#[tokio::test]
async fn builtin_coding_glob_reports_visited_entry_budget_as_truncated_result() {
    let temp = tempfile::tempdir().unwrap();
    let (_filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(ManySkippedEntriesFilesystem);

    let output = invoke_with_context(
        &runtime,
        GLOB_CAPABILITY_ID,
        json!({"path": "/workspace", "pattern": "*.txt"}),
        execution_context_with_mounts(coding_capability_ids(), mounts),
    )
    .await
    .unwrap();

    assert_eq!(output["truncated"], json!(true));
    assert_eq!(output["limit_reason"], json!("visited_entries"));
    assert_eq!(output["visited_entries"], json!(50_000));
    assert_eq!(output["max_visited_entries"], json!(50_000));
    assert_eq!(output["count"], json!(0));
    assert_eq!(output["files"], json!([]));
}

fn assert_aggregate_scan_limit(output: &Value) {
    assert_eq!(output["truncated"], json!(true));
    assert_eq!(output["limit_reason"], json!("aggregate_scan_bytes"));
    assert_eq!(output["bytes_scanned"], json!(60 * 1024 * 1024));
    assert_eq!(output["max_scan_bytes"], json!(64 * 1024 * 1024));
}

fn max_read_size() -> usize {
    10 * 1024 * 1024
}

async fn invoke_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> Result<Value, RuntimeFailureKind> {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

async fn invoke_completed_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> ironclaw_host_runtime::RuntimeCapabilityCompleted {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => *completed,
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

async fn invoke_failure_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> ironclaw_host_runtime::RuntimeCapabilityFailure {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

fn runtime_with_filesystem<F>(filesystem: F) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn registry() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

fn coding_capability_ids() -> [&'static str; 6] {
    [
        READ_FILE_CAPABILITY_ID,
        WRITE_FILE_CAPABILITY_ID,
        LIST_DIR_CAPABILITY_ID,
        GLOB_CAPABILITY_ID,
        GREP_CAPABILITY_ID,
        APPLY_PATCH_CAPABILITY_ID,
    ]
}

fn mounted_filesystem(path: &Path, permissions: MountPermissions) -> (LocalFilesystem, MountView) {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects/coding-pack").unwrap(),
            HostPath::from_path_buf(path.to_path_buf()),
        )
        .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/coding-pack").unwrap(),
        permissions,
    )])
    .unwrap();
    (filesystem, mounts)
}

struct StatFailureFilesystem {
    inner: LocalFilesystem,
    fail_suffix: &'static str,
}

#[async_trait]
impl RootFilesystem for StatFailureFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        if path.as_str().ends_with(self.fail_suffix) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
                reason: "injected stat failure".to_string(),
            });
        }
        self.inner.stat(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }
}

struct StatLenOverrideFilesystem {
    inner: LocalFilesystem,
    suffix: &'static str,
    len: u64,
}

#[async_trait]
impl RootFilesystem for StatLenOverrideFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let mut stat = self.inner.stat(path).await?;
        if path.as_str().ends_with(self.suffix) {
            stat.len = self.len;
        }
        Ok(stat)
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }
}

struct ReadFailureFilesystem {
    inner: LocalFilesystem,
    fail_suffix: &'static str,
}

#[async_trait]
impl RootFilesystem for ReadFailureFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        if path.as_str().ends_with(self.fail_suffix) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "injected read failure".to_string(),
            });
        }
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }
}

struct ReadInfrastructureFailureFilesystem {
    inner: LocalFilesystem,
    fail_suffix: &'static str,
}

#[async_trait]
impl RootFilesystem for ReadInfrastructureFailureFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        if path.as_str().ends_with(self.fail_suffix) {
            return Err(FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::ReadFile,
                reason: "injected read infrastructure failure".to_string(),
            });
        }
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }
}

struct ManySkippedEntriesFilesystem;

#[async_trait]
impl RootFilesystem for ManySkippedEntriesFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Ok((0..50_001)
            .map(|index| DirEntry {
                name: format!("skip-{index:05}.rs"),
                path: VirtualPath::new(format!("{}/skip-{index:05}.rs", path.as_str())).unwrap(),
                file_type: FileType::File,
            })
            .collect())
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        if !path.as_str().contains("/skip-") {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
                modified: None,
                sensitive: false,
            });
        }
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Stat,
            reason: "injected stat failure".to_string(),
        })
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReadFile,
            reason: "unexpected read".to_string(),
        })
    }

    async fn write_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
            reason: "unexpected write".to_string(),
        })
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::CreateDirAll,
            reason: "unexpected mkdir".to_string(),
        })
    }
}

fn execution_context_with_mounts<const N: usize>(
    grants: [&str; N],
    mounts: MountView,
) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant_with_mounts(grant, mounts.clone()))
            .collect(),
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        mounts,
    )
    .unwrap()
}

fn dispatch_grant_with_mounts(capability: &str, mounts: MountView) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability).unwrap(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: builtin_effects(),
            mounts,
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn builtin_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
    ]
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: builtin_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}
