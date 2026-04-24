//! TTL-bound cache for per-project git state surfaced in the conversation chrome.
//!
//! The gateway populates `ThreadInfo.project` by shelling out to `git` and
//! `gh` inside the project's `workspace_path`. Doing this synchronously on
//! every `/api/chat/threads` and `/api/chat/history` request would stall
//! the list view the first time `gh pr view` runs (cold network call to
//! GitHub), so this cache serves the last-known value immediately and
//! spawns a background refresh on stale/missing entries.
//!
//! All shell-outs go through `ToolDispatcher::dispatch("shell", ...)` —
//! the same path the agent would use — so they inherit the shell tool's
//! blocklist, fork-bomb detection, safe-env scrub, and audit trail.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::tools::dispatch::{DispatchSource, ToolDispatcher};
use ironclaw_engine::ProjectId;

/// The shape each shell probe returns into the cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedContext {
    /// Current git branch name, from `git rev-parse --abbrev-ref HEAD`.
    /// `None` until the first successful fetch.
    pub branch: Option<String>,
    /// Whether the working tree has uncommitted changes
    /// (`git status --porcelain` non-empty).
    pub dirty: Option<bool>,
    /// First ten lines of the porcelain output when dirty, for a chrome
    /// tooltip / summary. Truncated defensively to keep JSON payloads small.
    pub dirty_summary: Option<String>,
    /// The PR (if any) whose head ref is the current branch, from
    /// `gh pr view --json number,title,url,state`. `None` when no PR, when
    /// `gh` is unavailable, or when the fetch has not run yet.
    pub pr: Option<PrSummary>,

    /// Instants of the last successful refresh per field, used for
    /// independent per-field TTLs. Skipping serialization keeps the DTO
    /// clean for any debug dump of the cache.
    #[serde(skip)]
    branch_fetched_at: Option<Instant>,
    #[serde(skip)]
    dirty_fetched_at: Option<Instant>,
    #[serde(skip)]
    pr_fetched_at: Option<Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSummary {
    pub number: u32,
    pub title: String,
    pub url: String,
    pub state: PrState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

const BRANCH_TTL: Duration = Duration::from_secs(5);
const DIRTY_TTL: Duration = Duration::from_secs(5);
const PR_TTL: Duration = Duration::from_secs(60);
const DIRTY_SUMMARY_MAX_BYTES: usize = 2048;

/// TTL'd cache of per-project git state.
///
/// Cloning an `Arc<ProjectContextCache>` is cheap; handlers and the skill
/// injector share one instance via [`crate::channels::web::platform::state::GatewayState`].
pub struct ProjectContextCache {
    dispatcher: Arc<ToolDispatcher>,
    entries: RwLock<HashMap<ProjectId, CachedContext>>,
}

impl ProjectContextCache {
    pub fn new(dispatcher: Arc<ToolDispatcher>) -> Self {
        Self {
            dispatcher,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Return the last-known context for `project_id` and, when any field
    /// is stale or missing, fire-and-forget a background refresh. The
    /// returned value is always the *cached* snapshot at call time —
    /// callers never wait on a network I/O, which keeps the list endpoint
    /// responsive even on a cold cache or a slow `gh`.
    pub async fn get(
        self: &Arc<Self>,
        project_id: ProjectId,
        workspace_path: &Path,
        user_id: &str,
    ) -> CachedContext {
        let snapshot = {
            let guard = self.entries.read().await;
            guard.get(&project_id).cloned().unwrap_or_default()
        };

        self.spawn_stale_refreshes(project_id, workspace_path, user_id, &snapshot);
        snapshot
    }

    fn spawn_stale_refreshes(
        self: &Arc<Self>,
        project_id: ProjectId,
        workspace_path: &Path,
        user_id: &str,
        snapshot: &CachedContext,
    ) {
        let now = Instant::now();
        let workspace_path = workspace_path.to_path_buf();

        if is_stale(snapshot.branch_fetched_at, BRANCH_TTL, now) {
            let this = Arc::clone(self);
            let user = user_id.to_string();
            let wp = workspace_path.clone();
            tokio::spawn(async move {
                let _ = this.refresh_branch(project_id, &wp, &user).await;
            });
        }
        if is_stale(snapshot.dirty_fetched_at, DIRTY_TTL, now) {
            let this = Arc::clone(self);
            let user = user_id.to_string();
            let wp = workspace_path.clone();
            tokio::spawn(async move {
                let _ = this.refresh_dirty(project_id, &wp, &user).await;
            });
        }
        if is_stale(snapshot.pr_fetched_at, PR_TTL, now) {
            let this = Arc::clone(self);
            let user = user_id.to_string();
            let wp = workspace_path;
            tokio::spawn(async move {
                let _ = this.refresh_pr(project_id, &wp, &user).await;
            });
        }
    }

    async fn run_shell(
        &self,
        user_id: &str,
        workspace_path: &Path,
        command: &str,
    ) -> Option<ShellResult> {
        let params = serde_json::json!({
            "command": command,
            "workdir": workspace_path.display().to_string(),
            // The shell tool enforces its own timeout; this value is just a
            // generous upper bound so a hung subprocess doesn't hold the
            // refresh task indefinitely.
            "timeout": 15u64,
        });
        let output = self
            .dispatcher
            .dispatch(
                "shell",
                params,
                user_id,
                DispatchSource::Channel("gateway_project_ctx".into()),
            )
            .await
            .ok()?;
        parse_shell_output(&output.result)
    }

    async fn refresh_branch(
        self: Arc<Self>,
        project_id: ProjectId,
        workspace_path: &Path,
        user_id: &str,
    ) -> Option<()> {
        let res = self
            .run_shell(user_id, workspace_path, "git rev-parse --abbrev-ref HEAD")
            .await?;
        let branch = if res.exit_code == 0 {
            let name = res.stdout.trim();
            if name.is_empty() || name == "HEAD" {
                None
            } else {
                Some(name.to_string())
            }
        } else {
            None
        };

        let mut guard = self.entries.write().await;
        let entry = guard.entry(project_id).or_default();
        entry.branch = branch;
        entry.branch_fetched_at = Some(Instant::now());
        Some(())
    }

    async fn refresh_dirty(
        self: Arc<Self>,
        project_id: ProjectId,
        workspace_path: &Path,
        user_id: &str,
    ) -> Option<()> {
        let res = self
            .run_shell(user_id, workspace_path, "git status --porcelain")
            .await?;
        let (dirty, summary) = if res.exit_code == 0 {
            let stdout = res.stdout;
            let is_dirty = stdout.lines().any(|l| !l.trim().is_empty());
            let summary = if is_dirty {
                let trimmed: String = stdout.lines().take(10).collect::<Vec<_>>().join("\n");
                Some(truncate_bytes(&trimmed, DIRTY_SUMMARY_MAX_BYTES))
            } else {
                None
            };
            (Some(is_dirty), summary)
        } else {
            (None, None)
        };

        let mut guard = self.entries.write().await;
        let entry = guard.entry(project_id).or_default();
        entry.dirty = dirty;
        entry.dirty_summary = summary;
        entry.dirty_fetched_at = Some(Instant::now());
        Some(())
    }

    async fn refresh_pr(
        self: Arc<Self>,
        project_id: ProjectId,
        workspace_path: &Path,
        user_id: &str,
    ) -> Option<()> {
        let res = self
            .run_shell(
                user_id,
                workspace_path,
                "gh pr view --json number,title,url,state",
            )
            .await?;

        // Non-zero exit from `gh pr view` usually just means "no PR for the
        // current branch" — treat as "no PR", not as an error. The chrome
        // will simply omit the PR chip.
        let pr = if res.exit_code == 0 {
            parse_gh_pr(&res.stdout)
        } else {
            None
        };

        let mut guard = self.entries.write().await;
        let entry = guard.entry(project_id).or_default();
        entry.pr = pr;
        entry.pr_fetched_at = Some(Instant::now());
        Some(())
    }

    /// Clear a project's cache entry — used when `project_update` or
    /// `project_delete` mutates a project so the next read does not serve
    /// stale metadata for a moved `workspace_path`.
    pub async fn invalidate(&self, project_id: ProjectId) {
        self.entries.write().await.remove(&project_id);
    }
}

#[derive(Debug, Clone)]
struct ShellResult {
    stdout: String,
    exit_code: i32,
}

fn parse_shell_output(value: &serde_json::Value) -> Option<ShellResult> {
    // The shell tool returns a structured JSON object with stdout/stderr/
    // exit_code; older builds may have returned a string. Accept either
    // shape defensively — the cache should not panic on a shape drift.
    if let Some(obj) = value.as_object() {
        let stdout = obj
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let exit_code = obj.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        return Some(ShellResult { stdout, exit_code });
    }
    if let Some(s) = value.as_str() {
        return Some(ShellResult {
            stdout: s.to_string(),
            exit_code: 0,
        });
    }
    None
}

fn parse_gh_pr(stdout: &str) -> Option<PrSummary> {
    let raw = stdout.trim();
    if raw.is_empty() {
        return None;
    }
    #[derive(Deserialize)]
    struct GhPrRow {
        number: u32,
        title: String,
        url: String,
        state: String,
        #[serde(default)]
        is_draft: bool,
    }
    let row: GhPrRow = serde_json::from_str(raw).ok()?;
    // `gh` emits `state` as "OPEN"/"MERGED"/"CLOSED" and draft status as a
    // separate flag; normalize to our own enum so the chrome can pick a
    // colour without having to know the wire format.
    let state = match row.state.as_str() {
        "OPEN" if row.is_draft => PrState::Draft,
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    };
    Some(PrSummary {
        number: row.number,
        title: row.title,
        url: row.url,
        state,
    })
}

fn is_stale(last: Option<Instant>, ttl: Duration, now: Instant) -> bool {
    match last {
        Some(t) => now.duration_since(t) >= ttl,
        None => true,
    }
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Round down to the nearest char boundary so slicing does not panic on
    // multi-byte sequences (per `.claude/rules/review-discipline.md` § UTF-8
    // string safety).
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 3);
    out.push_str(&s[..end]);
    out.push('…');
    out
}

// `PathBuf` import silences clippy when the type is only named in arg
// positions elsewhere but not obviously used. Keeping the explicit import
// avoids a future refactor reintroducing a warning.
#[allow(dead_code)]
type _KeepPathBuf = PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gh_pr_open() {
        let json =
            r#"{"number": 42, "title": "t", "url": "u", "state": "OPEN", "is_draft": false}"#;
        let pr = parse_gh_pr(json).unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.state, PrState::Open);
    }

    #[test]
    fn parses_gh_pr_draft() {
        let json = r#"{"number": 7, "title": "t", "url": "u", "state": "OPEN", "is_draft": true}"#;
        let pr = parse_gh_pr(json).unwrap();
        assert_eq!(pr.state, PrState::Draft);
    }

    #[test]
    fn parses_gh_pr_merged() {
        let json = r#"{"number": 1, "title": "t", "url": "u", "state": "MERGED"}"#;
        let pr = parse_gh_pr(json).unwrap();
        assert_eq!(pr.state, PrState::Merged);
    }

    #[test]
    fn empty_gh_output_is_no_pr() {
        assert!(parse_gh_pr("").is_none());
        assert!(parse_gh_pr("   ").is_none());
    }

    #[test]
    fn parses_shell_output_obj() {
        let v = serde_json::json!({"stdout": "main\n", "exit_code": 0});
        let r = parse_shell_output(&v).unwrap();
        assert_eq!(r.stdout, "main\n");
        assert_eq!(r.exit_code, 0);
    }

    #[test]
    fn parses_shell_output_string_fallback() {
        let v = serde_json::Value::String("main".into());
        let r = parse_shell_output(&v).unwrap();
        assert_eq!(r.stdout, "main");
        assert_eq!(r.exit_code, 0);
    }

    #[test]
    fn staleness_initial_miss() {
        assert!(is_stale(None, Duration::from_secs(5), Instant::now()));
    }

    #[test]
    fn truncate_bytes_handles_multibyte() {
        // 'é' is two bytes in UTF-8; truncating at an odd byte boundary must
        // not panic.
        let s = "aééééé";
        let out = truncate_bytes(s, 3);
        assert!(out.ends_with('…'));
    }
}
