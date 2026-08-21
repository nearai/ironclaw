use super::*;
use ironclaw_host_api::{
    dispatch::CapabilityDisplayOutputPreview,
    ids::{AgentId, InvocationId, ProviderToolName, TenantId, ThreadId},
};
use ironclaw_loop_contracts::{RunProfileResolutionRequest, RunProfileResolver};
use ironclaw_loop_host::DurablePersistence;
use ironclaw_turn_runner::planned_driver_factory::default_planned_run_profile_resolver;
use ironclaw_turns::{TurnId, TurnRunId, TurnScope};

#[tokio::test]
async fn capability_io_records_read_display_preview() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context().await;
    let tool_call = ProviderToolCall {
        provider_id: "provider".to_string(),
        provider_model_id: "model".to_string(),
        turn_id: Some("turn_1".to_string()),
        id: "call_1".to_string(),
        name: ProviderToolName::new("read").expect("provider tool name"),
        arguments: serde_json::json!({"path": "src/main.rs", "api_key": "sk-secret"}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    };
    let input_ref = io
        .register_provider_tool_call_input(&run_context, &tool_call)
        .await
        .expect("input staged");
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("builtin.read").unwrap();
    io.write_capability_result(CapabilityResultWrite {
        receipt: None,
        completed_artifact: None,
        canonical_output_digest: None,
        run_context: &run_context,
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        output: serde_json::json!({"output": "[main.rs#1A2B]\n1:fn main() {}"}),
        display_preview: None,
        durable_persistence: DurablePersistence::Persist,
        canonical_item_count: None,
    })
    .await
    .map(|_| ())
    .expect("result staged");

    let record = io
        .display_previews
        .record_for_invocation(invocation_id)
        .expect("preview recorded");
    assert_eq!(record.title, "read");
    assert_eq!(record.subtitle.as_deref(), Some("src/main.rs"));
    assert!(
        record
            .input_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("path: src/main.rs")),
        "the coding read input summary must surface the path, got {:?}",
        record.input_summary
    );
    assert!(
        record.output_preview.as_deref().is_some_and(
            |preview| preview.contains("[main.rs#1A2B]") && preview.contains("fn main() {}")
        ),
        "the coding read output envelope must render as its extracted output string, got {:?}",
        record.output_preview
    );
    assert_eq!(
        record.output_kind.as_deref(),
        Some("text"),
        "the coding read output envelope must render as text, not a generic JSON dump"
    );
    assert!(
        !record
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.starts_with('{')),
        "the coding read preview must not be a JSON dump, got {:?}",
        record.output_preview
    );
    let rendered = serde_json::to_string(&record.input_summary).unwrap();
    assert!(!rendered.contains("sk-secret"));
}

#[tokio::test]
async fn capability_io_records_write_display_preview_without_content() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context().await;
    let tool_call = ProviderToolCall {
        provider_id: "provider".to_string(),
        provider_model_id: "model".to_string(),
        turn_id: Some("turn_1".to_string()),
        id: "call_1".to_string(),
        name: ProviderToolName::new("write").expect("provider tool name"),
        arguments: serde_json::json!({
            "path": "src/main.rs",
            "content": "fn main() {}\n// top-secret body"
        }),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    };
    let input_ref = io
        .register_provider_tool_call_input(&run_context, &tool_call)
        .await
        .expect("input staged");
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("builtin.write").unwrap();
    io.write_capability_result(CapabilityResultWrite {
        receipt: None,
        completed_artifact: None,
        canonical_output_digest: None,
        run_context: &run_context,
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        output: serde_json::json!({
            "output": "[main.rs#1A2B]\nSuccessfully wrote 31 bytes to main.rs"
        }),
        display_preview: None,
        durable_persistence: DurablePersistence::Persist,
        canonical_item_count: None,
    })
    .await
    .map(|_| ())
    .expect("result staged");

    let record = io
        .display_previews
        .record_for_invocation(invocation_id)
        .expect("preview recorded");
    let input_summary = record.input_summary.as_deref().unwrap();
    assert!(
        input_summary.contains("path: src/main.rs") && input_summary.contains("content_bytes: 31"),
        "the coding write input summary must surface path and byte count, got {input_summary}"
    );
    assert!(
        !input_summary.contains("fn main()") && !input_summary.contains("top-secret"),
        "the coding write input summary must never embed the written content, got {input_summary}"
    );
    assert_eq!(record.output_kind.as_deref(), Some("text"));
    assert!(
        record
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("Successfully wrote 31 bytes")),
        "the coding write output must render as its extracted output string, got {:?}",
        record.output_preview
    );
}

#[tokio::test]
async fn capability_io_records_edit_display_preview_without_grammar_payload() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context().await;
    let tool_call = ProviderToolCall {
        provider_id: "provider".to_string(),
        provider_model_id: "model".to_string(),
        turn_id: Some("turn_1".to_string()),
        id: "call_1".to_string(),
        name: ProviderToolName::new("edit").expect("provider tool name"),
        arguments: serde_json::json!({
            "input": "[main.rs#1A2B]\nPUT 1.=1:\n+fn main() {}\n"
        }),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    };
    let input_ref = io
        .register_provider_tool_call_input(&run_context, &tool_call)
        .await
        .expect("input staged");
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("builtin.edit").unwrap();
    io.write_capability_result(CapabilityResultWrite {
        receipt: None,
        completed_artifact: None,
        canonical_output_digest: None,
        run_context: &run_context,
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        output: serde_json::json!({"output": "[main.rs#1A2B]\nEdit applied."}),
        display_preview: None,
        durable_persistence: DurablePersistence::Persist,
        canonical_item_count: None,
    })
    .await
    .map(|_| ())
    .expect("result staged");

    let record = io
        .display_previews
        .record_for_invocation(invocation_id)
        .expect("preview recorded");
    let input_summary = record.input_summary.as_deref().unwrap();
    assert!(
        input_summary.contains("input_bytes: 39"),
        "the coding edit input summary must surface only the grammar byte count, got {input_summary}"
    );
    assert!(
        !input_summary.contains("main.rs")
            && !input_summary.contains("#1A2B")
            && !input_summary.contains("fn main()"),
        "the coding edit input summary must never embed the hashline grammar payload, got \
             {input_summary}"
    );
    assert_eq!(record.output_kind.as_deref(), Some("text"));
    assert!(
        record
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("Edit applied")),
        "the coding edit output must render as its extracted output string, got {:?}",
        record.output_preview
    );
}

/// Regression (#activity-card-args): in production the loop wraps this
/// resolver with `ProviderToolCallInputResolver`, which owns the canonical
/// (digest) ref and bypasses `register_provider_tool_call_input` — it drives
/// the display-preview recording via `record_provider_tool_call_display_input`
/// instead. Recording under that canonical ref must still surface
/// `input_summary` when the result is written under the same ref; otherwise
/// the activity card shows the tool with no arguments.
#[tokio::test]
async fn capability_io_records_display_input_via_provider_tool_call_hook() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context().await;
    // The model-facing provider tool name is the lossy `__` encoding; the
    // resolved capability id is the dotted form. The display must key off
    // the capability id so the card title and per-tool summary are correct.
    let tool_call = ProviderToolCall {
        provider_id: "provider".to_string(),
        provider_model_id: "model".to_string(),
        turn_id: Some("turn_1".to_string()),
        id: "call_1".to_string(),
        name: ProviderToolName::new("nearai__web_search").expect("provider tool name"),
        arguments: serde_json::json!({"query": "deploy status"}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    };
    let capability_id = CapabilityId::new("nearai.web_search").unwrap();
    // The decorator owns this ref; it is NOT produced by
    // `register_provider_tool_call_input` here.
    let input_ref = CapabilityInputRef::new(format!(
        "input:provider-tool-call:{}:digest",
        run_context.run_id
    ))
    .expect("valid ref");

    io.record_provider_tool_call_display_input(
        &run_context,
        &input_ref,
        &capability_id,
        &tool_call,
    );

    let invocation_id = InvocationId::new();
    io.write_capability_result(CapabilityResultWrite {
        receipt: None,
        completed_artifact: None,
        canonical_output_digest: None,
        run_context: &run_context,
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        output: serde_json::json!({"results": []}),
        display_preview: None,
        durable_persistence: DurablePersistence::Persist,
        canonical_item_count: None,
    })
    .await
    .map(|_| ())
    .expect("result staged");

    let record = io
        .display_previews
        .record_for_invocation(invocation_id)
        .expect("preview recorded");
    // Title is the dotted capability id, not the `__` provider tool name.
    assert_eq!(record.title, "nearai.web_search");
    // The per-tool web_search matcher fires (query summarized), rather than
    // falling back to a raw JSON dump.
    assert!(
        record
            .input_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("query: deploy status")),
        "input summary should use the web_search matcher, got {:?}",
        record.input_summary,
    );
}

#[tokio::test]
async fn capability_io_records_display_preview_side_channel() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context().await;
    let input_ref = io
        .stage_input(
            &run_context,
            serde_json::json!({"path": "/workspace/main.rs"}),
        )
        .expect("input staged");
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("builtin.write").unwrap();
    io.write_capability_result(CapabilityResultWrite { receipt: None, run_context: &run_context,
        completed_artifact: None,
        canonical_output_digest: None,
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        output: serde_json::json!({"output": "[main.rs#1A2B]\nSuccessfully wrote 3 bytes to main.rs"}),
        display_preview: Some(CapabilityDisplayOutputPreview {
            output_summary: Some("wrote main.rs".to_string()),
            output_preview: "[main.rs#1A2B]\n1:new\n".to_string(),
            output_kind: "text".to_string(),
            subtitle: Some("/workspace/main.rs".to_string()),
            truncated: false,
        }),
        durable_persistence: DurablePersistence::Persist, canonical_item_count: None })
        .await
        .map(|_| ())
        .expect("result staged");
    let record = io
        .display_previews
        .record_for_invocation(invocation_id)
        .expect("preview recorded");
    assert_eq!(record.output_kind.as_deref(), Some("text"));
    assert_eq!(record.output_summary.as_deref(), Some("wrote main.rs"));
    assert!(
        record
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("1:new"))
    );
    assert_eq!(record.subtitle.as_deref(), Some("/workspace/main.rs"));
}

#[tokio::test]
async fn capability_io_prunes_display_preview_with_run() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context().await;
    let input_ref = io
        .stage_input(&run_context, serde_json::json!({"text": "ok"}))
        .expect("input staged");
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    io.write_capability_result(CapabilityResultWrite {
        receipt: None,
        completed_artifact: None,
        canonical_output_digest: None,
        run_context: &run_context,
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        output: serde_json::json!({"reply": "ok"}),
        display_preview: None,
        durable_persistence: DurablePersistence::Persist,
        canonical_item_count: None,
    })
    .await
    .map(|_| ())
    .expect("result staged");
    assert!(
        io.display_previews
            .record_for_invocation(invocation_id)
            .is_some()
    );

    io.prune_run(&run_context).expect("run pruned");
    assert!(
        io.display_previews
            .record_for_invocation(invocation_id)
            .is_none()
    );
}

async fn loop_run_context() -> LoopRunContext {
    let resolved = default_planned_run_profile_resolver()
        .unwrap()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(
        TurnScope::new(
            TenantId::new("preview-tenant").unwrap(),
            Some(AgentId::new("preview-agent").unwrap()),
            None,
            ThreadId::new("preview-thread").unwrap(),
        ),
        TurnId::new(),
        TurnRunId::new(),
        resolved,
    )
}
