use ironclaw_loop_contracts::{
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleArtifact,
    ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail, ToolObservationStatus,
};
use ironclaw_turns::LoopResultRef;

pub(super) const RESULT_PREVIEW_MAX_BYTES: usize = 3 * 1024;
const RESULT_OBSERVATION_MAX_BYTES: usize = 4 * 1024;
const RESULT_PREVIEW_ESCAPED_MAX_BYTES: usize = 2 * 1024;
const RESULT_PREVIEW_RETRY_BYTES: usize = 2 * 1024;

/// A bounded, UTF-8-safe first-look slice of a serialized result payload.
pub(super) struct FirstLookResultPreview {
    pub(super) text: String,
    /// `None` when `text` already covers the entire payload.
    next_offset: Option<u64>,
    /// The text is a shallow, self-describing JSON selection rather than the
    /// complete result or a raw byte prefix.
    structured_json_view: bool,
}

/// Build the inline first look from the exact bytes stored durably.
pub(super) fn first_look_result_preview(
    serialized: &[u8],
    result_ref: &str,
) -> Option<FirstLookResultPreview> {
    let Ok(full_text) = std::str::from_utf8(serialized) else {
        return None;
    };
    if full_text.len() <= RESULT_PREVIEW_MAX_BYTES {
        return Some(FirstLookResultPreview {
            text: full_text.to_string(),
            next_offset: None,
            structured_json_view: false,
        });
    }
    for budget in [RESULT_PREVIEW_MAX_BYTES, RESULT_PREVIEW_RETRY_BYTES] {
        match ironclaw_threads::render_json_tool_result_page(
            result_ref, serialized, "", 0, budget, None,
        ) {
            Ok(page) => match ironclaw_threads::model_result_preview_from_json_page(&page) {
                Ok(redacted) => {
                    let text = redacted.into_inner();
                    let escaped_fits = serde_json::to_vec(&text)
                        .is_ok_and(|encoded| encoded.len() <= RESULT_PREVIEW_ESCAPED_MAX_BYTES);
                    if text.len() > RESULT_PREVIEW_MAX_BYTES || !escaped_fits {
                        continue;
                    }
                    return Some(FirstLookResultPreview {
                        text,
                        // JSON continuation metadata is carried inside the
                        // self-describing page. The outer offset remains reserved
                        // for exact legacy byte continuation.
                        next_offset: None,
                        structured_json_view: true,
                    });
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "large JSON first-look redaction failed; retaining legacy byte continuation"
                    );
                }
            },
            Err(ironclaw_threads::ToolResultRecordReadError::MalformedStoredJson { .. }) => break,
            Err(ironclaw_threads::ToolResultRecordReadError::JsonViewBudgetTooSmall { .. }) => {
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "large JSON first-look rendering failed; retaining legacy byte continuation"
                );
                break;
            }
        }
        break;
    }
    // Legacy byte previews can expand when replay applies credential masking.
    // Leave the same bounded headroom used by the structured retry so the
    // reconstructed preview still fits the independent first-look budget.
    let end = floor_char_boundary(full_text, RESULT_PREVIEW_RETRY_BYTES);
    Some(FirstLookResultPreview {
        text: full_text[..end].to_string(),
        next_offset: Some(end as u64),
        structured_json_view: false,
    })
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    let mut index = index;
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn truncated_preview_summary(next_offset: u64, item_count: Option<u64>) -> String {
    let base = format!(
        "Tool completed; preview truncated, use result_read with the result \
         reference and offset {next_offset} for more output."
    );
    match item_count {
        Some(count) => format!("{base} Full result is a JSON array of {count} items."),
        None => base,
    }
}

pub(super) fn result_reference_observation(
    result_ref: &LoopResultRef,
    byte_len: u64,
    preview: Option<FirstLookResultPreview>,
    item_count: Option<u64>,
) -> ModelVisibleToolObservation {
    let (summary, preview_text, structured_json_view, total_bytes, next_offset, item_count) =
        match preview {
            Some(FirstLookResultPreview {
                text,
                structured_json_view: true,
                ..
            }) => {
                let summary = match item_count {
                Some(count) => format!(
                    "Tool completed; preview is a bounded JSON view of an array with {count} items. Select omitted nodes or use the preview's next request to continue."
                ),
                None => "Tool completed; preview is a bounded JSON view. Select omitted nodes or use the preview's next request to continue.".to_string(),
            };
                // Selection continuation stays inside the page; the outer total
                // still reports the durable provider-result size.
                (summary, Some(text), true, Some(byte_len), None, None)
            }
            Some(FirstLookResultPreview {
                text,
                next_offset: Some(next_offset),
                structured_json_view: false,
            }) => (
                truncated_preview_summary(next_offset, item_count),
                Some(text),
                false,
                Some(byte_len),
                Some(next_offset),
                item_count,
            ),
            Some(FirstLookResultPreview {
                text,
                next_offset: None,
                structured_json_view: false,
            }) => (
                "Tool completed; preview contains the full result.".to_string(),
                Some(text),
                false,
                Some(byte_len),
                None,
                None,
            ),
            None => (
                "Tool completed; use result_read with the result reference for more output."
                    .to_string(),
                None,
                false,
                None,
                None,
                None,
            ),
        };
    let mut observation = ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary,
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.as_str().to_string(),
            byte_len,
            preview: preview_text,
            structured_json_view,
            total_bytes,
            next_offset,
            item_count,
        },
        artifacts: vec![ModelVisibleArtifact {
            artifact_ref: result_ref.as_str().to_string(),
            summary: "Stored tool result".to_string(),
        }],
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    };
    if !serde_json::to_vec(&observation)
        .is_ok_and(|encoded| encoded.len() <= RESULT_OBSERVATION_MAX_BYTES)
    {
        tracing::warn!(
            max_bytes = RESULT_OBSERVATION_MAX_BYTES,
            "automatic tool-result observation exceeded its budget; falling back to reference-only"
        );
        observation.summary =
            "Tool completed; use result_read with the result reference for more output."
                .to_string();
        if let ToolObservationDetail::ResultReference {
            preview,
            structured_json_view,
            total_bytes,
            next_offset,
            item_count,
            ..
        } = &mut observation.detail
        {
            *preview = None;
            *structured_json_view = false;
            *total_bytes = None;
            *next_offset = None;
            *item_count = None;
        }
    }
    observation
}
