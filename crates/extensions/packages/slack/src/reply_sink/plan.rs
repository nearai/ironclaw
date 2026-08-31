//! What one request carries: the chunk plan the reconciler sends in a single
//! `chat.startStream` / `chat.appendStream` / `chat.stopStream` — task cards
//! whose fingerprint moved, the answer text past the appended char offset,
//! a status line while the answer is still empty, a new attention block, the
//! terminal note — and the checkpoint fields that request establishes once
//! it lands. Pure functions over the document and the checkpoint; no I/O.

use ironclaw_extension_contracts::reply::{
    ReplyActivity, ReplyActivityState, ReplyAttention, ReplyAttentionKind, ReplyDocument,
    ReplyOutcome,
};
use serde_json::{Value, json};

use super::SLACK_MARKDOWN_CHUNK_MAX_CHARS;
use super::checkpoint::{SlackAppliedState, SlackReplyCheckpoint, char_prefix, fingerprint};

// ── Chunk planning ───────────────────────────────────────────────────────

pub(super) struct ChunkPlan {
    pub(super) chunks: Vec<Value>,
    pub(super) applied: SlackAppliedState,
}

/// The document's answer is no longer an extension of what the stream shows.
#[derive(Debug)]
pub(super) struct AnswerRewritten;

/// Everything the checkpoint has not seen, as one `chunks` array: task cards
/// whose fingerprint moved, the answer text past `from_chars`, a status line
/// while the answer is still empty, a new attention block, and the terminal
/// note. Never slices inside a char.
pub(super) fn plan_chunks(
    document: &ReplyDocument,
    checkpoint: &SlackReplyCheckpoint,
    from_chars: u64,
    from_hash: &str,
    terminal_note: Option<&str>,
) -> Result<ChunkPlan, AnswerRewritten> {
    let text = document.answer.text.as_str();
    let Some(prefix) = char_prefix(text, from_chars) else {
        return Err(AnswerRewritten);
    };
    if from_chars > 0 && !from_hash.is_empty() && fingerprint(&[prefix]) != from_hash {
        return Err(AnswerRewritten);
    }
    let delta = &text[prefix.len()..];

    let mut chunks = Vec::new();
    let mut applied = SlackAppliedState {
        to_chars: from_chars,
        to_hash: from_hash.to_string(),
        ..SlackAppliedState::default()
    };

    for activity in &document.activities {
        let id = activity.id.as_str();
        let published = checkpoint.tasks.get(id);
        if published.is_none() && activity.started_ordinal < checkpoint.tasks_floor_ordinal {
            // Settled and evicted from the checkpoint: fully published.
            continue;
        }
        let fingerprint = task_fingerprint(activity);
        if published != Some(&fingerprint) {
            chunks.push(task_update_chunk(activity));
            applied.tasks.insert(id.to_string(), fingerprint);
        }
    }

    if !delta.is_empty() {
        for piece in markdown_pieces(delta) {
            chunks.push(json!({ "type": "markdown_text", "text": piece }));
        }
        applied.to_chars = from_chars + delta.chars().count() as u64;
        applied.to_hash = fingerprint(&[text]);
    }

    if text.is_empty()
        && terminal_note.is_none()
        && let Some(status) = &document.status
    {
        let key = fingerprint(&["status", status.as_str()]);
        if checkpoint.status_key.as_deref() != Some(key.as_str()) {
            chunks.push(json!({
                "type": "markdown_text",
                "text": format!("_{}_\n", status.as_str()),
            }));
            applied.status_key = Some(key);
        }
    }

    if let Some(attention) = &document.attention {
        let key = attention_fingerprint(attention);
        if checkpoint.attention_key.as_deref() != Some(key.as_str()) {
            chunks.push(json!({
                "type": "markdown_text",
                "text": attention_markdown(attention),
            }));
            applied.attention_key = Some(key);
        }
    }

    if let Some(note) = terminal_note {
        let separator = if applied.to_chars > 0 { "\n\n" } else { "" };
        chunks.push(json!({
            "type": "markdown_text",
            "text": format!("{separator}{note}"),
        }));
    }

    Ok(ChunkPlan { chunks, applied })
}

fn task_update_chunk(activity: &ReplyActivity) -> Value {
    let (status, fallback_details) = match &activity.state {
        ReplyActivityState::Started | ReplyActivityState::Running => ("in_progress", None),
        ReplyActivityState::Completed => ("complete", None),
        ReplyActivityState::Failed { kind } => ("error", Some(format!("Failed: {kind}"))),
        ReplyActivityState::Killed => ("error", Some("Stopped".to_string())),
    };
    let mut chunk = json!({
        "type": "task_update",
        "id": activity.id.as_str(),
        "title": activity.title.as_str(),
        "status": status,
    });
    let details = activity
        .detail
        .as_ref()
        .map(|detail| detail.as_str().to_string())
        .or(fallback_details);
    if let Some(details) = details {
        chunk["details"] = json!(details);
    }
    if let Some(output) = &activity.output_preview {
        chunk["output"] = json!(output.as_str());
    }
    chunk
}

pub(super) fn task_fingerprint(activity: &ReplyActivity) -> String {
    let (state, kind) = match &activity.state {
        ReplyActivityState::Started => ("started", ""),
        ReplyActivityState::Running => ("running", ""),
        ReplyActivityState::Completed => ("completed", ""),
        ReplyActivityState::Failed { kind } => ("failed", kind.as_str()),
        ReplyActivityState::Killed => ("killed", ""),
    };
    fingerprint(&[
        state,
        kind,
        activity.title.as_str(),
        activity
            .detail
            .as_ref()
            .map_or("", |detail| detail.as_str()),
        activity
            .output_preview
            .as_ref()
            .map_or("", |output| output.as_str()),
    ])
}

fn attention_fingerprint(attention: &ReplyAttention) -> String {
    fingerprint(&[
        attention_label(attention.kind),
        attention.headline.as_str(),
        attention.body.as_ref().map_or("", |body| body.as_str()),
        attention.action_url.as_ref().map_or("", |url| url.as_str()),
        attention.gate_ref.as_ref().map_or("", |gate| gate.as_str()),
    ])
}

fn attention_label(kind: ReplyAttentionKind) -> &'static str {
    match kind {
        ReplyAttentionKind::Approval => "Approval needed",
        ReplyAttentionKind::Auth => "Sign-in needed",
        ReplyAttentionKind::Resource => "Attention needed",
    }
}

/// A blockquote carrying the headline, the body, and — only when the host
/// disclosed one for this audience — the action URL.
fn attention_markdown(attention: &ReplyAttention) -> String {
    let mut out = format!(
        "\n> **{}:** {}\n",
        attention_label(attention.kind),
        attention.headline.as_str()
    );
    if let Some(body) = &attention.body {
        for line in body.as_str().lines() {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(url) = &attention.action_url {
        out.push_str("> ");
        out.push_str(url.as_str());
        out.push('\n');
    }
    out
}

pub(super) fn terminal_note(document: &ReplyDocument) -> Option<String> {
    match &document.outcome {
        Some(ReplyOutcome::Failed { summary }) => Some(format!("**Failed:** {}", summary.as_str())),
        Some(ReplyOutcome::Cancelled) => Some("_Stopped._".to_string()),
        Some(ReplyOutcome::Completed) | None => None,
    }
}

/// Split a delta into markdown chunks of at most
/// [`SLACK_MARKDOWN_CHUNK_MAX_CHARS`] chars, on char boundaries.
pub(super) fn markdown_pieces(delta: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for character in delta.chars() {
        current.push(character);
        count += 1;
        if count >= SLACK_MARKDOWN_CHUNK_MAX_CHARS {
            pieces.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}
