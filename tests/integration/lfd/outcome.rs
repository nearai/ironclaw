//! Outcome schema (SCHEMA.md §2) — one `$LFD_OUT/<case_id>.outcome.json` per
//! case. Pinned runner code.

use std::collections::BTreeMap;

use serde::Serialize;

pub const OUTCOME_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    /// Scenario executed end-to-end; the arrays below are trustworthy.
    Ran,
    /// The harness raised (build, turn, extraction, or panic).
    Error,
    /// The profile (or the runner) cannot execute this case yet. Scores 0,
    /// never skipped silently (SCHEMA.md §2).
    Unsupported,
}

impl OutcomeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ran => "ran",
            Self::Error => "error",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub schema_version: u32,
    pub case_id: String,
    pub status: OutcomeStatus,
    pub error: Option<String>,
    pub replies: Vec<ReplyRecord>,
    pub tool_invocations: Vec<ToolInvocationRecord>,
    pub egress: Vec<EgressRecord>,
    pub events: Vec<EventRecord>,
    pub gates: Vec<GateRecord>,
    /// Results keyed by `state_queries[].id`, read from persisted storage
    /// after the run. `BTreeMap` for deterministic key order on disk.
    pub state: BTreeMap<String, serde_json::Value>,
    pub leaks: LeakReport,
    pub meta: OutcomeMeta,
}

impl Outcome {
    /// A no-extraction outcome for the `error`/`unsupported` paths: empty
    /// observation arrays, `error` set. `duration_ms` is stamped by the batch
    /// driver after the case future resolves.
    pub fn failure(
        case_id: String,
        profile: &str,
        status: OutcomeStatus,
        error: String,
        runner_hash: String,
    ) -> Self {
        Self {
            schema_version: OUTCOME_SCHEMA_VERSION,
            case_id,
            status,
            error: Some(error),
            replies: Vec::new(),
            tool_invocations: Vec::new(),
            egress: Vec::new(),
            events: Vec::new(),
            gates: Vec::new(),
            state: BTreeMap::new(),
            leaks: LeakReport {
                secret_scan_hits: 0,
            },
            meta: OutcomeMeta {
                profile: profile.to_string(),
                runner_hash,
                duration_ms: 0,
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReplyRecord {
    pub channel: String,
    pub text: String,
    pub seq: u64,
}

#[derive(Debug, Serialize)]
pub struct ToolInvocationRecord {
    pub name: String,
    pub params_json: String,
    pub ok: bool,
    pub seq: u64,
}

#[derive(Debug, Serialize)]
pub struct EgressRecord {
    pub method: String,
    pub url: String,
    pub seq: u64,
}

#[derive(Debug, Serialize)]
pub struct EventRecord {
    pub kind: String,
    pub seq: u64,
}

#[derive(Debug, Serialize)]
pub struct GateRecord {
    pub kind: String,
    pub resolution: String,
    pub seq: u64,
}

#[derive(Debug, Serialize)]
pub struct LeakReport {
    pub secret_scan_hits: u64,
}

#[derive(Debug, Serialize)]
pub struct OutcomeMeta {
    pub profile: String,
    pub runner_hash: String,
    pub duration_ms: u64,
}
