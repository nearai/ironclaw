//! Durable, run-scoped evidence for a host-owned structured-output finalizer.
//!
//! This module deliberately owns persistence vocabulary only.  It does not
//! know how a finalizer is selected, which model is called, or how a run lease
//! is acquired.  The loop host supplies an opaque lease fence and performs the
//! admission check before calling this CAS door.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::ThreadId,
    turn::{TurnId, TurnRunId},
};
use serde::{Deserialize, Serialize};

use crate::ThreadScope;

/// Provider usage captured for one structured finalizer call.
///
/// The fields intentionally match the provider-neutral usage dimensions that
/// are durable elsewhere, while keeping the threads crate independent of the
/// loop executor crate.  The host performs the typed conversion at this
/// boundary; storage never serializes a provider-specific response object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredFinalizationUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cache_read_input_tokens: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cache_creation_input_tokens: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// Stable accounting evidence for a structured finalizer call.
///
/// `model_profile_id` and route fields are opaque strings at this substrate
/// boundary.  They are evidence, not routing inputs, and are never used to
/// choose or retry a model during readback.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredFinalizationAccounting {
    pub usage: Option<StructuredFinalizationUsage>,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// Durable terminal structured-output evidence for one turn run.
///
/// `candidate` is retained as nonterminal LLM data. `raw_json` is the terminal
/// semantic representation persisted to the assistant transcript. The record
/// is immutable after the first successful CAS write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredFinalizationRecord {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub turn_run_id: TurnRunId,
    /// The durable output-contract identity used by the finalizer.
    pub contract_name: String,
    /// Digest of the exact schema bytes used by the finalizer.
    pub schema_digest: String,
    pub candidate: String,
    pub raw_json: String,
    pub accounting: StructuredFinalizationAccounting,
    /// Opaque claimed-run lease fence.  The threads service stores and
    /// compares this value but never interprets it.
    pub owner_fence: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StructuredFinalizationRecord {
    /// Validate the immutable record shape before any backend write.
    pub fn validate(&self) -> Result<(), String> {
        const MAX_CONTRACT_NAME_BYTES: usize = 128;
        const MAX_DIGEST_BYTES: usize = 256;
        const MAX_FENCE_BYTES: usize = 256;
        const MAX_CANDIDATE_BYTES: usize = 1_000_000;
        const MAX_RAW_JSON_BYTES: usize = 1_000_000;

        if self.contract_name.is_empty() || self.contract_name.len() > MAX_CONTRACT_NAME_BYTES {
            return Err("structured finalization contract name is invalid".to_string());
        }
        if self.schema_digest.is_empty() || self.schema_digest.len() > MAX_DIGEST_BYTES {
            return Err("structured finalization schema digest is invalid".to_string());
        }
        if self.owner_fence.is_empty() || self.owner_fence.len() > MAX_FENCE_BYTES {
            return Err("structured finalization owner fence is invalid".to_string());
        }
        if self.candidate.len() > MAX_CANDIDATE_BYTES {
            return Err("structured finalization candidate is too large".to_string());
        }
        if self.raw_json.len() > MAX_RAW_JSON_BYTES {
            return Err("structured finalization output is too large".to_string());
        }
        Ok(())
    }

    /// Content used for same-record idempotency. Timestamps and the owner
    /// fence are deliberately excluded: a successor lease may retry the same
    /// immutable finalization, while the persisted fence remains evidence of
    /// which worker won the original write.
    pub fn same_immutable_content(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.thread_id == other.thread_id
            && self.turn_id == other.turn_id
            && self.turn_run_id == other.turn_run_id
            && self.contract_name == other.contract_name
            && self.schema_digest == other.schema_digest
            && self.candidate == other.candidate
            && self.raw_json == other.raw_json
            && self.accounting == other.accounting
    }
}

/// Exact key for a run-scoped structured-finalization record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadStructuredFinalizationRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: TurnRunId,
}

/// Absent-write CAS request for a structured-finalization record.
#[derive(Debug, Clone, PartialEq)]
pub struct PutStructuredFinalizationRequest {
    pub record: StructuredFinalizationRecord,
}
