use std::ops::Range;

use ironclaw_safety::{InjectionScanner, LeakScanner};
use ironclaw_threads::MessageKind;

use super::{ANTI_INJECTION_PREFIX, CompactionError, ValidatedCompactionMessage};

const MAX_XML_ESCAPE_EXPANSION: usize = 5;

pub(super) struct SanitizedContent {
    pub(super) content: String,
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
            let sanitized =
                self.sanitize_retained_fragment(&message.body, Some(self.max_input_bytes))?;
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
        let sanitized = self.sanitize_retained_fragment(output_text, None)?;
        let content = format!(
            "{ANTI_INJECTION_PREFIX}<summary>{}</summary>",
            sanitized.content
        );
        self.validate_serialized_content(&content)?;

        Ok(SanitizedContent {
            content,
            redacted_leak_count: sanitized.redacted_leak_count,
        })
    }

    fn validate_unredacted_message_boundaries(
        &self,
        messages: &[ValidatedCompactionMessage],
    ) -> Result<(), CompactionError> {
        // Boundary validation must inspect the unredacted representation, but
        // its worst-case XML expansion is larger than the final model-input
        // cap. The actual sanitized serialization is still bounded separately
        // by `max_input_bytes` in `sanitize_messages`.
        let validation_cap = self
            .max_input_bytes
            .checked_mul(MAX_XML_ESCAPE_EXPANSION)
            .and_then(|expanded| expanded.checked_add(self.max_input_bytes))
            .ok_or(CompactionError::LeakRedactionFailed)?;
        let mut content = String::new();
        let mut body_ranges = Vec::with_capacity(messages.len());
        for message in messages {
            if !content.is_empty() {
                push_checked(&mut content, "\n", validation_cap)?;
            }
            let escaped_body = escape_xml_checked(&message.body, validation_cap)?;
            let body_start = content.len();
            push_checked(&mut content, &escaped_body, validation_cap)?;
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
        max_escaped_bytes: Option<usize>,
    ) -> Result<SanitizedContent, CompactionError> {
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
        let escaped = match max_escaped_bytes {
            Some(max_bytes) => escape_xml_checked(redacted_content, max_bytes)?,
            None => escape_xml(redacted_content),
        };
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
            tracing::error!(%error, "compaction leak match count exceeded telemetry bound");
            CompactionError::LeakRedactionFailed
        })?;
        let redacted = scan
            .redact_all_matches(content)
            .map_err(|error| {
                tracing::error!(%error, "compaction leak scanner returned an invalid range");
                CompactionError::LeakRedactionFailed
            })?
            .ok_or_else(|| {
                tracing::error!("non-clean compaction leak scan produced no redacted content");
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

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
