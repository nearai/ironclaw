use ironclaw_host_api::{resolution::Resolution, turn::LoopResultRef};
use ironclaw_loop_contracts::{
    CapabilityProgress, MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation,
    ObservationTrust, ToolObservationDetail, ToolObservationStatus, resolution::completed,
};

#[test]
fn malformed_structured_preview_falls_back_to_masked_ordinary_content() {
    let secret = "sk-ant-provider-secret-123456";
    let preview = serde_json::json!({
        "view": ironclaw_host_api::model_result_preview::MODEL_RESULT_JSON_PAGE_VIEW,
        "result_ref": "result:staged",
        "json_pointer": "",
        "node_type": "object",
        "offset": 0,
        "offset_unit": "items",
        "content": {"token": secret},
        "omitted": [],
        "total_bytes": 32,
        "next_offset": null,
        "next": null,
    })
    .to_string();
    let observation = ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary: "tool completed".to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: "result:staged".to_string(),
            byte_len: 32,
            preview: Some(preview),
            structured_json_view: true,
            total_bytes: None,
            next_offset: None,
            item_count: None,
        },
        artifacts: vec![],
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    };

    let Resolution::Done(outcome) = completed(
        LoopResultRef::new("result:child-1").expect("valid result ref"),
        "ok".to_string(),
        CapabilityProgress::Unknown,
        false,
        32,
        None,
        Some(observation),
    ) else {
        panic!("completed result must resolve as done");
    };
    let preview = outcome
        .refs
        .preview
        .as_ref()
        .expect("rejected typed page remains visible as masked content")
        .as_str();
    assert!(!preview.contains(secret));
    assert!(preview.contains("[redacted]"));
    assert!(!outcome.refs.preview_meta.structured_json_view);
}
