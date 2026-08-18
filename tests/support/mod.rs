pub mod assertions;
pub mod cleanup;
pub mod mock_mcp_server;
pub mod mock_openai_server;
pub mod trace_llm;

/// Minimal valid `execution_contract` payload for `builtin.trigger_create`
/// calls in tests. Shared so every suite scripts the same versioned contract
/// shape instead of hand-rolling near-duplicates.
#[allow(dead_code)] // Shared support is compiled into root tests that do not create triggers.
pub fn trigger_execution_contract(goal: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "goal": goal.into(),
        "success_criteria": ["Complete the requested task"],
        "output_instructions": "Return a concise result",
        "no_result_text": "No result",
        "policy": { "result_delivery": "deliver" }
    })
}
