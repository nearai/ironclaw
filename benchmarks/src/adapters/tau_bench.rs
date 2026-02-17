use std::io::BufRead;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::BenchError;
use crate::suite::{BenchScore, BenchSuite, BenchTask, ConversationTurn, TaskSubmission};

/// Tau-bench task entry.
#[derive(Debug, Deserialize)]
struct TauBenchEntry {
    id: String,
    #[serde(default)]
    domain: String,
    instruction: String,
    #[serde(default)]
    user_persona: Option<String>,
    #[serde(default)]
    expected_state: Option<serde_json::Value>,
    #[serde(default)]
    expected_actions: Vec<String>,
    #[serde(default)]
    max_turns: Option<usize>,
}

/// Tau-bench: multi-turn tool-calling dialog benchmark.
///
/// Tests agent ability to handle customer service scenarios with simulated
/// domain APIs (retail, airline). Scoring compares final state against expected.
pub struct TauBenchSuite {
    dataset_path: PathBuf,
    domain: String,
}

impl TauBenchSuite {
    pub fn new(dataset_path: impl Into<PathBuf>, domain: impl Into<String>) -> Self {
        Self {
            dataset_path: dataset_path.into(),
            domain: domain.into(),
        }
    }
}

#[async_trait]
impl BenchSuite for TauBenchSuite {
    fn name(&self) -> &str {
        "Tau-bench"
    }

    fn id(&self) -> &str {
        "tau_bench"
    }

    async fn load_tasks(&self) -> Result<Vec<BenchTask>, BenchError> {
        let file = std::fs::File::open(&self.dataset_path)?;
        let reader = std::io::BufReader::new(file);
        let mut tasks = Vec::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: TauBenchEntry = serde_json::from_str(trimmed).map_err(|e| {
                BenchError::Config(format!("tau_bench line {}: {}", line_num + 1, e))
            })?;

            let domain = if entry.domain.is_empty() {
                self.domain.clone()
            } else {
                entry.domain.clone()
            };

            let metadata = serde_json::json!({
                "domain": domain,
                "user_persona": entry.user_persona,
                "expected_state": entry.expected_state,
                "expected_actions": entry.expected_actions,
            });

            tasks.push(BenchTask {
                id: entry.id,
                prompt: entry.instruction,
                context: entry.user_persona.clone(),
                resources: vec![],
                tags: vec![format!("domain-{domain}")],
                expected_turns: entry.max_turns,
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
        // Score based on expected actions completion
        let expected_actions: Vec<String> = task
            .metadata
            .get("expected_actions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if expected_actions.is_empty() {
            // No expected actions defined; score based on whether agent responded
            if submission.response.is_empty() {
                return Ok(BenchScore::fail("no response"));
            }
            return Ok(BenchScore::partial(
                0.5,
                "no expected_actions to evaluate against",
            ));
        }

        // Check which expected actions were actually called
        let called: std::collections::HashSet<&str> =
            submission.tool_calls.iter().map(|s| s.as_str()).collect();
        let matched = expected_actions
            .iter()
            .filter(|a| called.contains(a.as_str()))
            .count();

        let ratio = matched as f64 / expected_actions.len() as f64;
        if ratio >= 1.0 {
            Ok(BenchScore::pass())
        } else if ratio > 0.0 {
            Ok(BenchScore::partial(
                ratio,
                format!(
                    "{}/{} expected actions completed",
                    matched,
                    expected_actions.len()
                ),
            ))
        } else {
            Ok(BenchScore::fail(format!(
                "0/{} expected actions completed",
                expected_actions.len()
            )))
        }
    }

    async fn next_user_message(
        &self,
        task: &BenchTask,
        conversation: &[ConversationTurn],
    ) -> Result<Option<String>, BenchError> {
        // Check if we've exceeded max turns
        if let Some(max) = task.expected_turns {
            let user_turns = conversation
                .iter()
                .filter(|t| matches!(t.role, crate::suite::TurnRole::User))
                .count();
            if user_turns >= max {
                return Ok(None);
            }
        }

        // For now, multi-turn simulation requires an LLM (not implemented yet).
        // Return None to end after the first turn.
        // TODO: Use LLM to simulate customer based on user_persona.
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_tau_bench_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tau.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "t1", "instruction": "Return my order", "expected_actions": ["lookup_order", "process_return"], "max_turns": 3}}"#
        )
        .unwrap();

        let suite = TauBenchSuite::new(&path, "retail");
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].expected_turns, Some(3));
    }

    #[tokio::test]
    async fn test_tau_bench_scoring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tau.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "t1", "instruction": "Return order", "expected_actions": ["lookup_order", "process_return"]}}"#
        )
        .unwrap();

        let suite = TauBenchSuite::new(&path, "retail");
        let tasks = suite.load_tasks().await.unwrap();

        // Partial completion
        let submission = TaskSubmission {
            response: "I found your order.".to_string(),
            conversation: vec![],
            tool_calls: vec!["lookup_order".to_string()],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 0.5);
        assert_eq!(score.label, "partial");

        // Full completion
        let submission = TaskSubmission {
            response: "Return processed.".to_string(),
            conversation: vec![],
            tool_calls: vec!["lookup_order".to_string(), "process_return".to_string()],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 1.0);
    }
}
