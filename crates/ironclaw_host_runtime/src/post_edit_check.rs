//! Operator-configured post-edit check for the coding tools.
//!
//! After a successful `builtin.write_file` / `builtin.apply_patch` dispatch,
//! host runtime can run one operator-configured shell command (for example
//! `cargo check --message-format=short 2>&1`) through the invocation's
//! [`RuntimeProcessPort`](crate::RuntimeProcessPort) and append only the
//! *new* diagnostic lines to the edit's model-visible output. This catches
//! the classic agent failure of breaking adjacent code without noticing,
//! without re-reporting the same diagnostics after every edit.
//!
//! The check is advisory: it never fails the edit (the edit already
//! succeeded), and a check that cannot run at all is only logged at debug.
//!
//! Configuration is resolved once at composition time via
//! [`PostEditCheckConfig::from_env`] (the composition layer calls this
//! module-owned factory; nothing here reads the environment per call) and is
//! threaded through `HostRuntimeServices::with_post_edit_check` into
//! [`InvocationServices`](crate::InvocationServices). Because the edit plans
//! never declare a process effect, the invocation-services resolver only
//! populates `InvocationServices::post_edit_check` when the effective
//! process policy permits local host execution (`ProcessBackendKind::
//! LocalHost` under `DeploymentMode::LocalSingleUser`); under
//! `ProcessBackendKind::None` or sandbox-backed policies the advisory check
//! is disabled instead of bypassing process-backend selection. A check that
//! does run is accounted as one spawned process, like `builtin.shell`.
//!
//! v1 limitation (kept deliberately simple): the seen-line registry is
//! global per scope — editing a file again does not clear previously
//! reported findings for that file, so a diagnostic that disappears and
//! later reappears unchanged is not re-reported until its line is evicted
//! from the bounded registry.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
    time::Duration,
};

use ironclaw_host_api::{MountView, ResourceScope};
use serde_json::{Value, json};

use crate::{CommandExecutionRequest, RuntimeProcessError, RuntimeProcessPort};

/// Shell command run after successful edits. Feature is OFF when unset/empty.
pub const POST_EDIT_CHECK_ENV: &str = "IRONCLAW_POST_EDIT_CHECK";
/// Check timeout in whole seconds. Defaults to [`DEFAULT_TIMEOUT_SECS`].
pub const POST_EDIT_CHECK_TIMEOUT_ENV: &str = "IRONCLAW_POST_EDIT_CHECK_TIMEOUT_SECS";

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Cap on remembered diagnostic lines per scope. FIFO eviction beyond this;
/// an evicted line may be re-reported by a later check.
const MAX_SEEN_LINES_PER_SCOPE: usize = 500;
/// Cap on tracked scopes; an arbitrary scope is evicted beyond this (same
/// fail-safe posture as the coding read-state registry).
const MAX_SEEN_SCOPES: usize = 512;
/// Report caps: at most this many new lines per edit...
const MAX_REPORT_LINES: usize = 30;
/// ...and at most this many bytes of new output per edit.
const MAX_REPORT_BYTES: usize = 4000;

/// Operator configuration for the post-edit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostEditCheckConfig {
    command: String,
    timeout: Duration,
}

impl PostEditCheckConfig {
    pub fn new(command: impl Into<String>, timeout: Duration) -> Self {
        Self {
            command: command.into(),
            timeout,
        }
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Resolve the config from the process environment.
    ///
    /// Module-owned factory intended to be called once from the composition
    /// layer (see `ironclaw_reborn_composition::factory`); per-call handlers
    /// must consume the already-resolved config instead of reading env.
    /// Returns `Ok(None)` when `IRONCLAW_POST_EDIT_CHECK` is unset or blank.
    pub fn from_env() -> Result<Option<Self>, PostEditCheckConfigError> {
        Self::from_values(
            optional_env(POST_EDIT_CHECK_ENV)?,
            optional_env(POST_EDIT_CHECK_TIMEOUT_ENV)?,
        )
    }

    fn from_values(
        command: Option<String>,
        timeout_secs: Option<String>,
    ) -> Result<Option<Self>, PostEditCheckConfigError> {
        let Some(command) = command.filter(|command| !command.trim().is_empty()) else {
            return Ok(None);
        };
        let timeout_secs = match timeout_secs {
            None => DEFAULT_TIMEOUT_SECS,
            Some(raw) => {
                let parsed =
                    raw.trim()
                        .parse::<u64>()
                        .map_err(|error| PostEditCheckConfigError {
                            reason: format!(
                                "{POST_EDIT_CHECK_TIMEOUT_ENV} must be a positive integer: {error}"
                            ),
                        })?;
                if parsed == 0 {
                    return Err(PostEditCheckConfigError {
                        reason: format!("{POST_EDIT_CHECK_TIMEOUT_ENV} must be greater than zero"),
                    });
                }
                parsed
            }
        };
        Ok(Some(Self::new(command, Duration::from_secs(timeout_secs))))
    }
}

fn optional_env(key: &'static str) -> Result<Option<String>, PostEditCheckConfigError> {
    match std::env::var(key) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(PostEditCheckConfigError {
            reason: format!("could not read {key}: {error}"),
        }),
    }
}

/// Invalid post-edit check configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid post-edit check config: {reason}")]
pub struct PostEditCheckConfigError {
    reason: String,
}

/// Diagnostic lines already reported to the model, keyed by the same scope
/// dimensions as the coding read-state registry. Bounded in both scope count
/// and lines per scope; eviction only means a line may be reported again.
#[derive(Debug, Default)]
pub(crate) struct PostEditCheckSeenLines {
    scopes: Mutex<HashMap<SeenScopeKey, ScopeSeenLines>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SeenScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
}

impl SeenScopeKey {
    fn from_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
        }
    }
}

#[derive(Debug, Default)]
struct ScopeSeenLines {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl ScopeSeenLines {
    fn record(&mut self, line: &str) {
        if !self.set.insert(line.to_string()) {
            return;
        }
        self.order.push_back(line.to_string());
        while self.order.len() > MAX_SEEN_LINES_PER_SCOPE {
            if let Some(evicted) = self.order.pop_front() {
                self.set.remove(&evicted);
            }
        }
    }
}

impl PostEditCheckSeenLines {
    /// Return the check-output lines not previously reported for this scope,
    /// capped at [`MAX_REPORT_LINES`] / [`MAX_REPORT_BYTES`] with a trailing
    /// `+N more new lines` note when trimmed. Only the *reported* lines are
    /// recorded as seen, so lines trimmed by the caps surface on a later
    /// check instead of being silently dropped. Returns `None` when the
    /// output carries no new lines.
    pub(crate) fn filter_new(&self, scope: &ResourceScope, output: &str) -> Option<String> {
        let key = SeenScopeKey::from_scope(scope);
        let mut scopes = match self.scopes.lock() {
            Ok(guard) => guard,
            // A poisoned lock means another thread panicked mid-update; the
            // map stays coherent (single-entry ops), so keep serving.
            Err(poisoned) => poisoned.into_inner(),
        };
        if scopes.len() >= MAX_SEEN_SCOPES && !scopes.contains_key(&key) {
            // Evict an arbitrary scope to keep memory bounded; that scope
            // just re-reports previously seen lines on its next check.
            if let Some(evicted) = scopes.keys().next().cloned() {
                scopes.remove(&evicted);
            }
        }
        let seen = scopes.entry(key).or_default();

        let mut batch: HashSet<&str> = HashSet::new();
        let new_lines: Vec<&str> = output
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.trim().is_empty())
            .filter(|line| !seen.set.contains(*line))
            .filter(|line| batch.insert(*line))
            .collect();
        if new_lines.is_empty() {
            return None;
        }

        let mut reported = Vec::new();
        let mut bytes = 0usize;
        for line in &new_lines {
            if reported.len() >= MAX_REPORT_LINES {
                break;
            }
            let cost = line.len() + usize::from(!reported.is_empty());
            if bytes + cost > MAX_REPORT_BYTES {
                if reported.is_empty() {
                    // A single oversized line: report a bounded prefix but
                    // remember the full line so it is never re-reported.
                    let budget = truncation_boundary(line, MAX_REPORT_BYTES);
                    reported.push(&line[..budget]); // safety: truncation_boundary returns is_char_boundary() indices
                    seen.record(line);
                }
                break;
            }
            bytes += cost;
            reported.push(line);
            seen.record(line);
        }

        let mut rendered = reported.join("\n");
        let trimmed = new_lines.len() - reported.len();
        if trimmed > 0 {
            rendered.push_str(&format!("\n+{trimmed} more new lines"));
        }
        Some(rendered)
    }
}

/// Largest index `<= max_bytes` that is a UTF-8 char boundary of `line`.
fn truncation_boundary(line: &str, max_bytes: usize) -> usize {
    let mut boundary = max_bytes.min(line.len());
    while boundary > 0 && !line.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Run the configured check after a successful edit and shape the advisory
/// `post_edit_check` value for the edit's model-visible output.
///
/// - New findings: `{"exit_code": N, "new_output": "..."}`
/// - No new findings: `{"exit_code": N}` (token-lean)
/// - Timeout: `{"timed_out": true}`
/// - Check could not run at all: `None` (debug-logged; the edit already
///   succeeded and must not fail because of the advisory check)
///
/// The command runs with the first read+write mount alias as its working
/// directory (the process port resolves the alias to the host root backing
/// `/workspace`, exactly as it does for shell workdirs); without mounts it
/// falls back to the port's default working directory like `builtin.shell`.
pub(crate) async fn run_post_edit_check(
    seen: &PostEditCheckSeenLines,
    process: &dyn RuntimeProcessPort,
    scope: &ResourceScope,
    mounts: Option<&MountView>,
    config: &PostEditCheckConfig,
) -> Option<Value> {
    let workdir = mounts.and_then(|mounts| {
        mounts
            .mounts
            .iter()
            .find(|grant| grant.permissions.read && grant.permissions.write)
            .map(|grant| grant.alias.as_str().to_string())
    });
    let outcome = process
        .run_command(CommandExecutionRequest {
            scope: scope.clone(),
            mounts: mounts.cloned(),
            command: config.command().to_string(),
            workdir,
            timeout_secs: Some(config.timeout().as_secs()),
            extra_env: HashMap::new(),
        })
        .await;
    match outcome {
        Ok(output) => {
            let mut value = json!({"exit_code": output.exit_code});
            if let Some(new_output) = seen.filter_new(scope, &output.output)
                && let Some(object) = value.as_object_mut()
            {
                object.insert("new_output".to_string(), Value::String(new_output));
            }
            Some(value)
        }
        Err(RuntimeProcessError::Timeout(_)) => Some(json!({"timed_out": true})),
        Err(RuntimeProcessError::ExecutionFailed(reason)) => {
            tracing::debug!(reason = %reason, "post-edit check could not run");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, UserId};

    fn scope(user: &str) -> ResourceScope {
        ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap()
    }

    #[test]
    fn filter_new_dedups_across_calls_within_a_scope() {
        let seen = PostEditCheckSeenLines::default();
        let scope = scope("dedup-user");

        let first = seen
            .filter_new(&scope, "error: one\nwarning: two\n")
            .expect("first call reports both lines");
        assert_eq!(first, "error: one\nwarning: two");

        assert_eq!(
            seen.filter_new(&scope, "error: one\nwarning: two\n"),
            None,
            "identical output must report nothing"
        );

        let third = seen
            .filter_new(&scope, "error: one\nerror: three\n")
            .expect("only the unseen line is reported");
        assert_eq!(third, "error: three");
    }

    #[test]
    fn filter_new_is_keyed_per_scope() {
        let seen = PostEditCheckSeenLines::default();

        assert!(seen.filter_new(&scope("scope-a"), "error: one\n").is_some());
        assert!(
            seen.filter_new(&scope("scope-b"), "error: one\n").is_some(),
            "another scope must get its own seen-set"
        );
    }

    #[test]
    fn filter_new_skips_blank_lines_and_batch_duplicates() {
        let seen = PostEditCheckSeenLines::default();

        let reported = seen
            .filter_new(&scope("blank-user"), "\nerror: one\r\n   \nerror: one\n")
            .expect("one real line");
        assert_eq!(reported, "error: one");
    }

    #[test]
    fn filter_new_caps_lines_and_counts_the_rest() {
        let seen = PostEditCheckSeenLines::default();
        let scope = scope("cap-user");
        let output: String = (0..40).map(|index| format!("error: e{index}\n")).collect();

        let reported = seen.filter_new(&scope, &output).expect("new lines");
        assert_eq!(reported.lines().count(), MAX_REPORT_LINES + 1);
        assert!(reported.ends_with("+10 more new lines"));
        assert!(reported.contains("error: e29"));
        assert!(!reported.contains("error: e30"));

        // Trimmed lines were not marked seen: the next identical run surfaces them.
        let next = seen.filter_new(&scope, &output).expect("trimmed remainder");
        assert!(next.starts_with("error: e30"));
        assert!(next.contains("error: e39"));
    }

    #[test]
    fn filter_new_caps_bytes_and_counts_the_rest() {
        let seen = PostEditCheckSeenLines::default();
        let scope = scope("byte-user");
        // 3980 + newline + 16-byte tail-one = 3997 fits; tail-two would not.
        let big = "x".repeat(3980);
        let output = format!("{big}\nerror: tail-one\nerror: tail-two\n");

        let reported = seen.filter_new(&scope, &output).expect("new lines");
        assert!(reported.len() <= MAX_REPORT_BYTES + "\n+1 more new lines".len());
        assert!(reported.contains(&big));
        assert!(reported.contains("tail-one"));
        assert!(!reported.contains("tail-two"));
        assert!(reported.ends_with("+1 more new lines"));

        // The byte-trimmed line was not marked seen and surfaces next run.
        assert_eq!(
            seen.filter_new(&scope, &output).as_deref(),
            Some("error: tail-two")
        );
    }

    #[test]
    fn filter_new_bounds_a_single_oversized_line_and_never_repeats_it() {
        let seen = PostEditCheckSeenLines::default();
        let scope = scope("oversized-user");
        let huge = format!("error: {}", "y".repeat(5000));

        let reported = seen.filter_new(&scope, &huge).expect("bounded prefix");
        assert_eq!(reported.len(), MAX_REPORT_BYTES);
        assert!(
            seen.filter_new(&scope, &huge).is_none(),
            "the full oversized line must be recorded as seen"
        );
    }

    #[test]
    fn seen_lines_evict_oldest_beyond_the_per_scope_cap() {
        let seen = PostEditCheckSeenLines::default();
        let scope = scope("evict-user");

        // Fill the registry past its cap in reported (<=30 line) batches.
        for batch in 0..20 {
            let output: String = (0..30)
                .map(|index| format!("error: b{batch}-l{index}\n"))
                .collect();
            assert!(seen.filter_new(&scope, &output).is_some());
        }
        // 600 recorded lines > 500 cap: the earliest batch was evicted and is
        // reported again; the latest batch is still deduplicated.
        assert!(seen.filter_new(&scope, "error: b0-l0\n").is_some());
        assert!(seen.filter_new(&scope, "error: b19-l29\n").is_none());
    }

    #[test]
    fn from_values_is_off_without_a_command() {
        assert_eq!(PostEditCheckConfig::from_values(None, None), Ok(None));
        assert_eq!(
            PostEditCheckConfig::from_values(Some("   ".to_string()), None),
            Ok(None)
        );
    }

    #[test]
    fn from_values_defaults_and_parses_the_timeout() {
        let default = PostEditCheckConfig::from_values(Some("cargo check".to_string()), None)
            .unwrap()
            .unwrap();
        assert_eq!(default.command(), "cargo check");
        assert_eq!(default.timeout(), Duration::from_secs(DEFAULT_TIMEOUT_SECS));

        let explicit = PostEditCheckConfig::from_values(
            Some("cargo check".to_string()),
            Some("90".to_string()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(explicit.timeout(), Duration::from_secs(90));
    }

    #[test]
    fn from_values_rejects_invalid_timeouts() {
        for invalid in ["0", "-1", "soon"] {
            let error = PostEditCheckConfig::from_values(
                Some("cargo check".to_string()),
                Some(invalid.to_string()),
            )
            .expect_err("invalid timeout must be a config error");
            assert!(error.to_string().contains(POST_EDIT_CHECK_TIMEOUT_ENV));
        }
    }
}
