use ironclaw_host_api::model_result_preview::ModelResultPreview;
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
