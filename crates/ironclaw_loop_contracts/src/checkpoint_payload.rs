//! The opaque, host-owned loop checkpoint payload.
//!
//! The payload bytes are loop-private resume state: never public turn status,
//! event, milestone, or transcript content, and never serialized into a wire
//! DTO. The durable store that holds them is turn-kernel authority and stays
//! in `ironclaw_turns`; only the payload newtype and its ceiling are contract.

use std::fmt;

/// Ceiling on an opaque loop checkpoint payload.
///
/// Loop checkpoints are journalled as process checkpoints, so this ceiling
/// must equal `ironclaw_processes::MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES`. A
/// contracts crate cannot depend on the process kernel to say so, so the
/// equality is pinned from `ironclaw_turns`, which depends on both
/// (`checkpoint_state::tests::checkpoint_payload_ceiling_matches_process_journal`).
pub const MAX_CHECKPOINT_STATE_PAYLOAD_BYTES: usize = 64 * 1024;

/// Internal loop checkpoint payload bytes.
///
/// This value is intentionally not serializable. It is host-owned resume state,
/// not public turn status, event, milestone, or transcript content.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedCheckpointPayload {
    bytes: Vec<u8>,
}

impl RedactedCheckpointPayload {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, String> {
        let bytes = bytes.into();
        validate_checkpoint_payload_len(bytes.len())?;
        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_payload_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for RedactedCheckpointPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactedCheckpointPayload")
            .field("len", &self.bytes.len())
            .field("payload", &"<redacted>")
            .finish()
    }
}

fn validate_checkpoint_payload_len(len: usize) -> Result<(), String> {
    if len > MAX_CHECKPOINT_STATE_PAYLOAD_BYTES {
        return Err(format!(
            "checkpoint payload must be at most {MAX_CHECKPOINT_STATE_PAYLOAD_BYTES} bytes"
        ));
    }
    Ok(())
}
