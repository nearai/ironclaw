use std::ops::Range;

use ironclaw_safety::{InjectionScanner, LeakScanner};
use ironclaw_threads::MessageKind;

use super::{ANTI_INJECTION_PREFIX, CompactionError, ValidatedCompactionMessage};

const SUMMARY_OPEN_TAG: &str = "<summary>";
const SUMMARY_CLOSE_TAG: &str = "</summary>";
// The canonical 128-message context window may additionally pin the accepted
// user task, so a valid compaction range can contain 129 transcript messages.
const MAX_UNTRUNCATED_TRANSCRIPT_MESSAGES: usize = 129;
const MAX_UNTRUNCATED_TOTAL_MESSAGES: usize = 257;
const MAX_UNTRUNCATED_COMPACTION_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct SanitizedContent {
    pub(super) content: String,
    pub(super) redacted_leak_count: u32,
}

pub(super) struct PreTruncationSanitizedMessages {
    pub(super) messages: Vec<ValidatedCompactionMessage>,
    pub(super) redacted_leak_count: u32,
}
struct LeakRedaction {
    content: Option<String>,
    count: u32,
}

pub(super) struct CompactionSanitizer<'a> {
    injection_scanner: &'a dyn InjectionScanner,
    leak_scanner: &'a dyn LeakScanner,
    max_input_bytes: usize,
}

impl<'a> CompactionSanitizer<'a> {
    pub(super) fn new(
        injection_scanner: &'a dyn InjectionScanner,
        leak_scanner: &'a dyn LeakScanner,
        max_input_bytes: usize,
    ) -> Self {
        Self {
            injection_scanner,
            leak_scanner,
            max_input_bytes,
        }
    }

    pub(super) fn sanitize_messages(
        &self,
        messages: &[ValidatedCompactionMessage],
    ) -> Result<SanitizedContent, CompactionError> {
        self.validate_unredacted_message_boundaries(messages)?;

        let mut content = String::new();
        let mut redacted_leak_count = 0_u32;
        for message in messages {
            let sanitized = self.sanitize_retained_fragment(&message.body, self.max_input_bytes)?;
            redacted_leak_count = redacted_leak_count
                .checked_add(sanitized.redacted_leak_count)
                .ok_or(CompactionError::LeakRedactionFailed)?;
            append_message_checked(
                &mut content,
                message.sequence,
                message.kind,
                &sanitized.content,
                self.max_input_bytes,
            )?;
        }
        self.validate_serialized_content(&content)?;

        Ok(SanitizedContent {
            content,
            redacted_leak_count,
        })
    }

    pub(super) fn sanitize_summary(
        &self,
        output_text: &str,
    ) -> Result<SanitizedContent, CompactionError> {
        let envelope_bytes = ANTI_INJECTION_PREFIX
            .len()
            .checked_add(SUMMARY_OPEN_TAG.len())
            .and_then(|bytes| bytes.checked_add(SUMMARY_CLOSE_TAG.len()))
            .ok_or(CompactionError::LeakRedactionFailed)?;
        let body_cap = self.max_input_bytes.checked_sub(envelope_bytes).ok_or(
            CompactionError::InputTooLarge {
                cap: self.max_input_bytes,
                observed_bytes: envelope_bytes,
            },
        )?;
        let sanitized = self.sanitize_retained_fragment(output_text, body_cap)?;
        let mut content = String::new();
        push_checked(&mut content, ANTI_INJECTION_PREFIX, self.max_input_bytes)?;
        push_checked(&mut content, SUMMARY_OPEN_TAG, self.max_input_bytes)?;
        push_checked(&mut content, &sanitized.content, self.max_input_bytes)?;
        push_checked(&mut content, SUMMARY_CLOSE_TAG, self.max_input_bytes)?;
        self.validate_serialized_content(&content)?;

        Ok(SanitizedContent {
            content,
            redacted_leak_count: sanitized.redacted_leak_count,
        })
    }

    pub(super) fn sanitize_messages_before_truncation(
        &self,
        messages: &[ValidatedCompactionMessage],
    ) -> Result<PreTruncationSanitizedMessages, CompactionError> {
        self.validate_unredacted_message_boundaries(messages)?;
        let mut sanitized_messages = Vec::new();
        sanitized_messages
            .try_reserve_exact(messages.len())
            .map_err(|error| {
                tracing::debug!(%error, "compaction pre-truncation message allocation failed");
                CompactionError::LeakRedactionFailed
            })?;
        let mut redacted_leak_count = 0_u32;
        for message in messages {
            let redaction = self.redact_leaks(&message.body)?;
            redacted_leak_count = redacted_leak_count
                .checked_add(redaction.count)
                .ok_or(CompactionError::LeakRedactionFailed)?;
            let body = redaction.content.unwrap_or_else(|| message.body.clone());
            if redaction.count > 0 && !self.injection_scanner.scan_injection(&body).is_empty() {
                return Err(CompactionError::InjectionDetected);
            }
            sanitized_messages.push(ValidatedCompactionMessage {
                sequence: message.sequence,
                kind: message.kind,
                body,
            });
        }
        Ok(PreTruncationSanitizedMessages {
            messages: sanitized_messages,
            redacted_leak_count,
        })
    }

    fn validate_unredacted_message_boundaries(
        &self,
        messages: &[ValidatedCompactionMessage],
    ) -> Result<(), CompactionError> {
        let transcript_message_count = messages
            .iter()
            .filter(|message| message.kind != MessageKind::Summary)
            .count();
        if transcript_message_count > MAX_UNTRUNCATED_TRANSCRIPT_MESSAGES {
            return Err(CompactionError::MessageCountExceeded {
                cap: MAX_UNTRUNCATED_TRANSCRIPT_MESSAGES,
                observed: transcript_message_count,
            });
        }
        if messages.len() > MAX_UNTRUNCATED_TOTAL_MESSAGES {
            return Err(CompactionError::MessageCountExceeded {
                cap: MAX_UNTRUNCATED_TOTAL_MESSAGES,
                observed: messages.len(),
            });
        }
        let validation_cap = messages.iter().try_fold(0_usize, |total, message| {
            let observed_bytes = total
                .checked_add(message.body.len())
                .and_then(|bytes| bytes.checked_add(1))
                .ok_or(CompactionError::InputTooLarge {
                    cap: MAX_UNTRUNCATED_COMPACTION_BYTES,
                    observed_bytes: usize::MAX,
                })?;
            if observed_bytes > MAX_UNTRUNCATED_COMPACTION_BYTES {
                return Err(CompactionError::InputTooLarge {
                    cap: MAX_UNTRUNCATED_COMPACTION_BYTES,
                    observed_bytes,
                });
            }
            Ok(observed_bytes)
        })?;

        // Inspect complete durable bodies before summarizer-only truncation.
        // Raw text is the trust boundary; XML escaping happens only after the
        // retained fragment is bounded.
        if let [message] = messages {
            if !self
                .injection_scanner
                .scan_injection(&message.body)
                .is_empty()
            {
                return Err(CompactionError::InjectionDetected);
            }
            let scan = self.leak_scanner.scan_leaks(&message.body);
            let body_range = 0..message.body.len();
            return validate_matches_stay_within_bodies(
                &message.body,
                std::slice::from_ref(&body_range),
                &scan.matches,
            );
        }

        let mut content = String::new();
        content.try_reserve_exact(validation_cap).map_err(|error| {
            tracing::debug!(%error, "compaction full-body validation allocation failed");
            CompactionError::LeakRedactionFailed
        })?;
        let mut body_ranges = Vec::with_capacity(messages.len());
        for message in messages {
            if !content.is_empty() {
                push_checked(&mut content, "\n", validation_cap)?;
            }
            let body_start = content.len();
            push_checked(&mut content, &message.body, validation_cap)?;
            body_ranges.push(body_start..content.len());
        }

        if !self.injection_scanner.scan_injection(&content).is_empty() {
            return Err(CompactionError::InjectionDetected);
        }
        let scan = self.leak_scanner.scan_leaks(&content);
        validate_matches_stay_within_bodies(&content, &body_ranges, &scan.matches)
    }

    fn sanitize_retained_fragment(
        &self,
        content: &str,
        max_escaped_bytes: usize,
    ) -> Result<SanitizedContent, CompactionError> {
        ensure_within_cap(content, max_escaped_bytes)?;
        if !self.injection_scanner.scan_injection(content).is_empty() {
            return Err(CompactionError::InjectionDetected);
        }

        let redaction = self.redact_leaks(content)?;
        let redacted_content = redaction.content.as_deref().unwrap_or(content);
        if redaction.count > 0
            && !self
                .injection_scanner
                .scan_injection(redacted_content)
                .is_empty()
        {
            return Err(CompactionError::InjectionDetected);
        }
        let escaped = escape_xml_checked(redacted_content, max_escaped_bytes)?;
        let escape_transformed_content = escaped != redacted_content;
        let escaped_redaction = if escape_transformed_content {
            self.redact_leaks(&escaped)?
        } else {
            LeakRedaction {
                content: None,
                count: 0,
            }
        };
        let escaped_content = escaped_redaction.content.unwrap_or(escaped);
        ensure_within_cap(&escaped_content, max_escaped_bytes)?;
        if (escape_transformed_content || escaped_redaction.count > 0)
            && !self
                .injection_scanner
                .scan_injection(&escaped_content)
                .is_empty()
        {
            return Err(CompactionError::InjectionDetected);
        }

        let redacted_leak_count = redaction
            .count
            .checked_add(escaped_redaction.count)
            .ok_or(CompactionError::LeakRedactionFailed)?;
        Ok(SanitizedContent {
            content: escaped_content,
            redacted_leak_count,
        })
    }

    fn redact_leaks(&self, content: &str) -> Result<LeakRedaction, CompactionError> {
        // Compaction is a retention boundary, so every declared action
        // (Block, Redact, or Warn) becomes deterministic value redaction.
        // Observability is the aggregate count returned to the loop; match
        // names, previews, ranges, and values never leave this method.
        let scan = self.leak_scanner.scan_leaks(content);
        if scan.is_clean() {
            return Ok(LeakRedaction {
                content: None,
                count: 0,
            });
        }
        let redacted_leak_count = u32::try_from(scan.matches.len()).map_err(|error| {
            tracing::debug!(%error, "compaction leak match count exceeded telemetry bound");
            CompactionError::LeakRedactionFailed
        })?;
        let redacted = scan
            .redact_all_matches(content)
            .map_err(|error| {
                tracing::debug!(%error, "compaction leak scanner returned an invalid range");
                CompactionError::LeakRedactionFailed
            })?
            .ok_or_else(|| {
                tracing::debug!("non-clean compaction leak scan produced no redacted content");
                CompactionError::LeakRedactionFailed
            })?;
        if !self.leak_scanner.scan_leaks(&redacted).is_clean() {
            return Err(CompactionError::LeakRedactionFailed);
        }
        Ok(LeakRedaction {
            content: Some(redacted),
            count: redacted_leak_count,
        })
    }

    fn validate_serialized_content(&self, content: &str) -> Result<(), CompactionError> {
        if !self.injection_scanner.scan_injection(content).is_empty() {
            return Err(CompactionError::InjectionDetected);
        }
        if !self.leak_scanner.scan_leaks(content).is_clean() {
            return Err(CompactionError::LeakRedactionFailed);
        }
        Ok(())
    }
}

fn validate_matches_stay_within_bodies(
    content: &str,
    body_ranges: &[Range<usize>],
    matches: &[ironclaw_safety::LeakMatch],
) -> Result<(), CompactionError> {
    for leak_match in matches {
        let location = &leak_match.location;
        if location.start >= location.end
            || location.end > content.len()
            || !content.is_char_boundary(location.start)
            || !content.is_char_boundary(location.end)
        {
            return Err(CompactionError::LeakRedactionFailed);
        }
        let candidate = body_ranges.partition_point(|body| body.end <= location.start);
        let Some(origin_body) = body_ranges
            .get(candidate)
            .filter(|body| body.start <= location.start)
        else {
            return Err(CompactionError::LeakRedactionFailed);
        };
        if location.end > origin_body.end {
            return Err(CompactionError::LeakRedactionFailed);
        }
    }
    Ok(())
}

fn append_message_checked(
    output: &mut String,
    sequence: u64,
    kind: MessageKind,
    escaped_body: &str,
    cap: usize,
) -> Result<(), CompactionError> {
    push_checked(output, "<message sequence=\"", cap)?;
    push_checked(output, &sequence.to_string(), cap)?;
    push_checked(output, "\" kind=\"", cap)?;
    push_checked(output, message_kind_name(kind), cap)?;
    push_checked(output, "\">", cap)?;
    push_checked(output, escaped_body, cap)?;
    push_checked(output, "</message>\n", cap)?;
    Ok(())
}

fn message_kind_name(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::User => "user",
        MessageKind::Assistant => "assistant",
        MessageKind::System => "system",
        MessageKind::Summary => "summary",
        MessageKind::CheckpointReference => "checkpoint_reference",
        MessageKind::ToolResultReference => "tool_result_reference",
        MessageKind::CapabilityDisplayPreview => "capability_display_preview",
    }
}

fn append_escaped_xml_checked(
    output: &mut String,
    value: &str,
    cap: usize,
) -> Result<(), CompactionError> {
    let mut run_start: Option<usize> = None;
    for (idx, character) in value.char_indices() {
        match character {
            '&' | '<' | '>' => {
                if let Some(start) = run_start.take() {
                    push_checked(output, &value[start..idx], cap)?;
                }
                let segment = if character == '&' {
                    "&amp;"
                } else if character == '<' {
                    "&lt;"
                } else {
                    "&gt;"
                };
                push_checked(output, segment, cap)?;
            }
            _ => {
                if run_start.is_none() {
                    run_start = Some(idx);
                }
            }
        }
    }
    if let Some(start) = run_start {
        push_checked(output, &value[start..], cap)?;
    }
    Ok(())
}

fn escape_xml_checked(value: &str, cap: usize) -> Result<String, CompactionError> {
    let mut escaped = String::new();
    append_escaped_xml_checked(&mut escaped, value, cap)?;
    Ok(escaped)
}

fn push_checked(output: &mut String, segment: &str, cap: usize) -> Result<(), CompactionError> {
    let observed_bytes = output.len().saturating_add(segment.len());
    if observed_bytes > cap {
        return Err(CompactionError::InputTooLarge {
            cap,
            observed_bytes,
        });
    }
    output.push_str(segment);
    Ok(())
}

fn ensure_within_cap(content: &str, cap: usize) -> Result<(), CompactionError> {
    if content.len() > cap {
        return Err(CompactionError::InputTooLarge {
            cap,
            observed_bytes: content.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_checked_accepts_exact_cap_and_rejects_one_over() {
        let mut output = String::from("abcd");

        assert_eq!(push_checked(&mut output, "ef", 6), Ok(()));
        assert_eq!(output, "abcdef");
        assert_eq!(
            push_checked(&mut output, "g", 6),
            Err(CompactionError::InputTooLarge {
                cap: 6,
                observed_bytes: 7,
            })
        );
    }
}
