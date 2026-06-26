use super::*;
use async_trait::async_trait;
use ironclaw_host_api::CapabilityDisplayOutputPreview;
use ironclaw_product_adapters::{
    CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES, CapabilityDisplayPreviewView, ProductAdapterError,
    RedactedString,
};
use ironclaw_turns::run_profile::CapabilityInputRef;

fn preview_input_ref(label: &str) -> CapabilityInputRef {
    CapabilityInputRef::new(format!("input:{label}")).unwrap()
}

struct FailingPreviewSource;

#[async_trait]
impl CapabilityDisplayPreviewSource for FailingPreviewSource {
    async fn preview_resolution(
        &self,
        _activity: &CapabilityActivityProjection,
    ) -> Result<CapabilityDisplayPreviewResolution, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: RedactedString::new("preview encoder failed"),
        })
    }
}

async fn completed_preview_for_input(
    tool_name: &str,
    capability_id: &str,
    arguments: serde_json::Value,
) -> CapabilityDisplayPreviewView {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new(capability_id).unwrap();
    let input_ref = preview_input_ref(&format!("preview-input-{tool_name}"));
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(&run_id.to_string(), &input_ref, tool_name, &arguments);
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:preview",
        output: &serde_json::json!({"ok": true}),
        output_bytes: 12,
    });
    store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn webui_event_stream_enriches_activity_with_display_preview_from_store() {
    let tenant_id = TenantId::new("webui-preview-tenant").unwrap();
    let user_id = UserId::new("webui-preview-user").unwrap();
    let agent_id = AgentId::new("webui-preview-agent").unwrap();
    let thread_id = ThreadId::new("webui-preview-thread").unwrap();
    let invocation_id = InvocationId::new();
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let input_ref = preview_input_ref("webui-preview-input");
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    display_previews.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({
            "path": "src/main.rs",
            "token": "sk-secret",
            "max_bytes": 4096
        }),
    );
    display_previews.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:preview-output",
        output: &serde_json::json!({"content": "fn main() {}"}),
        output_bytes: 64,
    });
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            capability.clone(),
            ExtensionId::new("builtin").unwrap(),
            RuntimeKind::FirstParty,
            64,
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-preview-reply").unwrap(),
    )
    .with_display_previews(Arc::clone(&display_previews));
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone()),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::CapabilityDisplayPreview(preview)
                    if preview.invocation_id == invocation_id
                        && preview.thread_id.as_ref() == Some(&thread_id)
                        && preview.capability_id == capability
                        && preview.title == "read_file"
                        && preview.subtitle.as_deref() == Some("src/main.rs")
                        && preview.input_summary.as_deref().is_some_and(|summary| summary.contains("path: src/main.rs"))
                        && preview.output_preview.as_deref() == Some("fn main() {}")
                        && preview.result_ref.as_deref() == Some("result:preview-output")
                        && preview.output_bytes == Some(64)
            )
        }),
        "events: {events:#?}"
    );
    let rendered = serde_json::to_string(&events).unwrap();
    assert!(!rendered.contains("sk-secret"));
}

#[tokio::test]
async fn capability_display_preview_error_does_not_drop_activity_payload() {
    let tenant_id = TenantId::new("webui-preview-error-tenant").unwrap();
    let user_id = UserId::new("webui-preview-error-user").unwrap();
    let agent_id = AgentId::new("webui-preview-error-agent").unwrap();
    let thread_id = ThreadId::new("webui-preview-error-thread").unwrap();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let projection_scope = runtime_projection_scope(&TurnActor::new(user_id), &scope);
    let cursor =
        EventProjectionCursor::for_scope(projection_scope, ironclaw_events::EventCursor::new(1));
    let invocation_id = InvocationId::new();
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.write_file").unwrap();

    let item = runtime_payloads_for_item(
        &scope,
        &FailingPreviewSource,
        RuntimePayloadItemInput {
            runs: Vec::new(),
            capability_activities: vec![CapabilityActivityProjection {
                invocation_id,
                run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
                capability_id: capability.clone(),
                thread_id: Some(thread_id),
                status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
                provider: None,
                runtime: None,
                process_id: None,
                output_bytes: Some(12),
                error_kind: None,
                first_cursor: ironclaw_events::EventCursor::new(1),
                last_cursor: ironclaw_events::EventCursor::new(1),
                updated_at: chrono::Utc::now(),
            }],
            cursor,
            state_kind: StatePayloadKind::Update,
        },
        None,
        0,
        8,
    )
    .await
    .expect("preview failure should not fail projection")
    .expect("activity payload remains renderable");

    assert_eq!(item.total, 1);
    assert!(matches!(
        item.payloads.as_slice(),
        [DeliveredRuntimePayload {
            payload: ProductOutboundPayload::CapabilityActivity(activity),
            ..
        }]
            if activity.invocation_id == invocation_id
                && activity.capability_id == capability
    ));
}

#[tokio::test]
async fn capability_display_preview_store_redacts_unsafe_paths_and_secrets() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let input_ref = preview_input_ref("redacted-preview-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({
            "path": "/Users/alice/secret.rs",
            "api_key": "sk-secret"
        }),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:redacted-preview",
        output: &serde_json::json!({"content": "{\"path\":\"/etc/passwd\", unc:\"\\\\host\\\\share\", token:\"sk-secret\"}"}),
        output_bytes: 42,
    });
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(42),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(preview.subtitle.is_none());
    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("sk-secret"));
    assert!(!rendered.contains("/Users/alice"));
    assert!(!rendered.contains("/etc/passwd"));
    assert!(!rendered.contains("\\\\host\\\\share"));
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_shell_command_safely() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.shell").unwrap();
    let input_ref = preview_input_ref("shell-preview-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &input_ref,
        "builtin.shell",
        &serde_json::json!({
            "command": "pwd && curl -H 'Authorization: Bearer sk-secret' https://example.test/path?token=secret"
        }),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:shell-preview",
        output: &serde_json::json!({"output": "ok"}),
        output_bytes: 2,
    });
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-shell-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(2),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    let input_summary = preview.input_summary.as_deref().unwrap();
    assert!(input_summary.contains("command: pwd && curl"));
    assert!(input_summary.contains("-H 'Authorization: [redacted]'"));
    assert!(input_summary.contains("https://example.test/path?..."));
    assert!(!input_summary.contains("sk-secret"));
    assert!(!input_summary.contains("token=secret"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_http_inputs_safely() {
    let preview = completed_preview_for_input(
        "builtin.http.save",
        "builtin.http.save",
        serde_json::json!({
            "method": "post",
            "url": "https://user:secret@example.test/reset/token/opaque-value?token=secret#frag",
            "save_to": "/workspace/tmp/result.json",
            "headers": {
                "Authorization": "Bearer sk-secret"
            },
            "body": "secret request body",
            "response_body_limit": 4096,
            "timeout_ms": 5000
        }),
    )
    .await;

    let input_summary = preview.input_summary.as_deref().unwrap();
    assert!(input_summary.contains("method: POST"));
    assert!(input_summary.contains("url: https://example.test/reset/[redacted]/[redacted]?..."));
    assert!(input_summary.contains("save_to: tmp/result.json"));
    assert!(input_summary.contains("response_body_limit: 4096"));
    assert!(input_summary.contains("timeout_ms: 5000"));
    assert!(!input_summary.contains("user:secret"));
    assert!(!input_summary.contains("opaque-value"));
    assert!(!input_summary.contains("sk-secret"));
    assert!(!input_summary.contains("token=secret"));
    assert!(!input_summary.contains("Authorization"));
    assert!(!input_summary.contains("secret request body"));
}

#[tokio::test]
async fn capability_display_preview_store_redacts_file_url_inputs() {
    let preview = completed_preview_for_input(
        "builtin.http.save",
        "builtin.http.save",
        serde_json::json!({
            "method": "get",
            "url": "file:///Users/alice/.ssh/id_rsa",
            "timeout_ms": 5000
        }),
    )
    .await;

    let input_summary = preview.input_summary.as_deref().unwrap();
    assert!(input_summary.contains("url: [redacted]"));
    assert!(!input_summary.contains("file:///Users/alice"));
    assert!(!input_summary.contains("id_rsa"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_file_inputs_without_contents() {
    let write_preview = completed_preview_for_input(
        "builtin.write_file",
        "builtin.write_file",
        serde_json::json!({
            "path": "/workspace/src/main.rs",
            "content": "fn main() {}"
        }),
    )
    .await;
    let write_summary = write_preview.input_summary.as_deref().unwrap();
    assert!(write_summary.contains("path: src/main.rs"));
    assert!(write_summary.contains("content_bytes: 12"));
    assert!(!write_summary.contains("fn main"));

    let patch_preview = completed_preview_for_input(
        "builtin.apply_patch",
        "builtin.apply_patch",
        serde_json::json!({
            "path": "src/lib.rs",
            "old_string": "let token = \"sk-secret\";",
            "new_string": "let token = load_token();",
            "replace_all": true
        }),
    )
    .await;
    let patch_summary = patch_preview.input_summary.as_deref().unwrap();
    assert!(patch_summary.contains("path: src/lib.rs"));
    assert!(patch_summary.contains("old_bytes: 24"));
    assert!(patch_summary.contains("new_bytes: 25"));
    assert!(patch_summary.contains("replace_all: true"));
    assert!(!patch_summary.contains("sk-secret"));
    assert!(!patch_summary.contains("load_token"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_read_limits_and_memory_tree_root() {
    let read_preview = completed_preview_for_input(
        "builtin.read_file",
        "builtin.read_file",
        serde_json::json!({
            "path": "/workspace/src/main.rs",
            "offset": 128,
            "max_bytes": 4096
        }),
    )
    .await;
    let read_summary = read_preview.input_summary.as_deref().unwrap();
    assert!(read_summary.contains("path: src/main.rs"));
    assert!(read_summary.contains("offset: 128"));
    assert!(read_summary.contains("limit: 4096"));

    let memory_tree_preview = completed_preview_for_input(
        "builtin.memory_tree",
        "builtin.memory_tree",
        serde_json::json!({
            "limit": 12
        }),
    )
    .await;
    let memory_tree_summary = memory_tree_preview.input_summary.as_deref().unwrap();
    assert!(memory_tree_summary.contains("path: /"));
    assert!(memory_tree_summary.contains("limit: 12"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_search_and_memory_inputs() {
    let search_preview = completed_preview_for_input(
        "nearai.web_search",
        "nearai.web_search",
        serde_json::json!({
            "query": "deployment status token: sk-secret",
            "limit": 5
        }),
    )
    .await;
    let search_summary = search_preview.input_summary.as_deref().unwrap();
    assert!(search_summary.contains("query: deployment status token: [redacted]"));
    assert!(search_summary.contains("limit: 5"));
    assert!(!search_summary.contains("sk-secret"));
    // The inline row subtitle is the primary argument (the query), redacted
    // the same way the summary is — so the row reads `web_search   <query>`
    // instead of a bare tool name.
    let search_subtitle = search_preview.subtitle.as_deref().unwrap();
    assert!(search_subtitle.contains("deployment status"));
    assert!(!search_subtitle.contains("sk-secret"));

    let memory_preview = completed_preview_for_input(
        "builtin.memory_write",
        "builtin.memory_write",
        serde_json::json!({
            "target": "/workspace/notes/deploy.md",
            "content": "token: sk-secret",
            "append": true
        }),
    )
    .await;
    let memory_summary = memory_preview.input_summary.as_deref().unwrap();
    assert!(memory_summary.contains("target: notes/deploy.md"));
    assert!(memory_summary.contains("append: true"));
    assert!(memory_summary.contains("content_bytes: 16"));
    assert!(!memory_summary.contains("sk-secret"));
}

#[tokio::test]
async fn running_input_surfaces_staged_input_until_result_lands() {
    let run_id = TurnRunId::new();
    let input_ref = preview_input_ref("running-web-search");
    let invocation_id = InvocationId::new();
    let store = CapabilityDisplayPreviewStore::default();

    // Nothing to show before the invocation is linked to its input.
    assert!(store.running_input(invocation_id).is_none());

    store.record_input(
        &run_id.to_string(),
        &input_ref,
        "nearai.web_search",
        &serde_json::json!({"query": "deploy status token: sk-secret", "limit": 5}),
    );
    store.record_running_invocation(invocation_id, &input_ref);

    // While running, the inline subtitle and parameters are surfaced — and they
    // carry the same projection-sanitized text the preview frame uses, so the
    // secret never reaches the activity frame.
    let running = store.running_input(invocation_id).expect("running input");
    assert!(
        running
            .subtitle
            .as_deref()
            .is_some_and(|s| s.contains("deploy status")),
        "subtitle should carry the query, got {:?}",
        running.subtitle,
    );
    assert!(
        running
            .input_summary
            .as_deref()
            .is_some_and(|s| s.contains("query: deploy status")),
    );
    assert!(!running.subtitle.as_deref().unwrap().contains("sk-secret"));
    assert!(
        !running
            .input_summary
            .as_deref()
            .unwrap()
            .contains("sk-secret")
    );

    // Once the result lands, the pending input is consumed and no longer
    // surfaced as in-flight (the completed preview takes over).
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &CapabilityId::new("nearai.web_search").unwrap(),
        result_ref: "result:running",
        output: &serde_json::json!({"results": []}),
        output_bytes: 12,
    });
    assert!(store.running_input(invocation_id).is_none());
}

#[tokio::test]
async fn capability_display_preview_store_uses_primary_arg_subtitle_for_non_path_tools() {
    // shell → the command (the inline row reads `shell   <command>`).
    let shell = completed_preview_for_input(
        "shell",
        "builtin.shell",
        serde_json::json!({"command": "cargo test -p ironclaw"}),
    )
    .await;
    assert!(
        shell
            .subtitle
            .as_deref()
            .is_some_and(|s| s.contains("cargo test")),
        "shell subtitle should carry the command, got {:?}",
        shell.subtitle,
    );

    // http / web_fetch → the URL (sensitive parts stripped).
    let http = completed_preview_for_input(
        "web_fetch",
        "builtin.web_fetch",
        serde_json::json!({"url": "https://example.com/docs"}),
    )
    .await;
    assert!(
        http.subtitle
            .as_deref()
            .is_some_and(|s| s.contains("example.com")),
        "web_fetch subtitle should carry the url, got {:?}",
        http.subtitle,
    );

    // Path-based tools keep the workspace-relative path subtitle.
    let read = completed_preview_for_input(
        "read_file",
        "builtin.read_file",
        serde_json::json!({"path": "/workspace/src/main.rs"}),
    )
    .await;
    assert_eq!(read.subtitle.as_deref(), Some("src/main.rs"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_list_dir_inputs() {
    let preview = completed_preview_for_input(
        "builtin.list_dir",
        "builtin.list_dir",
        serde_json::json!({
            "path": "/workspace/src",
            "recursive": true,
            "max_depth": 3
        }),
    )
    .await;
    let input_summary = preview.input_summary.as_deref().unwrap();
    assert!(input_summary.contains("path: src"));
    assert!(input_summary.contains("recursive: true"));
    assert!(input_summary.contains("max_depth: 3"));
}

#[tokio::test]
async fn capability_display_preview_store_summarizes_grep_and_glob_inputs() {
    let grep_preview = completed_preview_for_input(
        "builtin.grep",
        "builtin.grep",
        serde_json::json!({
            "pattern": "Authorization: Bearer sk-secret",
            "path": "/workspace/src",
            "glob": "*.rs",
            "output_mode": "content",
            "head_limit": 20
        }),
    )
    .await;
    let grep_summary = grep_preview.input_summary.as_deref().unwrap();
    assert!(grep_summary.contains("pattern: Authorization: [redacted]"));
    assert!(grep_summary.contains("path: src"));
    assert!(grep_summary.contains("glob: *.rs"));
    assert!(grep_summary.contains("output_mode: content"));
    assert!(grep_summary.contains("head_limit: 20"));
    assert!(!grep_summary.contains("sk-secret"));

    let glob_preview = completed_preview_for_input(
        "builtin.glob",
        "builtin.glob",
        serde_json::json!({
            "pattern": "**/*.rs",
            "path": "/workspace/crates",
            "max_results": 100
        }),
    )
    .await;
    let glob_summary = glob_preview.input_summary.as_deref().unwrap();
    assert!(glob_summary.contains("pattern: **/*.rs"));
    assert!(glob_summary.contains("path: crates"));
    assert!(glob_summary.contains("max_results: 100"));
}

#[tokio::test]
async fn capability_display_preview_store_admits_workspace_and_project_scoped_path_subtitles() {
    // /workspace/ and /project/ prefixed paths should appear as workspace-relative subtitles;
    // other absolute paths (e.g. /etc/passwd) must be dropped for safety.
    for (input_path, expected_subtitle) in [
        ("/workspace/src/main.rs", Some("src/main.rs")),
        ("/project/src/lib.rs", Some("src/lib.rs")),
        ("/etc/passwd", None),
        ("relative/path.rs", Some("relative/path.rs")),
    ] {
        let run_id = TurnRunId::new();
        let capability = CapabilityId::new("builtin.write_file").unwrap();
        let input_ref = preview_input_ref(&format!("subtitle-path-input-{input_path}"));
        let store = CapabilityDisplayPreviewStore::default();
        store.record_input(
            &run_id.to_string(),
            &input_ref,
            "write_file",
            &serde_json::json!({ "path": input_path }),
        );
        store.record_result(CapabilityDisplayPreviewResult {
            run_id: &run_id.to_string(),
            input_ref: &input_ref,
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            capability_id: &capability,
            result_ref: "result:subtitle-path",
            output: &serde_json::json!({"success": true}),
            output_bytes: 4,
        });
        let preview = store
            .preview(&CapabilityActivityProjection {
                invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
                run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
                capability_id: capability,
                thread_id: None,
                status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
                provider: None,
                runtime: None,
                process_id: None,
                output_bytes: Some(4),
                error_kind: None,
                first_cursor: ironclaw_events::EventCursor::new(1),
                last_cursor: ironclaw_events::EventCursor::new(1),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            preview.subtitle.as_deref(),
            expected_subtitle,
            "subtitle mismatch for input path: {input_path}"
        );
    }
}

#[tokio::test]
async fn capability_display_preview_store_redacts_common_secret_text_shapes() {
    let run_id = TurnRunId::new();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.output").unwrap();
    let input_ref = preview_input_ref("common-secret-text-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:common-secret-text",
        output: &serde_json::Value::String(
            "password: secret123 file:///etc/passwd ghp_abcdefghijklmnopqrstuvwxyz xoxb-1234567890 AKIAIOSFODNN7EXAMPLE eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.signature (https://example.test/reset/sk-secret) <https://example.test/reset/token/opaque-value> url=https://example.test/reset/token/query-value Authorization: Bearer header-value token =opaque-token password :opaque-password access_token: access-value refresh-token = refresh-value credential: credential-value"
                .to_string(),
        ),
        output_bytes: 256,
    });

    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(256),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("secret123"));
    assert!(!rendered.contains("file:///etc/passwd"));
    assert!(!rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
    assert!(!rendered.contains("xoxb-1234567890"));
    assert!(!rendered.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(!rendered.contains("eyJhbGciOiJIUzI1NiJ9"));
    assert!(!rendered.contains("sk-secret"));
    assert!(!rendered.contains("opaque-value"));
    assert!(!rendered.contains("query-value"));
    assert!(!rendered.contains("header-value"));
    assert!(!rendered.contains("opaque-token"));
    assert!(!rendered.contains("opaque-password"));
    assert!(!rendered.contains("access-value"));
    assert!(!rendered.contains("refresh-value"));
    assert!(!rendered.contains("credential-value"));
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn capability_display_preview_store_redacts_camel_case_api_key_json() {
    let run_id = TurnRunId::new();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.output").unwrap();
    let input_ref = preview_input_ref("camel-case-api-key-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:camel-case-api-key",
        output: &serde_json::json!({
            "apiKey": "live-api-key-secret",
            "nested": {
                "serviceCredential": "credential-secret"
            }
        }),
        output_bytes: 128,
    });

    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(128),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("live-api-key-secret"));
    assert!(!rendered.contains("credential-secret"));
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn capability_display_preview_store_keys_completed_results_by_invocation() {
    let run_id = TurnRunId::new();
    let first_invocation = InvocationId::new();
    let second_invocation = InvocationId::new();
    let first_capability = CapabilityId::new("script.first").unwrap();
    let second_capability = CapabilityId::new("script.second").unwrap();
    let first_input = preview_input_ref("first-preview-input");
    let second_input = preview_input_ref("second-preview-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &first_input,
        "first",
        &serde_json::json!({"path": "src/first.rs"}),
    );
    store.record_input(
        &run_id.to_string(),
        &second_input,
        "second",
        &serde_json::json!({"path": "src/second.rs"}),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &first_input,
        invocation_id: first_invocation,
        capability_id: &first_capability,
        result_ref: "result:first",
        output: &serde_json::json!({"content": "first output"}),
        output_bytes: 12,
    });
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &second_input,
        invocation_id: second_invocation,
        capability_id: &second_capability,
        result_ref: "result:second",
        output: &serde_json::json!({"content": "second output"}),
        output_bytes: 13,
    });

    let first_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: first_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: first_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();
    let second_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: second_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: second_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(13),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(2),
            last_cursor: ironclaw_events::EventCursor::new(2),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first_preview.result_ref.as_deref(), Some("result:first"));
    assert_eq!(
        first_preview.output_preview.as_deref(),
        Some("first output")
    );
    assert_eq!(first_preview.activity_order, Some(1));
    assert_eq!(second_preview.result_ref.as_deref(), Some("result:second"));
    assert_eq!(
        second_preview.output_preview.as_deref(),
        Some("second output")
    );
    assert_eq!(second_preview.activity_order, Some(2));
}

#[tokio::test]
async fn capability_display_preview_store_pairs_inputs_by_ref_when_results_complete_out_of_order() {
    let run_id = TurnRunId::new();
    let first_invocation = InvocationId::new();
    let second_invocation = InvocationId::new();
    let first_capability = CapabilityId::new("script.first").unwrap();
    let second_capability = CapabilityId::new("script.second").unwrap();
    let first_input = preview_input_ref("first-out-of-order-input");
    let second_input = preview_input_ref("second-out-of-order-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &first_input,
        "first",
        &serde_json::json!({"path": "src/first.rs"}),
    );
    store.record_input(
        &run_id.to_string(),
        &second_input,
        "second",
        &serde_json::json!({"path": "src/second.rs"}),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &second_input,
        invocation_id: second_invocation,
        capability_id: &second_capability,
        result_ref: "result:second",
        output: &serde_json::json!({"content": "second output"}),
        output_bytes: 13,
    });
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &first_input,
        invocation_id: first_invocation,
        capability_id: &first_capability,
        result_ref: "result:first",
        output: &serde_json::json!({"content": "first output"}),
        output_bytes: 12,
    });

    let first_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: first_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: first_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();
    let second_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: second_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: second_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(13),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(2),
            last_cursor: ironclaw_events::EventCursor::new(2),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first_preview.title, "first");
    assert_eq!(first_preview.subtitle.as_deref(), Some("src/first.rs"));
    assert_eq!(first_preview.result_ref.as_deref(), Some("result:first"));
    assert_eq!(
        first_preview.output_preview.as_deref(),
        Some("first output")
    );
    assert_eq!(first_preview.activity_order, Some(1));
    assert_eq!(second_preview.title, "second");
    assert_eq!(second_preview.subtitle.as_deref(), Some("src/second.rs"));
    assert_eq!(second_preview.result_ref.as_deref(), Some("result:second"));
    assert_eq!(
        second_preview.output_preview.as_deref(),
        Some("second output")
    );
    assert_eq!(second_preview.activity_order, Some(2));
}

#[test]
fn display_preview_sanitizer_does_not_redact_common_sk_substrings() {
    let sanitized = sanitize_text("mask disk risk sk-live");

    assert!(sanitized.contains("mask disk risk"));
    assert!(!sanitized.contains("sk-live"));
    assert!(sanitized.contains("[redacted]"));
}

#[test]
fn display_preview_json_sanitizer_bounds_nested_values() {
    let mut value = serde_json::json!("leaf");
    for _ in 0..(SANITIZE_JSON_MAX_DEPTH + 4) {
        value = serde_json::json!([value]);
    }

    let sanitized = sanitize_json_value(&value);
    let rendered = serde_json::to_string(&sanitized).unwrap();

    assert!(rendered.contains("[truncated]"));
    assert!(!rendered.contains("leaf"));
}

#[tokio::test]
async fn capability_display_preview_marks_json_depth_truncation() {
    let run_id = TurnRunId::new();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.deep_json").unwrap();
    let input_ref = preview_input_ref("deep-json-input");
    let store = CapabilityDisplayPreviewStore::default();
    let mut output = serde_json::json!("leaf");
    for _ in 0..(SANITIZE_JSON_MAX_DEPTH + 4) {
        output = serde_json::json!([output]);
    }
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:deep-json",
        output: &output,
        output_bytes: 256,
    });

    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(256),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(preview.truncated);
    assert!(
        preview
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("[truncated]"))
    );
}

#[tokio::test]
async fn capability_display_preview_falls_back_for_failed_tool_without_result() {
    let capability = CapabilityId::new("script.fail").unwrap();
    let store = CapabilityDisplayPreviewStore::default();
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::new(),
            run_id: None,
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Failed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: Some("operation_failed".to_string()),
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(preview.title, "fail");
    assert_eq!(preview.error_kind.as_deref(), Some("operation_failed"));
    assert_eq!(preview.output_kind.as_deref(), Some("text"));
    assert_eq!(preview.result_ref, None);
    assert!(
        preview
            .output_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("operation_failed"))
    );
}

#[tokio::test]
async fn capability_display_preview_store_preserves_long_line_counts() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("script.long_output").unwrap();
    let input_ref = preview_input_ref("long-output-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:long-preview",
        output: &serde_json::Value::String(
            (0..130)
                .map(|index| format!("line-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        output_bytes: 2048,
    });
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(2048),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(!preview.truncated);
    assert!(
        preview
            .output_preview
            .as_ref()
            .unwrap()
            .contains("line-129")
    );
    assert!(
        preview
            .output_preview
            .as_ref()
            .unwrap()
            .contains("line-120")
    );
}

#[tokio::test]
async fn capability_display_preview_store_marks_truncated_side_channel_summary() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.write_file").unwrap();
    let input_ref = preview_input_ref("long-summary-preview-input");
    let invocation_id = InvocationId::new();
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result_with_preview(
        CapabilityDisplayPreviewResult {
            run_id: &run_id.to_string(),
            input_ref: &input_ref,
            invocation_id,
            capability_id: &capability,
            result_ref: "result:long-summary-preview",
            output: &serde_json::json!({"success": true}),
            output_bytes: 32,
        },
        Some(&CapabilityDisplayOutputPreview {
            output_summary: Some("x".repeat(CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES + 1)),
            output_preview: "--- a/workspace/main.rs\n+++ b/workspace/main.rs\n".to_string(),
            output_kind: "unified_diff".to_string(),
            subtitle: Some("/workspace/main.rs".to_string()),
            truncated: false,
        }),
    );

    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(32),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(preview.output_kind.as_deref(), Some("unified_diff"));
    assert_eq!(preview.subtitle.as_deref(), Some("/workspace/main.rs"));
    assert!(preview.truncated);
}
