//! Telegram protocol target encoding and text chunking.
//!
//! Product views are reduced to [`ironclaw_extension_contracts::channel_adapter::OutboundPart`]
//! before they reach this package. The adapter in `channel.rs` owns Bot API
//! request rendering; this module keeps only the reusable Telegram protocol
//! primitives shared with preference-target decoding.

use ironclaw_host_api::turn::ReplyTargetBindingRef;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramRenderError {
    #[error("reply target {target} did not parse as Telegram chat#message: {reason}")]
    InvalidReplyTarget { target: String, reason: String },
}

/// Reply-target encoding used by Telegram outbound:
/// `tg:<chat_id>:<topic_id|_>:<reply_message_id|_>`.
pub fn parse_reply_target(
    target: &ReplyTargetBindingRef,
) -> Result<TelegramReplyTarget, TelegramRenderError> {
    let raw = target.as_str();
    let stripped = raw
        .strip_prefix("tg:")
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing tg: prefix".to_string(),
        })?;
    let mut segments = stripped.split(':');
    let chat_id = segments
        .next()
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing chat_id segment".to_string(),
        })?;
    let topic_segment = segments
        .next()
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing topic segment".to_string(),
        })?;
    let reply_segment = segments
        .next()
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing reply_message_id segment".to_string(),
        })?;
    if segments.next().is_some() {
        return Err(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "extra segments after reply_message_id".to_string(),
        });
    }

    let chat_id =
        chat_id
            .parse::<i64>()
            .map_err(|error| TelegramRenderError::InvalidReplyTarget {
                target: raw.to_string(),
                reason: format!("chat_id parse: {error}"),
            })?;
    let topic_id = parse_optional_id(raw, topic_segment, "topic_id")?;
    let reply_message_id = parse_optional_id(raw, reply_segment, "reply_message_id")?;
    Ok(TelegramReplyTarget {
        chat_id,
        topic_id,
        reply_message_id,
    })
}

fn parse_optional_id(
    target: &str,
    segment: &str,
    label: &'static str,
) -> Result<Option<i64>, TelegramRenderError> {
    if segment == "_" {
        return Ok(None);
    }
    segment
        .parse::<i64>()
        .map(Some)
        .map_err(|error| TelegramRenderError::InvalidReplyTarget {
            target: target.to_string(),
            reason: format!("{label} parse: {error}"),
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelegramReplyTarget {
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub reply_message_id: Option<i64>,
}

pub fn build_reply_target_binding(
    chat_id: i64,
    topic_id: Option<i64>,
    reply_message_id: Option<i64>,
) -> Option<ReplyTargetBindingRef> {
    let topic = topic_id.map_or_else(|| "_".to_string(), |id| id.to_string());
    let reply = reply_message_id.map_or_else(|| "_".to_string(), |id| id.to_string());
    ReplyTargetBindingRef::new(format!("tg:{chat_id}:{topic}:{reply}")).ok()
}

/// Telegram limits message text to 4096 UTF-16 code units.
pub const TELEGRAM_MESSAGE_MAX_UTF16_UNITS: usize = 4096;

/// Split without tearing surrogate pairs. Concatenation reproduces `text`.
pub(crate) fn chunk_text_utf16(text: &str, max_units: usize) -> Vec<&str> {
    if text.is_empty() {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut units = 0usize;
    for (offset, ch) in text.char_indices() {
        let ch_units = ch.len_utf16();
        if units + ch_units > max_units && units > 0 {
            chunks.push(&text[start..offset]);
            start = offset;
            units = 0;
        }
        units += ch_units;
    }
    chunks.push(&text[start..]);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_target_round_trips_and_rejects_trailing_segments() {
        let target = build_reply_target_binding(-100, Some(7), Some(42)).expect("target");
        assert_eq!(
            parse_reply_target(&target).expect("parse"),
            TelegramReplyTarget {
                chat_id: -100,
                topic_id: Some(7),
                reply_message_id: Some(42),
            }
        );
        let trailing = ReplyTargetBindingRef::new("tg:1:_:2:extra").expect("bounded ref");
        assert!(parse_reply_target(&trailing).is_err());
    }

    #[test]
    fn utf16_chunks_are_bounded_lossless_and_surrogate_safe() {
        let text = format!("{}{}", "x".repeat(4095), "🦀".repeat(2));
        let chunks = chunk_text_utf16(&text, TELEGRAM_MESSAGE_MAX_UTF16_UNITS);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.encode_utf16().count() <= TELEGRAM_MESSAGE_MAX_UTF16_UNITS)
        );
        assert_eq!(chunks.concat(), text);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.is_char_boundary(chunk.len()))
        );
    }
}
