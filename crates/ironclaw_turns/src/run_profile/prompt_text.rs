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
    /// host paths, credential-shaped values, and control characters.
    ///
    /// Disabled only for [`PromptTextSurface::TrustedSkillInstruction`] —
    /// certified/trusted skill instruction bodies. Their content is first-party
    /// and reviewed (the skill ships certified via `CERTIFIED_SKILLS` and is
    /// installed into the trusted system-skill root), and these denylists
    /// false-positived constantly on legitimate skill docs that describe OAuth
    /// headers, API keys, and host paths — failing the whole turn (#5169).
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

    const fn policy(self) -> PromptTextPolicy {
        PromptTextPolicy {
            enforce_content_checks: !matches!(self, Self::TrustedSkillInstruction),
        }
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
    // Structural limits always apply, even to trusted skill content.
    if value.is_empty() || value.len() > surface.max_bytes() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} is not model-safe"),
        ));
    }
    // Trusted skill instructions skip content denylisting; see PromptTextPolicy.
    if !surface.policy().enforce_content_checks {
        return Ok(value);
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
    reject_sensitive_text(&value, label)?;
    Ok(value)
}

fn reject_sensitive_text(value: &str, label: &'static str) -> Result<(), AgentLoopHostError> {
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
    for term in SENSITIVE_TERMS {
        if term.reject_as_phrase && contains_token_phrase(&lower, term.phrase) {
            return non_model_safe(label);
        }
        if term.reject_value_after_label
            && contains_credential_value_after_label(&lower, term.phrase)
        {
            return non_model_safe(label);
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

    /// Trusted skill content also bypasses the control-character check.
    #[test]
    fn trusted_skill_instruction_bypasses_control_characters() {
        validate_prompt_text(
            "bell\u{0007}inside trusted skill".to_string(),
            "skill content",
            PromptTextSurface::TrustedSkillInstruction,
        )
        .expect("trusted skill content bypasses the control-character check");
        let error = validate_prompt_text(
            "bell\u{0007}inside generic content".to_string(),
            "context content",
            PromptTextSurface::GenericModelContent,
        )
        .expect_err("untrusted content still rejects control characters");
        assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    }

    /// Structural limits (empty, byte budget) apply to every surface, including
    /// trusted skill content.
    #[test]
    fn structural_limits_apply_even_to_trusted_skill_instruction() {
        let empty = validate_prompt_text(
            String::new(),
            "skill content",
            PromptTextSurface::TrustedSkillInstruction,
        )
        .expect_err("empty content is rejected on every surface");
        assert_eq!(empty.kind, AgentLoopHostErrorKind::PolicyDenied);

        let oversized = "x".repeat(LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES + 1);
        let too_big = validate_prompt_text(
            oversized,
            "skill content",
            PromptTextSurface::TrustedSkillInstruction,
        )
        .expect_err("oversized content is rejected on every surface");
        assert_eq!(too_big.kind, AgentLoopHostErrorKind::PolicyDenied);
    }
}
