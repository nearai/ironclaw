// arch-exempt: large_file, caller-tier coding-tool suite shares one runtime/mount fixture set, plan #4539
use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    DirEntry, DiskFilesystem, FileStat, FileType, FilesystemError, FilesystemOperation,
    RootFilesystem,
};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, CapabilitySurfaceVersion, CommandExecutionOutput,
    CommandExecutionRequest, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HostRuntime,
    HostRuntimeServices, LIST_DIR_CAPABILITY_ID, PostEditCheckConfig, READ_FILE_CAPABILITY_ID,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeProcessError,
    RuntimeProcessPort, SandboxCommandTransport, TenantSandboxProcessPort,
    WRITE_FILE_CAPABILITY_ID, builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_turns::run_profile::LoopSafeSummary;
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
    seed_read_state(&runtime, "/workspace/main.rs", context.clone()).await;

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
async fn builtin_write_file_maps_filesystem_provider_write_failure_to_backend() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(WriteFailureFilesystem {
        inner: filesystem,
        fail_suffix: "/main.rs",
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.rs", context.clone()).await;

    let error = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs", "content": "new\n"}),
        context,
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.rs")).unwrap(),
        "old\n"
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
async fn builtin_apply_patch_maps_filesystem_provider_write_failure_to_backend() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(WriteFailureFilesystem {
        inner: filesystem,
        fail_suffix: "/main.rs",
    });
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.rs", context.clone()).await;

    let error = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs", "old_string": "old", "new_string": "new"}),
        context,
    )
    .await
    .unwrap_err();

    assert_eq!(error, RuntimeFailureKind::Backend);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.rs")).unwrap(),
        "old\n"
    );
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
async fn builtin_apply_patch_accepts_multi_edit_with_fuzzy_unicode_matching() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.txt"),
        "hello\u{00A0}world\nrange: 1\u{2013}5\n\u{FF21}\u{FF22}\u{FF23}123\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.txt", context.clone()).await;

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "edits": [
                { "old_string": "hello world\n", "new_string": "hello universe\n" },
                { "old_string": "range: 1-5\nABC123\n", "new_string": "range: 10-50\nASCII\n" }
            ]
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(patched["replacements"], json!(2));
    assert_eq!(patched["match_method"], json!("FuzzyNormalization"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "hello universe\nrange: 10-50\nASCII\n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_fuzzy_match_preserves_unrelated_original_content() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.txt"),
        "target\u{00A0}text\nuntouched\u{00A0}text   \n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.txt", context.clone()).await;

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "old_string": "target text",
            "new_string": "changed text"
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(patched["match_method"], json!("FuzzyNormalization"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "changed text\nuntouched\u{00A0}text   \n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_ignores_null_edits_placeholder_for_single_edit() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("null.txt"), "old\n").unwrap();
    std::fs::write(temp.path().join("string-null.txt"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    for (file_name, edits) in [
        ("null.txt", Value::Null),
        ("string-null.txt", Value::String("null".to_string())),
    ] {
        seed_read_state(
            &runtime,
            &format!("/workspace/{file_name}"),
            context.clone(),
        )
        .await;
        let patched = invoke_with_context(
            &runtime,
            APPLY_PATCH_CAPABILITY_ID,
            json!({
                "path": format!("/workspace/{file_name}"),
                "old_string": "old",
                "new_string": "new",
                "edits": edits
            }),
            context.clone(),
        )
        .await
        .unwrap();

        assert_eq!(patched["success"], json!(true));
        assert_eq!(
            std::fs::read_to_string(temp.path().join(file_name)).unwrap(),
            "new\n"
        );
    }
}

#[tokio::test]
async fn builtin_apply_patch_ignores_top_level_null_placeholders_for_multi_edit() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.txt"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.txt", context.clone()).await;

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "old_string": "null",
            "new_string": Value::Null,
            "edits": [
                {"old_string": "old", "new_string": "new"}
            ]
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "new\n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_rejects_active_null_string_placeholders() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("old-null.txt"), "null\n").unwrap();
    std::fs::write(temp.path().join("new-null.txt"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    for (file_name, old_string, new_string, expected_content) in [
        ("old-null.txt", "null", "new", "null\n"),
        ("new-null.txt", "old", "null", "old\n"),
    ] {
        let failure = invoke_failure_with_context(
            &runtime,
            APPLY_PATCH_CAPABILITY_ID,
            json!({
                "path": format!("/workspace/{file_name}"),
                "old_string": old_string,
                "new_string": new_string
            }),
            context.clone(),
        )
        .await;

        assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(file_name)).unwrap(),
            expected_content
        );
    }
}

#[tokio::test]
async fn builtin_apply_patch_replace_all_replaces_fuzzy_matches_when_exact_text_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.txt"),
        "hello\u{00A0}world\nhello\u{2003}world\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.txt", context.clone()).await;

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "old_string": "hello world",
            "new_string": "hello universe",
            "replace_all": true
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(patched["replacements"], json!(2));
    assert_eq!(patched["match_method"], json!("FuzzyNormalization"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "hello universe\nhello universe\n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_replace_all_replaces_mixed_exact_and_fuzzy_matches() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.txt"),
        "hello world\nhello\u{00A0}world\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.txt", context.clone()).await;

    let patched = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "old_string": "hello world",
            "new_string": "hello universe",
            "replace_all": true
        }),
        context,
    )
    .await
    .unwrap();

    assert_eq!(patched["success"], json!(true));
    assert_eq!(patched["replacements"], json!(2));
    assert_eq!(patched["match_method"], json!("FuzzyNormalization"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "hello universe\nhello universe\n"
    );
}

#[tokio::test]
async fn builtin_apply_patch_rejects_duplicate_after_fuzzy_normalization() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.txt"),
        "hello world\nhello\u{00A0}world\n",
    )
    .unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.txt", context.clone()).await;

    let failure = invoke_failure_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "old_string": "hello world",
            "new_string": "hello universe"
        }),
        context,
    )
    .await;

    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    assert_eq!(
        failure.message.as_deref(),
        Some(
            "apply_patch failed for path workspace main.txt: old_string matched 2 times; set replace_all=true or provide a unique old_string"
        )
    );
}

#[tokio::test]
async fn builtin_coding_read_state_is_scoped_to_the_run() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.txt"), "original content\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let mut run_a = execution_context_with_mounts(coding_capability_ids(), mounts.clone());
    run_a.run_id = Some(RunId::new());
    // Same tenant/user/agent/project identity, but a different loop run and a
    // different tool call: a run that never read the file itself.
    let mut run_b = execution_context_with_mounts(coding_capability_ids(), mounts);
    run_b.run_id = Some(RunId::new());

    // Run A reads the file in full.
    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.txt"}),
        run_a.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["truncated"], json!(false));

    // Run B never read the file; run A's read (with a still-matching
    // fingerprint) must not authorize run B's edits.
    let failure = invoke_failure_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.txt", "content": "cross-run overwrite"}),
        run_b.clone(),
    )
    .await;
    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("read it in full with read_file"),
        "cross-run write rejection must carry the read-before-edit guidance, got: {message}"
    );
    let failure = invoke_failure_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/main.txt",
            "old_string": "original",
            "new_string": "patched"
        }),
        run_b,
    )
    .await;
    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "original content\n",
        "cross-run edits must be rejected before touching the file"
    );

    // Within the SAME run, the read from an earlier tool call still unlocks a
    // later tool call's edit: run scoping must not degrade to per-invocation
    // scoping, which would break the read -> edit flow entirely.
    let mut run_a_later_call = run_a;
    let later_invocation = InvocationId::new();
    run_a_later_call.invocation_id = later_invocation;
    run_a_later_call.resource_scope.invocation_id = later_invocation;
    let written = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/main.txt", "content": "same-run informed overwrite\n"}),
        run_a_later_call,
    )
    .await
    .unwrap();
    assert_eq!(written["success"], json!(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.txt")).unwrap(),
        "same-run informed overwrite\n"
    );
}

#[tokio::test]
async fn builtin_write_file_rejects_edit_when_default_read_was_truncated_by_lines() {
    let temp = tempfile::tempdir().unwrap();
    let original: String = (0..2_100).map(|index| format!("line {index}\n")).collect();
    std::fs::write(temp.path().join("long.txt"), &original).unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    // A default read of a >2,000-line file returns a truncated window.
    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/long.txt"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["truncated"], json!(true));
    assert_eq!(read["truncated_by"], json!("lines"));

    // The truncated read must not unlock a whole-file overwrite.
    let failure = invoke_failure_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/long.txt", "content": "blind overwrite"}),
        context.clone(),
    )
    .await;
    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("read it in full with read_file"),
        "rejection must tell the model to read the file in full, got: {message}"
    );
    assert!(
        message.contains("truncated"),
        "rejection must explain that a truncated read does not count, got: {message}"
    );

    // An explicit offset/limit read (even one wide enough to cover the whole
    // file) still does not unlock edits.
    let ranged = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/long.txt", "offset": 1, "limit": 2_100}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(ranged["truncated"], json!(false));
    let failure = invoke_failure_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/long.txt", "content": "blind overwrite"}),
        context,
    )
    .await;
    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);

    assert_eq!(
        std::fs::read_to_string(temp.path().join("long.txt")).unwrap(),
        original,
        "rejected writes must not touch the file"
    );
}

#[tokio::test]
async fn builtin_apply_patch_rejects_edit_when_default_read_was_truncated_by_bytes() {
    let temp = tempfile::tempdir().unwrap();
    // Fewer lines than the 2,000-line cap, but wide enough that the rendered
    // body exceeds the 64 KiB byte budget: byte truncation, not line truncation.
    let wide_line = "x".repeat(120);
    let original: String = std::iter::once("seed-marker line\n".to_string())
        .chain((0..900).map(|_| format!("{wide_line}\n")))
        .collect();
    std::fs::write(temp.path().join("wide.txt"), &original).unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let read = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/wide.txt"}),
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(read["truncated"], json!(true));
    assert_eq!(read["truncated_by"], json!("bytes"));

    // Even a patch anchored in the visible part of the window is rejected:
    // the guard requires the model to have seen the complete file.
    let failure = invoke_failure_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({
            "path": "/workspace/wide.txt",
            "old_string": "seed-marker line",
            "new_string": "patched line"
        }),
        context,
    )
    .await;
    assert_eq!(failure.kind, RuntimeFailureKind::OperationFailed);
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("read it in full with read_file"),
        "rejection must tell the model to read the file in full, got: {message}"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("wide.txt")).unwrap(),
        original,
        "rejected patches must not touch the file"
    );
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
async fn builtin_read_file_out_of_scope_rejection_reaches_the_model_through_the_summary() {
    // Loop-boundary pin for the "model-visible tool-failure reasons" feature:
    // an out-of-scope absolute path (copied verbatim from a task description)
    // must produce a failure whose message BOTH survives the strict loop
    // safe-summary validator (FilesystemDenied surfaces as a Denied loop
    // outcome, whose only model-visible channel is the summary) AND names the
    // path and the available scoped roots so the model can correct course.
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let failure = invoke_failure_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/testbed/replacer.go"}),
        context,
    )
    .await;

    // FilesystemDenied maps to Authorization at the runtime boundary.
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    let message = failure
        .message
        .as_deref()
        .expect("failure carries a reason");
    assert!(
        LoopSafeSummary::new(message.to_string()).is_ok(),
        "the reason must survive the strict loop safe-summary validator \
         instead of degrading to the generic category sentence: {message}"
    );
    assert!(
        message.contains("testbed replacer.go"),
        "the reason must name the offending path: {message}"
    );
    assert!(
        message.contains("workspace"),
        "the reason must name an available scoped root: {message}"
    );
    assert_eq!(
        failure.detail, None,
        "a validator-safe reason travels on the message; no diagnostic fallback needed"
    );
}

#[tokio::test]
async fn builtin_write_file_to_read_only_mount_reports_an_actionable_denial() {
    // A write through a read-only scoped mount must fail as a filesystem
    // denial AND tell the model which path hit the permission wall, through a
    // summary that survives the strict loop validator (the Denied loop
    // outcome has no diagnostic detail channel).
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let failure = invoke_failure_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/notes.txt", "content": "hello"}),
        context,
    )
    .await;

    // FilesystemDenied maps to Authorization at the runtime boundary.
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    let message = failure
        .message
        .as_deref()
        .expect("failure carries a reason");
    assert!(
        LoopSafeSummary::new(message.to_string()).is_ok(),
        "the reason must survive the strict loop safe-summary validator: {message}"
    );
    assert!(
        message.contains("workspace notes.txt"),
        "the reason must name the denied path: {message}"
    );
    assert!(
        message.contains("does not permit"),
        "the reason must say the mount refused the operation: {message}"
    );
    assert!(
        !temp.path().join("notes.txt").exists(),
        "the denied write must not touch the filesystem"
    );
}

#[tokio::test]
async fn builtin_read_file_extracts_supported_document_text() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("hello.pdf"),
        include_bytes!("../../../tests/fixtures/hello.pdf"),
    )
    .unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let output = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/hello.pdf", "limit": 5}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(output["path"], json!("/workspace/hello.pdf"));
    assert_eq!(output["truncated_by_default"], json!(false));
    assert!(
        output["content"]
            .as_str()
            .expect("read_file content")
            .contains("Hello")
    );
}

#[tokio::test]
async fn builtin_read_file_prefers_text_for_pdf_named_git_lfs_pointer() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("GPT4.pdf"),
        "version https://git-lfs.github.com/spec/v1\n\
oid sha256:dee926384a7c107a9b51273a99fca2aecb3ed6c27ba7ace0fba67a147a63d2aa\n\
size 7370286\n",
    )
    .unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let output = invoke_with_context(
        &runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/GPT4.pdf"}),
        context,
    )
    .await
    .unwrap();

    let content = output["content"].as_str().expect("read_file content");
    assert!(content.contains("version https://git-lfs.github.com/spec/v1"));
    assert!(content.contains("size 7370286"));
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

#[tokio::test]
async fn builtin_edit_tools_append_new_post_edit_check_findings_only() {
    // The operator-configured post-edit check runs after a successful edit and
    // surfaces its diagnostics to the model. A second edit whose check output
    // is identical must not repeat previously-reported lines (new-only diff).
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "alpha beta\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing(
        "error[E0308]: mismatched types\nwarning: unused variable `x`\n",
        1,
    ));
    let runtime = runtime_with_filesystem_process_port_and_post_edit_check(
        filesystem,
        Arc::clone(&check_port),
        PostEditCheckConfig::new(
            "cargo check --message-format=short 2>&1",
            Duration::from_secs(7),
        ),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    seed_read_state(&runtime, "/workspace/main.rs", context.clone()).await;
    assert!(
        check_port.requests().is_empty(),
        "read_file must not trigger the post-edit check"
    );

    let first_completed = invoke_completed_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs", "old_string": "alpha", "new_string": "gamma"}),
        context.clone(),
    )
    .await;
    assert_eq!(
        first_completed.usage.process_count, 1,
        "an edit whose post-edit check ran must account for the spawned \
         process exactly like builtin.shell"
    );
    let first = first_completed.output;

    assert_eq!(first["success"], json!(true), "edit itself must succeed");
    assert_eq!(first["post_edit_check"]["exit_code"], json!(1));
    let new_output = first["post_edit_check"]["new_output"]
        .as_str()
        .expect("first edit surfaces the check findings as new_output");
    assert!(new_output.contains("error[E0308]: mismatched types"));
    assert!(new_output.contains("unused variable"));

    let requests = check_port.requests();
    assert_eq!(requests.len(), 1, "one check per successful edit");
    assert_eq!(
        requests[0].command,
        "cargo check --message-format=short 2>&1"
    );
    assert_eq!(requests[0].timeout_secs, Some(7));
    assert_eq!(
        requests[0].workdir.as_deref(),
        Some("/workspace"),
        "check must run at the writable mount root so the process port \
         resolves it exactly like a shell workdir"
    );

    let second = invoke_with_context(
        &runtime,
        APPLY_PATCH_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs", "old_string": "beta", "new_string": "delta"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(
        second["post_edit_check"],
        json!({"exit_code": 1}),
        "identical check output must carry no repeated lines"
    );
    assert_eq!(check_port.requests().len(), 2);
}

#[tokio::test]
async fn builtin_edit_tools_skip_post_edit_check_when_unconfigured() {
    // Feature off (no config): the mutating tools must not touch the process
    // port at all and the model-facing output carries no post_edit_check field.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing("diagnostics", 1));
    let runtime = runtime_with_filesystem_and_process_port(filesystem, Arc::clone(&check_port));
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let written = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(written["success"], json!(true));
    assert!(
        written.get("post_edit_check").is_none(),
        "unconfigured runtime must not emit a post_edit_check field"
    );
    assert!(
        check_port.requests().is_empty(),
        "unconfigured runtime must not invoke the process port"
    );
}

#[tokio::test]
async fn builtin_edit_tools_report_post_edit_check_timeout_without_failing_the_edit() {
    // The check is advisory: a timed-out check must not fail the already
    // successful edit, and the model learns the check timed out.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::timing_out(Duration::from_secs(7)));
    let runtime = runtime_with_filesystem_process_port_and_post_edit_check(
        filesystem,
        Arc::clone(&check_port),
        PostEditCheckConfig::new("cargo check", Duration::from_secs(7)),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let written = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(written["success"], json!(true), "edit must not fail");
    assert_eq!(written["post_edit_check"], json!({"timed_out": true}));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("new.rs")).unwrap(),
        "fn hello() {}\n"
    );
}

#[tokio::test]
async fn builtin_edit_tools_omit_new_output_when_check_passes_clean() {
    // A passing check with no new findings stays token-lean: exit_code only.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing("", 0));
    let runtime = runtime_with_filesystem_process_port_and_post_edit_check(
        filesystem,
        Arc::clone(&check_port),
        PostEditCheckConfig::new("cargo check", Duration::from_secs(30)),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let written = invoke_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(written["post_edit_check"], json!({"exit_code": 0}));
}

#[tokio::test]
async fn builtin_edit_tools_disable_post_edit_check_when_process_backend_is_none() {
    // Regression (PR #5979 review): write_file/apply_patch declare only
    // filesystem effects, so their plan never requires a process — but a
    // configured post-edit check used to spawn through the default process
    // port anyway, bypassing ProcessBackendKind::None entirely. Under a
    // no-process policy the advisory check must be withheld: the edit
    // succeeds, no process port is touched, and nothing is accounted.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing("diagnostics", 1));
    let runtime = runtime_with_post_edit_check_and_policy(
        filesystem,
        Arc::clone(&check_port),
        None,
        PostEditCheckConfig::new("cargo check", Duration::from_secs(30)),
        process_denied_runtime_policy(),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await;

    assert_eq!(
        completed.output["success"],
        json!(true),
        "edit must succeed"
    );
    assert!(
        completed.output.get("post_edit_check").is_none(),
        "ProcessBackendKind::None must disable the post-edit check"
    );
    assert!(
        check_port.requests().is_empty(),
        "a no-process policy must never spawn the check on the local host port"
    );
    assert_eq!(
        completed.usage.process_count, 0,
        "no process ran, so none may be accounted"
    );
}

#[tokio::test]
async fn builtin_edit_tools_run_post_edit_check_in_tenant_sandbox_not_on_local_host() {
    // Regression (PR #5978 review): the edit plans declare no process effect, so
    // the default process port handed to them is the deployment-blind local host
    // port. Running the configured check through it would escape the sandbox onto
    // the shared provider host under a tenant-sandbox policy. The resolver instead
    // bundles the check with the port matching the plan's process backend, so
    // under a tenant-sandbox policy the check runs ISOLATED in the tenant's own
    // sandbox — never on the local host port.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let local_port = Arc::new(ScriptedProcessPort::completing("diagnostics", 1));
    let sandbox_transport = Arc::new(RecordingSandboxTransport::default());
    let runtime = runtime_with_post_edit_check_and_policy(
        filesystem,
        Arc::clone(&local_port),
        Some(Arc::new(TenantSandboxProcessPort::new(
            Arc::clone(&sandbox_transport) as Arc<dyn SandboxCommandTransport>,
        ))),
        PostEditCheckConfig::new("cargo check", Duration::from_secs(30)),
        tenant_sandbox_runtime_policy(),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        WRITE_FILE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await;

    assert_eq!(
        completed.output["success"],
        json!(true),
        "edit must succeed"
    );
    assert_eq!(
        completed.output["post_edit_check"]["new_output"]
            .as_str()
            .expect("the sandbox-run check surfaces its diagnostics as new_output"),
        "sandbox diagnostics",
        "a tenant-sandbox policy runs the check ISOLATED in the tenant sandbox \
         and surfaces its output to the model"
    );
    assert!(
        local_port.requests().is_empty(),
        "the check must not escape the sandbox policy onto the local host port"
    );
    assert_eq!(
        sandbox_transport.request_count(),
        1,
        "the check runs through the tenant sandbox port, never the local host"
    );
    assert_eq!(
        completed.usage.process_count, 1,
        "the sandbox-run check is accounted as one spawned process"
    );
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

/// Editing an existing file requires a prior full `read_file` (read-before-edit
/// guard). Tests that exercise unrelated write/patch behavior seed that state
/// through the public read path.
async fn seed_read_state<R: HostRuntime + ?Sized>(
    runtime: &R,
    path: &str,
    context: ExecutionContext,
) {
    invoke_with_context(
        runtime,
        READ_FILE_CAPABILITY_ID,
        json!({"path": path}),
        context,
    )
    .await
    .expect("read_file seeds read-before-edit state");
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

fn runtime_with_filesystem_and_process_port<F, P>(
    filesystem: F,
    process_port: Arc<P>,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    P: RuntimeProcessPort + 'static,
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
    .with_runtime_process_port(process_port)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_filesystem_process_port_and_post_edit_check<F, P>(
    filesystem: F,
    process_port: Arc<P>,
    post_edit_check: PostEditCheckConfig,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    P: RuntimeProcessPort + 'static,
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
    .with_runtime_process_port(process_port)
    .with_post_edit_check(post_edit_check)
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

/// Like `runtime_with_filesystem_process_port_and_post_edit_check`, but with
/// an explicit runtime policy (and optionally a tenant sandbox process port)
/// so tests can pin how the process policy gates the post-edit check.
fn runtime_with_post_edit_check_and_policy<F, P>(
    filesystem: F,
    process_port: Arc<P>,
    tenant_sandbox_process_port: Option<Arc<TenantSandboxProcessPort>>,
    post_edit_check: PostEditCheckConfig,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    P: RuntimeProcessPort + 'static,
{
    let mut services = HostRuntimeServices::new(
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
    .with_runtime_process_port(process_port)
    .with_post_edit_check(post_edit_check)
    .with_runtime_policy(policy)
    .with_trust_policy(Arc::new(trust_policy()));
    if let Some(tenant_sandbox_process_port) = tenant_sandbox_process_port {
        services = services.with_tenant_sandbox_process_port(tenant_sandbox_process_port);
    }
    services.host_runtime_for_local_testing()
}

/// SecureDefault-shaped local policy: scoped-virtual filesystem, no process
/// backend. Approval stays at AskDestructive so the only axis under test is
/// the process backend.
fn process_denied_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Brokered,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

/// HostedDev-shaped tenant policy with a tenant-sandbox process backend. The
/// filesystem backend is ScopedVirtual (not the hosted TenantWorkspace)
/// because the local invocation-services resolver under test can only serve
/// mount-scoped filesystem plans; the axis under test is the process backend.
fn tenant_sandbox_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedDev,
        resolved_profile: RuntimeProfile::HostedDev,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Allowlist,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    }
}

/// Sandbox transport double that counts requests; the tenant-sandbox test
/// asserts the post-edit check runs through it (isolated in the tenant
/// sandbox) rather than escaping onto the local host port.
#[derive(Default)]
struct RecordingSandboxTransport {
    requests: std::sync::Mutex<Vec<CommandExecutionRequest>>,
}

impl RecordingSandboxTransport {
    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.requests.lock().unwrap().push(request);
        Ok(CommandExecutionOutput {
            output: "sandbox diagnostics".to_string(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::from_millis(3),
        })
    }
}

/// Process-port double that records every request and replays one scripted
/// outcome, mirroring the recording port used by the builtin.shell tests.
struct ScriptedProcessPort {
    requests: std::sync::Mutex<Vec<CommandExecutionRequest>>,
    response: Result<(String, i64), RuntimeProcessError>,
}

impl ScriptedProcessPort {
    fn completing(output: &str, exit_code: i64) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            response: Ok((output.to_string(), exit_code)),
        }
    }

    fn timing_out(timeout: Duration) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            response: Err(RuntimeProcessError::Timeout(timeout)),
        }
    }

    fn requests(&self) -> Vec<CommandExecutionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl RuntimeProcessPort for ScriptedProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.requests.lock().unwrap().push(request);
        match &self.response {
            Ok((output, exit_code)) => Ok(CommandExecutionOutput {
                output: output.clone(),
                saved_output: None,
                exit_code: *exit_code,
                sandboxed: false,
                duration: Duration::from_millis(3),
            }),
            Err(error) => Err(error.clone()),
        }
    }
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

fn mounted_filesystem(path: &Path, permissions: MountPermissions) -> (DiskFilesystem, MountView) {
    let mut filesystem = DiskFilesystem::new();
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
    inner: DiskFilesystem,
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
    inner: DiskFilesystem,
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
    inner: DiskFilesystem,
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

struct WriteFailureFilesystem {
    inner: DiskFilesystem,
    fail_suffix: &'static str,
}

#[async_trait]
impl RootFilesystem for WriteFailureFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        if path.as_str().ends_with(self.fail_suffix) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
                reason: "disk full".to_string(),
            });
        }
        self.inner.write_file(path, _bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }
}

struct ReadInfrastructureFailureFilesystem {
    inner: DiskFilesystem,
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
