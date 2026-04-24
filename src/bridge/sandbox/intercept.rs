//! Tool dispatch interception for the per-project sandbox.
//!
//! See [`maybe_intercept`].

use std::collections::HashMap;
use std::path::Path;

use ironclaw_engine::{MountError, ProjectId, WorkspaceMounts};
use serde_json::Value;
use tracing::debug;

/// Tool names that the sandbox **may** handle when their path argument
/// resolves into a workspace mount.
///
/// Used by [`maybe_intercept`] and by `EffectBridgeAdapter` to advertise
/// the sandbox-eligible tool surface. Keep in sync with the daemon's
/// registered tool list (see `src/bin/sandbox_daemon.rs`).
/// Includes both engine-v2 names (`file_read`/`file_write`) and the host's
/// actual v1 tool registry names (`read_file`/`write_file`) so the
/// interceptor catches calls regardless of which alias the agent uses. The
/// daemon also accepts both on the container side, keeping the pair fully
/// symmetric.
pub const SANDBOX_TOOL_NAMES: &[&str] = &[
    "file_read",
    "file_write",
    "read_file",
    "write_file",
    "list_dir",
    "apply_patch",
    "shell",
];

/// Outcome of a sandbox interception attempt.
///
/// `Handled` means the call was dispatched through a mount backend and the
/// included `String` is the JSON-pretty-serialized result, ready to slot
/// into the existing post-`execute_tool_with_safety` pipeline (sanitization,
/// `wrap_for_llm`, `ActionResult` construction).
///
/// `FellThrough` means the call was not eligible for sandbox dispatch and
/// the caller should run normal host-side tool execution. Reasons include:
/// no mount table configured, action name outside the sandbox set, no
/// recognizable path in params, path doesn't resolve to any mount, or the
/// matched backend returned [`MountError::Unsupported`].
#[derive(Debug)]
pub enum InterceptOutcome {
    Handled(String),
    FellThrough,
}

/// Try to handle a tool call via the per-project sandbox mount table.
///
/// Returns:
/// - `Ok(Handled(json))` — sandbox handled the call; `json` is the
///   pretty-serialized JSON tool output, matching what
///   `execute_tool_with_safety` would have returned.
/// - `Ok(FellThrough)` — sandbox declined; caller should run host execution.
/// - `Err(MountError)` — backend reported a real failure (NotFound,
///   InvalidPath, PermissionDenied, Tool, Backend). The caller converts
///   this into the appropriate engine error.
///
/// `Unsupported` errors from the backend are converted to `FellThrough` so
/// the bridge falls back to host execution gracefully — that's how the
/// `FilesystemBackend` Phase 1 stubs for `apply_patch` and `shell` keep
/// working without breaking the agent.
pub async fn maybe_intercept(
    action_name: &str,
    parameters: &Value,
    project_id: ProjectId,
    mounts: &WorkspaceMounts,
    thread_worktree_subdir: Option<&str>,
) -> Result<InterceptOutcome, MountError> {
    if !SANDBOX_TOOL_NAMES.contains(&action_name) {
        return Ok(InterceptOutcome::FellThrough);
    }

    let Some(mut path_str) = extract_path_param(action_name, parameters) else {
        debug!(
            action = action_name,
            "sandbox intercept: no path param, falling through"
        );
        return Ok(InterceptOutcome::FellThrough);
    };
    if !is_mountable_path(&path_str) {
        debug!(action = action_name, path = %path_str, "sandbox intercept: path not mountable, falling through");
        return Ok(InterceptOutcome::FellThrough);
    }

    // Per-thread worktree rewrite: if the thread has a `dev.worktree`
    // subdir in its metadata and the requested path is under `/project/`
    // but not already under `/project/worktrees/`, splice the worktree
    // subdir in so every tool call targets the thread's branch.
    //
    // The agent may still bypass this by targeting `/project/worktrees/...`
    // explicitly (e.g. to inspect a sibling thread's checkout) — the
    // rewrite only applies when the path would otherwise resolve to the
    // shared project root.
    if let Some(subdir) = thread_worktree_subdir {
        path_str = rewrite_path_for_worktree(&path_str, subdir);
    }

    let Some((backend, rel_path)) = mounts.resolve(project_id, &path_str).await? else {
        debug!(action = action_name, path = %path_str, "sandbox intercept: no mount matched, falling through");
        return Ok(InterceptOutcome::FellThrough);
    };

    debug!(action = action_name, path = %path_str, rel = %rel_path.display(), "sandbox intercept: routing to mount backend");

    let result = match action_name {
        "file_read" | "read_file" => match backend.read(&rel_path).await {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes).into_owned();
                serde_json::json!({
                    "path": path_str,
                    "content": content,
                    "size": bytes.len(),
                })
            }
            Err(MountError::Unsupported { .. }) => return Ok(InterceptOutcome::FellThrough),
            Err(e) => return Err(e),
        },
        "file_write" | "write_file" => {
            let content = match parameters.get("content").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => {
                    return Err(MountError::Tool {
                        reason: "file_write requires 'content' parameter".into(),
                    });
                }
            };
            match backend.write(&rel_path, content.as_bytes()).await {
                Ok(()) => serde_json::json!({
                    "path": path_str,
                    "bytes_written": content.len(),
                    "success": true,
                }),
                Err(MountError::Unsupported { .. }) => return Ok(InterceptOutcome::FellThrough),
                Err(e) => return Err(e),
            }
        }
        "list_dir" => {
            const MAX_DEPTH: usize = 10;
            let depth = parameters
                .get("depth")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(MAX_DEPTH as u64) as usize;
            match backend.list(&rel_path, depth).await {
                Ok(entries) => {
                    let entry_strings: Vec<String> = entries
                        .iter()
                        .map(|e| {
                            let suffix = match e.kind {
                                ironclaw_engine::workspace::EntryKind::Directory => "/",
                                _ => "",
                            };
                            format!("{}{}", e.path.display(), suffix)
                        })
                        .collect();
                    serde_json::json!({
                        "path": path_str,
                        "entries": entry_strings,
                        "count": entries.len(),
                        "truncated": false,
                    })
                }
                Err(MountError::Unsupported { .. }) => return Ok(InterceptOutcome::FellThrough),
                Err(e) => return Err(e),
            }
        }
        "apply_patch" => {
            let old_string = match parameters.get("old_string").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => {
                    return Err(MountError::Tool {
                        reason: "apply_patch requires 'old_string' parameter".into(),
                    });
                }
            };
            let new_string = match parameters.get("new_string").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => {
                    return Err(MountError::Tool {
                        reason: "apply_patch requires 'new_string' parameter".into(),
                    });
                }
            };
            let replace_all = parameters
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match backend
                .patch(&rel_path, old_string, new_string, replace_all)
                .await
            {
                Ok(()) => serde_json::json!({
                    "path": path_str,
                    "success": true,
                }),
                Err(MountError::Unsupported { .. }) => return Ok(InterceptOutcome::FellThrough),
                Err(e) => return Err(e),
            }
        }
        "shell" => {
            let command = parameters
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Pass through environment variables from the tool call so
            // `shell_exec(env={"FOO": "bar"})` works inside the sandbox.
            let mut env = HashMap::new();
            if let Some(env_obj) = parameters.get("env").and_then(|v| v.as_object()) {
                for (k, v) in env_obj {
                    if let Some(s) = v.as_str() {
                        env.insert(k.clone(), s.to_string());
                    }
                }
            }
            // shell may declare its workdir via the same path arg we already
            // resolved. Convert it to None when the workdir is the mount root.
            let cwd: Option<&Path> = if rel_path.as_os_str().is_empty() {
                None
            } else {
                Some(rel_path.as_path())
            };
            match backend.shell(command, env, cwd).await {
                Ok(out) => serde_json::json!({
                    "stdout": out.stdout,
                    "stderr": out.stderr,
                    "exit_code": out.exit_code,
                }),
                Err(MountError::Unsupported { .. }) => return Ok(InterceptOutcome::FellThrough),
                Err(e) => return Err(e),
            }
        }
        _ => return Ok(InterceptOutcome::FellThrough),
    };

    let serialized = serde_json::to_string_pretty(&result).map_err(|e| MountError::Backend {
        reason: format!("failed to serialize sandbox result: {e}"),
    })?;
    Ok(InterceptOutcome::Handled(serialized))
}

/// Extract the path argument for a sandbox tool, falling back to None when
/// the parameter shape doesn't carry one.
///
/// Shell has a subtle default: when the sandbox is enabled and the caller
/// doesn't supply a `workdir`, we default to `/project/` so the command
/// always runs inside the container. Without this default, `shell(command:
/// "git clone ...")` would fall through to host execution even though the
/// whole point of enabling the sandbox is to keep shell work off the host.
/// The agent can still override by passing an explicit `workdir` somewhere
/// under `/project/`.
fn extract_path_param(action_name: &str, params: &Value) -> Option<String> {
    match action_name {
        "file_read" | "read_file" | "file_write" | "write_file" | "list_dir" | "apply_patch" => {
            params
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from)
        }
        "shell" => params
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| Some("/project/".to_string())),
        _ => None,
    }
}

/// A path is mountable when it falls under a known agent-facing prefix.
/// Currently only `/project/` has registered mounts; extend this when
/// `/memory/` or `/home/` mounts are wired up.
///
/// Defense-in-depth: `WorkspaceMounts::resolve` also rejects unknown
/// prefixes, but this fast-path avoids the lock+lookup cost for paths
/// like `/etc/passwd` or `/Users/coder/notes.md` that the agent might
/// hallucinate.
fn is_mountable_path(path: &str) -> bool {
    // Only `/project/` mounts are registered today. When `/memory/` or
    // `/home/` mounts are wired up, extend this list.
    path.starts_with("/project/") || path == "/project"
}

/// Splice a worktree subdir into a `/project/...` path so per-thread
/// git worktrees are transparent to the agent.
///
/// Rules:
/// - `/project` (no trailing slash) → `/project/<subdir>`
/// - `/project/` → `/project/<subdir>/`
/// - `/project/foo/bar` → `/project/<subdir>/foo/bar`
/// - `/project/worktrees/...` → **unchanged** (explicit opt-out)
/// - Anything else → unchanged (only `/project/`-prefixed paths are rewritten)
///
/// The `subdir` caller ensures safety (no `..`, not absolute) via
/// [`super::workspace_path::thread_worktree_subdir`].
fn rewrite_path_for_worktree(path: &str, subdir: &str) -> String {
    if path == "/project" {
        return format!("/project/{subdir}");
    }
    let Some(rest) = path.strip_prefix("/project/") else {
        return path.to_string();
    };
    // Agent-initiated escape hatch: paths already under worktrees/ stay put.
    if rest.starts_with("worktrees/") || rest == "worktrees" {
        return path.to_string();
    }
    if rest.is_empty() {
        return format!("/project/{subdir}/");
    }
    format!("/project/{subdir}/{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_engine::workspace::{DirEntry, EntryKind, FilesystemBackend, ShellOutput};
    use ironclaw_engine::{MountBackend, ProjectMountFactory, ProjectMounts};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    // ── rewrite_path_for_worktree unit tests ─────────────────────
    // These gate the worktree-per-thread transparency: if these drift,
    // file_* calls start landing in the shared project root instead of
    // the thread's own branch checkout.
    #[test]
    fn rewrite_splices_worktree_into_project_paths() {
        assert_eq!(
            rewrite_path_for_worktree("/project/src/foo.rs", "worktrees/abc"),
            "/project/worktrees/abc/src/foo.rs",
        );
        assert_eq!(
            rewrite_path_for_worktree("/project/", "worktrees/abc"),
            "/project/worktrees/abc/",
        );
        assert_eq!(
            rewrite_path_for_worktree("/project", "worktrees/abc"),
            "/project/worktrees/abc",
        );
    }

    #[test]
    fn rewrite_leaves_worktrees_prefix_alone() {
        // Explicit /project/worktrees/* is the agent's escape hatch to
        // inspect siblings — must not double-splice.
        assert_eq!(
            rewrite_path_for_worktree("/project/worktrees/other/src/foo.rs", "worktrees/abc"),
            "/project/worktrees/other/src/foo.rs",
        );
        assert_eq!(
            rewrite_path_for_worktree("/project/worktrees", "worktrees/abc"),
            "/project/worktrees",
        );
    }

    #[test]
    fn rewrite_ignores_non_project_paths() {
        assert_eq!(
            rewrite_path_for_worktree("/etc/passwd", "worktrees/abc"),
            "/etc/passwd",
        );
        assert_eq!(rewrite_path_for_worktree("", "worktrees/abc"), "");
    }

    #[derive(Debug)]
    struct StaticFactory {
        root: PathBuf,
    }

    #[async_trait]
    impl ProjectMountFactory for StaticFactory {
        async fn build(&self, _: ProjectId) -> Result<ProjectMounts, MountError> {
            let mut mounts = ProjectMounts::new();
            mounts.add("/project/", Arc::new(FilesystemBackend::new(&self.root)));
            Ok(mounts)
        }
    }

    fn make_mounts() -> (WorkspaceMounts, ProjectId, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let factory = StaticFactory {
            root: dir.path().to_path_buf(),
        };
        (
            WorkspaceMounts::new(Arc::new(factory)),
            ProjectId::new(),
            dir,
        )
    }

    #[tokio::test]
    async fn write_and_read_through_intercept() {
        let (mounts, pid, _dir) = make_mounts();

        let write = serde_json::json!({"path": "/project/foo.txt", "content": "hello"});
        let outcome = maybe_intercept("file_write", &write, pid, &mounts, None)
            .await
            .unwrap();
        match outcome {
            InterceptOutcome::Handled(s) => {
                assert!(s.contains("\"bytes_written\": 5"));
                assert!(s.contains("\"path\": \"/project/foo.txt\""));
            }
            InterceptOutcome::FellThrough => panic!("expected Handled"),
        }

        let read = serde_json::json!({"path": "/project/foo.txt"});
        let outcome = maybe_intercept("file_read", &read, pid, &mounts, None)
            .await
            .unwrap();
        match outcome {
            InterceptOutcome::Handled(s) => {
                let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!(parsed["content"], "hello");
                assert_eq!(parsed["size"], 5);
            }
            InterceptOutcome::FellThrough => panic!("expected Handled"),
        }
    }

    #[tokio::test]
    async fn list_dir_through_intercept() {
        let (mounts, pid, dir) = make_mounts();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let params = serde_json::json!({"path": "/project/"});
        let outcome = maybe_intercept("list_dir", &params, pid, &mounts, None)
            .await
            .unwrap();
        match outcome {
            InterceptOutcome::Handled(s) => {
                let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
                let entries = parsed["entries"].as_array().unwrap();
                let names: Vec<String> = entries
                    .iter()
                    .map(|e| e.as_str().unwrap().to_string())
                    .collect();
                assert!(names.contains(&"a.txt".to_string()));
                assert!(names.contains(&"sub/".to_string()));
            }
            InterceptOutcome::FellThrough => panic!("expected Handled"),
        }
    }

    #[tokio::test]
    async fn host_path_falls_through() {
        let (mounts, pid, _dir) = make_mounts();
        // a path the agent might pass when not using /project/ — should not
        // resolve and the interception should fall through.
        let params = serde_json::json!({"path": "/Users/coder/notes.md"});
        let outcome = maybe_intercept("file_read", &params, pid, &mounts, None)
            .await
            .unwrap();
        assert!(matches!(outcome, InterceptOutcome::FellThrough));
    }

    #[tokio::test]
    async fn relative_path_falls_through() {
        let (mounts, pid, _dir) = make_mounts();
        let params = serde_json::json!({"path": "notes.md"});
        let outcome = maybe_intercept("file_read", &params, pid, &mounts, None)
            .await
            .unwrap();
        assert!(matches!(outcome, InterceptOutcome::FellThrough));
    }

    #[tokio::test]
    async fn non_sandbox_action_falls_through() {
        let (mounts, pid, _dir) = make_mounts();
        let params = serde_json::json!({"path": "/project/foo.txt"});
        let outcome = maybe_intercept("memory_read", &params, pid, &mounts, None)
            .await
            .unwrap();
        assert!(matches!(outcome, InterceptOutcome::FellThrough));
    }

    /// Regression test for the "shell escapes the sandbox" bug caught by the
    /// live e2e test. When the sandbox is enabled and the agent invokes
    /// `shell` without an explicit `workdir`, the call MUST still route
    /// through the project mount — otherwise commands like
    /// `git clone ... /project/repo` run on the host, hit permission denied
    /// on `/project/`, and the agent silently works around by using host
    /// paths. The fix defaults the shell workdir to `/project/`.
    ///
    /// Uses a counting backend that actually implements `shell` so the
    /// interception reaches the backend (unlike `FilesystemBackend` which
    /// returns `Unsupported` and falls through).
    #[tokio::test]
    async fn shell_without_workdir_routes_to_sandbox() {
        let counter = Arc::new(CountingBackend::default());
        let factory = CountingFactory {
            backend: Arc::clone(&counter),
        };
        let mounts = WorkspaceMounts::new(Arc::new(factory));
        let pid = ProjectId::new();

        let params = serde_json::json!({"command": "echo hi"});
        let outcome = maybe_intercept("shell", &params, pid, &mounts, None)
            .await
            .unwrap();
        assert!(
            matches!(outcome, InterceptOutcome::Handled(_)),
            "shell without workdir must be handled by the sandbox, not fall through"
        );
        assert_eq!(
            counter.shells.load(Ordering::Relaxed),
            1,
            "backend.shell() must be called exactly once"
        );
    }

    #[tokio::test]
    async fn unsupported_backend_op_falls_through() {
        // `FilesystemBackend::patch` still returns `Unsupported`; the
        // interceptor must map that to `FellThrough` so the bridge runs
        // the host apply_patch tool instead. `shell` is deliberately
        // *not* tested here anymore — it used to be `Unsupported` but
        // now runs via `/bin/sh -c` so it would succeed.
        let (mounts, pid, _dir) = make_mounts();
        let params = serde_json::json!({
            "path": "/project/foo.txt",
            "old_string": "a",
            "new_string": "b",
        });
        let outcome = maybe_intercept("apply_patch", &params, pid, &mounts, None)
            .await
            .unwrap();
        assert!(matches!(outcome, InterceptOutcome::FellThrough));
    }

    #[tokio::test]
    async fn missing_path_param_falls_through() {
        let (mounts, pid, _dir) = make_mounts();
        let params = serde_json::json!({"content": "hello"});
        let outcome = maybe_intercept("file_write", &params, pid, &mounts, None)
            .await
            .unwrap();
        assert!(matches!(outcome, InterceptOutcome::FellThrough));
    }

    #[tokio::test]
    async fn invalid_path_returns_error_not_falls_through() {
        // The path resolves to the /project/ mount but contains a `..`
        // escape — the backend rejects it as InvalidPath, which the
        // interceptor must surface as a real error rather than falling
        // through to host execution (host execution would silently allow
        // the escape).
        let (mounts, pid, _dir) = make_mounts();
        let params = serde_json::json!({"path": "/project/../etc/passwd"});
        let result = maybe_intercept("file_read", &params, pid, &mounts, None).await;
        assert!(matches!(result, Err(MountError::InvalidPath { .. })));
    }

    /// Counts how many times each backend method gets called. Lets tests
    /// verify that "intercept" actually dispatches into the backend (not
    /// just into the host tool registry).
    #[derive(Debug, Default)]
    struct CountingBackend {
        reads: AtomicUsize,
        writes: AtomicUsize,
        lists: AtomicUsize,
        shells: AtomicUsize,
    }

    #[async_trait]
    impl MountBackend for CountingBackend {
        async fn read(&self, _: &Path) -> Result<Vec<u8>, MountError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(b"counted".to_vec())
        }
        async fn write(&self, _: &Path, _: &[u8]) -> Result<(), MountError> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn list(&self, _: &Path, _: usize) -> Result<Vec<DirEntry>, MountError> {
            self.lists.fetch_add(1, Ordering::Relaxed);
            Ok(vec![DirEntry {
                path: PathBuf::from("a.txt"),
                kind: EntryKind::File,
                size: Some(1),
            }])
        }
        async fn patch(&self, _: &Path, _: &str, _: &str, _: bool) -> Result<(), MountError> {
            Err(MountError::Unsupported {
                operation: "patch".into(),
            })
        }
        async fn shell(
            &self,
            _: &str,
            _: HashMap<String, String>,
            _: Option<&Path>,
        ) -> Result<ShellOutput, MountError> {
            self.shells.fetch_add(1, Ordering::Relaxed);
            Ok(ShellOutput {
                stdout: "counted".into(),
                stderr: String::new(),
                exit_code: 0,
            })
        }
    }

    #[derive(Debug)]
    struct CountingFactory {
        backend: Arc<CountingBackend>,
    }

    #[async_trait]
    impl ProjectMountFactory for CountingFactory {
        async fn build(&self, _: ProjectId) -> Result<ProjectMounts, MountError> {
            let mut mounts = ProjectMounts::new();
            mounts.add(
                "/project/",
                Arc::clone(&self.backend) as Arc<dyn MountBackend>,
            );
            Ok(mounts)
        }
    }

    /// **Test through the caller, not just the helper.** This is the test
    /// that catches the bug-class described in `.claude/rules/testing.md`:
    /// if `maybe_intercept` decides "yes, this is a sandbox tool" but
    /// silently fails to actually call the backend (e.g. wrong key extraction,
    /// wrong dispatch arm, accidental clone instead of move), the
    /// `CountingBackend` records nothing and the assertion fails. This
    /// directly tests that the interception path reaches the backend, not
    /// just that the helper compiles or returns the right outcome variant.
    #[tokio::test]
    async fn intercept_actually_dispatches_into_backend() {
        let counter = Arc::new(CountingBackend::default());
        let factory = CountingFactory {
            backend: Arc::clone(&counter),
        };
        let mounts = WorkspaceMounts::new(Arc::new(factory));
        let pid = ProjectId::new();

        // file_read
        maybe_intercept(
            "file_read",
            &serde_json::json!({"path": "/project/foo.txt"}),
            pid,
            &mounts,
            None,
        )
        .await
        .unwrap();
        assert_eq!(counter.reads.load(Ordering::Relaxed), 1);

        // file_write
        maybe_intercept(
            "file_write",
            &serde_json::json!({"path": "/project/foo.txt", "content": "x"}),
            pid,
            &mounts,
            None,
        )
        .await
        .unwrap();
        assert_eq!(counter.writes.load(Ordering::Relaxed), 1);

        // list_dir
        maybe_intercept(
            "list_dir",
            &serde_json::json!({"path": "/project/"}),
            pid,
            &mounts,
            None,
        )
        .await
        .unwrap();
        assert_eq!(counter.lists.load(Ordering::Relaxed), 1);

        // apply_patch returns Unsupported → falls through. The patch counter
        // increments but the outcome must be FellThrough.
        let outcome = maybe_intercept(
            "apply_patch",
            &serde_json::json!({"path": "/project/foo.txt", "old_string": "x", "new_string": "y"}),
            pid,
            &mounts,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(outcome, InterceptOutcome::FellThrough));
    }
}
