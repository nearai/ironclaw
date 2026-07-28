use super::{
    AgentLoopHostError, AgentLoopHostErrorKind, LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES,
};

const MODEL_SAFE_SUMMARY_MAX_BYTES: usize = 4096;
const SENSITIVE_TERMS: &[SensitiveTerm] = &[
    sensitive_term("access token", true, true),
    sensitive_term("api key", true, true),
    sensitive_term("api_key", true, true),
    sensitive_term("api secret", true, true),
    sensitive_term("authorization", true, true),
    sensitive_term("bearer", true, true),
    sensitive_term("client secret", true, true),
    sensitive_term("invalid api key", true, false),
    sensitive_term("password", true, true),
    sensitive_term("passwd", true, true),
    sensitive_term("secret key", true, true),
    sensitive_term("secret-key", true, true),
    sensitive_term("secret token", true, true),
    sensitive_term("secret_token", true, true),
    sensitive_term("shared secret", true, true),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SensitiveTerm {
    /// Some terms, such as "invalid api key", are phrase-only so ordinary
    /// diagnostic prose after the phrase is not parsed as a credential value.
    phrase: &'static str,
    reject_as_phrase: bool,
    reject_value_after_label: bool,
}

const fn sensitive_term(
    phrase: &'static str,
    reject_as_phrase: bool,
    reject_value_after_label: bool,
) -> SensitiveTerm {
    SensitiveTerm {
        phrase,
        reject_as_phrase,
        reject_value_after_label,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PromptTextPolicy {
    /// Whether host-assembled content is denylisted for security vocabulary,
    /// host paths, and credential-shaped values. Structural and framing checks
    /// (empty/oversize content and control characters) are enforced separately
    /// on every surface and are NOT governed by this flag.
    ///
    /// Disabled only for [`PromptTextSurface::TrustedSkillInstruction`] and
    /// [`PromptTextSurface::VerifiedCatalogDescription`]. Trusted skill
    /// instruction bodies have exactly two provenances:
    /// first-party skills shipped in the repo `skills/` directory (installed
    /// into the trusted system-skill root) and user-placed local skills (the
    /// user `skills/` root). Registry/marketplace/URL skills are `Installed`,
    /// not trusted: they keep the full checks here and their body is withheld
    /// from the prompt entirely. This denylist false-positived constantly on
    /// legitimate skill docs describing OAuth headers, API keys, and host paths
    /// — failing the whole turn (#5169).
    ///
    /// Untrusted surfaces (memory snippets, runtime-context labels, generic
    /// model content, safe summaries) keep the full checks; those surfaces also
    /// have independent guards (`validate_loop_safe_summary`,
    /// `sanitize_prompt_string`, the skill-context validators, egress credential
    /// blocking), so this remains defense in depth rather than the sole control.
    enforce_content_checks: bool,
}

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

    const fn policy(self) -> PromptTextPolicy {
        PromptTextPolicy {
            enforce_content_checks: !matches!(
                self,
                Self::TrustedSkillInstruction | Self::VerifiedCatalogDescription
            ),
        }
    }
}

#[derive(Debug)]
pub(super) struct PromptTextValidationError {
    host_error: AgentLoopHostError,
    matched_pattern: Option<&'static str>,
}

impl PromptTextValidationError {
    pub(super) fn host_error(&self) -> &AgentLoopHostError {
        &self.host_error
    }

    pub(super) fn matched_pattern(&self) -> Option<&'static str> {
        self.matched_pattern
    }

    fn structural(host_error: AgentLoopHostError) -> Self {
        Self {
            host_error,
            matched_pattern: None,
        }
    }

    fn content(host_error: AgentLoopHostError, matched_pattern: &'static str) -> Self {
        Self {
            host_error,
            matched_pattern: Some(matched_pattern),
        }
    }
}

impl From<PromptTextValidationError> for AgentLoopHostError {
    fn from(error: PromptTextValidationError) -> Self {
        error.host_error
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
    // The content denylist (host paths, security vocabulary, credential-shaped
    // values) is skipped for trusted skill instructions; see PromptTextPolicy.
    if !surface.policy().enforce_content_checks {
        return Ok(value);
    }
    reject_sensitive_text(&value, label)?;
    Ok(value)
}

fn reject_sensitive_text(
    value: &str,
    label: &'static str,
) -> Result<(), PromptTextValidationError> {
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
            return non_model_safe(label, forbidden_path);
        }
    }
    for term in SENSITIVE_TERMS {
        if term.reject_as_phrase && contains_token_phrase(&lower, term.phrase) {
            return non_model_safe(label, term.phrase);
        }
        if term.reject_value_after_label
            && contains_credential_value_after_label(&lower, term.phrase)
        {
            return non_model_safe(label, term.phrase);
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return non_model_safe(label, "sk-");
    }
    Ok(())
}

fn contains_credential_value_after_label(value: &str, label: &str) -> bool {
    value.match_indices(label).any(|(start, matched)| {
        let end = start + matched.len();
        if !is_token_boundary(char_before(value, start)) || !is_token_boundary(char_at(value, end))
        {
            return false;
        }
        let suffix = &value[end..];
        credential_value_candidate(suffix).is_some_and(is_secret_like_token)
            || (label == "authorization"
                && authorization_scheme_value_candidate(suffix).is_some_and(is_secret_like_token))
    })
}

fn credential_value_candidate(suffix: &str) -> Option<&str> {
    credential_value_candidates(suffix).next()
}

fn authorization_scheme_value_candidate(suffix: &str) -> Option<&str> {
    let mut candidates = credential_value_candidates(suffix);
    let scheme = candidates.next()?;
    is_authorization_scheme(scheme)
        .then(|| candidates.next())
        .flatten()
}

fn credential_value_candidates(suffix: &str) -> impl Iterator<Item = &str> {
    suffix
        .trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, ':' | '=' | '\'' | '"' | '`')
        })
        .split(|character: char| character.is_ascii_whitespace() || matches!(character, ',' | ';'))
        .map(|candidate| {
            candidate.trim_matches(|character| {
                matches!(
                    character,
                    '\'' | '"'
                        | '`'
                        | '.'
                        | ','
                        | ';'
                        | ':'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                )
            })
        })
        .filter(|candidate| !candidate.is_empty())
}

fn is_authorization_scheme(candidate: &str) -> bool {
    ["basic", "bearer", "digest", "negotiate", "oauth", "token"].contains(&candidate)
}

fn is_secret_like_token(candidate: &str) -> bool {
    if candidate.starts_with('$') {
        return false;
    }
    if [
        "token",
        "secret",
        "password",
        "key",
        "value",
        "example",
        "placeholder",
        "redacted",
        "your-token",
        "your_token",
        "api-key",
        "api_key",
        "bearer",
    ]
    .contains(&candidate)
        || candidate.contains("redacted")
        || candidate.contains("placeholder")
        || candidate.contains("example")
        || candidate.contains("...")
    {
        return false;
    }
    if [
        "ghp_",
        "github_pat_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "akia",
        "asiai",
        "sk-",
        "pk_",
    ]
    .iter()
    .any(|prefix| candidate.starts_with(prefix))
    {
        return true;
    }
    let contains_alpha = candidate
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    let contains_digit = candidate
        .chars()
        .any(|character| character.is_ascii_digit());
    if candidate.len() >= 6 && contains_alpha && contains_digit {
        return true;
    }
    candidate.len() >= 16
        && candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
}
fn non_model_safe<T>(
    label: &'static str,
    matched_pattern: &'static str,
) -> Result<T, PromptTextValidationError> {
    Err(PromptTextValidationError::content(
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} contains non-model-safe content"),
        ),
        matched_pattern,
    ))
}

fn contains_token_phrase(value: &str, phrase: &str) -> bool {
    value.match_indices(phrase).any(|(start, matched)| {
        let end = start + matched.len();
        is_token_boundary(char_before(value, start)) && is_token_boundary(char_at(value, end))
    })
}

fn char_before(value: &str, byte_index: usize) -> Option<char> {
    value.get(..byte_index)?.chars().next_back()
}

fn char_at(value: &str, byte_index: usize) -> Option<char> {
    value.get(byte_index..)?.chars().next()
}

fn is_token_boundary(character: Option<char>) -> bool {
    match character {
        Some(character) => !character.is_ascii_alphanumeric() && character != '_',
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENSITIVE_SAMPLES: &[&str] = &[
        "Use the Authorization: Bearer ghp_secretvalue123 header.", // vocab + credential value
        "Read /Users/alice/.config/token first.",                   // host path
        "here is my key sk-abc123def456ghi789",                     // sk- token
    ];

    /// #5169: trusted/certified skill instruction content bypasses content
    /// denylisting (security vocabulary, host paths, credential-shaped values).
    #[test]
    fn trusted_skill_instruction_bypasses_content_denylist() {
        for sample in SENSITIVE_SAMPLES {
            validate_prompt_text(
                sample.to_string(),
                "skill content",
                PromptTextSurface::TrustedSkillInstruction,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "trusted skill content must bypass content checks; got {error:?}: {sample:?}"
                )
            });
        }
    }

    /// Untrusted surfaces keep the full content denylist — the trust gate is the
    /// only thing that relaxes it, so a non-skill surface still rejects the same
    /// samples a trusted skill is allowed to carry.
    #[test]
    fn untrusted_surfaces_still_reject_content_denylist() {
        for surface in [
            PromptTextSurface::GenericModelContent,
            PromptTextSurface::SafeSummary,
        ] {
            for sample in SENSITIVE_SAMPLES {
                let error = validate_prompt_text(sample.to_string(), "context content", surface)
                    .expect_err(&format!("untrusted surface must reject {sample:?}"));
                assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
            }
        }
    }

    /// Control characters corrupt prompt/log/terminal framing, so they are
    /// rejected on every surface — including trusted skill content. The trust
    /// gate relaxes only the vocabulary/path/credential content denylist.
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
