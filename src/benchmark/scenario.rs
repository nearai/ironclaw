//! Benchmark scenario definitions and success criteria evaluation.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// JSON-native benchmark types (Phase 2)
// ---------------------------------------------------------------------------

/// Default timeout for a benchmark scenario in seconds.
fn default_timeout_secs() -> u64 {
    120
}

/// Default maximum tool iterations for a benchmark scenario.
fn default_max_tool_iterations() -> usize {
    20
}

/// Top-level benchmark scenario supporting multi-turn conversations,
/// workspace seeding, identity overrides, tool/skill restriction, per-turn
/// assertions, and LLM-as-judge scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchScenario {
    /// Unique scenario name (e.g., "file-write-read-roundtrip").
    pub name: String,
    /// Human-readable description of what this scenario tests.
    pub description: String,
    /// Tags for filtering/grouping scenarios (e.g., ["tool_selection", "memory"]).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional setup block: tools, skills, workspace seeding, identity overrides.
    #[serde(default)]
    pub setup: ScenarioSetup,
    /// Ordered list of conversation turns (user messages with per-turn assertions).
    pub turns: Vec<Turn>,
    /// Maximum seconds before the scenario times out (default: 120).
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum tool iterations the agent is allowed across all turns (default: 20).
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: usize,
}

/// Setup block for a benchmark scenario. All fields are optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioSetup {
    /// Allowlisted tool names. If non-empty, only these tools are available.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Allowlisted skill names. If non-empty, only these skills are available.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Workspace seeding configuration: pre-populate memory documents and fixtures.
    #[serde(default)]
    pub workspace: Option<WorkspaceSetup>,
    /// Identity overrides injected into the LLM system prompt (key-value pairs).
    #[serde(default)]
    pub identity: HashMap<String, String>,
}

/// Workspace seeding configuration for pre-populating memory before a scenario runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSetup {
    /// Documents to seed into the workspace memory system.
    #[serde(default)]
    pub documents: Vec<SeedDocument>,
    /// Optional directory containing fixture files to copy into the workspace.
    #[serde(default)]
    pub fixtures_dir: Option<String>,
}

/// A single document to seed into workspace memory before a scenario runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedDocument {
    /// The workspace path for this document (e.g., "context/vision.md").
    pub path: String,
    /// The document content (markdown or plain text).
    pub content: String,
}

/// A single conversation turn: a user message paired with expected outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// The user message to send to the agent.
    pub user: String,
    /// Optional per-turn assertions evaluated against the agent's response.
    #[serde(default)]
    pub assertions: TurnAssertions,
    /// Optional LLM-as-judge configuration for qualitative evaluation.
    #[serde(default)]
    pub judge: Option<JudgeConfig>,
}

/// Per-turn assertions evaluated programmatically against the agent's response
/// and tool usage. All fields are optional with sensible defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnAssertions {
    /// Tools that must have been called during this turn.
    #[serde(default)]
    pub tools_called: Vec<String>,
    /// Tools that must NOT have been called during this turn.
    #[serde(default)]
    pub tools_not_called: Vec<String>,
    /// Substrings that must appear in the response (case-insensitive).
    #[serde(default)]
    pub response_contains: Vec<String>,
    /// Substrings that must NOT appear in the response (case-insensitive).
    #[serde(default)]
    pub response_not_contains: Vec<String>,
    /// Regex patterns the response must match.
    #[serde(default)]
    pub response_matches: Vec<String>,
    /// Maximum number of tool calls allowed during this turn.
    #[serde(default)]
    pub max_tool_calls: Option<usize>,
    /// Maximum cost in USD allowed during this turn.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Maximum latency in seconds allowed for this turn.
    #[serde(default)]
    pub max_latency_secs: Option<f64>,
}

impl TurnAssertions {
    /// Convert these turn assertions into a `Vec<Criterion>` for backward
    /// compatibility with the existing evaluation engine.
    pub fn to_criteria(&self) -> Vec<Criterion> {
        let mut criteria = Vec::new();

        for tool in &self.tools_called {
            criteria.push(Criterion::ToolUsed { tool: tool.clone() });
        }
        for tool in &self.tools_not_called {
            criteria.push(Criterion::ToolNotUsed { tool: tool.clone() });
        }
        for text in &self.response_contains {
            criteria.push(Criterion::ResponseContains { text: text.clone() });
        }
        for text in &self.response_not_contains {
            criteria.push(Criterion::ResponseNotContains { text: text.clone() });
        }
        for pattern in &self.response_matches {
            criteria.push(Criterion::ResponseMatches {
                pattern: pattern.clone(),
            });
        }
        if let Some(max) = self.max_tool_calls {
            criteria.push(Criterion::ToolCallCountMax { max });
        }

        criteria
    }
}

/// LLM-as-judge configuration for qualitative evaluation of a turn's response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeConfig {
    /// Free-form criteria description passed to the judge LLM
    /// (e.g., "response should be helpful and accurate").
    pub criteria: String,
    /// Minimum acceptable score (0-100) from the judge.
    pub min_score: u8,
}

// ---------------------------------------------------------------------------
// Legacy JSON-oriented types (backward compatible)
// ---------------------------------------------------------------------------

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
    /// The agent's final response must NOT contain this text (case-insensitive).
    ResponseNotContains { text: String },
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
            Criterion::ResponseNotContains { text } => {
                let contains = ctx.response.to_lowercase().contains(&text.to_lowercase());
                CriterionResult {
                    criterion: format!("response_not_contains:{text}"),
                    passed: !contains,
                    reason: if !contains {
                        format!("Response correctly does not contain '{text}'")
                    } else {
                        format!("Response contains '{text}' but should not")
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

    // -----------------------------------------------------------------------
    // Phase 2 BenchScenario tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bench_scenario_basic_deserialize() {
        let json = r#"{
            "name": "echo-roundtrip",
            "description": "Verify the agent can echo a message back",
            "tags": ["tool_selection", "basic"],
            "turns": [
                {
                    "user": "Echo the word hello"
                }
            ]
        }"#;
        let scenario: BenchScenario = serde_json::from_str(json).unwrap();
        assert_eq!(scenario.name, "echo-roundtrip");
        assert_eq!(
            scenario.description,
            "Verify the agent can echo a message back"
        );
        assert_eq!(scenario.tags, vec!["tool_selection", "basic"]);
        assert_eq!(scenario.turns.len(), 1);
        assert_eq!(scenario.turns[0].user, "Echo the word hello");
        // Defaults
        assert_eq!(scenario.timeout_secs, 120);
        assert_eq!(scenario.max_tool_iterations, 20);
        assert!(scenario.setup.tools.is_empty());
        assert!(scenario.setup.skills.is_empty());
        assert!(scenario.setup.workspace.is_none());
        assert!(scenario.setup.identity.is_empty());
    }

    #[test]
    fn test_bench_scenario_multi_turn_with_workspace() {
        let json = r#"{
            "name": "file-write-read",
            "description": "Write a file then read it back",
            "turns": [
                {
                    "user": "Write 'hello world' to /tmp/test.txt",
                    "assertions": {
                        "tools_called": ["write_file"],
                        "response_contains": ["wrote", "test.txt"]
                    }
                },
                {
                    "user": "Read the file /tmp/test.txt",
                    "assertions": {
                        "tools_called": ["read_file"],
                        "response_contains": ["hello world"]
                    }
                }
            ],
            "setup": {
                "tools": ["write_file", "read_file"],
                "workspace": {
                    "documents": [
                        {
                            "path": "context/instructions.md",
                            "content": "Always confirm file operations."
                        }
                    ],
                    "fixtures_dir": "/tmp/test-fixtures"
                }
            }
        }"#;
        let scenario: BenchScenario = serde_json::from_str(json).unwrap();
        assert_eq!(scenario.turns.len(), 2);
        assert_eq!(
            scenario.turns[0].assertions.tools_called,
            vec!["write_file"]
        );
        assert_eq!(
            scenario.turns[0].assertions.response_contains,
            vec!["wrote", "test.txt"]
        );
        assert_eq!(scenario.turns[1].assertions.tools_called, vec!["read_file"]);

        let ws = scenario.setup.workspace.as_ref().unwrap();
        assert_eq!(ws.documents.len(), 1);
        assert_eq!(ws.documents[0].path, "context/instructions.md");
        assert!(ws.documents[0].content.contains("Always confirm"));
        assert_eq!(ws.fixtures_dir.as_deref(), Some("/tmp/test-fixtures"));
    }

    #[test]
    fn test_bench_scenario_with_judge() {
        let json = r#"{
            "name": "helpful-response",
            "description": "Agent should give a helpful answer",
            "turns": [
                {
                    "user": "Explain how to use git rebase",
                    "judge": {
                        "criteria": "The response should clearly explain interactive rebase with examples",
                        "min_score": 70
                    }
                }
            ]
        }"#;
        let scenario: BenchScenario = serde_json::from_str(json).unwrap();
        let judge = scenario.turns[0].judge.as_ref().unwrap();
        assert_eq!(
            judge.criteria,
            "The response should clearly explain interactive rebase with examples"
        );
        assert_eq!(judge.min_score, 70);
    }

    #[test]
    fn test_bench_scenario_with_identity_overrides() {
        let json = r#"{
            "name": "identity-test",
            "description": "Test with custom identity",
            "setup": {
                "identity": {
                    "AGENT_NAME": "TestBot",
                    "PERSONALITY": "You are a concise technical assistant."
                }
            },
            "turns": [
                {
                    "user": "What is your name?"
                }
            ]
        }"#;
        let scenario: BenchScenario = serde_json::from_str(json).unwrap();
        assert_eq!(scenario.setup.identity.len(), 2);
        assert_eq!(
            scenario.setup.identity.get("AGENT_NAME").unwrap(),
            "TestBot"
        );
        assert_eq!(
            scenario.setup.identity.get("PERSONALITY").unwrap(),
            "You are a concise technical assistant."
        );
    }

    #[test]
    fn test_turn_assertions_circuit_breakers() {
        let json = r#"{
            "name": "cost-limited",
            "description": "Test circuit breakers",
            "turns": [
                {
                    "user": "Do something expensive",
                    "assertions": {
                        "max_tool_calls": 5,
                        "max_cost_usd": 0.10,
                        "max_latency_secs": 30.0
                    }
                }
            ]
        }"#;
        let scenario: BenchScenario = serde_json::from_str(json).unwrap();
        let assertions = &scenario.turns[0].assertions;
        assert_eq!(assertions.max_tool_calls, Some(5));
        assert!((assertions.max_cost_usd.unwrap() - 0.10).abs() < f64::EPSILON);
        assert!((assertions.max_latency_secs.unwrap() - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_turn_assertions_response_not_contains() {
        let json = r#"{
            "name": "no-secrets",
            "description": "Agent should not leak secrets",
            "turns": [
                {
                    "user": "Show me the config",
                    "assertions": {
                        "response_not_contains": ["password", "secret_key", "api_token"]
                    }
                }
            ]
        }"#;
        let scenario: BenchScenario = serde_json::from_str(json).unwrap();
        assert_eq!(
            scenario.turns[0].assertions.response_not_contains,
            vec!["password", "secret_key", "api_token"]
        );
    }

    #[test]
    fn test_criterion_response_not_contains_pass() {
        let criterion = Criterion::ResponseNotContains {
            text: "secret".to_string(),
        };
        let ctx = EvalContext {
            response: "Here is the public configuration data.".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
        assert!(result.reason.contains("correctly does not contain"));
    }

    #[test]
    fn test_criterion_response_not_contains_fail() {
        let criterion = Criterion::ResponseNotContains {
            text: "secret".to_string(),
        };
        let ctx = EvalContext {
            response: "The SECRET key is abc123.".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(!result.passed);
        assert!(result.reason.contains("should not"));
    }

    #[test]
    fn test_turn_assertions_to_criteria() {
        let assertions = TurnAssertions {
            tools_called: vec!["echo".to_string(), "time".to_string()],
            tools_not_called: vec!["shell".to_string()],
            response_contains: vec!["hello".to_string()],
            response_not_contains: vec!["error".to_string()],
            response_matches: vec![r"\d{4}".to_string()],
            max_tool_calls: Some(10),
            max_cost_usd: Some(0.50),
            max_latency_secs: Some(60.0),
        };

        let criteria = assertions.to_criteria();

        // tools_called: 2 + tools_not_called: 1 + response_contains: 1
        // + response_not_contains: 1 + response_matches: 1 + max_tool_calls: 1 = 7
        assert_eq!(criteria.len(), 7);

        // Verify ordering and types
        assert!(matches!(&criteria[0], Criterion::ToolUsed { tool } if tool == "echo"));
        assert!(matches!(&criteria[1], Criterion::ToolUsed { tool } if tool == "time"));
        assert!(matches!(&criteria[2], Criterion::ToolNotUsed { tool } if tool == "shell"));
        assert!(matches!(&criteria[3], Criterion::ResponseContains { text } if text == "hello"));
        assert!(matches!(&criteria[4], Criterion::ResponseNotContains { text } if text == "error"));
        assert!(
            matches!(&criteria[5], Criterion::ResponseMatches { pattern } if pattern == r"\d{4}")
        );
        assert!(matches!(&criteria[6], Criterion::ToolCallCountMax { max } if *max == 10));

        // max_cost_usd and max_latency_secs are not converted to Criterion
        // (they are circuit breakers handled by the runner, not the evaluator)
    }
}
