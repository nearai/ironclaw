use std::io::BufRead;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::BenchError;
use crate::scoring;
use crate::suite::{BenchScore, BenchSuite, BenchTask, TaskResource, TaskSubmission};

/// GAIA dataset entry (Hugging Face JSONL format).
#[derive(Debug, Deserialize)]
struct GaiaEntry {
    task_id: String,
    #[serde(alias = "Question")]
    question: String,
    #[serde(alias = "Final answer", alias = "final_answer")]
    final_answer: String,
    #[serde(alias = "Level", default)]
    level: Option<u32>,
    #[serde(alias = "file_name", default)]
    file_name: Option<String>,
    #[serde(alias = "Annotator Metadata", default)]
    annotator_metadata: Option<serde_json::Value>,
}

/// GAIA benchmark suite.
///
/// Tasks are loaded from HuggingFace JSONL exports. Scoring uses normalized
/// exact match against the `final_answer` field.
pub struct GaiaSuite {
    dataset_path: PathBuf,
    attachments_dir: Option<PathBuf>,
}

impl GaiaSuite {
    pub fn new(
        dataset_path: impl Into<PathBuf>,
        attachments_dir: Option<impl Into<PathBuf>>,
    ) -> Self {
        Self {
            dataset_path: dataset_path.into(),
            attachments_dir: attachments_dir.map(|d| d.into()),
        }
    }
}

#[async_trait]
impl BenchSuite for GaiaSuite {
    fn name(&self) -> &str {
        "GAIA"
    }

    fn id(&self) -> &str {
        "gaia"
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
            let entry: GaiaEntry = serde_json::from_str(trimmed)
                .map_err(|e| BenchError::Config(format!("GAIA line {}: {}", line_num + 1, e)))?;

            let mut resources = Vec::new();
            if let Some(ref file_name) = entry.file_name {
                if !file_name.is_empty() {
                    if let Some(ref dir) = self.attachments_dir {
                        resources.push(TaskResource {
                            name: file_name.clone(),
                            path: dir.join(file_name).to_string_lossy().to_string(),
                            resource_type: crate::suite::ResourceType::File,
                        });
                    }
                }
            }

            let mut tags = Vec::new();
            if let Some(level) = entry.level {
                tags.push(format!("level-{level}"));
            }

            let metadata = serde_json::json!({
                "expected": entry.final_answer,
                "level": entry.level,
            });

            tasks.push(BenchTask {
                id: entry.task_id,
                prompt: entry.question,
                context: None,
                resources,
                tags,
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
        let expected = task
            .metadata
            .get("expected")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BenchError::Scoring {
                task_id: task.id.clone(),
                reason: "missing expected answer in metadata".to_string(),
            })?;

        Ok(scoring::exact_match(expected, &submission.response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_gaia_load_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gaia.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"task_id": "g1", "question": "What is the capital of France?", "final_answer": "Paris", "Level": 1}}"#
        )
        .unwrap();

        let suite = GaiaSuite::new(&path, None::<PathBuf>);
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "g1");
        assert!(tasks[0].tags.contains(&"level-1".to_string()));
    }

    #[tokio::test]
    async fn test_gaia_scoring() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gaia.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"task_id": "g1", "question": "Capital of France?", "final_answer": "Paris"}}"#
        )
        .unwrap();

        let suite = GaiaSuite::new(&path, None::<PathBuf>);
        let tasks = suite.load_tasks().await.unwrap();

        // Exact match (case insensitive)
        let submission = TaskSubmission {
            response: "paris".to_string(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 1.0);

        // Wrong answer
        let submission = TaskSubmission {
            response: "London".to_string(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 0.0);
    }
}
