use super::{
    AgentLoopHostError, AgentLoopHostErrorKind, LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES,
};

const MODEL_SAFE_SUMMARY_MAX_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptTextSurface {
    SafeSummary,
    GenericModelContent,
    TrustedSkillInstruction,
}

impl PromptTextSurface {
    const fn max_bytes(self) -> usize {
        match self {
            Self::SafeSummary => MODEL_SAFE_SUMMARY_MAX_BYTES,
            Self::GenericModelContent | Self::TrustedSkillInstruction => {
                LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES
            }
        }
    }

    const fn allows_security_vocabulary(self) -> bool {
        matches!(self, Self::TrustedSkillInstruction)
    }
}

pub(super) fn validate_model_safe_text(
    value: String,
    label: &'static str,
) -> Result<String, AgentLoopHostError> {
    validate_prompt_text(value, label, PromptTextSurface::SafeSummary)
}

pub(super) fn validate_prompt_text(
    value: String,
    label: &'static str,
    surface: PromptTextSurface,
) -> Result<String, AgentLoopHostError> {
    if value.is_empty() || value.len() > surface.max_bytes() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} is not model-safe"),
        ));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} contains control characters"),
        ));
    }
    reject_sensitive_text(&value, label, surface)?;
    Ok(value)
}

fn reject_sensitive_text(
    value: &str,
    label: &'static str,
    surface: PromptTextSurface,
) -> Result<(), AgentLoopHostError> {
    let lower = value.to_ascii_lowercase();
    for forbidden_path in [
        "/users/",
        "/home/",
        "/private/",
        "/tmp/", // safety: model-safety denylist literal, not a filesystem temp path.
        "/var/",
        "/etc/",
    ] {
        if lower.contains(forbidden_path) {
            return non_model_safe(label);
        }
    }
    if !surface.allows_security_vocabulary() {
        for forbidden_phrase in [
            "access token",
            "api key",
            "api_key",
            "api secret",
            "authorization",
            "bearer",
            "client secret",
            "invalid api key",
            "password",
            "passwd",
            "secret key",
            "secret-key",
            "secret token",
            "secret_token",
            "shared secret",
        ] {
            if contains_token_phrase(&lower, forbidden_phrase) {
                return non_model_safe(label);
            }
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return non_model_safe(label);
    }
    Ok(())
}

fn non_model_safe<T>(label: &'static str) -> Result<T, AgentLoopHostError> {
    Err(AgentLoopHostError::new(
        AgentLoopHostErrorKind::PolicyDenied,
        format!("{label} contains non-model-safe content"),
    ))
}

fn contains_token_phrase(value: &str, phrase: &str) -> bool {
    value.match_indices(phrase).any(|(start, matched)| {
        let end = start + matched.len();
        is_token_boundary(char_before(value, start)) && is_token_boundary(char_at(value, end))
    })
}

fn char_before(value: &str, byte_index: usize) -> Option<char> {
    value
        .char_indices()
        .take_while(|(index, _)| *index < byte_index)
        .last()
        .map(|(_, character)| character)
}

fn char_at(value: &str, byte_index: usize) -> Option<char> {
    value
        .char_indices()
        .find(|(index, _)| *index == byte_index)
        .map(|(_, character)| character)
}

fn is_token_boundary(character: Option<char>) -> bool {
    match character {
        Some(character) => !character.is_ascii_alphanumeric() && character != '_',
        None => true,
    }
}
