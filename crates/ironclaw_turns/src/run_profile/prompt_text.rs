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
}

pub(super) fn validate_model_safe_text(
    value: String,
    label: &'static str,
) -> Result<String, AgentLoopHostError> {
    validate_prompt_text(value, label, PromptTextSurface::SafeSummary)
}

/// Validates host-assembled text bound for the model prompt.
///
/// This enforces only *structural* safety: non-empty, within the surface byte
/// budget, and free of control characters. Content-based secret/vocabulary
/// denylisting was removed — it false-positived constantly on ordinary skill
/// and tool docs (which mention "authorization", "api key", host paths, etc.)
/// and protected nothing that credential injection and egress credential
/// blocking don't already cover. See #5169.
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
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #5169: host-assembled content (skill docs, instructions, memory) may freely
    /// contain security vocabulary, host paths, and even credential-shaped values.
    /// The prompt validator no longer denylists content — secrets are guarded by
    /// credential injection + egress blocking, not here.
    #[test]
    fn content_is_not_denylisted_only_structurally_validated() {
        for content in [
            "Never construct Authorization headers manually; the system injects them.",
            "Provide your api key in settings, then we use the bearer token for you.",
            "Reset your password from the account page.",
            "Build artifacts go to /tmp/build and config lives in /etc/myapp.",
            "authorization: Bearer ey9aZ1c2d3e4f5g6h7",
            "api key: AKIA1234567890ABCD",
            "here is my key sk-abc123def456ghi789",
        ] {
            validate_prompt_text(
                content.to_string(),
                "context snippet content",
                PromptTextSurface::GenericModelContent,
            )
            .unwrap_or_else(|error| {
                panic!("content must pass structural validation; got {error:?}: {content:?}")
            });
        }
    }

    /// Structural limits still apply: control characters are rejected.
    #[test]
    fn control_characters_are_still_rejected() {
        let error = validate_prompt_text(
            "bad\u{0007}content".to_string(),
            "context snippet content",
            PromptTextSurface::GenericModelContent,
        )
        .expect_err("control characters must be rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    }

    /// Structural limits still apply: empty content is rejected.
    #[test]
    fn empty_content_is_rejected() {
        let error = validate_prompt_text(
            String::new(),
            "context snippet content",
            PromptTextSurface::GenericModelContent,
        )
        .expect_err("empty content must be rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    }
}
