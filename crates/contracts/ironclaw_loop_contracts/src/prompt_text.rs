use super::{
    AgentLoopHostError, AgentLoopHostErrorKind, LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES,
};

const MODEL_SAFE_SUMMARY_MAX_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptTextSurface {
    SafeSummary,
    GenericModelContent,
    TrustedSkillInstruction,
    VerifiedCatalogDescription,
}

impl PromptTextSurface {
    const fn max_bytes(self) -> usize {
        match self {
            Self::SafeSummary | Self::VerifiedCatalogDescription => MODEL_SAFE_SUMMARY_MAX_BYTES,
            Self::GenericModelContent | Self::TrustedSkillInstruction => {
                LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct PromptTextValidationError {
    host_error: Box<AgentLoopHostError>,
}

impl PromptTextValidationError {
    pub(super) fn host_error(&self) -> &AgentLoopHostError {
        &self.host_error
    }

    fn structural(host_error: AgentLoopHostError) -> Self {
        Self {
            host_error: Box::new(host_error),
        }
    }
}

impl From<PromptTextValidationError> for AgentLoopHostError {
    fn from(error: PromptTextValidationError) -> Self {
        *error.host_error
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
    validate_prompt_text_with_diagnostics(value, label, surface).map_err(Into::into)
}

pub(super) fn validate_prompt_text_with_diagnostics(
    value: String,
    label: &'static str,
    surface: PromptTextSurface,
) -> Result<String, PromptTextValidationError> {
    // Structural and framing limits apply on every surface, even trusted skill
    // content: empty/oversize content and control characters (NUL/ESC/BEL,
    // which can corrupt prompt/log/terminal framing) are not the false-positive
    // class #5169 relaxes, so they are always rejected.
    if value.is_empty() || value.len() > surface.max_bytes() {
        return Err(PromptTextValidationError::structural(
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                format!("{label} is not model-safe"),
            ),
        ));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(PromptTextValidationError::structural(
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                format!("{label} contains control characters"),
            ),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDENTIAL_SAMPLES: &[&str] = &[
        "Use the Authorization: Bearer ghp_secretvalue123 header.",
        "Use the Authorization: Basic dXNlcjpwYXNz header.",
        "api key: abc123def456",
        "here is my key sk-abc123def456ghi789",
    ];
    const SECURITY_PROSE_SAMPLES: &[&str] = &[
        "The report documents an authorization flow and API key rotation.",
        "Read /Users/alice/.config/token before reviewing the report.",
        "The upstream service returned invalid API key.",
    ];

    #[test]
    fn prompt_text_validation_error_stays_below_large_error_threshold() {
        assert!(
            std::mem::size_of::<PromptTextValidationError>() <= 128,
            "prompt validation errors are returned by value and must stay below Clippy's large-error threshold"
        );
    }

    /// Credential handling belongs to the final model-input redaction boundary;
    /// prompt contracts enforce framing and size without rejecting the turn.
    #[test]
    fn every_surface_allows_credential_values_for_later_redaction() {
        for surface in [
            PromptTextSurface::TrustedSkillInstruction,
            PromptTextSurface::VerifiedCatalogDescription,
            PromptTextSurface::GenericModelContent,
            PromptTextSurface::SafeSummary,
        ] {
            for sample in CREDENTIAL_SAMPLES {
                validate_prompt_text(sample.to_string(), "context content", surface)
                    .unwrap_or_else(|error| {
                        panic!("credential content must reach the redaction boundary: {error:?}")
                    });
            }
        }
    }

    #[test]
    fn untrusted_surfaces_allow_security_prose_and_paths() {
        for surface in [
            PromptTextSurface::GenericModelContent,
            PromptTextSurface::SafeSummary,
        ] {
            for sample in SECURITY_PROSE_SAMPLES {
                validate_prompt_text(sample.to_string(), "context content", surface)
                    .unwrap_or_else(|error| {
                        panic!(
                            "ordinary security prose must remain usable; got {error:?}: {sample:?}"
                        )
                    });
            }
        }
    }

    /// Control characters corrupt prompt/log/terminal framing, so they are
    /// rejected on every surface — including trusted skill content. Surfaces
    /// now differ only in byte budget; credential values are handled by the
    /// provider-bound redaction pass.
    #[test]
    fn control_characters_are_rejected_on_all_surfaces() {
        for surface in [
            PromptTextSurface::TrustedSkillInstruction,
            PromptTextSurface::VerifiedCatalogDescription,
            PromptTextSurface::GenericModelContent,
            PromptTextSurface::SafeSummary,
        ] {
            for control in ['\0', '\u{001b}', '\u{0007}'] {
                let error = validate_prompt_text(
                    format!("control{control}inside content"),
                    "content",
                    surface,
                )
                .expect_err("control characters must be rejected on all surfaces");
                assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
            }
        }
    }

    /// Structural limits (empty, byte budget) apply to every surface, including
    /// trusted skill and verified catalog content.
    #[test]
    fn structural_limits_apply_on_every_surface() {
        for surface in [
            PromptTextSurface::TrustedSkillInstruction,
            PromptTextSurface::VerifiedCatalogDescription,
            PromptTextSurface::GenericModelContent,
            PromptTextSurface::SafeSummary,
        ] {
            let empty = validate_prompt_text(String::new(), "content", surface)
                .expect_err("empty content is rejected on every surface");
            assert_eq!(empty.kind, AgentLoopHostErrorKind::PolicyDenied);

            let oversized = "x".repeat(surface.max_bytes() + 1);
            let too_big = validate_prompt_text(oversized, "content", surface)
                .expect_err("oversized content is rejected on every surface");
            assert_eq!(too_big.kind, AgentLoopHostErrorKind::PolicyDenied);
        }
    }
}
