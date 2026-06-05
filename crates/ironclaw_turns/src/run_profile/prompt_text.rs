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
    reject_security_vocabulary: bool,
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
            reject_security_vocabulary: !matches!(self, Self::TrustedSkillInstruction),
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
    let policy = surface.policy();
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
        if policy.reject_security_vocabulary
            && term.reject_as_phrase
            && contains_token_phrase(&lower, term.phrase)
        {
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
