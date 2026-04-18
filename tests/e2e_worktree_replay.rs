//! E2E trace test: replays imported Claude Code traces against a git worktree
//! checked out at the historical commit, verifying that IronClaw's tools
//! produce correct results against the same repo state.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// The source repo that the Claude Code sessions were recorded against.
    const SOURCE_REPO: &str = "/Users/coder/ironclaw";

    /// Create a git worktree at the given commit in a temp directory.
    /// Returns the worktree path. The worktree must be cleaned up by the caller.
    fn create_worktree(repo: &str, commit: &str) -> PathBuf {
        let worktree_dir = std::env::temp_dir().join(format!("ironclaw_trace_wt_{}", &commit[..8]));

        // Clean up any stale worktree from a previous failed run.
        if worktree_dir.exists() {
            let _ = Command::new("git")
                .args(["-C", repo, "worktree", "remove", "--force"])
                .arg(&worktree_dir)
                .output();
            let _ = std::fs::remove_dir_all(&worktree_dir);
        }

        let output = Command::new("git")
            .args(["-C", repo, "worktree", "add", "--detach"])
            .arg(&worktree_dir)
            .arg(commit)
            .output()
            .expect("failed to run git worktree add");

        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(
            worktree_dir.exists(),
            "worktree was not created at {}",
            worktree_dir.display()
        );

        worktree_dir
    }

    /// Remove a git worktree.
    fn remove_worktree(repo: &str, worktree_path: &Path) {
        let _ = Command::new("git")
            .args(["-C", repo, "worktree", "remove", "--force"])
            .arg(worktree_path)
            .output();
    }

    /// RAII guard that cleans up the worktree on drop.
    struct WorktreeGuard {
        repo: String,
        path: PathBuf,
    }

    impl Drop for WorktreeGuard {
        fn drop(&mut self) {
            remove_worktree(&self.repo, &self.path);
        }
    }

    /// Load a trace, resolve its repo metadata, create a worktree, and run it.
    async fn run_worktree_trace(fixture_name: &str) {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/llm_traces/imported")
            .join(fixture_name);

        let mut trace = LlmTrace::from_file(&fixture_path)
            .unwrap_or_else(|e| panic!("Failed to load {}: {}", fixture_name, e));

        // Extract commit from trace metadata.
        let commit = trace
            .repo
            .commit
            .as_deref()
            .expect("trace must have repo.commit for worktree replay");

        // Skip if the source repo isn't present (e.g. CI without this checkout).
        if !Path::new(SOURCE_REPO).join(".git").exists() {
            eprintln!("[skip] source repo {SOURCE_REPO} not found — worktree replay skipped");
            return;
        }

        // Skip if the commit isn't reachable in this repo.
        let commit_check = Command::new("git")
            .args(["-C", SOURCE_REPO, "cat-file", "-e"])
            .arg(commit)
            .output()
            .expect("failed to run git cat-file");
        if !commit_check.status.success() {
            eprintln!(
                "[skip] commit {commit} not found in {SOURCE_REPO} — worktree replay skipped"
            );
            return;
        }

        // Create worktree at the commit.
        let worktree_path = create_worktree(SOURCE_REPO, commit);
        let _guard = WorktreeGuard {
            repo: SOURCE_REPO.to_string(),
            path: worktree_path.clone(),
        };

        // Resolve {{repo_root}} placeholders to the worktree path.
        trace.resolve_repo_root(&worktree_path);

        // Build a test rig with tools sandboxed to the worktree.
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_working_dir(worktree_path)
            .build()
            .await;

        // Run all turns and verify expects.
        let all_responses = rig
            .run_and_verify_trace(&trace, Duration::from_secs(30))
            .await;

        for (i, responses) in all_responses.iter().enumerate() {
            assert!(
                !responses.is_empty(),
                "Turn {i}: expected at least one response"
            );
        }

        rig.shutdown();
    }

    /// Replay a trace that reads files and greps the transcription module.
    /// The worktree is checked out at the commit where these files existed.
    #[tokio::test]
    async fn worktree_read_and_grep() {
        run_worktree_trace("worktree_read_test.json").await;
    }
}
