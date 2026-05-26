use chrono::{DateTime, Utc};
use ironclaw_host_api::{CapabilityId, InvocationId};
use serde::{Deserialize, Serialize};

const CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES: usize = 2 * 1024;
const CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES: usize = 16 * 1024;
const CAPABILITY_DISPLAY_PREVIEW_MAX_LINES: usize = 120;
const CAPABILITY_DISPLAY_KIND_MAX_BYTES: usize = 32;
const CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDisplayPreviewStatus {
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDisplayPreviewEnvelope {
    pub version: u32,
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub status: CapabilityDisplayPreviewStatus,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ref: Option<String>,
    pub truncated: bool,
    pub updated_at: DateTime<Utc>,
}

impl CapabilityDisplayPreviewEnvelope {
    pub fn new(input: CapabilityDisplayPreviewEnvelopeInput) -> Result<Self, String> {
        let envelope = Self {
            version: 1,
            invocation_id: input.invocation_id,
            capability_id: input.capability_id,
            status: input.status,
            title: input.title,
            subtitle: input.subtitle,
            input_summary: input.input_summary,
            output_summary: input.output_summary,
            output_preview: input.output_preview,
            output_kind: input.output_kind,
            output_bytes: input.output_bytes,
            result_ref: input.result_ref,
            truncated: input.truncated,
            updated_at: input.updated_at,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_bounded_text(
            "capability display title",
            &self.title,
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability display subtitle",
            self.subtitle.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability display input summary",
            self.input_summary.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability display output summary",
            self.output_summary.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_output_preview(self.output_preview.as_deref())?;
        validate_output_kind(self.output_kind.as_deref())?;
        validate_optional_display_text(
            "capability display result ref",
            self.result_ref.as_deref(),
            CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDisplayPreviewEnvelopeInput {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub status: CapabilityDisplayPreviewStatus,
    pub title: String,
    pub subtitle: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub output_preview: Option<String>,
    pub output_kind: Option<String>,
    pub output_bytes: Option<u64>,
    pub result_ref: Option<String>,
    pub truncated: bool,
    pub updated_at: DateTime<Utc>,
}

fn validate_optional_display_text(
    label: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), String> {
    if let Some(value) = value {
        validate_bounded_text(label, value, max)?;
    }
    Ok(())
}

fn validate_bounded_text(label: &'static str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max {
        return Err(format!("{label} must be at most {max} bytes"));
    }
    if value.chars().any(|character| {
        character == '\0' || character.is_control() && character != '\n' && character != '\t'
    }) {
        return Err(format!(
            "{label} must not contain unsupported control characters"
        ));
    }
    Ok(())
}

fn validate_output_preview(value: Option<&str>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_bounded_text(
        "capability display output preview",
        value,
        CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES,
    )?;
    if value
        .lines()
        .nth(CAPABILITY_DISPLAY_PREVIEW_MAX_LINES)
        .is_some()
    {
        return Err(format!(
            "capability display output preview must be at most {CAPABILITY_DISPLAY_PREVIEW_MAX_LINES} lines"
        ));
    }
    Ok(())
}

fn validate_output_kind(value: Option<&str>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_bounded_text(
        "capability display output kind",
        value,
        CAPABILITY_DISPLAY_KIND_MAX_BYTES,
    )?;
    if !value.as_bytes()[0].is_ascii_lowercase()
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && byte != b'_')
    {
        return Err("capability display output kind must be snake_case ASCII".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_host_api::{CapabilityId, InvocationId};

    use crate::{
        CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
        CapabilityDisplayPreviewStatus,
    };

    #[test]
    fn preview_envelope_rejects_raw_control_characters() {
        let mut input = preview_input();
        input.output_preview = Some("line\u{0}break".to_string());

        assert!(CapabilityDisplayPreviewEnvelope::new(input).is_err());
    }

    #[test]
    fn preview_envelope_accepts_multiline_preview() {
        let mut input = preview_input();
        input.output_preview = Some("line one\nline two".to_string());

        CapabilityDisplayPreviewEnvelope::new(input).expect("multiline preview is safe");
    }

    fn preview_input() -> CapabilityDisplayPreviewEnvelopeInput {
        CapabilityDisplayPreviewEnvelopeInput {
            invocation_id: InvocationId::new(),
            capability_id: CapabilityId::new("demo.echo").expect("capability id"),
            status: CapabilityDisplayPreviewStatus::Completed,
            title: "echo".to_string(),
            subtitle: None,
            input_summary: Some("{\"message\":\"hello\"}".to_string()),
            output_summary: Some("text output".to_string()),
            output_preview: Some("hello".to_string()),
            output_kind: Some("text".to_string()),
            output_bytes: Some(5),
            result_ref: Some("result:demo".to_string()),
            truncated: false,
            updated_at: Utc::now(),
        }
    }
}
