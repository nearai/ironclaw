//! Deterministic pre-ship audit command.
//!
//! `ironclaw audit` consumes local verification state, git state, and optional
//! GitHub PR checks to produce a machine-readable ship/no-ship signal.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use clap::Args;
use serde::{Deserialize, Serialize};

const DEFAULT_STATE_FILE: &str = ".autoverify.state.json";

#[derive(Args, Debug, Clone)]
pub struct AuditCommand {
    /// Project directory to audit
    #[arg(long, default_value = ".")]
    pub target: PathBuf,

    /// Git base for diff checks. Defaults to origin/main, main, then HEAD~1.
    #[arg(long)]
    pub base: Option<String>,

    /// Verification state file. Relative paths are resolved from --target.
    #[arg(long, default_value = DEFAULT_STATE_FILE)]
    pub state: PathBuf,

    /// Skip GitHub PR check inspection
    #[arg(long)]
    pub no_checks: bool,

    /// Print a compact single-line JSON audit result
    #[arg(long)]
    pub compact: bool,

    /// Return a non-zero exit code unless the audit verdict is ship
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AuditVerdict {
    Ship,
    NeedsReview,
    Blocked,
}

impl fmt::Display for AuditVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditVerdict::Ship => f.write_str("ship"),
            AuditVerdict::NeedsReview => f.write_str("needs_review"),
            AuditVerdict::Blocked => f.write_str("blocked"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FindingLevel {
    Info,
    Warning,
    Blocker,
}

impl fmt::Display for FindingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FindingLevel::Info => f.write_str("info"),
            FindingLevel::Warning => f.write_str("warning"),
            FindingLevel::Blocker => f.write_str("blocker"),
        }
    }
}

#[derive(Debug, Serialize)]
struct AuditFinding {
    level: FindingLevel,
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AuditOutput {
    verdict: AuditVerdict,
    target: String,
    git: GitAudit,
    verify: VerifyAudit,
    checks: Option<ChecksAudit>,
    findings: Vec<AuditFinding>,
    started_at: String,
    finished_at: String,
}

#[derive(Debug, Default, Serialize)]
struct GitAudit {
    branch: Option<String>,
    base: Option<String>,
    dirty: bool,
    changed_files: Vec<String>,
    shortstat: Option<String>,
    diff_check_pass: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
struct VerifyAudit {
    state_path: String,
    found: bool,
    verdict: Option<String>,
    git_head: Option<String>,
    current_git_head: Option<String>,
    target_matches: Option<bool>,
    tiers_requested: Vec<String>,
    finished_at: Option<String>,
    summary: Option<serde_json::Value>,
}

#[derive(Debug, Default, Serialize)]
struct ChecksAudit {
    pr_number: Option<u64>,
    pr_url: Option<String>,
    pr_state: Option<String>,
    is_draft: Option<bool>,
    head_ref_oid: Option<String>,
    total: usize,
    pass: usize,
    fail: usize,
    pending: usize,
    skipping: usize,
    cancel: usize,
    unavailable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrView {
    number: Option<u64>,
    url: Option<String>,
    state: Option<String>,
    is_draft: Option<bool>,
    head_ref_oid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhCheck {
    bucket: Option<String>,
}

struct ProcessOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

pub async fn run_audit_command(cmd: AuditCommand) -> anyhow::Result<()> {
    let started_at = chrono::Utc::now();
    let target = cmd
        .target
        .canonicalize()
        .with_context(|| format!("target does not exist: {}", cmd.target.display()))?;

    let mut findings = Vec::new();
    let git = audit_git(&target, cmd.base.as_deref(), &mut findings);
    let verify = audit_verify(&target, &cmd.state, &mut findings);
    let checks = if cmd.no_checks {
        findings.push(AuditFinding {
            level: FindingLevel::Info,
            code: "checks_skipped",
            message: "GitHub PR checks were skipped by --no-checks".to_string(),
        });
        None
    } else {
        Some(audit_checks(&target, &mut findings))
    };

    let verdict = decide_verdict(&findings);
    let output = AuditOutput {
        verdict,
        target: target.display().to_string(),
        git,
        verify,
        checks,
        findings,
        started_at: started_at.to_rfc3339(),
        finished_at: chrono::Utc::now().to_rfc3339(),
    };

    render_output(&output, cmd.compact)?;

    if cmd.strict && output.verdict != AuditVerdict::Ship {
        bail!("audit verdict is {}", output.verdict);
    }

    Ok(())
}

fn audit_git(
    target: &Path,
    requested_base: Option<&str>,
    findings: &mut Vec<AuditFinding>,
) -> GitAudit {
    let mut audit = GitAudit::default();

    match run_process(target, "git", &["status", "--porcelain=v1", "--branch"]) {
        Ok(output) if output.code == Some(0) => {
            let mut lines = output.stdout.lines();
            if let Some(first) = lines.next() {
                audit.branch = parse_branch(first);
            }
            let dirty_entries: Vec<_> = lines.filter(|line| !line.trim().is_empty()).collect();
            audit.dirty = !dirty_entries.is_empty();
            if audit.dirty {
                findings.push(AuditFinding {
                    level: FindingLevel::Blocker,
                    code: "git_dirty",
                    message: format!(
                        "worktree has {} uncommitted status entr{}",
                        dirty_entries.len(),
                        if dirty_entries.len() == 1 { "y" } else { "ies" }
                    ),
                });
            }
        }
        Ok(output) => findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "git_status_failed",
            message: command_failure_message("git status", &output),
        }),
        Err(error) => findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "git_status_failed",
            message: error.to_string(),
        }),
    }

    let base = requested_base
        .map(ToOwned::to_owned)
        .or_else(|| first_valid_revision(target, &["origin/main", "main", "HEAD~1"]));
    audit.base = base.clone();

    let Some(base) = base else {
        findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "git_base_missing",
            message: "could not resolve a base revision for diff checks".to_string(),
        });
        return audit;
    };

    let range = format!("{base}...HEAD");
    match run_process(target, "git", &["diff", "--name-only", &range]) {
        Ok(output) if output.code == Some(0) => {
            audit.changed_files = output
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if audit.changed_files.is_empty() {
                findings.push(AuditFinding {
                    level: FindingLevel::Warning,
                    code: "diff_empty",
                    message: format!("no changed files found against {base}"),
                });
            }
        }
        Ok(output) => findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "diff_name_failed",
            message: command_failure_message("git diff --name-only", &output),
        }),
        Err(error) => findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "diff_name_failed",
            message: error.to_string(),
        }),
    }

    if let Ok(output) = run_process(target, "git", &["diff", "--shortstat", &range])
        && output.code == Some(0)
    {
        let stat = output.stdout.trim();
        if !stat.is_empty() {
            audit.shortstat = Some(stat.to_string());
        }
    }

    match run_process(target, "git", &["diff", "--check", &range]) {
        Ok(output) if output.code == Some(0) => {
            audit.diff_check_pass = Some(true);
        }
        Ok(output) => {
            audit.diff_check_pass = Some(false);
            findings.push(AuditFinding {
                level: FindingLevel::Blocker,
                code: "diff_check_failed",
                message: command_failure_message("git diff --check", &output),
            });
        }
        Err(error) => findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "diff_check_failed",
            message: error.to_string(),
        }),
    }

    audit
}

fn audit_verify(target: &Path, state: &Path, findings: &mut Vec<AuditFinding>) -> VerifyAudit {
    let state_path = if state.is_absolute() {
        state.to_path_buf()
    } else {
        target.join(state)
    };
    let mut audit = VerifyAudit {
        state_path: state_path.display().to_string(),
        ..VerifyAudit::default()
    };

    let raw = match std::fs::read_to_string(&state_path) {
        Ok(raw) => raw,
        Err(error) => {
            findings.push(AuditFinding {
                level: FindingLevel::Blocker,
                code: "verify_state_missing",
                message: format!("could not read {}: {error}", state_path.display()),
            });
            return audit;
        }
    };

    audit.found = true;
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            findings.push(AuditFinding {
                level: FindingLevel::Blocker,
                code: "verify_state_invalid",
                message: format!("could not parse {}: {error}", state_path.display()),
            });
            return audit;
        }
    };

    audit.verdict = parsed
        .get("verdict")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    audit.finished_at = parsed
        .get("finished_at")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    audit.git_head = parsed
        .get("git_head")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    audit.tiers_requested = parsed
        .get("tiers_requested")
        .and_then(serde_json::Value::as_array)
        .map(|tiers| {
            tiers
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    audit.summary = parsed.get("summary").cloned();
    audit_verify_git_head(&mut audit, current_git_head(target), findings);

    match audit.verdict.as_deref() {
        Some("pass") => {}
        Some(other) => findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "verify_not_pass",
            message: format!("latest verification verdict is {other}"),
        }),
        None => findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "verify_verdict_missing",
            message: "verification state has no verdict".to_string(),
        }),
    }

    if let Some(state_target) = parsed.get("target").and_then(serde_json::Value::as_str) {
        let target_matches = state_target == target.display().to_string();
        audit.target_matches = Some(target_matches);
        if !target_matches {
            findings.push(AuditFinding {
                level: FindingLevel::Blocker,
                code: "verify_target_mismatch",
                message: format!(
                    "verification state target is {state_target}, expected {}",
                    target.display()
                ),
            });
        }
    } else {
        findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "verify_target_missing",
            message: "verification state has no target field".to_string(),
        });
    }

    audit
}

fn audit_verify_git_head(
    audit: &mut VerifyAudit,
    current_head: Option<String>,
    findings: &mut Vec<AuditFinding>,
) {
    audit.current_git_head = current_head.clone();
    let Some(current_head) = current_head else {
        return;
    };

    match audit.git_head.as_deref() {
        Some(state_head) if state_head == current_head => {}
        Some(state_head) => findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "verify_state_stale",
            message: format!(
                "verification state was written for git head {state_head}, current head is {current_head}"
            ),
        }),
        None => findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "verify_git_head_missing",
            message: "verification state has no git_head; rerun `ironclaw verify`".to_string(),
        }),
    }
}

fn current_git_head(target: &Path) -> Option<String> {
    let output = run_process(target, "git", &["rev-parse", "HEAD"]).ok()?;
    if output.code != Some(0) {
        return None;
    }

    let head = output.stdout.trim();
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

fn audit_checks(target: &Path, findings: &mut Vec<AuditFinding>) -> ChecksAudit {
    let mut audit = ChecksAudit::default();

    let pr = match run_process(
        target,
        "gh",
        &[
            "pr",
            "view",
            "--json",
            "number,url,state,isDraft,headRefOid",
        ],
    ) {
        Ok(output) if output.code == Some(0) => {
            match serde_json::from_str::<GhPrView>(&output.stdout) {
                Ok(pr) => pr,
                Err(error) => {
                    audit.unavailable = true;
                    findings.push(AuditFinding {
                        level: FindingLevel::Warning,
                        code: "pr_view_invalid",
                        message: format!("could not parse gh pr view output: {error}"),
                    });
                    return audit;
                }
            }
        }
        Ok(output) => {
            audit.unavailable = true;
            findings.push(AuditFinding {
                level: FindingLevel::Warning,
                code: "pr_unavailable",
                message: command_failure_message("gh pr view", &output),
            });
            return audit;
        }
        Err(error) => {
            audit.unavailable = true;
            findings.push(AuditFinding {
                level: FindingLevel::Warning,
                code: "pr_unavailable",
                message: error.to_string(),
            });
            return audit;
        }
    };

    audit.pr_number = pr.number;
    audit.pr_url = pr.url;
    audit.pr_state = pr.state;
    audit.is_draft = pr.is_draft;
    audit.head_ref_oid = pr.head_ref_oid;

    if let Some(local_head) = current_git_head(target) {
        match audit.head_ref_oid.as_deref() {
            Some(pr_head) if pr_head == local_head => {}
            Some(pr_head) => findings.push(AuditFinding {
                level: FindingLevel::Blocker,
                code: "pr_head_mismatch",
                message: format!("pull request head is {pr_head}, local HEAD is {local_head}"),
            }),
            None => findings.push(AuditFinding {
                level: FindingLevel::Warning,
                code: "pr_head_missing",
                message: "pull request metadata did not include headRefOid".to_string(),
            }),
        }
    }

    if audit.is_draft == Some(true) {
        findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "pr_draft",
            message: "pull request is still a draft".to_string(),
        });
    }
    if !matches!(audit.pr_state.as_deref(), Some("OPEN") | Some("open")) {
        findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "pr_not_open",
            message: format!(
                "pull request state is {}",
                audit.pr_state.as_deref().unwrap_or("unknown")
            ),
        });
    }

    let checks_output = match run_process(
        target,
        "gh",
        &["pr", "checks", "--json", "name,state,bucket,link,workflow"],
    ) {
        Ok(output) => output,
        Err(error) => {
            audit.unavailable = true;
            findings.push(AuditFinding {
                level: FindingLevel::Warning,
                code: "checks_unavailable",
                message: error.to_string(),
            });
            return audit;
        }
    };

    let checks = match serde_json::from_str::<Vec<GhCheck>>(&checks_output.stdout) {
        Ok(checks) => checks,
        Err(error) => {
            audit.unavailable = true;
            findings.push(AuditFinding {
                level: FindingLevel::Warning,
                code: "checks_invalid",
                message: format!(
                    "could not parse gh pr checks output: {error}; {}",
                    command_failure_message("gh pr checks", &checks_output)
                ),
            });
            return audit;
        }
    };

    audit_check_buckets(&checks, &mut audit, findings);

    audit
}

fn audit_check_buckets(
    checks: &[GhCheck],
    audit: &mut ChecksAudit,
    findings: &mut Vec<AuditFinding>,
) {
    audit.total = checks.len();
    let mut unknown = 0;
    for check in checks {
        match check.bucket.as_deref() {
            Some("pass") => audit.pass += 1,
            Some("fail") => audit.fail += 1,
            Some("pending") => audit.pending += 1,
            Some("skipping") => audit.skipping += 1,
            Some("cancel") => audit.cancel += 1,
            _ => {
                audit.unavailable = true;
                unknown += 1;
            }
        }
    }

    if audit.total == 0 {
        findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "checks_empty",
            message: "no GitHub PR checks were reported".to_string(),
        });
    }
    if unknown > 0 {
        findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "checks_unknown",
            message: format!("{unknown} PR check bucket(s) had an unknown status"),
        });
    }
    if audit.fail > 0 || audit.cancel > 0 {
        findings.push(AuditFinding {
            level: FindingLevel::Blocker,
            code: "checks_failed",
            message: format!(
                "{} failing and {} canceled PR check bucket(s)",
                audit.fail, audit.cancel
            ),
        });
    }
    if audit.pending > 0 {
        findings.push(AuditFinding {
            level: FindingLevel::Warning,
            code: "checks_pending",
            message: format!("{} PR check bucket(s) still pending", audit.pending),
        });
    }
}

fn render_output(output: &AuditOutput, compact: bool) -> anyhow::Result<()> {
    if compact {
        println!("{}", serde_json::to_string(output)?);
        return Ok(());
    }

    println!("IronClaw audit: {}", output.verdict);
    println!("target: {}", output.target);
    if let Some(branch) = &output.git.branch {
        println!("branch: {branch}");
    }
    if let Some(base) = &output.git.base {
        println!("base: {base}");
    }
    if let Some(shortstat) = &output.git.shortstat {
        println!("diff: {shortstat}");
    }
    println!(
        "verify: {}",
        output.verify.verdict.as_deref().unwrap_or("missing")
    );
    if let Some(checks) = &output.checks
        && !checks.unavailable
    {
        println!(
            "checks: {} pass, {} pending, {} fail, {} cancel, {} skipped",
            checks.pass, checks.pending, checks.fail, checks.cancel, checks.skipping
        );
    }
    for finding in &output.findings {
        println!("- {} {}: {}", finding.level, finding.code, finding.message);
    }

    Ok(())
}

fn decide_verdict(findings: &[AuditFinding]) -> AuditVerdict {
    if findings
        .iter()
        .any(|finding| finding.level == FindingLevel::Blocker)
    {
        return AuditVerdict::Blocked;
    }
    if findings
        .iter()
        .any(|finding| finding.level == FindingLevel::Warning)
    {
        return AuditVerdict::NeedsReview;
    }
    AuditVerdict::Ship
}

fn parse_branch(status_header: &str) -> Option<String> {
    let branch = status_header.strip_prefix("## ")?;
    let current = match branch.split("...").next() {
        Some(value) => value,
        None => branch,
    };
    Some(current.to_string())
}

fn first_valid_revision(target: &Path, candidates: &[&str]) -> Option<String> {
    candidates.iter().find_map(|candidate| {
        let output = run_process(target, "git", &["rev-parse", "--verify", candidate]).ok()?;
        if output.code == Some(0) {
            Some((*candidate).to_string())
        } else {
            None
        }
    })
}

fn run_process(target: &Path, program: &str, args: &[&str]) -> anyhow::Result<ProcessOutput> {
    let output = Command::new(program)
        .args(args)
        .current_dir(target)
        .output()
        .with_context(|| format!("failed to run {program} {}", args.join(" ")))?;

    Ok(ProcessOutput {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn command_failure_message(command: &str, output: &ProcessOutput) -> String {
    let detail = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if detail.is_empty() {
        format!("{command} exited with {:?}", output.code)
    } else {
        format!("{command} exited with {:?}: {detail}", output.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_verdict_blocks_on_blocker() {
        let findings = vec![AuditFinding {
            level: FindingLevel::Blocker,
            code: "x",
            message: "blocked".into(),
        }];

        assert_eq!(decide_verdict(&findings), AuditVerdict::Blocked);
    }

    #[test]
    fn decide_verdict_needs_review_on_warning() {
        let findings = vec![AuditFinding {
            level: FindingLevel::Warning,
            code: "x",
            message: "review".into(),
        }];

        assert_eq!(decide_verdict(&findings), AuditVerdict::NeedsReview);
    }

    #[test]
    fn parse_branch_strips_upstream_suffix() {
        assert_eq!(
            parse_branch("## codex/native-autoverify...origin/codex/native-autoverify"),
            Some("codex/native-autoverify".to_string())
        );
    }

    #[test]
    fn audit_verify_blocks_missing_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut findings = Vec::new();

        let audit = audit_verify(temp.path(), Path::new(DEFAULT_STATE_FILE), &mut findings);

        assert!(!audit.found);
        assert_eq!(decide_verdict(&findings), AuditVerdict::Blocked);
        assert_eq!(findings[0].code, "verify_state_missing");
    }

    #[test]
    fn audit_verify_accepts_pass_state_for_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = serde_json::json!({
            "verdict": "pass",
            "target": temp.path().display().to_string(),
            "tiers_requested": ["smoke"],
            "summary": {"pass": 1},
            "finished_at": "2026-05-02T00:00:00Z"
        });
        std::fs::write(
            temp.path().join(DEFAULT_STATE_FILE),
            serde_json::to_string(&state).expect("serialize state"),
        )
        .expect("write state");
        let mut findings = Vec::new();

        let audit = audit_verify(temp.path(), Path::new(DEFAULT_STATE_FILE), &mut findings);

        assert!(audit.found);
        assert_eq!(audit.verdict.as_deref(), Some("pass"));
        assert_eq!(audit.target_matches, Some(true));
        assert_eq!(decide_verdict(&findings), AuditVerdict::Ship);
    }

    #[test]
    fn audit_verify_blocks_missing_git_head_when_current_head_exists() {
        let mut audit = VerifyAudit::default();
        let mut findings = Vec::new();

        audit_verify_git_head(&mut audit, Some("abc123".to_string()), &mut findings);

        assert_eq!(audit.current_git_head.as_deref(), Some("abc123"));
        assert_eq!(decide_verdict(&findings), AuditVerdict::Blocked);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "verify_git_head_missing")
        );
    }

    #[test]
    fn audit_verify_blocks_stale_git_head() {
        let mut audit = VerifyAudit {
            git_head: Some("old".to_string()),
            ..VerifyAudit::default()
        };
        let mut findings = Vec::new();

        audit_verify_git_head(&mut audit, Some("new".to_string()), &mut findings);

        assert_eq!(decide_verdict(&findings), AuditVerdict::Blocked);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "verify_state_stale")
        );
    }

    #[test]
    fn audit_verify_blocks_non_pass_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = serde_json::json!({
            "verdict": "fail",
            "target": temp.path().display().to_string()
        });
        std::fs::write(
            temp.path().join(DEFAULT_STATE_FILE),
            serde_json::to_string(&state).expect("serialize state"),
        )
        .expect("write state");
        let mut findings = Vec::new();

        let audit = audit_verify(temp.path(), Path::new(DEFAULT_STATE_FILE), &mut findings);

        assert_eq!(audit.verdict.as_deref(), Some("fail"));
        assert_eq!(decide_verdict(&findings), AuditVerdict::Blocked);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "verify_not_pass")
        );
    }

    #[test]
    fn audit_check_buckets_blocks_failed_checks() {
        let checks = vec![
            GhCheck {
                bucket: Some("pass".to_string()),
            },
            GhCheck {
                bucket: Some("fail".to_string()),
            },
        ];
        let mut audit = ChecksAudit::default();
        let mut findings = Vec::new();

        audit_check_buckets(&checks, &mut audit, &mut findings);

        assert_eq!(audit.total, 2);
        assert_eq!(audit.pass, 1);
        assert_eq!(audit.fail, 1);
        assert_eq!(decide_verdict(&findings), AuditVerdict::Blocked);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "checks_failed")
        );
    }

    #[test]
    fn audit_check_buckets_warns_on_unknown_status() {
        let checks = vec![GhCheck {
            bucket: Some("mystery".to_string()),
        }];
        let mut audit = ChecksAudit::default();
        let mut findings = Vec::new();

        audit_check_buckets(&checks, &mut audit, &mut findings);

        assert!(audit.unavailable);
        assert_eq!(decide_verdict(&findings), AuditVerdict::NeedsReview);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "checks_unknown")
        );
    }
}
