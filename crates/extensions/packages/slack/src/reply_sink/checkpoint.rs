//! The Slack reply sink's checkpoint: everything Slack has been told about
//! one reply, as of the last applied request, plus the codec that keeps it
//! within the host's byte bound and the text helpers the offsets rely on.
//! Version 1, JSON; the host persists it opaquely between reconciles.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ironclaw_extension_contracts::reply::{
    REPLY_SINK_CHECKPOINT_MAX_BYTES, ReplyDocument, ReplyOutcomeReason, ReplyReconcileRequest,
    ReplySinkCheckpoint,
};
use serde::{Deserialize, Serialize};

use super::{READ_BACK_TAIL_CHARS, SLACK_REPLY_CHECKPOINT_VERSION};

// ── Checkpoint ───────────────────────────────────────────────────────────

/// Everything Slack has been told about one reply, as of the last applied
/// request. Version 1, JSON, bounded to `REPLY_SINK_CHECKPOINT_MAX_BYTES`
/// by evicting settled task fingerprints behind a floor ordinal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SlackReplyCheckpoint {
    #[serde(default)]
    pub(super) session_status: SlackSessionStatus,
    #[serde(default)]
    pub(super) stream: Option<SlackStreamState>,
    /// Hidden provider sentinel or real activity id → fingerprint of the last
    /// `task_update` sent for it.
    #[serde(default)]
    pub(super) tasks: BTreeMap<String, String>,
    /// Activities with `started_ordinal` below this and no fingerprint entry
    /// are settled: fully published and evicted to keep the checkpoint
    /// within its byte bound.
    #[serde(default)]
    pub(super) tasks_floor_ordinal: u64,
    /// Fingerprint of the last status line appended (answer still empty).
    #[serde(default)]
    pub(super) status_key: Option<String>,
    /// Fingerprint of the last `plan_update` title Slack accepted.
    #[serde(default)]
    pub(super) plan_title_key: Option<String>,
    /// Fingerprint of the attention block currently shown in the message.
    #[serde(default)]
    pub(super) attention_key: Option<String>,
    #[serde(default)]
    pub(super) terminal: Option<SlackTerminalState>,
    #[serde(default)]
    pub(super) attachments_delivered: bool,
    /// How many terminal attachments (in request order) Slack has confirmed
    /// shared — files before this index are never re-sent by a retry.
    #[serde(default)]
    pub(super) attachments_progress: u64,
    /// A `files.completeUploadExternal` crossed into transport without an
    /// answer: the files may already be shared, and re-completing (or
    /// re-uploading) could show them twice. While set, no attachment is ever
    /// sent again; the publication stays `Ambiguous` until the host settles
    /// it `Unknown`.
    #[serde(default)]
    pub(super) attachment_upload_ambiguous: bool,
    /// The extension generation this presentation was minted under.
    #[serde(default)]
    pub(super) generation: u64,
    /// When `agents.sessions.setStatus` last succeeded (RFC 3339).
    #[serde(default)]
    pub(super) status_asserted_at: Option<DateTime<Utc>>,
    /// Slack rejected the session shape (`thread_ts_required` /
    /// `thread_ts_not_allowed`): the stream still works for this reply, the
    /// session indicator is skipped.
    #[serde(default)]
    pub(super) session_unavailable: bool,
    /// A `chat.startStream` crossed into transport without an answer: Slack
    /// may or may not have created a streaming message, and the response's
    /// `ts` is the only handle Slack documents (no idempotency key, no way
    /// to list or locate streams — docs.slack.dev, verified 2026-08-31).
    /// While set, this sink never starts another stream and never posts the
    /// terminal text conventionally — either could duplicate content the
    /// ghost stream already shows — so the publication stays `Ambiguous`
    /// until the host settles it `Unknown`.
    #[serde(default)]
    pub(super) stream_open_ambiguous: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SlackSessionStatus {
    #[default]
    None,
    Processing,
    Suspended,
    Active,
    Closed,
}

impl SlackSessionStatus {
    pub(super) fn wire(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Processing => Some("processing"),
            Self::Suspended => Some("suspended"),
            Self::Active => Some("active"),
            Self::Closed => Some("closed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SlackStreamState {
    pub(super) channel: String,
    pub(super) ts: String,
    /// Chars of `document.answer.text` already appended.
    pub(super) appended_chars: u64,
    /// Fingerprint of that appended prefix; a document whose prefix no
    /// longer matches was rewritten under the stream and is re-presented.
    #[serde(default)]
    pub(super) appended_hash: String,
    pub(super) opened_at_revision: u64,
    /// A request that crossed into transport without an answer. Resolved by
    /// read-back before anything else is appended.
    #[serde(default)]
    pub(super) pending: Option<SlackAppliedState>,
}

/// The checkpoint fields one request establishes once it lands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SlackAppliedState {
    pub(super) to_chars: u64,
    #[serde(default)]
    pub(super) to_hash: String,
    #[serde(default)]
    pub(super) tasks: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) status_key: Option<String>,
    #[serde(default)]
    pub(super) plan_title_key: Option<String>,
    #[serde(default)]
    pub(super) attention_key: Option<String>,
    #[serde(default)]
    pub(super) closes_stream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SlackTerminalState {
    /// The terminal text is in Slack; attachments may still be pending.
    StreamClosed,
    /// Everything the terminal revision asked for is in Slack.
    Applied,
}

// ── Checkpoint codec ─────────────────────────────────────────────────────

pub(super) fn load_checkpoint(request: &ReplyReconcileRequest) -> SlackReplyCheckpoint {
    let fresh = || SlackReplyCheckpoint {
        generation: request.extension_generation,
        ..SlackReplyCheckpoint::default()
    };
    let Some(checkpoint) = request.checkpoint.as_ref() else {
        return fresh();
    };
    if checkpoint.version() != SLACK_REPLY_CHECKPOINT_VERSION {
        tracing::debug!(
            version = checkpoint.version(),
            "slack reply checkpoint version unknown; starting a fresh presentation"
        );
        return fresh();
    }
    match serde_json::from_str::<SlackReplyCheckpoint>(checkpoint.payload()) {
        Ok(parsed) if parsed.generation == request.extension_generation => parsed,
        Ok(parsed) => {
            tracing::debug!(
                checkpoint_generation = parsed.generation,
                request_generation = request.extension_generation,
                "slack reply checkpoint was minted under another generation; starting fresh"
            );
            fresh()
        }
        Err(error) => {
            tracing::debug!(
                error = %error,
                "slack reply checkpoint did not parse; starting a fresh presentation"
            );
            fresh()
        }
    }
}

/// Serialize within the host bound, evicting settled task fingerprints from
/// the lowest ordinal up (advancing the floor only past rows that are
/// terminal and have nothing unpublished below them).
pub(super) fn encode_checkpoint(
    checkpoint: &mut SlackReplyCheckpoint,
    document: &ReplyDocument,
) -> Result<ReplySinkCheckpoint, ReplyOutcomeReason> {
    loop {
        let payload = serde_json::to_string(checkpoint).map_err(|error| {
            ReplyOutcomeReason::new(format!("slack reply checkpoint did not serialize: {error}"))
        })?;
        if payload.len() <= REPLY_SINK_CHECKPOINT_MAX_BYTES {
            return ReplySinkCheckpoint::new(SLACK_REPLY_CHECKPOINT_VERSION, payload)
                .map_err(|error| ReplyOutcomeReason::new(error.to_string()));
        }
        let lowest = document
            .activities
            .iter()
            .filter(|activity| checkpoint.tasks.contains_key(activity.id.as_str()))
            .min_by_key(|activity| activity.started_ordinal);
        let Some(lowest) = lowest else {
            // Entries for rows the document no longer lists (impossible by
            // the reducer's contract, but never loop forever on it).
            let Some(stray) = checkpoint.tasks.keys().next().cloned() else {
                return Err(ReplyOutcomeReason::new(
                    "slack reply checkpoint exceeds the host bound with no evictable state",
                ));
            };
            checkpoint.tasks.remove(&stray);
            continue;
        };
        let nothing_unpublished_below = !document.activities.iter().any(|activity| {
            activity.started_ordinal >= checkpoint.tasks_floor_ordinal
                && activity.started_ordinal < lowest.started_ordinal
                && !checkpoint.tasks.contains_key(activity.id.as_str())
        });
        if lowest.state.is_terminal() && nothing_unpublished_below {
            checkpoint.tasks_floor_ordinal = lowest.started_ordinal.saturating_add(1);
        }
        let id = lowest.id.as_str().to_string();
        checkpoint.tasks.remove(&id);
    }
}

// ── Text helpers ─────────────────────────────────────────────────────────

/// The first `chars` chars of `text`, or `None` when the text is shorter.
pub(super) fn char_prefix(text: &str, chars: u64) -> Option<&str> {
    if chars == 0 {
        return Some("");
    }
    let mut seen = 0u64;
    for (index, _) in text.char_indices() {
        if seen == chars {
            return Some(&text[..index]); // safety: `index` comes from char_indices, a char boundary.
        }
        seen += 1;
    }
    (seen == chars).then_some(text)
}

/// Only the alphanumeric content: Slack renders markdown to mrkdwn, so
/// punctuation and formatting cannot be compared, letters and digits can.
pub(super) fn normalize_for_match(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).collect()
}

pub(super) fn normalized_tail(text: &str) -> String {
    let normalized = normalize_for_match(text);
    let total = normalized.chars().count();
    normalized
        .chars()
        .skip(total.saturating_sub(READ_BACK_TAIL_CHARS))
        .collect()
}

/// FNV-1a 64 over the parts (unit-separated), rendered as 16 hex chars. A
/// stable algorithm, because a checkpoint outlives the process that wrote
/// it.
pub(super) fn fingerprint(parts: &[&str]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0x1f)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}
