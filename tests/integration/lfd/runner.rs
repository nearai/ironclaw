//! Pinned batch orchestration: read every `*.json` case under `$LFD_CASES`,
//! execute each through its profile's harness, write one
//! `$LFD_OUT/<case_id>.outcome.json`. One bad case never kills the batch —
//! parse/build/turn failures and panics become `status: "error"` outcomes,
//! profile gaps become `status: "unsupported"` (SCHEMA.md §2).

use std::path::{Path, PathBuf};
use std::time::Instant;

use futures::FutureExt;

use crate::case::Case;
use crate::extract::{self, StateQueryFailure};
use crate::leak_scan;
use crate::outcome::{LeakReport, OUTCOME_SCHEMA_VERSION, Outcome, OutcomeMeta, OutcomeStatus};
use crate::profiles::{self, ProfileError};
use crate::runner_hash;

/// The Case schema version this runner implements (SCHEMA.md §1).
const SUPPORTED_CASE_SCHEMA_VERSION: u32 = 1;

pub async fn run_batch(cases_dir: &Path, out_dir: &Path) -> Result<(), String> {
    let runner_hash = runner_hash::compute()?;
    std::fs::create_dir_all(out_dir)
        .map_err(|error| format!("cannot create LFD_OUT dir {out_dir:?}: {error}"))?;
    let mut case_files: Vec<PathBuf> = std::fs::read_dir(cases_dir)
        .map_err(|error| format!("cannot read LFD_CASES dir {cases_dir:?}: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    case_files.sort();
    if case_files.is_empty() {
        return Err(format!("no *.json case files under {cases_dir:?}"));
    }

    for path in case_files {
        let started = Instant::now();
        let fallback_case_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown_case")
            .to_string();
        // Panic isolation: a panicking harness (or profile) yields an `error`
        // outcome for THIS case; the batch continues.
        let mut outcome = match std::panic::AssertUnwindSafe(run_case_file(&path, &runner_hash))
            .catch_unwind()
            .await
        {
            Ok(outcome) => outcome,
            Err(panic) => Outcome::failure(
                fallback_case_id,
                "unknown",
                OutcomeStatus::Error,
                format!(
                    "panic while executing case: {}",
                    panic_message(panic.as_ref())
                ),
                runner_hash.clone(),
            ),
        };
        outcome.meta.duration_ms = started.elapsed().as_millis() as u64;
        let out_path = out_dir.join(format!("{}.outcome.json", outcome.case_id));
        let json = serde_json::to_string_pretty(&outcome).map_err(|error| {
            format!(
                "outcome for {:?} does not serialize: {error}",
                outcome.case_id
            )
        })?;
        std::fs::write(&out_path, json)
            .map_err(|error| format!("cannot write outcome {out_path:?}: {error}"))?;
        println!(
            "lfd_runner: {} -> {} [{}]",
            path.display(),
            out_path.display(),
            outcome.status.as_str()
        );
    }
    Ok(())
}

async fn run_case_file(path: &Path, runner_hash: &str) -> Outcome {
    let fallback_case_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown_case")
        .to_string();
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            return Outcome::failure(
                fallback_case_id,
                "unknown",
                OutcomeStatus::Error,
                format!("cannot read case file: {error}"),
                runner_hash.to_string(),
            );
        }
    };
    let case: Case = match serde_json::from_str(&raw) {
        Ok(case) => case,
        Err(error) => {
            return Outcome::failure(
                fallback_case_id,
                "unknown",
                OutcomeStatus::Error,
                format!("case JSON does not match the Case schema: {error}"),
                runner_hash.to_string(),
            );
        }
    };
    run_case(case, runner_hash).await
}

async fn run_case(case: Case, runner_hash: &str) -> Outcome {
    let fail = |status: OutcomeStatus, error: String| {
        Outcome::failure(
            case.case_id.clone(),
            &case.profile,
            status,
            error,
            runner_hash.to_string(),
        )
    };

    if case.schema_version != SUPPORTED_CASE_SCHEMA_VERSION {
        return fail(
            OutcomeStatus::Unsupported,
            format!(
                "case schema_version {} is not the supported version {SUPPORTED_CASE_SCHEMA_VERSION}",
                case.schema_version
            ),
        );
    }
    if case.live {
        return fail(
            OutcomeStatus::Unsupported,
            "live-model mode is not implemented by this runner".to_string(),
        );
    }
    let Some(profile) = profiles::resolve(&case.profile) else {
        return fail(
            OutcomeStatus::Unsupported,
            format!("unknown profile {:?}", case.profile),
        );
    };

    let harness = match profile.assemble(&case).await {
        Ok(harness) => harness,
        Err(ProfileError::Unsupported(reason)) => return fail(OutcomeStatus::Unsupported, reason),
        Err(ProfileError::Harness(reason)) => return fail(OutcomeStatus::Error, reason),
    };

    for entry in &case.inbound {
        match profile.submit_inbound(&harness, entry).await {
            Ok(()) => {}
            Err(ProfileError::Unsupported(reason)) => {
                return fail(OutcomeStatus::Unsupported, reason);
            }
            Err(ProfileError::Harness(reason)) => {
                return fail(OutcomeStatus::Error, reason);
            }
        }
    }

    // Reply records carry the case's inbound channel label (the harness
    // ingress itself is channel-less — synthetic inbound, no adapter).
    let reply_channel = case
        .inbound
        .first()
        .map(|entry| entry.channel.clone())
        .filter(|channel| !channel.is_empty())
        .unwrap_or_else(|| "harness".to_string());

    let mut extraction = match extract::extract(&harness, &reply_channel).await {
        Ok(extraction) => extraction,
        Err(error) => {
            return fail(
                OutcomeStatus::Error,
                format!("outcome extraction failed: {error}"),
            );
        }
    };
    extract::assign_seq(&mut extraction);

    let state =
        match extract::run_state_queries(&harness, profile.as_ref(), &case.state_queries).await {
            Ok(state) => state,
            Err(StateQueryFailure::Unsupported(reason)) => {
                return fail(OutcomeStatus::Unsupported, reason);
            }
            Err(StateQueryFailure::Failed(reason)) => return fail(OutcomeStatus::Error, reason),
        };

    let secret_values: Vec<&str> = case
        .setup
        .secrets
        .iter()
        .map(|secret| secret.value.as_str())
        .collect();
    let secret_scan_hits = leak_scan::secret_scan_hits(&extraction.scan_surfaces, &secret_values);

    Outcome {
        schema_version: OUTCOME_SCHEMA_VERSION,
        case_id: case.case_id,
        status: OutcomeStatus::Ran,
        error: None,
        replies: extraction.replies,
        tool_invocations: extraction.tool_invocations,
        egress: extraction.egress,
        events: extraction.events,
        gates: extraction.gates,
        state,
        leaks: LeakReport { secret_scan_hits },
        meta: OutcomeMeta {
            profile: case.profile,
            runner_hash: runner_hash.to_string(),
            duration_ms: 0,
        },
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}
