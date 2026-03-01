wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct TestPtcTool;

impl exports::near::agent::tool::Guest for TestPtcTool {
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
        r#"{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}"#.to_string()
    }

    fn description() -> String {
        "Test tool for PTC: calls echo via tool_invoke".to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(params)
        .map_err(|e| format!("Invalid params: {}", e))?;

    let message = parsed.get("message")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'message' parameter")?;

    // Build the parameters for the echo tool
    let echo_params = serde_json::json!({"message": message});

    // Call tool_invoke with alias "echo_alias" which should resolve to "echo"
    let result = near::agent::host::tool_invoke(
        "echo_alias",
        &echo_params.to_string(),
    )?;

    // Prefix to prove it went through WASM
    Ok(format!("via_wasm:{}", result))
}

export!(TestPtcTool);
