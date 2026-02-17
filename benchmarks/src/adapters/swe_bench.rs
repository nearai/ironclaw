use std::io::BufRead;
use std::path::PathBuf;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;

use crate::error::BenchError;
use crate::suite::{BenchScore, BenchSuite, BenchTask, TaskSubmission};

/// Validate that a string is safe for use as a filesystem path component.
/// Allows alphanumerics, hyphens, underscores, dots, and forward slashes (for nested paths).
fn is_safe_path_component(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Validate that a repo string matches the expected `owner/repo` GitHub format.
fn is_valid_github_repo(repo: &str) -> bool {
    // Match "owner/repo" where both parts are alphanumeric with hyphens/underscores/dots
    static REPO_PATTERN: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$").unwrap());
    REPO_PATTERN.is_match(repo)
}

/// Validate that a string looks like a git ref (hex SHA or valid ref name).
fn is_valid_git_ref(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        && !s.contains("..")
}

/// SWE-bench dataset entry.
#[derive(Debug, Deserialize)]
struct SweBenchEntry {
    instance_id: String,
    repo: String,
    base_commit: String,
    #[serde(default)]
    problem_statement: String,
    #[serde(default)]
    hints_text: Option<String>,
    #[serde(default)]
    test_patch: Option<String>,
    #[serde(default)]
    patch: Option<String>,
}

/// SWE-bench Pro: real-world software engineering tasks.
///
/// Each task clones a repo at a specific commit, presents the problem statement,
/// and expects the agent to produce a patch. Scoring runs the test suite.
pub struct SweBenchSuite {
    dataset_path: PathBuf,
    workspace_dir: PathBuf,
    use_docker: bool,
}

impl SweBenchSuite {
    pub fn new(
        dataset_path: impl Into<PathBuf>,
        workspace_dir: impl Into<PathBuf>,
        use_docker: bool,
    ) -> Self {
        Self {
            dataset_path: dataset_path.into(),
            workspace_dir: workspace_dir.into(),
            use_docker,
        }
    }
}

#[async_trait]
impl BenchSuite for SweBenchSuite {
    fn name(&self) -> &str {
        "SWE-bench Pro"
    }

    fn id(&self) -> &str {
        "swe_bench"
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
            let entry: SweBenchEntry = serde_json::from_str(trimmed).map_err(|e| {
                BenchError::Config(format!("swe_bench line {}: {}", line_num + 1, e))
            })?;

            if !is_safe_path_component(&entry.instance_id) {
                return Err(BenchError::Config(format!(
                    "swe_bench line {}: unsafe instance_id \"{}\"",
                    line_num + 1,
                    entry.instance_id,
                )));
            }
            if !is_valid_github_repo(&entry.repo) {
                return Err(BenchError::Config(format!(
                    "swe_bench line {}: invalid repo format \"{}\"",
                    line_num + 1,
                    entry.repo,
                )));
            }
            if !is_valid_git_ref(&entry.base_commit) {
                return Err(BenchError::Config(format!(
                    "swe_bench line {}: invalid base_commit \"{}\"",
                    line_num + 1,
                    entry.base_commit,
                )));
            }

            let metadata = serde_json::json!({
                "repo": entry.repo,
                "base_commit": entry.base_commit,
                "test_patch": entry.test_patch,
                "gold_patch": entry.patch,
                "use_docker": self.use_docker,
                "workspace_dir": self.workspace_dir.to_string_lossy(),
            });

            let prompt = if let Some(ref hints) = entry.hints_text {
                format!("{}\n\nHints:\n{}", entry.problem_statement, hints)
            } else {
                entry.problem_statement
            };

            tasks.push(BenchTask {
                id: entry.instance_id,
                prompt,
                context: Some(format!(
                    "Repository: {}, Commit: {}",
                    entry.repo, entry.base_commit
                )),
                resources: vec![],
                tags: vec![format!("repo-{}", entry.repo.replace('/', "-"))],
                expected_turns: None,
                timeout: None,
                metadata,
            });
        }

        Ok(tasks)
    }

    async fn setup_task(&self, task: &BenchTask) -> Result<(), BenchError> {
        let repo = task
            .metadata
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BenchError::TaskFailed {
                task_id: task.id.clone(),
                reason: "missing repo in metadata".to_string(),
            })?;
        let base_commit = task
            .metadata
            .get("base_commit")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BenchError::TaskFailed {
                task_id: task.id.clone(),
                reason: "missing base_commit in metadata".to_string(),
            })?;

        let task_dir = self.workspace_dir.join(&task.id);

        // Clone repo if not already present
        if !task_dir.exists() {
            let repo_url = format!("https://github.com/{}.git", repo);
            let output = tokio::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    &repo_url,
                    &task_dir.to_string_lossy(),
                ])
                .output()
                .await
                .map_err(|e| BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git clone failed: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git clone failed: {stderr}"),
                });
            }
        }

        // Checkout the base commit
        let output = tokio::process::Command::new("git")
            .args(["checkout", base_commit])
            .current_dir(&task_dir)
            .output()
            .await
            .map_err(|e| BenchError::TaskFailed {
                task_id: task.id.clone(),
                reason: format!("git checkout failed: {e}"),
            })?;

        if !output.status.success() {
            // Shallow clone might not have the commit; fetch more history
            let _ = tokio::process::Command::new("git")
                .args(["fetch", "--unshallow"])
                .current_dir(&task_dir)
                .output()
                .await;

            let output = tokio::process::Command::new("git")
                .args(["checkout", base_commit])
                .current_dir(&task_dir)
                .output()
                .await
                .map_err(|e| BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git checkout retry failed: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git checkout failed: {stderr}"),
                });
            }
        }

        Ok(())
    }

    async fn teardown_task(&self, task: &BenchTask) -> Result<(), BenchError> {
        let task_dir = self.workspace_dir.join(&task.id);
        if task_dir.exists() {
            // Reset any changes
            let _ = tokio::process::Command::new("git")
                .args(["checkout", "."])
                .current_dir(&task_dir)
                .output()
                .await;
            let _ = tokio::process::Command::new("git")
                .args(["clean", "-fdx"])
                .current_dir(&task_dir)
                .output()
                .await;
        }
        Ok(())
    }

    async fn score(
        &self,
        task: &BenchTask,
        submission: &TaskSubmission,
    ) -> Result<BenchScore, BenchError> {
        // For SWE-bench, scoring requires running the test patch against the agent's changes.
        // This is a simplified version that checks if the agent produced any code changes.

        let test_patch = task.metadata.get("test_patch").and_then(|v| v.as_str());

        if submission.response.is_empty() {
            return Ok(BenchScore::fail("no response from agent"));
        }

        // If we have a test patch, try to verify the submission
        if let Some(_test_patch) = test_patch {
            // TODO: Apply agent's patch, then apply test patch, then run tests.
            // For now, give partial credit if the agent produced some output.
            tracing::warn!(
                task_id = %task.id,
                "SWE-bench test execution not implemented, returning placeholder 0.25"
            );
            Ok(BenchScore::partial(
                0.25,
                "test execution not yet implemented; partial credit for response",
            ))
        } else {
            tracing::warn!(
                task_id = %task.id,
                "no test_patch available, returning placeholder 0.25"
            );
            Ok(BenchScore::partial(
                0.25,
                "no test_patch available for automated scoring",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_swe_bench_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "django__django-12345", "repo": "django/django", "base_commit": "abc123", "problem_statement": "Fix the ORM bug"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, "/tmp/swe-test", false);
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "django__django-12345");
        assert!(tasks[0].tags.contains(&"repo-django-django".to_string()));
    }

    #[tokio::test]
    async fn test_swe_bench_scoring_no_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "s1", "repo": "org/repo", "base_commit": "abc", "problem_statement": "Fix bug"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, "/tmp/swe-test", false);
        let tasks = suite.load_tasks().await.unwrap();

        let submission = TaskSubmission {
            response: String::new(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 0.0);
    }

    #[test]
    fn test_is_safe_path_component() {
        assert!(is_safe_path_component("django__django-12345"));
        assert!(is_safe_path_component("org/repo"));
        assert!(is_safe_path_component("abc123"));
        assert!(!is_safe_path_component(""));
        assert!(!is_safe_path_component("../../etc/passwd"));
        assert!(!is_safe_path_component("foo;rm -rf /"));
        assert!(!is_safe_path_component("foo bar"));
    }

    #[test]
    fn test_is_valid_github_repo() {
        assert!(is_valid_github_repo("django/django"));
        assert!(is_valid_github_repo("org/repo-name"));
        assert!(is_valid_github_repo("Org.Name/Repo_v2"));
        assert!(!is_valid_github_repo(""));
        assert!(!is_valid_github_repo("no-slash"));
        assert!(!is_valid_github_repo("too/many/slashes"));
        assert!(!is_valid_github_repo("spa ce/repo"));
    }

    #[test]
    fn test_is_valid_git_ref() {
        assert!(is_valid_git_ref("abc123"));
        assert!(is_valid_git_ref("deadbeef0123456789abcdef0123456789abcdef"));
        assert!(is_valid_git_ref("v1.2.3"));
        assert!(is_valid_git_ref("main"));
        assert!(!is_valid_git_ref(""));
        assert!(!is_valid_git_ref("bad..ref"));
        assert!(!is_valid_git_ref("has space"));
        assert!(!is_valid_git_ref("semi;colon"));
    }

    #[tokio::test]
    async fn test_swe_bench_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "../../etc/passwd", "repo": "org/repo", "base_commit": "abc", "problem_statement": "evil"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, "/tmp/swe-test", false);
        let err = suite.load_tasks().await.unwrap_err();
        assert!(err.to_string().contains("unsafe instance_id"));
    }

    #[tokio::test]
    async fn test_swe_bench_rejects_bad_repo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "task1", "repo": "not-a-repo-format", "base_commit": "abc", "problem_statement": "bad"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, "/tmp/swe-test", false);
        let err = suite.load_tasks().await.unwrap_err();
        assert!(err.to_string().contains("invalid repo format"));
    }
}
