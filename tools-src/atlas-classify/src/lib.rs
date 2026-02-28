//! Atlas Classification & Routing Tool for IronClaw.
//! Handles keyword routing and local Llama routing via LiteLLM.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ClassifyRequest {
    text: String,
}

#[derive(Serialize)]
struct ClassifyResponse {
    category: String,
    confidence: f32,
    routed_by: String,
}

struct AtlasClassifyTool;

impl exports::near::agent::tool::Guest for AtlasClassifyTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The message text to classify"
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Classifies incoming messages into PARA categories using keyword routing and local Llama 1B. \
         Routes to: GREETING, SIMPLE_FACT, ORCHESTRATION, SIMPLE_RESEARCH, DEEP_RESEARCH, COMPLEX_ANALYSIS, CODE_SIMPLE, CODE_COMPLEX, UNCERTAIN."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let req: ClassifyRequest = serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;
    let text = req.text.trim();

    // 1. Keyword Router (replicated from PARA logic)
    if let Some(category) = try_keyword_router(text) {
        return Ok(serde_json::to_string(&ClassifyResponse {
            category: category.to_string(),
            confidence: 1.0,
            routed_by: "keyword".to_string(),
        }).unwrap());
    }

    // 2. Local Llama Router via LiteLLM
    classify_via_local_llama(text)
}

fn try_keyword_router(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    if lower.starts_with("task:") { return Some("TASK"); }
    if lower.starts_with("note:") { return Some("NOTE"); }
    if lower.starts_with("project:") { return Some("PROJECT"); }
    if lower.starts_with("area:") { return Some("AREA"); }
    if lower.starts_with("person:") || lower.starts_with("contact:") { return Some("PEOPLE"); }
    None
}

fn classify_via_local_llama(text: &str) -> Result<String, String> {
    let system_prompt = "You are a routing classifier. Analyze the user request and categorize into exactly one of: GREETING, SIMPLE_FACT, ORCHESTRATION, SIMPLE_RESEARCH, DEEP_RESEARCH, COMPLEX_ANALYSIS, CODE_SIMPLE, CODE_COMPLEX, UNCERTAIN. Respond with only the category label.";
    
    let body = serde_json::json!({
        "model": "local-router",
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": text }
        ],
        "temperature": 0.0,
        "max_tokens": 10
    });

    let headers = r#"{"Content-Type": "application/json"}"#;
    let url = "http://host.docker.internal:4000/v1/chat/completions";

    let response = crate::near::agent::host::http_request(
        "POST",
        url,
        headers,
        Some(&body.to_string().as_bytes().to_vec()),
        None
    )?;

    if response.status != 200 {
        return Err(format!("Local router (LiteLLM) returned status {}", response.status));
    }

    let resp_json: serde_json::Value = serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
    let content = resp_json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("Invalid response from local router")?
        .trim()
        .to_uppercase();

    Ok(serde_json::to_string(&ClassifyResponse {
        category: content,
        confidence: 0.9, // Local router confidence estimate
        routed_by: "local-llama".to_string(),
    }).unwrap())
}

export!(AtlasClassifyTool);
