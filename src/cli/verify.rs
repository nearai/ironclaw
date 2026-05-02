//! Layered project verification command.
//!
//! `ironclaw verify` is intentionally compatible with the lightweight
//! `.autoverify.json` shape used by Hermes while adding a native Rust runner
//! and structured state file for IronClaw agents.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use clap::Args;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const DEFAULT_CONFIG_FILES: &[&str] = &[".ironclaw-verify.json", ".autoverify.json"];
const DEFAULT_STATE_FILE: &str = ".autoverify.state.json";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const DEFAULT_INTERVAL_SECS: u64 = 60;
const DEFAULT_MAX_ATTEMPTS: u32 = 1;
const OUTPUT_TAIL_CHARS: usize = 4000;

#[derive(Args, Debug, Clone)]
pub struct VerifyCommand {
    /// Project directory containing the verification config
    #[arg(long, default_value = ".")]
    pub target: PathBuf,

    /// Explicit config file path. Defaults to .ironclaw-verify.json, then .autoverify.json.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Run one tier by name
    #[arg(long, conflicts_with = "upto")]
    pub tier: Option<String>,

    /// Run tiers from the beginning through this tier name
    #[arg(long)]
    pub upto: Option<String>,

    /// Print a compact single-line JSON verdict
    #[arg(long)]
    pub compact: bool,

    /// List configured tiers without running commands
    #[arg(long)]
    pub list: bool,

    /// Repeat verification until it passes or --max attempts is reached
    #[arg(long)]
    pub r#loop: bool,

    /// Seconds between loop attempts
    #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
    pub interval: u64,

    /// Maximum attempts when --loop is set
    #[arg(long, default_value_t = DEFAULT_MAX_ATTEMPTS)]
    pub max: u32,

    /// Override the state file path. Relative paths are resolved from --target.
    #[arg(long)]
    pub state: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VerifyConfig {
    version: u32,
    tiers: Vec<VerifyTier>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VerifyTier {
    name: String,
    #[serde(default = "default_timeout_secs")]
    timeout_s: u64,
    #[serde(default)]
    retry_on_fail: bool,
    #[serde(default)]
    continue_on_fail: bool,
    commands: Vec<VerifyCommandSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VerifyCommandSpec {
    name: String,
    run: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Pass,
    Flaky,
    Fail,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Pass => f.write_str("pass"),
            Verdict::Flaky => f.write_str("flaky"),
            Verdict::Fail => f.write_str("fail"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandStatus {
    Pass,
    Flaky,
    Fail,
    Timeout,
    Error,
}

impl fmt::Display for CommandStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandStatus::Pass => f.write_str("pass"),
            CommandStatus::Flaky => f.write_str("flaky"),
            CommandStatus::Fail => f.write_str("fail"),
            CommandStatus::Timeout => f.write_str("timeout"),
            CommandStatus::Error => f.write_str("error"),
        }
    }
}

impl CommandStatus {
    fn is_terminal_failure(self) -> bool {
        matches!(
            self,
            CommandStatus::Fail | CommandStatus::Timeout | CommandStatus::Error
        )
    }
}

#[derive(Debug, Serialize)]
struct VerifyOutput {
    verdict: Verdict,
    target: String,
    config: String,
    attempt: u32,
    max_attempts: u32,
    tiers_requested: Vec<String>,
    commands: Vec<CommandOutcome>,
    summary: VerifySummary,
    started_at: String,
    finished_at: String,
}

#[derive(Debug, Serialize)]
struct CommandOutcome {
    tier: String,
    name: String,
    status: CommandStatus,
    exit: Option<i32>,
    duration_ms: u128,
    attempts: u8,
    tail: String,
}

#[derive(Debug, Default, Serialize)]
struct VerifySummary {
    total: usize,
    pass: usize,
    fail: usize,
    flaky: usize,
    timeout: usize,
    error: usize,
    total_duration_ms: u128,
}

#[derive(Debug, Serialize)]
struct VerifyListing<'a> {
    target: String,
    config: String,
    tiers: Vec<&'a VerifyTier>,
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

pub async fn run_verify_command(cmd: VerifyCommand) -> anyhow::Result<()> {
    let target = cmd
        .target
        .canonicalize()
        .with_context(|| format!("target does not exist: {}", cmd.target.display()))?;
    let config_path = resolve_config_path(&target, cmd.config.as_deref())?;
    let state_path = resolve_state_path(&target, cmd.state.as_deref());
    let config = load_config(&config_path)?;
    let tiers = select_tiers(&config, cmd.tier.as_deref(), cmd.upto.as_deref())?;

    if cmd.list {
        render_listing(&target, &config_path, &tiers, cmd.compact)?;
        return Ok(());
    }

    let max_attempts = if cmd.r#loop { cmd.max.max(1) } else { 1 };
    let mut last_output = None;

    for attempt in 1..=max_attempts {
        let output = run_once(&target, &config_path, &tiers, attempt, max_attempts).await;
        write_state(&state_path, &output)?;
        render_output(&output, cmd.compact, attempt, max_attempts)?;

        let verdict = output.verdict;
        last_output = Some(output);
        if verdict == Verdict::Pass || !cmd.r#loop || attempt == max_attempts {
            break;
        }

        tokio::time::sleep(Duration::from_secs(cmd.interval)).await;
    }

    if let Some(output) = last_output
        && output.verdict != Verdict::Pass
    {
        bail!(
            "verification {} ({} pass, {} flaky, {} fail, {} timeout, {} error)",
            output.verdict,
            output.summary.pass,
            output.summary.flaky,
            output.summary.fail,
            output.summary.timeout,
            output.summary.error
        );
    }

    Ok(())
}

fn resolve_config_path(target: &Path, explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            target.join(path)
        };
        if resolved.is_file() {
            return Ok(resolved);
        }
        bail!("verification config not found: {}", resolved.display());
    }

    for name in DEFAULT_CONFIG_FILES {
        let candidate = target.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    bail!(
        "no verification config found in {} (looked for {})",
        target.display(),
        DEFAULT_CONFIG_FILES.join(", ")
    )
}

fn resolve_state_path(target: &Path, explicit: Option<&Path>) -> PathBuf {
    match explicit {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => target.join(path),
        None => target.join(DEFAULT_STATE_FILE),
    }
}

fn load_config(path: &Path) -> anyhow::Result<VerifyConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: VerifyConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if config.version != 1 {
        bail!(
            "unsupported verification config version: {}",
            config.version
        );
    }
    if config.tiers.is_empty() {
        bail!("verification config has no tiers");
    }
    let mut tier_names = HashSet::new();
    for tier in &config.tiers {
        if tier.name.trim().is_empty() {
            bail!("verification tier name may not be empty");
        }
        if !tier_names.insert(tier.name.as_str()) {
            bail!("duplicate verification tier '{}'", tier.name);
        }
        if tier.timeout_s == 0 {
            bail!("verification tier '{}' must have timeout_s > 0", tier.name);
        }
        if tier.commands.is_empty() {
            bail!("verification tier '{}' has no commands", tier.name);
        }
        let mut command_names = HashSet::new();
        for command in &tier.commands {
            if command.name.trim().is_empty() {
                bail!("verification tier '{}' has an unnamed command", tier.name);
            }
            if !command_names.insert(command.name.as_str()) {
                bail!(
                    "duplicate verification command '{}:{}'",
                    tier.name,
                    command.name
                );
            }
            if command.run.trim().is_empty() {
                bail!(
                    "verification command '{}:{}' has an empty run string",
                    tier.name,
                    command.name
                );
            }
        }
    }

    Ok(config)
}

fn select_tiers<'a>(
    config: &'a VerifyConfig,
    tier: Option<&str>,
    upto: Option<&str>,
) -> anyhow::Result<Vec<&'a VerifyTier>> {
    if let Some(name) = tier {
        let selected = config
            .tiers
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| unknown_tier_error(config, name))?;
        return Ok(vec![selected]);
    }

    if let Some(name) = upto {
        let index = config
            .tiers
            .iter()
            .position(|candidate| candidate.name == name)
            .ok_or_else(|| unknown_tier_error(config, name))?;
        return Ok(config.tiers.iter().take(index + 1).collect());
    }

    Ok(config.tiers.iter().collect())
}

fn unknown_tier_error(config: &VerifyConfig, name: &str) -> anyhow::Error {
    anyhow!(
        "unknown verification tier: {name} (available: {})",
        config
            .tiers
            .iter()
            .map(|tier| tier.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

async fn run_once(
    target: &Path,
    config_path: &Path,
    tiers: &[&VerifyTier],
    attempt: u32,
    max_attempts: u32,
) -> VerifyOutput {
    let started_at = chrono::Utc::now();
    let mut commands = Vec::new();

    for tier in tiers {
        let mut tier_failed = false;
        for spec in &tier.commands {
            let outcome = run_command_with_retry(target, tier, spec).await;
            if outcome.status.is_terminal_failure() {
                tier_failed = true;
            }
            commands.push(outcome);

            if tier_failed && !tier.continue_on_fail {
                break;
            }
        }
        if tier_failed && !tier.continue_on_fail {
            break;
        }
    }

    let summary = summarize(&commands);
    let verdict = if summary.fail > 0 || summary.timeout > 0 || summary.error > 0 {
        Verdict::Fail
    } else if summary.flaky > 0 {
        Verdict::Flaky
    } else {
        Verdict::Pass
    };

    VerifyOutput {
        verdict,
        target: target.display().to_string(),
        config: config_path.display().to_string(),
        attempt,
        max_attempts,
        tiers_requested: tiers.iter().map(|tier| tier.name.clone()).collect(),
        commands,
        summary,
        started_at: started_at.to_rfc3339(),
        finished_at: chrono::Utc::now().to_rfc3339(),
    }
}

async fn run_command_with_retry(
    target: &Path,
    tier: &VerifyTier,
    spec: &VerifyCommandSpec,
) -> CommandOutcome {
    let first = run_command_once(target, tier, spec).await;
    if !tier.retry_on_fail || first.status == CommandStatus::Pass {
        return first;
    }

    let retry = run_command_once(target, tier, spec).await;
    if retry.status == CommandStatus::Pass {
        CommandOutcome {
            status: CommandStatus::Flaky,
            attempts: 2,
            duration_ms: first.duration_ms + retry.duration_ms,
            tail: format!(
                "first attempt failed:\n{}\n\nretry passed:\n{}",
                first.tail, retry.tail
            ),
            exit: retry.exit,
            tier: retry.tier,
            name: retry.name,
        }
    } else {
        CommandOutcome {
            attempts: 2,
            duration_ms: first.duration_ms + retry.duration_ms,
            tail: format!(
                "first attempt:\n{}\n\nretry attempt:\n{}",
                first.tail, retry.tail
            ),
            ..retry
        }
    }
}

async fn run_command_once(
    target: &Path,
    tier: &VerifyTier,
    spec: &VerifyCommandSpec,
) -> CommandOutcome {
    let start = Instant::now();
    let mut command = shell_command(&spec.run);
    command
        .current_dir(target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let result =
        tokio::time::timeout(Duration::from_secs(tier.timeout_s.max(1)), command.output()).await;

    match result {
        Ok(Ok(output)) => {
            let mut combined = String::new();
            combined.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push_str("\n--- stderr ---\n");
                }
                combined.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            let success = output.status.success();
            CommandOutcome {
                tier: tier.name.clone(),
                name: spec.name.clone(),
                status: if success {
                    CommandStatus::Pass
                } else {
                    CommandStatus::Fail
                },
                exit: output.status.code(),
                duration_ms: start.elapsed().as_millis(),
                attempts: 1,
                tail: tail(&combined),
            }
        }
        Ok(Err(error)) => CommandOutcome {
            tier: tier.name.clone(),
            name: spec.name.clone(),
            status: CommandStatus::Error,
            exit: None,
            duration_ms: start.elapsed().as_millis(),
            attempts: 1,
            tail: tail(&error.to_string()),
        },
        Err(_) => CommandOutcome {
            tier: tier.name.clone(),
            name: spec.name.clone(),
            status: CommandStatus::Timeout,
            exit: None,
            duration_ms: start.elapsed().as_millis(),
            attempts: 1,
            tail: format!("timed out after {}s", tier.timeout_s.max(1)),
        },
    }
}

#[cfg(windows)]
fn shell_command(script: &str) -> Command {
    let mut command = Command::new("cmd");
    command.arg("/C").arg(script);
    command
}

#[cfg(not(windows))]
fn shell_command(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-lc").arg(script);
    command
}

fn summarize(commands: &[CommandOutcome]) -> VerifySummary {
    let mut summary = VerifySummary {
        total: commands.len(),
        ..VerifySummary::default()
    };
    for command in commands {
        summary.total_duration_ms += command.duration_ms;
        match command.status {
            CommandStatus::Pass => summary.pass += 1,
            CommandStatus::Fail => summary.fail += 1,
            CommandStatus::Flaky => summary.flaky += 1,
            CommandStatus::Timeout => summary.timeout += 1,
            CommandStatus::Error => summary.error += 1,
        }
    }
    summary
}

fn write_state(path: &Path, output: &VerifyOutput) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(output)?;
    std::fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn render_output(
    output: &VerifyOutput,
    compact: bool,
    attempt: u32,
    max_attempts: u32,
) -> anyhow::Result<()> {
    if compact {
        println!("{}", serde_json::to_string(output)?);
        return Ok(());
    }

    println!(
        "IronClaw verify attempt {attempt}/{max_attempts}: {}",
        output.verdict
    );
    println!("target: {}", output.target);
    println!("config: {}", output.config);
    println!("tiers: {}", output.tiers_requested.join(", "));
    println!(
        "summary: {} pass, {} flaky, {} fail, {} timeout, {} error ({} commands, {}ms)",
        output.summary.pass,
        output.summary.flaky,
        output.summary.fail,
        output.summary.timeout,
        output.summary.error,
        output.summary.total,
        output.summary.total_duration_ms
    );

    for command in &output.commands {
        println!(
            "- {}:{} {} ({}ms, attempts={})",
            command.tier, command.name, command.status, command.duration_ms, command.attempts
        );
        if command.status != CommandStatus::Pass && !command.tail.trim().is_empty() {
            println!("{}", indent_tail(&command.tail));
        }
    }

    Ok(())
}

fn render_listing(
    target: &Path,
    config_path: &Path,
    tiers: &[&VerifyTier],
    compact: bool,
) -> anyhow::Result<()> {
    let listing = VerifyListing {
        target: target.display().to_string(),
        config: config_path.display().to_string(),
        tiers: tiers.to_vec(),
    };

    if compact {
        println!("{}", serde_json::to_string(&listing)?);
        return Ok(());
    }

    println!("IronClaw verify tiers");
    println!("target: {}", listing.target);
    println!("config: {}", listing.config);
    for tier in tiers {
        println!(
            "- {} (timeout={}s, retry_on_fail={}, continue_on_fail={})",
            tier.name, tier.timeout_s, tier.retry_on_fail, tier.continue_on_fail
        );
        for command in &tier.commands {
            println!("  - {}: {}", command.name, command.run);
        }
    }

    Ok(())
}

fn tail(text: &str) -> String {
    if text.chars().count() <= OUTPUT_TAIL_CHARS {
        return text.trim().to_string();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(OUTPUT_TAIL_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("[truncated]\n{}", tail.trim())
}

fn indent_tail(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify_cmd(target: &Path) -> VerifyCommand {
        VerifyCommand {
            target: target.to_path_buf(),
            config: None,
            tier: None,
            upto: None,
            compact: true,
            list: false,
            r#loop: false,
            interval: DEFAULT_INTERVAL_SECS,
            max: DEFAULT_MAX_ATTEMPTS,
            state: None,
        }
    }

    fn write_config(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join(".autoverify.json");
        std::fs::write(&path, body).expect("write config");
        path
    }

    #[test]
    fn select_tier_by_name() {
        let config = VerifyConfig {
            version: 1,
            tiers: vec![
                VerifyTier {
                    name: "smoke".into(),
                    timeout_s: 1,
                    retry_on_fail: false,
                    continue_on_fail: false,
                    commands: vec![VerifyCommandSpec {
                        name: "a".into(),
                        run: "true".into(),
                    }],
                },
                VerifyTier {
                    name: "unit".into(),
                    timeout_s: 1,
                    retry_on_fail: false,
                    continue_on_fail: false,
                    commands: vec![VerifyCommandSpec {
                        name: "b".into(),
                        run: "true".into(),
                    }],
                },
            ],
        };

        let selected = select_tiers(&config, Some("unit"), None).expect("select tier");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "unit");
    }

    #[tokio::test]
    async fn run_once_marks_success_pass() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = write_config(
            temp.path(),
            r#"{
              "version": 1,
              "tiers": [
                {"name": "smoke", "timeout_s": 5, "commands": [{"name": "ok", "run": "printf done"}]}
              ]
            }"#,
        );
        let config = load_config(&config_path).expect("load config");
        let tiers = select_tiers(&config, None, None).expect("select tiers");

        let output = run_once(temp.path(), &config_path, &tiers, 1, 1).await;

        assert_eq!(output.verdict, Verdict::Pass);
        assert_eq!(output.attempt, 1);
        assert_eq!(output.max_attempts, 1);
        assert_eq!(output.summary.pass, 1);
        assert_eq!(output.commands[0].tail, "done");
    }

    #[tokio::test]
    async fn retry_pass_is_flaky() {
        let temp = tempfile::tempdir().expect("tempdir");
        let marker = temp.path().join("marker");
        let command = format!(
            "if [ -f {0} ]; then exit 0; else touch {0}; exit 1; fi",
            marker.display()
        );
        let config_path = write_config(
            temp.path(),
            &format!(
                r#"{{
                  "version": 1,
                  "tiers": [
                    {{"name": "smoke", "timeout_s": 5, "retry_on_fail": true, "commands": [{{"name": "eventual", "run": "{}"}}]}}
                  ]
                }}"#,
                command.replace('\\', "\\\\").replace('"', "\\\"")
            ),
        );
        let config = load_config(&config_path).expect("load config");
        let tiers = select_tiers(&config, None, None).expect("select tiers");

        let output = run_once(temp.path(), &config_path, &tiers, 1, 1).await;

        assert_eq!(output.verdict, Verdict::Flaky);
        assert_eq!(output.summary.flaky, 1);
        assert_eq!(output.commands[0].attempts, 2);
    }

    #[tokio::test]
    async fn stops_after_failing_tier_by_default() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = write_config(
            temp.path(),
            r#"{
              "version": 1,
              "tiers": [
                {"name": "smoke", "timeout_s": 5, "commands": [{"name": "fail", "run": "exit 9"}]},
                {"name": "unit", "timeout_s": 5, "commands": [{"name": "should-not-run", "run": "printf nope"}]}
              ]
            }"#,
        );
        let config = load_config(&config_path).expect("load config");
        let tiers = select_tiers(&config, None, None).expect("select tiers");

        let output = run_once(temp.path(), &config_path, &tiers, 1, 1).await;

        assert_eq!(output.verdict, Verdict::Fail);
        assert_eq!(output.commands.len(), 1);
        assert_eq!(output.commands[0].exit, Some(9));
    }

    #[test]
    fn resolve_config_prefers_ironclaw_verify_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join(".autoverify.json"), "{}").expect("write fallback");
        std::fs::write(temp.path().join(".ironclaw-verify.json"), "{}")
            .expect("write native config");

        let selected = resolve_config_path(temp.path(), None).expect("resolve config");

        assert_eq!(selected, temp.path().join(".ironclaw-verify.json"));
    }

    #[test]
    fn load_config_rejects_duplicate_tiers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = write_config(
            temp.path(),
            r#"{
              "version": 1,
              "tiers": [
                {"name": "smoke", "commands": [{"name": "a", "run": "true"}]},
                {"name": "smoke", "commands": [{"name": "b", "run": "true"}]}
              ]
            }"#,
        );

        let error = load_config(&config_path).expect_err("duplicate tiers should fail");

        assert!(error.to_string().contains("duplicate verification tier"));
    }

    #[test]
    fn load_config_rejects_duplicate_commands() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = write_config(
            temp.path(),
            r#"{
              "version": 1,
              "tiers": [
                {"name": "smoke", "commands": [
                  {"name": "same", "run": "true"},
                  {"name": "same", "run": "true"}
                ]}
              ]
            }"#,
        );

        let error = load_config(&config_path).expect_err("duplicate commands should fail");

        assert!(
            error
                .to_string()
                .contains("duplicate verification command 'smoke:same'")
        );
    }

    #[test]
    fn select_unknown_tier_lists_available_tiers() {
        let config = VerifyConfig {
            version: 1,
            tiers: vec![VerifyTier {
                name: "smoke".into(),
                timeout_s: 1,
                retry_on_fail: false,
                continue_on_fail: false,
                commands: vec![VerifyCommandSpec {
                    name: "a".into(),
                    run: "true".into(),
                }],
            }],
        };

        let error = select_tiers(&config, Some("missing"), None).expect_err("unknown tier");

        assert!(
            error
                .to_string()
                .contains("unknown verification tier: missing (available: smoke)")
        );
    }

    #[tokio::test]
    async fn run_verify_command_returns_error_on_failure_without_exiting() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_config(
            temp.path(),
            r#"{
              "version": 1,
              "tiers": [
                {"name": "smoke", "timeout_s": 5, "commands": [{"name": "fail", "run": "exit 7"}]}
              ]
            }"#,
        );

        let error = run_verify_command(verify_cmd(temp.path()))
            .await
            .expect_err("failing verification should return an error");
        let state = std::fs::read_to_string(temp.path().join(DEFAULT_STATE_FILE))
            .expect("state file should be written");
        let state: serde_json::Value = serde_json::from_str(&state).expect("parse state");

        assert!(error.to_string().contains("verification fail"));
        assert_eq!(state["verdict"], "fail");
        assert_eq!(state["attempt"], 1);
        assert_eq!(state["commands"][0]["exit"], 7);
    }

    #[tokio::test]
    async fn list_mode_does_not_run_commands_or_write_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let marker = temp.path().join("marker");
        write_config(
            temp.path(),
            &format!(
                r#"{{
                  "version": 1,
                  "tiers": [
                    {{"name": "smoke", "commands": [{{"name": "touch", "run": "touch {}"}}]}}
                  ]
                }}"#,
                marker.display()
            ),
        );
        let mut cmd = verify_cmd(temp.path());
        cmd.list = true;

        run_verify_command(cmd).await.expect("list tiers");

        assert!(!marker.exists(), "list mode should not run commands");
        assert!(
            !temp.path().join(DEFAULT_STATE_FILE).exists(),
            "list mode should not write verifier state"
        );
    }
}
