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
use super::{
    SLACK_MARKDOWN_CHUNK_MAX_CHARS, SLACK_TASK_FIELD_MAX_CHARS, SLACK_TEXT_HOLD_MAX_CHARS,
};

// ── Chunk planning ───────────────────────────────────────────────────────

pub(super) struct ChunkPlan {
    pub(super) chunks: Vec<Value>,
    pub(super) applied: SlackAppliedState,
}

/// The document's answer is no longer an extension of what the stream shows.
#[derive(Debug)]
pub(super) struct AnswerRewritten;

/// Everything the checkpoint has not seen, as one `chunks` array: task cards
/// whose fingerprint moved (and the plan header, once a real task exists),
/// the answer text past `from_chars` — by whole paragraph while the run is
/// live, because Slack renders every markdown chunk as its own block, and in
/// full at the terminal — a status line while the answer is still empty, a
/// new attention block, and the terminal note. Never slices inside a char.
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

    // The plan header exists only alongside real task cards: Slack renders a
    // header without a task as an empty message, and a tool-less answer needs
    // no header — its thinking state is the session status.
    if !task_chunks.is_empty()
        || !checkpoint.tasks.is_empty()
        || checkpoint.plan_title_key.is_some()
    {
        append_plan_title(document, checkpoint, &mut chunks, &mut applied);
    }
    chunks.extend(task_chunks);

    // Held text flushes when the answer is complete (terminal, or a canonical
    // finalized text) and before an attention block, which belongs after
    // whatever the run had already said.
    let attention_is_new = document.attention.as_ref().is_some_and(|attention| {
        checkpoint.attention_key.as_deref() != Some(attention_fingerprint(attention).as_str())
    });
    let publish = if document.is_terminal() || document.answer.finalized || attention_is_new {
        delta
    } else {
        &delta[..publishable_len(prefix, delta)]
    };
    if !publish.is_empty() {
        for piece in markdown_pieces(publish) {
            chunks.push(json!({ "type": "markdown_text", "text": piece }));
        }
        applied.to_chars = from_chars + publish.chars().count() as u64;
        // The hash covers exactly what Slack shows, so the next plan still
        // detects a rewrite of the shown prefix.
        applied.to_hash = fingerprint(&[&text[..prefix.len() + publish.len()]]);
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
        let key = fingerprint(&["note", note]);
        if checkpoint.note_key.as_deref() != Some(key.as_str()) {
            let separator = if applied.to_chars > 0 { "\n\n" } else { "" };
            chunks.push(json!({
                "type": "markdown_text",
                "text": format!("{separator}{note}"),
            }));
            applied.note_key = Some(key);
        }
    }

    Ok(ChunkPlan { chunks, applied })
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

/// How much of `delta` is complete enough to show: through its last
/// paragraph boundary (a blank line outside a code fence), or — once the
/// unfinished paragraph has passed [`SLACK_TEXT_HOLD_MAX_CHARS`] with no
/// blank line — through its last sentence end. Fence state carries over from
/// `prefix`, the text already shown, so a blank line inside a fence opened
/// earlier is never a boundary. Zero when nothing is complete yet.
pub(super) fn publishable_len(prefix: &str, delta: &str) -> usize {
    let mut in_fence = fence_state(prefix);
    let mut paragraph_end = 0usize;
    let mut sentence_end = 0usize;
    let mut offset = 0usize;
    for line in delta.split_inclusive('\n') {
        let end = offset + line.len();
        if is_fence_line(line) {
            in_fence = !in_fence;
        } else if !in_fence {
            if line.trim().is_empty() {
                paragraph_end = end;
            }
            let mut chars = line.char_indices().peekable();
            while let Some((index, character)) = chars.next() {
                if matches!(character, '.' | '!' | '?')
                    && let Some(&(_, next)) = chars.peek()
                    && next.is_whitespace()
                {
                    sentence_end = offset + index + character.len_utf8() + next.len_utf8();
                }
            }
        }
        offset = end;
    }
    if paragraph_end > 0 {
        paragraph_end
    } else if delta.chars().count() > SLACK_TEXT_HOLD_MAX_CHARS {
        sentence_end
    } else {
        0
    }
}

/// Whether `text` ends inside a fenced code block.
fn fence_state(text: &str) -> bool {
    text.split_inclusive('\n')
        .filter(|line| is_fence_line(line))
        .count()
        % 2
        == 1
}

fn is_fence_line(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    line.len() - trimmed.len() <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~"))
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
