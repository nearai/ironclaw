use std::collections::HashSet;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::BenchError;
use crate::suite::{BenchScore, BenchSuite, BenchTask, TaskSubmission};

/// Multi-criterion assertions for a spot check scenario.
///
/// Each field generates one or more individual checks. The final score is
/// `passed_checks / total_checks`, giving a value between 0.0 and 1.0.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpotAssertions {
    /// All must appear in the response (case-insensitive).
    #[serde(default)]
    pub response_contains: Vec<String>,

    /// None may appear in the response (case-insensitive).
    #[serde(default)]
    pub response_not_contains: Vec<String>,

    /// Each tool name must appear in the tool_calls list (checked by name,
    /// not by count; duplicates in tool_calls are collapsed).
    #[serde(default)]
    pub tools_used: Vec<String>,

    /// None of these tool names may appear in the tool_calls list.
    #[serde(default)]
    pub tools_not_used: Vec<String>,

    /// Regex pattern the response must match.
    #[serde(default)]
    pub response_matches: Option<String>,

    /// Hard fail if the task produced an error.
    #[serde(default)]
    pub no_error: bool,

    /// Minimum number of tool calls expected (counts duplicates).
    #[serde(default)]
    pub min_tool_calls: Option<usize>,

    /// Maximum number of tool calls allowed (counts duplicates).
    #[serde(default)]
    pub max_tool_calls: Option<usize>,
}

impl SpotAssertions {
    /// Evaluate all assertions against a submission, returning (score, failure_details).
    pub fn evaluate(&self, submission: &TaskSubmission) -> (f64, Vec<String>) {
        let mut passed: usize = 0;
        let mut total: usize = 0;
        let mut failures: Vec<String> = Vec::new();

        // Hard fail: error check
        if self.no_error {
            total += 1;
            if let Some(ref err) = submission.error {
                failures.push(format!("no_error: task errored with: {err}"));
                // Hard fail: return 0.0 immediately
                return (0.0, failures);
            }
            passed += 1;
        }

        let response_lower = submission.response.to_lowercase();

        // response_contains: all must appear
        for needle in &self.response_contains {
            total += 1;
            if response_lower.contains(&needle.to_lowercase()) {
                passed += 1;
            } else {
                failures.push(format!("response_contains: missing \"{needle}\""));
            }
        }

        // response_not_contains: none may appear
        for needle in &self.response_not_contains {
            total += 1;
            if response_lower.contains(&needle.to_lowercase()) {
                failures.push(format!("response_not_contains: found \"{needle}\""));
            } else {
                passed += 1;
            }
        }

        let tool_set: HashSet<&str> = submission.tool_calls.iter().map(|s| s.as_str()).collect();

        // tools_used: each must appear
        for tool in &self.tools_used {
            total += 1;
            if tool_set.contains(tool.as_str()) {
                passed += 1;
            } else {
                failures.push(format!("tools_used: \"{tool}\" not called"));
            }
        }

        // tools_not_used: none may appear
        for tool in &self.tools_not_used {
            total += 1;
            if tool_set.contains(tool.as_str()) {
                failures.push(format!("tools_not_used: \"{tool}\" was called"));
            } else {
                passed += 1;
            }
        }

        // response_matches: regex pattern
        if let Some(ref pattern) = self.response_matches {
            total += 1;
            match Regex::new(pattern) {
                Ok(re) => {
                    if re.is_match(&submission.response) {
                        passed += 1;
                    } else {
                        failures.push(format!("response_matches: /{pattern}/ did not match"));
                    }
                }
                Err(e) => {
                    failures.push(format!("response_matches: bad regex: {e}"));
                }
            }
        }

        let call_count = submission.tool_calls.len();

        // min_tool_calls
        if let Some(min) = self.min_tool_calls {
            total += 1;
            if call_count >= min {
                passed += 1;
            } else {
                failures.push(format!(
                    "min_tool_calls: expected >= {min}, got {call_count}"
                ));
            }
        }

        // max_tool_calls
        if let Some(max) = self.max_tool_calls {
            total += 1;
            if call_count <= max {
                passed += 1;
            } else {
                failures.push(format!(
                    "max_tool_calls: expected <= {max}, got {call_count}"
                ));
            }
        }

        if total == 0 {
            return (1.0, failures);
        }

        let score = passed as f64 / total as f64;
        (score, failures)
    }
}

/// JSONL entry for a spot check scenario.
#[derive(Debug, Deserialize)]
struct SpotEntry {
    id: String,
    prompt: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    assertions: SpotAssertions,
}

/// Spot benchmark suite: end-to-end checks for real user workflows.
///
/// Tests conversation, individual tool use, multi-tool chaining, and robustness.
/// Each task declares multi-criterion assertions scored as passed/total.
pub struct SpotSuite {
    dataset_path: PathBuf,
}

impl SpotSuite {
    pub fn new(dataset_path: impl Into<PathBuf>) -> Self {
        Self {
            dataset_path: dataset_path.into(),
        }
    }
}

#[async_trait]
impl BenchSuite for SpotSuite {
    fn name(&self) -> &str {
        "Spot Checks"
    }

    fn id(&self) -> &str {
        "spot"
    }

    async fn load_tasks(&self) -> Result<Vec<BenchTask>, BenchError> {
        let file = std::fs::File::open(&self.dataset_path).map_err(BenchError::Io)?;
        let reader = std::io::BufReader::new(file);
        let mut tasks = Vec::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: SpotEntry = serde_json::from_str(trimmed)
                .map_err(|e| BenchError::Config(format!("spot line {}: {}", line_num + 1, e)))?;

            let metadata = serde_json::json!({
                "assertions": serde_json::to_value(&entry.assertions)
                    .map_err(|e| BenchError::Config(format!("spot {}: {}", entry.id, e)))?,
            });

            tasks.push(BenchTask {
                id: entry.id,
                prompt: entry.prompt,
                context: entry.context,
                resources: vec![],
                tags: entry.tags,
                expected_turns: None,
                timeout: None,
                metadata,
            });
        }

        Ok(tasks)
    }

    async fn score(
        &self,
        task: &BenchTask,
        submission: &TaskSubmission,
    ) -> Result<BenchScore, BenchError> {
        let assertions: SpotAssertions = task
            .metadata
            .get("assertions")
            .ok_or_else(|| BenchError::Scoring {
                task_id: task.id.clone(),
                reason: "missing assertions in metadata".to_string(),
            })
            .and_then(|v| {
                serde_json::from_value(v.clone()).map_err(|e| BenchError::Scoring {
                    task_id: task.id.clone(),
                    reason: format!("bad assertions: {e}"),
                })
            })?;

        let (score, failures) = assertions.evaluate(submission);

        if score >= 1.0 {
            Ok(BenchScore::pass())
        } else if score <= 0.0 {
            Ok(BenchScore::fail(failures.join("; ")))
        } else {
            Ok(BenchScore::partial(score, failures.join("; ")))
        }
    }

    fn additional_tools(&self) -> Vec<Arc<dyn ironclaw::tools::Tool>> {
        vec![
            Arc::new(ironclaw::tools::builtin::ShellTool::new()),
            Arc::new(ironclaw::tools::builtin::ReadFileTool::new()),
            Arc::new(ironclaw::tools::builtin::WriteFileTool::new()),
            Arc::new(ironclaw::tools::builtin::ListDirTool::new()),
            Arc::new(ironclaw::tools::builtin::ApplyPatchTool::new()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_submission(
        response: &str,
        tool_calls: Vec<&str>,
        error: Option<&str>,
    ) -> TaskSubmission {
        TaskSubmission {
            response: response.to_string(),
            conversation: vec![],
            tool_calls: tool_calls.into_iter().map(|s| s.to_string()).collect(),
            error: error.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_all_pass() {
        let assertions = SpotAssertions {
            response_contains: vec!["hello".to_string()],
            tools_used: vec!["echo".to_string()],
            no_error: true,
            ..Default::default()
        };
        let sub = make_submission("Hello, world!", vec!["echo"], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 1.0);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_hard_fail_on_error() {
        let assertions = SpotAssertions {
            no_error: true,
            response_contains: vec!["hello".to_string()],
            ..Default::default()
        };
        let sub = make_submission("Hello!", vec![], Some("timeout after 60s"));
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 0.0);
        assert!(failures[0].contains("no_error"));
    }

    #[test]
    fn test_partial_score() {
        let assertions = SpotAssertions {
            response_contains: vec!["alpha".to_string(), "beta".to_string()],
            ..Default::default()
        };
        let sub = make_submission("alpha is here but not the other", vec![], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 0.5);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("beta"));
    }

    #[test]
    fn test_response_not_contains() {
        let assertions = SpotAssertions {
            response_not_contains: vec!["error".to_string(), "fail".to_string()],
            ..Default::default()
        };
        let sub = make_submission("This is an error message", vec![], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 0.5);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("error"));
    }

    #[test]
    fn test_tools_used_and_not_used() {
        let assertions = SpotAssertions {
            tools_used: vec!["time".to_string()],
            tools_not_used: vec!["shell".to_string(), "echo".to_string()],
            ..Default::default()
        };
        let sub = make_submission("The time is now", vec!["time"], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 1.0);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_tools_not_used_fails() {
        let assertions = SpotAssertions {
            tools_not_used: vec!["shell".to_string()],
            ..Default::default()
        };
        let sub = make_submission("result", vec!["shell", "time"], None);
        let (score, _) = assertions.evaluate(&sub);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_response_matches_regex() {
        let assertions = SpotAssertions {
            response_matches: Some(r"\d{4}".to_string()),
            ..Default::default()
        };
        let sub = make_submission("The year is 2026", vec![], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 1.0);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_response_matches_regex_fail() {
        let assertions = SpotAssertions {
            response_matches: Some(r"^\d+$".to_string()),
            ..Default::default()
        };
        let sub = make_submission("not a number", vec![], None);
        let (score, _) = assertions.evaluate(&sub);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_min_max_tool_calls() {
        let assertions = SpotAssertions {
            min_tool_calls: Some(2),
            max_tool_calls: Some(4),
            ..Default::default()
        };

        // Within range
        let sub = make_submission("ok", vec!["a", "b", "c"], None);
        let (score, _) = assertions.evaluate(&sub);
        assert_eq!(score, 1.0);

        // Too few
        let sub = make_submission("ok", vec!["a"], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 0.5);
        assert!(failures[0].contains("min_tool_calls"));

        // Too many
        let sub = make_submission("ok", vec!["a", "b", "c", "d", "e"], None);
        let (score, failures) = assertions.evaluate(&sub);
        assert_eq!(score, 0.5);
        assert!(failures[0].contains("max_tool_calls"));
    }

    #[test]
    fn test_max_zero_tool_calls() {
        let assertions = SpotAssertions {
            max_tool_calls: Some(0),
            ..Default::default()
        };
        let sub = make_submission("just talking", vec![], None);
        let (score, _) = assertions.evaluate(&sub);
        assert_eq!(score, 1.0);

        let sub = make_submission("oops", vec!["echo"], None);
        let (score, _) = assertions.evaluate(&sub);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_empty_assertions() {
        let assertions = SpotAssertions::default();
        let sub = make_submission("anything", vec!["whatever"], None);
        let (score, _) = assertions.evaluate(&sub);
        assert_eq!(score, 1.0);
    }

    #[tokio::test]
    async fn test_spot_load_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spot.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "s1", "prompt": "Hello", "tags": ["smoke"], "assertions": {{"response_contains": ["hello"], "no_error": true}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"id": "s2", "prompt": "Echo test", "assertions": {{"tools_used": ["echo"]}}}}"#
        )
        .unwrap();

        let suite = SpotSuite::new(&path);
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "s1");
        assert_eq!(tasks[1].id, "s2");
        assert!(tasks[0].tags.contains(&"smoke".to_string()));
    }

    #[tokio::test]
    async fn test_spot_scoring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spot.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "s1", "prompt": "Hello", "assertions": {{"response_contains": ["hello", "world"], "no_error": true}}}}"#
        )
        .unwrap();

        let suite = SpotSuite::new(&path);
        let tasks = suite.load_tasks().await.unwrap();

        // Full pass
        let sub = make_submission("Hello World!", vec![], None);
        let score = suite.score(&tasks[0], &sub).await.unwrap();
        assert_eq!(score.value, 1.0);
        assert_eq!(score.label, "pass");

        // Partial
        let sub = make_submission("Hello there", vec![], None);
        let score = suite.score(&tasks[0], &sub).await.unwrap();
        assert!(score.value > 0.0 && score.value < 1.0);
        assert_eq!(score.label, "partial");

        // Error hard fail
        let sub = make_submission("Hello World!", vec![], Some("boom"));
        let score = suite.score(&tasks[0], &sub).await.unwrap();
        assert_eq!(score.value, 0.0);
        assert_eq!(score.label, "fail");
    }
}
