//! Guards the coverage-lane stack-overflow class: every CI job that runs this
//! package's test targets under `cargo llvm-cov` must declare `RUST_MIN_STACK`
//! headroom at the job level, sized to the tier it actually executes.
//!
//! Motivation. libtest gives each test thread a 2 MiB stack by default. Two
//! tiers in this package need more than that, and when either overruns, the
//! process aborts (`has overflowed its stack` / `fatal runtime error: stack
//! overflow`, SIGABRT, exit 101) — killing the whole run rather than failing
//! one test:
//!
//! - **integration tier** (`tests/integration/*`): llvm-cov instrumentation
//!   inflates async frames past the harness's own depth (group build ->
//!   `submit_turn` -> composition). `reborn-tests.yml`'s
//!   `reborn-integration-coverage` lane has carried 8 MiB since #6609.
//! - **root QA tier** (`tests/reborn_qa_*`): `reborn_qa_smoke_scenarios_e2e`
//!   drives whole turns on the libtest stack and measures ~10 MiB
//!   *uninstrumented*, so 8 MiB does not cover it.
//!   `reborn-tests.yml`'s `root-reborn-parity-tests` lane carries 64 MiB.
//!
//! `reborn-tests.yml` splits the two tiers across separate jobs, so each
//! declares only what it needs. `coverage.yml` runs `cargo llvm-cov
//! --workspace`, i.e. **both tiers in one job**, so it needs the larger value.
//! It previously declared neither, which is why `Code Coverage` was red on
//! every push to main for 20+ consecutive commits — on a *different* test each
//! time, as unrelated PRs shifted which future sat deepest:
//!
//! - `unbound_telegram_actor_pairs_via_web_minted_code_…` (extension_delivery)
//! - `duplicate_and_restart_replay_converge_exactly_once::case_1` (extension_ingress)
//! - `extension_install_survives_independent_reopen` (durable)
//!
//! Because the depth lives in shared harness code rather than in any one test,
//! `Box::pin`-ing individual futures only moves the failure to the next-deepest
//! test — the headroom has to be declared per lane. This test pins the
//! invariant on every covered lane so they cannot drift apart again.
//!
//! Out of scope: llvm-cov subcommands that spawn no test threads (`clean`,
//! `show-env`, `report`) and per-crate lanes (`llvm-cov -p <pkg>`, which never
//! reach this package's `tests/`).

use std::path::PathBuf;

/// Headroom for a lane that runs only the integration tier, in bytes (8 MiB) —
/// the value `reborn-tests.yml`'s `reborn-integration-coverage` lane uses.
const INTEGRATION_TIER_BYTES: u64 = 8 * 1024 * 1024;

/// Headroom for a lane that runs the whole workspace, in bytes (64 MiB) — the
/// value `reborn-tests.yml`'s `root-reborn-parity-tests` lane uses. A
/// whole-workspace lane also executes `tests/reborn_qa_*`, so the integration
/// tier's 8 MiB is not sufficient.
const WHOLE_WORKSPACE_BYTES: u64 = 64 * 1024 * 1024;

/// Workflows scanned for covered jobs.
const WORKFLOWS: &[&str] = &[
    ".github/workflows/coverage.yml",
    ".github/workflows/reborn-tests.yml",
];

fn repo_file(relative: &str) -> PathBuf {
    let repo_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("repo root should be discoverable");
    repo_root.join(relative)
}

/// One top-level job in a workflow file, with the lines of its block.
struct Job {
    name: String,
    body: Vec<String>,
}

/// Which test tier a job executes under llvm-cov, and thus how much headroom it
/// must declare.
enum Scope {
    /// `cargo llvm-cov … --workspace` — integration tier *and* root QA tier.
    WholeWorkspace,
    /// The shared lane runner — integration tier only.
    IntegrationTier,
}

impl Scope {
    fn required_bytes(&self) -> u64 {
        match self {
            Self::WholeWorkspace => WHOLE_WORKSPACE_BYTES,
            Self::IntegrationTier => INTEGRATION_TIER_BYTES,
        }
    }
}

/// Indentation of a line, in spaces (tabs are invalid in these workflows).
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Split a workflow into its top-level jobs. Job headers sit at indent 2 under
/// the `jobs:` key; the block runs until the next indent-2 key.
fn parse_jobs(workflow: &str) -> Vec<Job> {
    let mut jobs = Vec::new();
    let mut in_jobs = false;
    let mut current: Option<Job> = None;

    for line in workflow.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.trim_start().starts_with('#') {
            if let Some(job) = current.as_mut() {
                job.body.push(line.to_string());
            }
            continue;
        }

        if indent_of(trimmed) == 0 {
            // A new top-level key ends the `jobs:` mapping.
            if let Some(job) = current.take() {
                jobs.push(job);
            }
            in_jobs = trimmed.starts_with("jobs:");
            continue;
        }

        if in_jobs && indent_of(trimmed) == 2 && trimmed.trim_end().ends_with(':') {
            if let Some(job) = current.take() {
                jobs.push(job);
            }
            current = Some(Job {
                name: trimmed.trim().trim_end_matches(':').to_string(),
                body: Vec::new(),
            });
            continue;
        }

        if let Some(job) = current.as_mut() {
            job.body.push(line.to_string());
        }
    }

    if let Some(job) = current.take() {
        jobs.push(job);
    }
    jobs
}

/// Classify a job by which instrumented test tier it executes, if any.
///
/// Whole-workspace wins when both shapes appear, since it is the wider scope.
fn instrumented_scope(job: &Job) -> Option<Scope> {
    let mut integration_tier = false;

    for line in &job.body {
        if line.contains("reborn-coverage-lane-run.sh") {
            integration_tier = true;
        }
        if !line.contains("cargo llvm-cov") || !line.contains("--workspace") {
            continue;
        }
        // `clean` / `show-env` / `report` do not spawn test threads.
        let non_test = ["llvm-cov clean", "llvm-cov show-env", "llvm-cov report"];
        if !non_test.iter().any(|sub| line.contains(sub)) {
            return Some(Scope::WholeWorkspace);
        }
    }

    integration_tier.then_some(Scope::IntegrationTier)
}

/// Read `RUST_MIN_STACK` from the job-level `env:` mapping (indent 4, keys at
/// indent 6). Step-level `env:` blocks sit deeper and are ignored on purpose —
/// a step-scoped value would not cover the test-running step.
fn job_level_rust_min_stack(job: &Job) -> Option<u64> {
    let mut in_env = false;
    for line in &job.body {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        let indent = indent_of(trimmed);
        if in_env {
            if indent < 6 {
                in_env = false;
            } else if indent == 6 {
                if let Some(value) = trimmed.trim().strip_prefix("RUST_MIN_STACK:") {
                    return value.trim().trim_matches(['"', '\'']).parse::<u64>().ok();
                }
                continue;
            }
        }
        if indent == 4 && trimmed.trim() == "env:" {
            in_env = true;
        }
    }
    None
}

#[test]
fn instrumented_lanes_declare_stack_headroom_for_their_tier() {
    let mut whole_workspace = Vec::new();
    let mut integration_tier = Vec::new();
    let mut violations = Vec::new();

    for workflow in WORKFLOWS {
        let contents = std::fs::read_to_string(repo_file(workflow))
            .unwrap_or_else(|e| panic!("{workflow} should be readable: {e}"));

        for job in parse_jobs(&contents) {
            let Some(scope) = instrumented_scope(&job) else {
                continue;
            };
            let required = scope.required_bytes();
            let label = format!("{workflow}:{}", job.name);

            match job_level_rust_min_stack(&job) {
                Some(bytes) if bytes >= required => match scope {
                    Scope::WholeWorkspace => whole_workspace.push(label),
                    Scope::IntegrationTier => integration_tier.push(label),
                },
                Some(bytes) => violations.push(format!(
                    "{label} declares RUST_MIN_STACK={bytes}, below the {required}-byte floor \
                     for the tier it runs"
                )),
                None => violations.push(format!(
                    "{label} runs this package's test targets under llvm-cov but declares no \
                     job-level RUST_MIN_STACK (needs {required} bytes)"
                )),
            }
        }
    }

    assert!(
        violations.is_empty(),
        "instrumented lanes are missing stack headroom; the suites overrun libtest's 2 MiB \
         default and the run aborts with a stack overflow instead of failing a test:\n  {}",
        violations.join("\n  ")
    );

    // Non-vacuity: the scan must actually reach both known lanes. Without this,
    // a renamed job or a reworded `run:` line would silently empty the scan and
    // leave the assertion above trivially true.
    assert!(
        whole_workspace
            .iter()
            .any(|job| job.starts_with(".github/workflows/coverage.yml:")),
        "expected coverage.yml to contribute a whole-workspace instrumented lane; \
         found whole_workspace={whole_workspace:?} integration_tier={integration_tier:?}"
    );
    assert!(
        integration_tier
            .iter()
            .any(|job| job.starts_with(".github/workflows/reborn-tests.yml:")),
        "expected reborn-tests.yml to contribute an integration-tier instrumented lane; \
         found whole_workspace={whole_workspace:?} integration_tier={integration_tier:?}"
    );
}
