//! Trace format / infrastructure tests.
//!
//! These tests verify JSON deserialization and backward compatibility of the
//! trace format. They do NOT require a rig, database, or the `libsql` feature.

mod support;

mod trace_format_tests {
    use crate::support::trace_llm::{LlmTrace, TraceExpects};

    /// A trace with only user_input steps and no playable steps deserializes.
    #[test]
    fn all_user_input_steps() {
        let json = r#"{
            "model_name": "recorded-all-user-input",
            "memory_snapshot": [],
            "steps": [
                { "response": { "type": "user_input", "content": "hello" } },
                { "response": { "type": "user_input", "content": "world" } }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.steps.len(), 2);
        assert_eq!(trace.playable_steps().len(), 0);
    }

    /// Backward compatibility: a trace without the new fields loads correctly.
    #[test]
    fn backward_compat_no_memory_snapshot() {
        let json = r#"{
            "model_name": "old-format",
            "steps": [
                {
                    "response": {
                        "type": "text",
                        "content": "hello",
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert!(trace.memory_snapshot.is_empty());
        assert!(trace.http_exchanges.is_empty());
        assert!(trace.expects.is_empty());
        assert_eq!(trace.playable_steps().len(), 1);
    }

    /// Expects round-trips through JSON serialization.
    #[test]
    fn expects_deserialization() {
        let json = r#"{
            "model_name": "expects-test",
            "expects": {
                "response_contains": ["hello", "world"],
                "tools_used": ["echo"],
                "all_tools_succeeded": true,
                "min_responses": 1,
                "tool_results_contain": { "echo": "greeting" }
            },
            "steps": [
                {
                    "response": {
                        "type": "text",
                        "content": "hello world",
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert!(!trace.expects.is_empty());
        assert_eq!(trace.expects.response_contains, vec!["hello", "world"]);
        assert_eq!(trace.expects.tools_used, vec!["echo"]);
        assert_eq!(trace.expects.all_tools_succeeded, Some(true));
        assert_eq!(trace.expects.min_responses, Some(1));
        assert_eq!(
            trace
                .expects
                .tool_results_contain
                .get("echo")
                .map(|s| s.as_str()),
            Some("greeting")
        );

        // Round-trip: serialize back and deserialize again.
        let serialized = serde_json::to_string(&trace).unwrap();
        let trace2: LlmTrace = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            trace2.expects.response_contains,
            trace.expects.response_contains
        );
        assert_eq!(trace2.expects.tools_used, trace.expects.tools_used);
    }

    /// A trace without `expects` loads with empty defaults.
    #[test]
    fn expects_default_empty() {
        let json = r#"{
            "model_name": "no-expects",
            "steps": [
                {
                    "response": {
                        "type": "text",
                        "content": "hi",
                        "input_tokens": 1,
                        "output_tokens": 1
                    }
                }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert!(trace.expects.is_empty());
    }

    /// Per-turn expects deserializes correctly.
    #[test]
    fn per_turn_expects() {
        let json = r#"{
            "model_name": "turn-expects",
            "turns": [
                {
                    "user_input": "hello",
                    "expects": {
                        "response_contains": ["greeting"],
                        "tools_not_used": ["shell"]
                    },
                    "steps": [
                        {
                            "response": {
                                "type": "text",
                                "content": "greeting back",
                                "input_tokens": 1,
                                "output_tokens": 1
                            }
                        }
                    ]
                }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.turns.len(), 1);
        assert!(!trace.turns[0].expects.is_empty());
        assert_eq!(trace.turns[0].expects.response_contains, vec!["greeting"]);
        assert_eq!(trace.turns[0].expects.tools_not_used, vec!["shell"]);
    }

    /// TraceExpects::is_empty() returns true for default.
    #[test]
    fn trace_expects_is_empty() {
        let e = TraceExpects::default();
        assert!(e.is_empty());
    }

    /// Flat steps with UserInput markers are split into multiple turns.
    #[test]
    fn recorded_multi_turn_splits_at_user_input() {
        let json = r#"{
            "model_name": "test",
            "steps": [
                { "response": { "type": "user_input", "content": "hello" } },
                { "response": { "type": "text", "content": "hi", "input_tokens": 10, "output_tokens": 5 } },
                { "response": { "type": "user_input", "content": "bye" } },
                { "response": { "type": "text", "content": "goodbye", "input_tokens": 20, "output_tokens": 5 } }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.turns.len(), 2);
        assert_eq!(trace.turns[0].user_input, "hello");
        assert_eq!(trace.turns[0].steps.len(), 1);
        assert_eq!(trace.turns[1].user_input, "bye");
        assert_eq!(trace.turns[1].steps.len(), 1);
    }

    /// Imported Claude Code traces can be deserialized by TraceLlm.
    #[test]
    fn imported_claude_code_trace_loads() {
        let dir = std::path::Path::new("tests/fixtures/llm_traces/imported");
        if !dir.exists() {
            // Skip if no imported traces present.
            return;
        }
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let trace = LlmTrace::from_file(&path)
                .unwrap_or_else(|e| panic!("Failed to load {}: {}", path.display(), e));
            assert!(!trace.turns.is_empty(), "{}: no turns", path.display());
            for (i, turn) in trace.turns.iter().enumerate() {
                assert!(
                    !turn.steps.is_empty(),
                    "{}: turn {} has no steps",
                    path.display(),
                    i
                );
            }
        }
    }

    /// Steps before the first UserInput get placeholder input.
    #[test]
    fn steps_before_first_user_input_get_placeholder() {
        let json = r#"{
            "model_name": "test",
            "steps": [
                { "response": { "type": "text", "content": "preamble", "input_tokens": 5, "output_tokens": 3 } },
                { "response": { "type": "user_input", "content": "hello" } },
                { "response": { "type": "text", "content": "hi", "input_tokens": 10, "output_tokens": 5 } }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.turns.len(), 2);
        assert_eq!(trace.turns[0].user_input, "(test input)");
        assert_eq!(trace.turns[0].steps.len(), 1);
        assert_eq!(trace.turns[1].user_input, "hello");
        assert_eq!(trace.turns[1].steps.len(), 1);
    }
}
