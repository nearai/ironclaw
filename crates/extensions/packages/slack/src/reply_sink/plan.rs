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

use super::checkpoint::{SlackAppliedState, SlackReplyCheckpoint, char_prefix, fingerprint};
use super::{SLACK_MARKDOWN_CHUNK_MAX_CHARS, SLACK_TASK_FIELD_MAX_CHARS};

const RUN_TASK_ID: &str = "ironclaw-run";
const RUN_TASK_TITLE: &str = "IronClaw run";

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

    let mut task_chunks = Vec::new();
    append_hidden_run_task(document, checkpoint, &mut task_chunks, &mut applied);
    for activity in &document.activities {
        let id = activity.id.as_str();
        let published = checkpoint.tasks.get(id);
        if published.is_none() && activity.started_ordinal < checkpoint.tasks_floor_ordinal {
            // Settled and evicted from the checkpoint: fully published.
            continue;
        }
        let fingerprint = rendered_task_fingerprint(activity, document.is_terminal());
        if published != Some(&fingerprint) {
            task_chunks.extend(task_update_chunks(
                activity,
                document.is_terminal(),
                published.is_some(),
            ));
            applied.tasks.insert(id.to_string(), fingerprint);
        }
    }

    append_plan_title(document, checkpoint, &mut chunks, &mut applied);
    chunks.extend(task_chunks);

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

fn append_hidden_run_task(
    document: &ReplyDocument,
    checkpoint: &SlackReplyCheckpoint,
    task_chunks: &mut Vec<Value>,
    applied: &mut SlackAppliedState,
) {
    let status = match &document.outcome {
        Some(ReplyOutcome::Completed) => "complete",
        Some(ReplyOutcome::Failed { .. } | ReplyOutcome::Cancelled) => "error",
        None => "in_progress",
    };
    let rendered = fingerprint(&["hidden-run", status]);
    if checkpoint.tasks.get(RUN_TASK_ID) == Some(&rendered) {
        return;
    }
    task_chunks.push(json!({
        "type": "task_update",
        "id": RUN_TASK_ID,
        "title": RUN_TASK_TITLE,
        "hide_title": true,
        "status": status,
    }));
    applied.tasks.insert(RUN_TASK_ID.to_string(), rendered);
}

fn append_plan_title(
    document: &ReplyDocument,
    checkpoint: &SlackReplyCheckpoint,
    chunks: &mut Vec<Value>,
    applied: &mut SlackAppliedState,
) {
    let title = plan_title(document);
    let key = fingerprint(&["plan", title]);
    if checkpoint.plan_title_key.as_deref() == Some(key.as_str()) {
        return;
    }
    chunks.push(json!({ "type": "plan_update", "title": title }));
    applied.plan_title_key = Some(key);
}

fn plan_title(document: &ReplyDocument) -> &'static str {
    match &document.outcome {
        Some(ReplyOutcome::Completed) => "Thinking completed",
        Some(ReplyOutcome::Failed { .. }) => "Thinking failed",
        Some(ReplyOutcome::Cancelled) => "Thinking stopped",
        None if document.attention.is_some() => "Thinking paused",
        None => "Thinking",
    }
}

fn slack_task_field(value: &str) -> String {
    value.chars().take(SLACK_TASK_FIELD_MAX_CHARS).collect()
}

fn rendered_task_state(activity: &ReplyActivity, terminal: bool) -> (&'static str, Option<String>) {
    match &activity.state {
        ReplyActivityState::Started if terminal => (
            "error",
            Some("Did not finish before the run ended".to_string()),
        ),
        ReplyActivityState::Started => ("in_progress", None),
        ReplyActivityState::Completed => ("complete", None),
        ReplyActivityState::Failed { kind } => ("error", Some(format!("Failed: {kind}"))),
    }
}

fn task_update_chunks(
    activity: &ReplyActivity,
    terminal: bool,
    input_already_published: bool,
) -> Vec<Value> {
    let (status, fallback_details) = rendered_task_state(activity, terminal);
    let mut chunk = json!({
        "type": "task_update",
        "id": activity.id.as_str(),
        "title": slack_task_field(activity.title.as_str()),
        "status": status,
    });
    // Slack appends repeated task details instead of visually replacing them.
    // Send immutable input arguments on the first update only. A later error
    // detail is new information and always takes precedence.
    let details = fallback_details
        .map(|details| ("Details", details))
        .or_else(|| {
            (!input_already_published)
                .then_some(activity.detail.as_ref())
                .flatten()
                .map(|detail| ("Arguments", detail.as_str().to_string()))
        });
    let mut overflow = Vec::new();
    if let Some((label, details)) = details {
        if let Some(payload) = compact_task_payload(&details) {
            chunk["details"] = json!(payload);
        } else {
            overflow.push(rich_text_payload_chunk(label, &details));
        }
    }
    let mut chunks = Vec::with_capacity(1 + overflow.len());
    chunks.push(chunk);
    chunks.extend(overflow);
    chunks
}

fn compact_task_payload(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.starts_with('{') && !value.starts_with('[') {
        return (value.chars().count() <= SLACK_TASK_FIELD_MAX_CHARS).then(|| value.to_string());
    }

    const PREFIX: &str = "```json\n";
    const SUFFIX: &str = "\n```";
    let rendered = format!("{PREFIX}{value}{SUFFIX}");
    (rendered.chars().count() <= SLACK_TASK_FIELD_MAX_CHARS).then_some(rendered)
}

fn rich_text_payload_chunk(label: &str, value: &str) -> Value {
    let mut preformatted = json!({
        "type": "rich_text_preformatted",
        "elements": [{ "type": "text", "text": value.trim() }],
    });
    if value.trim().starts_with('{') || value.trim().starts_with('[') {
        preformatted["language"] = json!("json");
    }
    json!({
        "type": "blocks",
        "blocks": [{
            "type": "rich_text",
            "elements": [
                {
                    "type": "rich_text_section",
                    "elements": [{
                        "type": "text",
                        "text": label,
                        "style": { "bold": true },
                    }],
                },
                preformatted,
            ],
        }],
    })
}

#[cfg(test)]
pub(super) fn task_fingerprint(activity: &ReplyActivity) -> String {
    rendered_task_fingerprint(activity, false)
}

fn rendered_task_fingerprint(activity: &ReplyActivity, terminal: bool) -> String {
    let (rendered_state, fallback_details) = rendered_task_state(activity, terminal);
    let (state, kind) = match &activity.state {
        ReplyActivityState::Started => ("started", ""),
        ReplyActivityState::Completed => ("completed", ""),
        ReplyActivityState::Failed { kind } => ("failed", kind.as_str()),
    };
    fingerprint(&[
        rendered_state,
        state,
        kind,
        activity.title.as_str(),
        activity.detail.as_ref().map_or_else(
            || fallback_details.as_deref().unwrap_or(""),
            |detail| detail.as_str(),
        ),
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
