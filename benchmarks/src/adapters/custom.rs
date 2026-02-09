use std::io::BufRead;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::BenchError;
use crate::scoring;
use crate::suite::{BenchScore, BenchSuite, BenchTask, TaskSubmission};

/// A single entry in the custom JSONL format.
#[derive(Debug, Deserialize)]
struct CustomEntry {
    id: String,
    prompt: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    expected: Option<String>,
    #[serde(default)]
    expected_contains: Option<String>,
    #[serde(default)]
    expected_regex: Option<String>,
    /// "exact", "contains", "regex", or "llm" (default: "exact")
    #[serde(default = "default_scorer")]
    scorer: String,
}

fn default_scorer() -> String {
    "exact".to_string()
}

/// Custom JSONL benchmark suite.
///
/// Each line of the JSONL file is a task with `id`, `prompt`, and scoring
/// criteria (`expected`, `expected_contains`, `expected_regex`).
pub struct CustomSuite {
    dataset_path: PathBuf,
}

impl CustomSuite {
    pub fn new(dataset_path: impl Into<PathBuf>) -> Self {
        Self {
            dataset_path: dataset_path.into(),
        }
    }
}

#[async_trait]
impl BenchSuite for CustomSuite {
    fn name(&self) -> &str {
        "Custom JSONL"
    }

    fn id(&self) -> &str {
        "custom"
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
            let entry: CustomEntry = serde_json::from_str(trimmed)
                .map_err(|e| BenchError::Config(format!("line {}: {}", line_num + 1, e)))?;

            let mut metadata = serde_json::json!({
                "scorer": entry.scorer,
            });
            if let Some(ref expected) = entry.expected {
                metadata["expected"] = serde_json::Value::String(expected.clone());
            }
            if let Some(ref expected_contains) = entry.expected_contains {
                metadata["expected_contains"] =
                    serde_json::Value::String(expected_contains.clone());
            }
            if let Some(ref expected_regex) = entry.expected_regex {
                metadata["expected_regex"] = serde_json::Value::String(expected_regex.clone());
            }

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
        let scorer = task
            .metadata
            .get("scorer")
            .and_then(|v| v.as_str())
            .unwrap_or("exact");

        match scorer {
            "exact" => {
                if let Some(expected) = task.metadata.get("expected").and_then(|v| v.as_str()) {
                    Ok(scoring::exact_match(expected, &submission.response))
                } else {
                    Err(BenchError::Scoring {
                        task_id: task.id.clone(),
                        reason: "no 'expected' field for exact scoring".to_string(),
                    })
                }
            }
            "contains" => {
                if let Some(expected) = task
                    .metadata
                    .get("expected_contains")
                    .and_then(|v| v.as_str())
                {
                    Ok(scoring::contains_match(expected, &submission.response))
                } else {
                    Err(BenchError::Scoring {
                        task_id: task.id.clone(),
                        reason: "no 'expected_contains' field for contains scoring".to_string(),
                    })
                }
            }
            "regex" => {
                if let Some(pattern) = task.metadata.get("expected_regex").and_then(|v| v.as_str())
                {
                    Ok(scoring::regex_match(pattern, &submission.response))
                } else {
                    Err(BenchError::Scoring {
                        task_id: task.id.clone(),
                        reason: "no 'expected_regex' field for regex scoring".to_string(),
                    })
                }
            }
            "llm" => {
                // TODO: LLM-as-judge scoring
                Ok(BenchScore::partial(0.5, "LLM scoring not yet implemented"))
            }
            other => Err(BenchError::Scoring {
                task_id: task.id.clone(),
                reason: format!("unknown scorer: {other}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_custom_load_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "t1", "prompt": "What is 2+2?", "expected": "4"}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"id": "t2", "prompt": "Say hello", "expected_contains": "hello", "scorer": "contains"}}"#
        )
        .unwrap();

        let suite = CustomSuite::new(&path);
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "t1");
        assert_eq!(tasks[1].id, "t2");
    }

    #[tokio::test]
    async fn test_custom_exact_scoring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "t1", "prompt": "What is 2+2?", "expected": "4"}}"#
        )
        .unwrap();

        let suite = CustomSuite::new(&path);
        let tasks = suite.load_tasks().await.unwrap();

        let submission = TaskSubmission {
            response: "4".to_string(),
            conversation: vec![],
            tool_calls: vec![],
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 1.0);
        assert_eq!(score.label, "pass");
    }

    #[tokio::test]
    async fn test_custom_contains_scoring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"id": "t1", "prompt": "Greet me", "expected_contains": "hello", "scorer": "contains"}}"#
        )
        .unwrap();

        let suite = CustomSuite::new(&path);
        let tasks = suite.load_tasks().await.unwrap();

        let submission = TaskSubmission {
            response: "Hello there!".to_string(),
            conversation: vec![],
            tool_calls: vec![],
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 1.0);
    }
}
