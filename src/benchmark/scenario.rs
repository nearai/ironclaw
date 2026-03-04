//! Benchmark scenario definitions and success criteria evaluation.

use serde::{Deserialize, Serialize};

/// A benchmark scenario: a task with input, success criteria, and resource limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    /// Unique identifier (e.g., "file-write-read-roundtrip").
    pub id: String,
    /// Category for grouping (e.g., "tool_selection", "error_recovery").
    pub category: String,
    /// The user message to send to the agent.
    pub input: String,
    /// All criteria must pass for the scenario to pass.
    pub success_criteria: Vec<Criterion>,
    /// Maximum seconds before the scenario times out.
    pub timeout_secs: u64,
    /// Maximum tool iterations the agent is allowed.
    pub max_tool_iterations: usize,
}

/// A single success criterion. Evaluated programmatically -- no LLM judgment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Criterion {
    /// The agent must have called this tool at least once.
    ToolUsed { tool: String },
    /// The agent must NOT have called this tool.
    ToolNotUsed { tool: String },
    /// Total tool calls must not exceed this count.
    ToolCallCountMax { max: usize },
    /// The agent's final response must contain this text (case-insensitive).
    ResponseContains { text: String },
    /// The agent's final response must match this regex pattern.
    ResponseMatches { pattern: String },
}

/// Context provided to criteria for evaluation.
pub struct EvalContext {
    /// The agent's final text response.
    pub response: String,
    /// Tool calls made: (name, success).
    pub tool_calls: Vec<(String, bool)>,
}

/// Result of evaluating a single criterion.
#[derive(Debug, Clone)]
pub struct CriterionResult {
    /// Which criterion was evaluated.
    pub criterion: String,
    /// Whether it passed.
    pub passed: bool,
    /// Human-readable explanation.
    pub reason: String,
}

impl Criterion {
    /// Evaluate this criterion against the given context.
    pub fn evaluate(&self, ctx: &EvalContext) -> CriterionResult {
        match self {
            Criterion::ToolUsed { tool } => {
                let used = ctx.tool_calls.iter().any(|(name, _)| name == tool);
                CriterionResult {
                    criterion: format!("tool_used:{tool}"),
                    passed: used,
                    reason: if used {
                        format!("Tool '{tool}' was called")
                    } else {
                        format!(
                            "Tool '{tool}' was NOT called. Tools used: {:?}",
                            ctx.tool_calls
                                .iter()
                                .map(|(n, _)| n.as_str())
                                .collect::<Vec<_>>()
                        )
                    },
                }
            }
            Criterion::ToolNotUsed { tool } => {
                let used = ctx.tool_calls.iter().any(|(name, _)| name == tool);
                CriterionResult {
                    criterion: format!("tool_not_used:{tool}"),
                    passed: !used,
                    reason: if !used {
                        format!("Tool '{tool}' was correctly not called")
                    } else {
                        format!("Tool '{tool}' was called but should not have been")
                    },
                }
            }
            Criterion::ToolCallCountMax { max } => {
                let count = ctx.tool_calls.len();
                CriterionResult {
                    criterion: format!("tool_call_count_max:{max}"),
                    passed: count <= *max,
                    reason: format!("{count} tool calls (max {max})"),
                }
            }
            Criterion::ResponseContains { text } => {
                let contains = ctx.response.to_lowercase().contains(&text.to_lowercase());
                CriterionResult {
                    criterion: format!("response_contains:{text}"),
                    passed: contains,
                    reason: if contains {
                        format!("Response contains '{text}'")
                    } else {
                        format!("Response does NOT contain '{text}'")
                    },
                }
            }
            Criterion::ResponseMatches { pattern } => match regex::Regex::new(pattern) {
                Ok(re) => {
                    let matches = re.is_match(&ctx.response);
                    CriterionResult {
                        criterion: format!("response_matches:{pattern}"),
                        passed: matches,
                        reason: if matches {
                            format!("Response matches pattern '{pattern}'")
                        } else {
                            format!("Response does NOT match pattern '{pattern}'")
                        },
                    }
                }
                Err(e) => CriterionResult {
                    criterion: format!("response_matches:{pattern}"),
                    passed: false,
                    reason: format!("Invalid regex pattern: {e}"),
                },
            },
        }
    }
}

/// Evaluate all criteria against the context. Returns (all_passed, individual_results).
pub fn evaluate_criteria(
    criteria: &[Criterion],
    ctx: &EvalContext,
) -> (bool, Vec<CriterionResult>) {
    let results: Vec<CriterionResult> = criteria.iter().map(|c| c.evaluate(ctx)).collect();
    let all_passed = results.iter().all(|r| r.passed);
    (all_passed, results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_deserialize() {
        let json = r#"{
            "id": "test-echo",
            "category": "tool_selection",
            "input": "Say hello",
            "success_criteria": [
                {"type": "tool_used", "tool": "echo"},
                {"type": "response_contains", "text": "hello"}
            ],
            "timeout_secs": 30,
            "max_tool_iterations": 10
        }"#;
        let scenario: Scenario = serde_json::from_str(json).unwrap();
        assert_eq!(scenario.id, "test-echo");
        assert_eq!(scenario.success_criteria.len(), 2);
    }

    #[test]
    fn test_criterion_tool_used_pass() {
        let criterion = Criterion::ToolUsed {
            tool: "echo".to_string(),
        };
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_tool_used_fail() {
        let criterion = Criterion::ToolUsed {
            tool: "echo".to_string(),
        };
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("time".to_string(), true)],
        };
        let result = criterion.evaluate(&ctx);
        assert!(!result.passed);
    }

    #[test]
    fn test_criterion_tool_not_used_pass() {
        let criterion = Criterion::ToolNotUsed {
            tool: "shell".to_string(),
        };
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_response_contains_pass() {
        let criterion = Criterion::ResponseContains {
            text: "hello".to_string(),
        };
        let ctx = EvalContext {
            response: "I said hello to you".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_response_contains_case_insensitive() {
        let criterion = Criterion::ResponseContains {
            text: "hello".to_string(),
        };
        let ctx = EvalContext {
            response: "I said HELLO to you".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_response_matches_regex() {
        let criterion = Criterion::ResponseMatches {
            pattern: r"20\d{2}".to_string(),
        };
        let ctx = EvalContext {
            response: "The year is 2026".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_tool_call_count_max() {
        let criterion = Criterion::ToolCallCountMax { max: 3 };
        let ctx = EvalContext {
            response: "done".to_string(),
            tool_calls: vec![
                ("a".to_string(), true),
                ("b".to_string(), true),
                ("c".to_string(), true),
                ("d".to_string(), true),
            ],
        };
        let result = criterion.evaluate(&ctx);
        assert!(!result.passed);
        assert!(result.reason.contains("4"));
    }

    #[test]
    fn test_evaluate_all_criteria() {
        let criteria = vec![
            Criterion::ToolUsed {
                tool: "echo".to_string(),
            },
            Criterion::ResponseContains {
                text: "hello".to_string(),
            },
        ];
        let ctx = EvalContext {
            response: "hello world".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let (passed, results) = evaluate_criteria(&criteria, &ctx);
        assert!(passed);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.passed));
    }

    #[test]
    fn test_evaluate_criteria_one_fails() {
        let criteria = vec![
            Criterion::ToolUsed {
                tool: "echo".to_string(),
            },
            Criterion::ToolUsed {
                tool: "time".to_string(),
            },
        ];
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let (passed, results) = evaluate_criteria(&criteria, &ctx);
        assert!(!passed);
        assert_eq!(results.iter().filter(|r| !r.passed).count(), 1);
    }

    #[test]
    fn test_load_scenario_files() {
        let scenarios_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benchmarks/scenarios");
        for entry in std::fs::read_dir(scenarios_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                let content = std::fs::read_to_string(&path).unwrap();
                let scenarios: Vec<Scenario> = serde_json::from_str(&content)
                    .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));
                assert!(
                    !scenarios.is_empty(),
                    "Empty scenario file: {}",
                    path.display()
                );
                for s in &scenarios {
                    assert!(!s.id.is_empty(), "Empty scenario ID in {}", path.display());
                    assert!(!s.input.is_empty(), "Empty input in scenario {}", s.id);
                }
            }
        }
    }

    #[test]
    fn test_load_scenarios_from_json_array() {
        let json = r#"[
            {
                "id": "s1",
                "category": "test",
                "input": "hello",
                "success_criteria": [],
                "timeout_secs": 10,
                "max_tool_iterations": 5
            },
            {
                "id": "s2",
                "category": "test",
                "input": "world",
                "success_criteria": [],
                "timeout_secs": 10,
                "max_tool_iterations": 5
            }
        ]"#;
        let scenarios: Vec<Scenario> = serde_json::from_str(json).unwrap();
        assert_eq!(scenarios.len(), 2);
    }
}
