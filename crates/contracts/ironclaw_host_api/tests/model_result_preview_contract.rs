use ironclaw_host_api::model_result_preview::{ModelResultJsonPage, ModelResultPreview};
use serde_json::json;

#[test]
fn redacts_nested_and_malformed_structured_credentials() {
    let canary = "never-before-uploaded-canary-host-contract";
    let malformed = json!({
        "marker": "safe-context",
        "content": format!(r#"1| {{"password":"{canary}"}}]"#),
    })
    .to_string();
    let mut deeply_nested = json!({"password": canary});
    for _ in 0..20 {
        deeply_nested = json!([deeply_nested]);
    }
    let deep = json!({"marker": "safe-context", "content": deeply_nested.to_string()}).to_string();

    for input in [malformed, deep] {
        let preview = ModelResultPreview::redacted(input).expect("preview is redacted");
        assert!(preview.as_str().contains("safe-context"));
        assert!(!preview.as_str().contains(canary));
    }
}

#[test]
fn json_page_requires_model_visible_content() {
    let missing_content = json!({
        "view": "ironclaw.json_page.v1",
        "result_ref": "trr_test",
        "json_pointer": "",
        "node_type": "object",
        "offset": 0,
        "offset_unit": "items",
        "omitted": [],
        "total_bytes": 2,
        "next_offset": null,
        "next": null,
    });

    let error = ModelResultJsonPage::from_json_str(&missing_content.to_string())
        .expect_err("a JSON page without content must fail closed");

    assert!(error.to_string().contains("missing field `content`"));
}
