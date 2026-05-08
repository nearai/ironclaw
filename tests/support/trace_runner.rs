#![allow(dead_code)]
//! Gateway-ops trace replay harness (#643).
//!
//! Complements the agentic [`TraceLlm`](super::trace_llm) replay system.
//! Where `TraceLlm` replays a recorded LLM stream and asserts the agent
//! produces the same tool calls, this harness takes the opposite
//! direction: a caller supplies an ordered sequence of tool invocations
//! (mimicking what a gateway handler would dispatch), and the runner
//! executes them against real `Tool` implementations backed by a test DB.
//!
//! The runner:
//! 1. Creates a parent `agent_jobs` row (the `job_actions` table has a
//!    `FK → agent_jobs(id)` constraint that `save_action` depends on).
//! 2. For each operation, looks up the tool by name, executes it, builds
//!    the `ActionRecord`, and persists it via `Database::save_action`.
//! 3. Checks each result against the declared `TraceExpectation`, logging
//!    any mismatches as `TraceFailure` entries.
//!
//! See `tests/fixtures/gateway_traces/README.md` for the JSON wire format.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ironclaw::context::{ActionRecord, JobContext};
use ironclaw::db::Database;
use ironclaw::tools::{ToolError, ToolOutput, ToolRegistry};

/// An ordered sequence of tool invocations with expected outcomes.
///
/// Serialization format matches the fixture JSON files under
/// `tests/fixtures/gateway_traces/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    /// Human-readable identifier (used in error messages and logs).
    pub name: String,
    /// The sequence of operations to execute in order.
    pub operations: Vec<TraceOperation>,
}

/// A single tool invocation within a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceOperation {
    /// Tool name as registered in the `ToolRegistry`.
    pub tool_name: String,
    /// Input parameters (JSON) passed to `Tool::execute`.
    pub params: serde_json::Value,
    /// What the runner should assert about the outcome.
    pub expected: TraceExpectation,
}

/// What the runner should assert about an operation's outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceExpectation {
    /// The tool call should succeed. Optional `assertions` is a JSON
    /// object describing field matches on the tool output.
    ///
    /// Supported assertion keys:
    /// - `eq`: deep equality against the entire tool output
    /// - `contains_text`: the output (stringified) must contain this substring
    /// - `fields`: an object whose keys are JSON paths (dot-separated) and
    ///   whose values must match the value at that path in the output
    Success {
        #[serde(default)]
        assertions: serde_json::Value,
    },
    /// The tool call should fail. The error's `Display` must contain
    /// `error_contains`.
    Failure { error_contains: String },
}

/// The result of replaying a trace.
#[derive(Debug, Clone)]
pub struct TraceResult {
    /// Job-scoped id the runner created before persisting actions. Use
    /// this with `Database::get_job_actions` to verify persistence.
    pub job_id: Uuid,
    /// Recorded `ActionRecord` for every operation (including failed ones).
    pub records: Vec<ActionRecord>,
    /// Any assertion mismatches encountered during the replay. An empty
    /// vector means every operation matched its `TraceExpectation`.
    pub failures: Vec<TraceFailure>,
}

/// A single assertion failure inside a trace replay.
#[derive(Debug, Clone)]
pub struct TraceFailure {
    /// Zero-based index of the failing operation in `Trace::operations`.
    pub operation_index: usize,
    /// Tool name for the failing operation (for quick scanning).
    pub tool_name: String,
    /// Human-readable explanation (what was expected vs what happened).
    pub reason: String,
}

/// Errors that can stop a trace before it finishes.
#[derive(Debug, thiserror::Error)]
pub enum TraceRunnerError {
    /// The parent `agent_jobs` row could not be created.
    #[error("failed to create parent job row: {0}")]
    JobRowCreate(String),
    /// Persisting an `ActionRecord` failed. The trace is aborted because
    /// subsequent operations would be orphaned.
    #[error("failed to persist ActionRecord at operation {index}: {reason}")]
    PersistFailed { index: usize, reason: String },
}

/// Replays a [`Trace`] against a live `ToolRegistry` + `Database`.
pub struct TraceRunner {
    tools: Arc<ToolRegistry>,
    store: Arc<dyn Database>,
    user_id: String,
}

impl TraceRunner {
    /// Construct a new runner.
    pub fn new(
        tools: Arc<ToolRegistry>,
        store: Arc<dyn Database>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            tools,
            store,
            user_id: user_id.into(),
        }
    }

    /// Replay a trace and return the recorded actions + any failures.
    ///
    /// Creates exactly one `agent_jobs` parent row so the `job_actions`
    /// FK constraint is satisfied. Operations are executed sequentially;
    /// a later op may depend on state mutated by an earlier one.
    ///
    /// A missing tool is treated as a `Failure` outcome: an `ActionRecord`
    /// is still persisted with an error message, and the expectation is
    /// checked against that. This mirrors how gateway handlers would
    /// report an unknown tool to the caller.
    pub async fn replay(&self, trace: &Trace) -> Result<TraceResult, TraceRunnerError> {
        let ctx = JobContext::with_user(
            self.user_id.clone(),
            format!("trace:{}", trace.name),
            "gateway-ops trace replay",
        );

        self.store
            .save_job(&ctx)
            .await
            .map_err(|e| TraceRunnerError::JobRowCreate(e.to_string()))?;

        let mut records = Vec::with_capacity(trace.operations.len());
        let mut failures = Vec::new();

        for (index, op) in trace.operations.iter().enumerate() {
            let (result, duration) = run_one(&self.tools, op, &ctx).await;

            let record = match &result {
                Ok(output) => ActionRecord::new(index as u32, &op.tool_name, op.params.clone())
                    .succeed(None, output.result.clone(), duration),
                Err(err) => ActionRecord::new(index as u32, &op.tool_name, op.params.clone())
                    .fail(err.to_string(), duration),
            };

            self.store
                .save_action(ctx.job_id, &record)
                .await
                .map_err(|e| TraceRunnerError::PersistFailed {
                    index,
                    reason: e.to_string(),
                })?;

            if let Some(failure) = evaluate_expectation(index, op, &result) {
                failures.push(failure);
            }

            records.push(record);
        }

        Ok(TraceResult {
            job_id: ctx.job_id,
            records,
            failures,
        })
    }
}

async fn run_one(
    tools: &Arc<ToolRegistry>,
    op: &TraceOperation,
    ctx: &JobContext,
) -> (Result<ToolOutput, ToolError>, Duration) {
    let start = Instant::now();
    let result = match tools.get(&op.tool_name).await {
        Some(tool) => tool.execute(op.params.clone(), ctx).await,
        None => Err(ToolError::ExecutionFailed(format!(
            "tool not registered: {}",
            op.tool_name
        ))),
    };
    (result, start.elapsed())
}

fn evaluate_expectation(
    index: usize,
    op: &TraceOperation,
    result: &Result<ToolOutput, ToolError>,
) -> Option<TraceFailure> {
    match (result, &op.expected) {
        (Ok(output), TraceExpectation::Success { assertions }) => {
            check_success_assertions(&output.result, assertions).map(|reason| TraceFailure {
                operation_index: index,
                tool_name: op.tool_name.clone(),
                reason,
            })
        }
        (Err(err), TraceExpectation::Failure { error_contains }) => {
            if error_contains.trim().is_empty() {
                return Some(TraceFailure {
                    operation_index: index,
                    tool_name: op.tool_name.clone(),
                    reason: "failure expectations must set a non-empty error_contains".into(),
                });
            }
            let msg = err.to_string();
            if msg.contains(error_contains) {
                None
            } else {
                Some(TraceFailure {
                    operation_index: index,
                    tool_name: op.tool_name.clone(),
                    reason: format!("expected error containing {error_contains:?}, got {msg:?}"),
                })
            }
        }
        (Ok(_), TraceExpectation::Failure { error_contains }) => Some(TraceFailure {
            operation_index: index,
            tool_name: op.tool_name.clone(),
            reason: format!("expected failure containing {error_contains:?}, got success instead"),
        }),
        (Err(err), TraceExpectation::Success { .. }) => Some(TraceFailure {
            operation_index: index,
            tool_name: op.tool_name.clone(),
            reason: format!("expected success, got error: {err}"),
        }),
    }
}

/// Walk the assertion DSL and return `Some(reason)` on mismatch.
fn check_success_assertions(
    output: &serde_json::Value,
    assertions: &serde_json::Value,
) -> Option<String> {
    if assertions.is_null() {
        return None;
    }

    let Some(obj) = assertions.as_object() else {
        return Some(format!(
            "assertions must be a JSON object, got: {assertions}"
        ));
    };

    for (key, expected) in obj {
        match key.as_str() {
            "eq" => {
                if output != expected {
                    return Some(format!("eq mismatch: expected {expected}, got {output}"));
                }
            }
            "contains_text" => {
                let Some(needle) = expected.as_str() else {
                    return Some(format!("contains_text must be a string, got: {expected}"));
                };
                let haystack = output.to_string();
                if !haystack.contains(needle) {
                    return Some(format!(
                        "contains_text mismatch: {haystack:?} does not contain {needle:?}"
                    ));
                }
            }
            "fields" => {
                let Some(fields) = expected.as_object() else {
                    return Some(format!("fields must be an object, got: {expected}"));
                };
                for (path, want) in fields {
                    let got = json_path(output, path);
                    if got != Some(want) {
                        return Some(format!(
                            "fields[{path}] mismatch: expected {want}, got {got:?}"
                        ));
                    }
                }
            }
            other => {
                return Some(format!("unknown assertion key: {other}"));
            }
        }
    }

    None
}

/// Resolve a dot-separated path in a JSON value. `a.b.c` walks objects;
/// numeric segments index arrays.
fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = value;
    for seg in path.split('.') {
        cur = match cur {
            serde_json::Value::Object(map) => map.get(seg)?,
            serde_json::Value::Array(arr) => {
                let idx: usize = seg.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_path_walks_nested_objects() {
        let v = serde_json::json!({"a": {"b": {"c": 42}}});
        assert_eq!(json_path(&v, "a.b.c"), Some(&serde_json::Value::from(42)));
        assert_eq!(json_path(&v, "a.b.missing"), None);
    }

    #[test]
    fn json_path_indexes_arrays() {
        let v = serde_json::json!({"items": ["x", "y", "z"]});
        assert_eq!(
            json_path(&v, "items.1"),
            Some(&serde_json::Value::from("y"))
        );
        assert_eq!(json_path(&v, "items.99"), None);
    }

    #[test]
    fn check_assertions_eq_matches() {
        let out = serde_json::json!({"foo": 1});
        let asserts = serde_json::json!({"eq": {"foo": 1}});
        assert!(check_success_assertions(&out, &asserts).is_none());
    }

    #[test]
    fn check_assertions_eq_reports_mismatch() {
        let out = serde_json::json!({"foo": 1});
        let asserts = serde_json::json!({"eq": {"foo": 2}});
        let reason = check_success_assertions(&out, &asserts).expect("should fail");
        assert!(reason.contains("eq mismatch"));
    }

    #[test]
    fn check_assertions_contains_text() {
        let out = serde_json::json!("hello world");
        let asserts = serde_json::json!({"contains_text": "world"});
        assert!(check_success_assertions(&out, &asserts).is_none());

        let asserts_bad = serde_json::json!({"contains_text": "nope"});
        assert!(check_success_assertions(&out, &asserts_bad).is_some());
    }

    #[test]
    fn check_assertions_fields_paths() {
        let out = serde_json::json!({"data": {"items": [{"id": 7}]}});
        let asserts = serde_json::json!({
            "fields": {
                "data.items.0.id": 7,
            }
        });
        assert!(check_success_assertions(&out, &asserts).is_none());
    }

    #[test]
    fn failure_expectation_rejects_empty_error_contains() {
        let op = TraceOperation {
            tool_name: "missing".into(),
            params: serde_json::Value::Null,
            expected: TraceExpectation::Failure {
                error_contains: " ".into(),
            },
        };
        let failure = evaluate_expectation(0, &op, &Err(ToolError::ExecutionFailed("boom".into())))
            .expect("empty error_contains must be rejected");
        assert!(failure.reason.contains("non-empty error_contains"));
    }

    #[test]
    fn trace_roundtrips_json() {
        let trace = Trace {
            name: "demo".into(),
            operations: vec![
                TraceOperation {
                    tool_name: "echo".into(),
                    params: serde_json::json!({"message": "hi"}),
                    expected: TraceExpectation::Success {
                        assertions: serde_json::json!({"contains_text": "hi"}),
                    },
                },
                TraceOperation {
                    tool_name: "missing".into(),
                    params: serde_json::Value::Null,
                    expected: TraceExpectation::Failure {
                        error_contains: "not found".into(),
                    },
                },
            ],
        };
        let j = serde_json::to_value(&trace).unwrap();
        let back: Trace = serde_json::from_value(j).unwrap();
        assert_eq!(back.name, "demo");
        assert_eq!(back.operations.len(), 2);
    }
}
