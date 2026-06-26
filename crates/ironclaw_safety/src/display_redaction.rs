//! Display-oriented redaction for capability inputs and previews.

use std::sync::OnceLock;

use regex::Regex;

pub const SHELL_COMMAND_DISPLAY_MAX_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeDisplayText {
    pub text: String,
    pub truncated: bool,
}

pub fn shell_command_display_text(command: &str) -> SafeDisplayText {
    let sanitized = redact_shell_command_for_display(command);
    truncate_display_text(&sanitized, SHELL_COMMAND_DISPLAY_MAX_BYTES)
}

fn redact_shell_command_for_display(cmd: &str) -> String {
    static SHELL_FLAG_PATTERNS: OnceLock<[Regex; 2]> = OnceLock::new();
    let shell_flag_patterns = SHELL_FLAG_PATTERNS.get_or_init(|| {
        [
            Regex::new(
                r#"(?i)(-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)(["'])[^"']*(["'])"#,
            )
            .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(
                r#"(?i)(-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)([^\s"'][^\s]*)"#,
            )
            .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
        ]
    });
    let shared_patterns = shared_display_redaction_patterns();
    let mut out = shell_flag_patterns[0]
        .replace_all(cmd, "$1$2$3[redacted]$4")
        .into_owned();
    out = shell_flag_patterns[1]
        .replace_all(&out, "$1$2[redacted]")
        .into_owned();
    out = shared_patterns[0]
        .replace_all(&out, "$1: [redacted]")
        .into_owned();
    out = shared_patterns[1]
        .replace_all(&out, |captures: &regex::Captures<'_>| {
            sanitize_url_substring_for_display(&captures[0])
        })
        .into_owned();
    sanitize_display_text(&out)
}

pub fn sanitize_display_text(text: &str) -> String {
    let text = redact_shared_display_substrings(text);
    let mut out = String::with_capacity(text.len());
    let mut credential_value_state = CredentialValueState::None;
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        if trimmed.is_empty() {
            push_safe_text(&mut out, token);
            continue;
        }
        let suffix = &token[trimmed.len()..];
        if matches!(credential_value_state, CredentialValueState::AfterKey)
            && is_credential_separator(trimmed)
        {
            push_safe_text(&mut out, token);
            credential_value_state = if suffix.is_empty() {
                CredentialValueState::AfterKey
            } else {
                CredentialValueState::AfterSeparator
            };
            continue;
        }
        if matches!(credential_value_state, CredentialValueState::AfterKey)
            && let Some(separator) = credential_separator_with_value(trimmed)
        {
            out.push(separator);
            out.push_str("[redacted]");
            push_safe_text(&mut out, suffix);
            credential_value_state = CredentialValueState::None;
            continue;
        }
        let redacts_credential_value =
            matches!(credential_value_state, CredentialValueState::AfterSeparator);
        if let Some((leading, url, trailing)) = wrapped_url_parts(trimmed) {
            if redacts_credential_value {
                out.push_str("[redacted]");
            } else {
                push_safe_text(&mut out, leading);
                out.push_str(&sanitize_url_for_display(url));
                push_safe_text(&mut out, trailing);
            }
            push_safe_text(&mut out, suffix);
            credential_value_state = CredentialValueState::None;
            continue;
        }
        let redact_current =
            redacts_credential_value || is_secret_like(trimmed) || is_unsafe_path_like(trimmed);
        if redact_current {
            out.push_str("[redacted]");
            push_safe_text(&mut out, suffix);
        } else {
            push_safe_text(&mut out, token);
        }
        credential_value_state = if credential_key_expects_value(trimmed) && !suffix.is_empty() {
            CredentialValueState::AfterSeparator
        } else if credential_key_may_have_spaced_value(trimmed) && !suffix.is_empty() {
            CredentialValueState::AfterKey
        } else {
            CredentialValueState::None
        };
    }
    out
}

fn redact_shared_display_substrings(text: &str) -> String {
    let patterns = shared_display_redaction_patterns();
    let out = patterns[0].replace_all(text, "$1: [redacted]").into_owned();
    patterns[1]
        .replace_all(&out, |captures: &regex::Captures<'_>| {
            sanitize_url_substring_for_display(&captures[0])
        })
        .into_owned()
}

fn shared_display_redaction_patterns() -> &'static [Regex; 2] {
    static PATTERNS: OnceLock<[Regex; 2]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            Regex::new(
                r#"(?i)(Authorization|X-Api-Key|X-Auth-Token|Bearer)\s*:\s*(?:[a-zA-Z]+\s+[^\s"']+|[^\s"']+)"#,
            )
            .expect("hardcoded display redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(r#"[a-zA-Z][a-zA-Z0-9+.\-]*://[^\s"']+"#)
                .expect("hardcoded display redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
        ]
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialValueState {
    None,
    AfterKey,
    AfterSeparator,
}

fn is_url_like(token: &str) -> bool {
    let Some(scheme_end) = token.find("://") else {
        return false;
    };
    let scheme = &token[..scheme_end];
    let Some(first) = scheme.chars().next() else {
        return false;
    };
    !scheme.eq_ignore_ascii_case("file")
        && first.is_ascii_alphabetic()
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')
        })
}

fn wrapped_url_parts(token: &str) -> Option<(&str, &str, &str)> {
    if is_url_like(token) {
        return Some(("", token, ""));
    }

    let start = token
        .char_indices()
        .find(|(_, character)| !url_leading_wrapper_punctuation(*character))
        .map(|(index, _)| index)?;
    if start == 0 {
        return None;
    }

    let mut end = token.len();
    while end > start {
        let Some(character) = token[..end].chars().next_back() else {
            break;
        };
        if !url_trailing_wrapper_punctuation(character) {
            break;
        }
        end -= character.len_utf8();
    }
    if end <= start {
        return None;
    }

    let candidate = &token[start..end];
    is_url_like(candidate).then_some((&token[..start], candidate, &token[end..]))
}

fn url_leading_wrapper_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | '(' | '[' | '{' | '<' | ',' | ';'
    )
}

fn url_trailing_wrapper_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ')' | ']' | '}' | '>' | ',' | ';'
    )
}

fn sanitize_url_substring_for_display(url: &str) -> String {
    let mut end = url.len();
    while end > 0 {
        let Some(character) = url[..end].chars().next_back() else {
            break;
        };
        if !url_trailing_wrapper_punctuation(character) {
            break;
        }
        let candidate_end = end - character.len_utf8();
        if candidate_end == 0 || !is_url_like(&url[..candidate_end]) {
            break;
        }
        end = candidate_end;
    }
    let sanitized = sanitize_url_for_display(&url[..end]);
    format!("{sanitized}{}", &url[end..])
}

pub fn sanitize_url_for_display(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return sanitize_display_text(url);
    };
    let (scheme, rest) = url.split_at(scheme_end + 3);
    if scheme[..scheme.len() - 3].eq_ignore_ascii_case("file") {
        return "[redacted]".to_string();
    }
    let (rest, suffix) = match rest.find(['?', '#']) {
        Some(index) => {
            let marker = &rest[index..=index];
            let replacement = if marker == "?" { "?..." } else { "#..." };
            (&rest[..index], replacement)
        }
        None => (rest, ""),
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    let authority = authority
        .rfind('@')
        .map(|index| &authority[index + 1..])
        .unwrap_or(authority);
    let path = redact_secret_url_path_segments(path);
    strip_unsupported_control_chars(&format!("{scheme}{authority}{path}{suffix}"))
}

fn redact_secret_url_path_segments(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut redact_next = false;
    path.split('/')
        .enumerate()
        .map(|(index, segment)| {
            if index == 0 {
                return String::new();
            }
            let parts = secret_url_path_segment_parts(segment);
            let segment_is_secret = parts
                .iter()
                .any(|part| is_secret_url_path_segment_value(part));
            let should_redact = redact_next || segment_is_secret;
            redact_next = parts
                .iter()
                .rev()
                .find(|part| !part.is_empty())
                .is_some_and(|part| is_secret_url_path_label(part));
            if should_redact {
                "[redacted]".to_string()
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn secret_url_path_segment_parts(segment: &str) -> Vec<String> {
    let mut parts = secret_url_path_subparts(segment);
    if let Ok(decoded) = urlencoding::decode(segment)
        && decoded.as_ref() != segment
    {
        parts.extend(secret_url_path_subparts(decoded.as_ref()));
    }
    parts
}

fn secret_url_path_subparts(segment: &str) -> Vec<String> {
    segment
        .trim_matches(token_boundary_punctuation)
        .split(['/', '\\'])
        .map(|part| part.trim_matches(token_boundary_punctuation).to_string())
        .collect()
}

fn is_secret_url_path_segment_value(segment: &str) -> bool {
    is_secret_like(segment) || is_secret_url_path_segment_label_prefix(segment)
}

fn is_secret_url_path_segment_label_prefix(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower.starts_with("secret")
        || lower.starts_with("token")
        || lower.starts_with("access-token")
        || lower.starts_with("access_token")
        || lower.starts_with("refresh-token")
        || lower.starts_with("refresh_token")
        || lower.starts_with("api-key")
        || lower.starts_with("apikey")
        || lower.starts_with("api_key")
        || lower.starts_with("password")
        || lower.starts_with("credential")
        || lower.starts_with("bearer")
}

fn is_secret_url_path_label(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "secret"
            | "secrets"
            | "token"
            | "tokens"
            | "api-key"
            | "api-keys"
            | "apikey"
            | "apikeys"
            | "api_key"
            | "api_keys"
            | "access_token"
            | "access_tokens"
            | "access-token"
            | "access-tokens"
            | "refresh_token"
            | "refresh_tokens"
            | "refresh-token"
            | "refresh-tokens"
            | "password"
            | "passwords"
            | "credential"
            | "credentials"
            | "bearer"
    )
}

fn push_safe_text(out: &mut String, text: &str) {
    out.extend(
        text.chars().filter(|character| {
            *character == '\n' || *character == '\t' || !character.is_control()
        }),
    );
}

fn strip_unsupported_control_chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_safe_text(&mut out, text);
    out
}

fn is_secret_like(token: &str) -> bool {
    let trimmed = token.trim_matches(token_boundary_punctuation);
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("ghu_")
        || lower.starts_with("ghs_")
        || lower.starts_with("xoxb-")
        || lower.starts_with("xoxa-")
        || lower.starts_with("xoxp-")
        || looks_like_aws_access_key(trimmed)
        || looks_like_jwt(trimmed)
        || lower.contains("api_key=")
        || lower.contains("api_key:")
        || lower.contains("apikey=")
        || lower.contains("apikey:")
        || lower.contains("access_token=")
        || lower.contains("access_token:")
        || lower.contains("secret=")
        || lower.contains("secret:")
        || lower.contains("password=")
        || lower.contains("password:")
        || lower.contains("token=")
        || lower.contains("token:")
}

fn is_unsafe_path_like(token: &str) -> bool {
    let token = token.trim_matches(token_boundary_punctuation);
    token.to_ascii_lowercase().starts_with("file:/")
        || token_contains_absolute_posix_path(token)
        || token.starts_with("\\\\")
        || token.contains("\\\\")
        || token.get(1..3) == Some(":\\")
}

fn credential_key_expects_value(token: &str) -> bool {
    let lower = token
        .trim_matches(non_credential_boundary_punctuation)
        .to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "api_key:"
            | "api_key="
            | "apikey:"
            | "apikey="
            | "access_token:"
            | "access_token="
            | "access-token:"
            | "access-token="
            | "refresh_token:"
            | "refresh_token="
            | "refresh-token:"
            | "refresh-token="
            | "secret:"
            | "secret="
            | "password:"
            | "password="
            | "token:"
            | "token="
            | "credential:"
            | "credential="
            | "bearer:"
            | "bearer="
    )
}

fn credential_key_may_have_spaced_value(token: &str) -> bool {
    let lower = token
        .trim_matches(non_credential_boundary_punctuation)
        .to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "api_key"
            | "apikey"
            | "access_token"
            | "access-token"
            | "refresh_token"
            | "refresh-token"
            | "secret"
            | "password"
            | "token"
            | "credential"
            | "bearer"
    )
}

fn is_credential_separator(token: &str) -> bool {
    let trimmed = token.trim_matches(non_credential_boundary_punctuation);
    matches!(trimmed, ":" | "=")
}

fn credential_separator_with_value(token: &str) -> Option<char> {
    let trimmed = token.trim_matches(non_credential_boundary_punctuation);
    let mut characters = trimmed.chars();
    let separator = characters.next()?;
    if !matches!(separator, ':' | '=') || characters.as_str().trim().is_empty() {
        return None;
    }
    Some(separator)
}

fn non_credential_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

fn looks_like_aws_access_key(token: &str) -> bool {
    (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token.len() >= 16
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn looks_like_jwt(token: &str) -> bool {
    token.starts_with("eyJ")
        && token.matches('.').count() >= 2
        && token.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn token_contains_absolute_posix_path(token: &str) -> bool {
    if token == "/" {
        return false;
    }
    let mut previous = None;
    let mut characters = token.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '/'
            && previous.is_none_or(token_boundary_punctuation)
            && !matches!(previous, Some('/'))
            && !matches!(characters.peek(), Some('/'))
        {
            return true;
        }
        previous = Some(character);
    }
    false
}

fn token_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | ':' | '=' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
    )
}

fn truncate_display_text(text: &str, max_bytes: usize) -> SafeDisplayText {
    if text.len() <= max_bytes {
        return SafeDisplayText {
            text: text.to_string(),
            truncated: false,
        };
    }

    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    SafeDisplayText {
        text: text[..end].to_string(),
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{sanitize_display_text, shell_command_display_text};

    #[test]
    fn shell_command_display_text_redacts_auth_and_url_query() {
        let display = shell_command_display_text(
            "curl -H 'Authorization: Bearer sk-secret' https://example.test/path?token=secret && echo ok",
        );

        assert!(display.text.contains("curl -H 'Authorization: [redacted]'"));
        assert!(display.text.contains("https://example.test/path?..."));
        assert!(display.text.contains("echo ok"));
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("token=secret"));
    }

    #[test]
    fn shell_command_display_text_redacts_bare_secrets_and_host_paths() {
        let display = shell_command_display_text(
            "cat /home/alice/.ssh/id_rsa && echo sk-secret && token: ghp_secret",
        );

        assert!(display.text.contains("cat [redacted]"));
        assert!(display.text.contains("echo [redacted]"));
        assert!(display.text.contains("token: [redacted]"));
        assert!(!display.text.contains("/home/alice"));
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("ghp_secret"));
    }

    #[test]
    fn shell_command_display_text_redacts_secret_url_path_segments() {
        let display = shell_command_display_text(
            "curl https://example.test/reset/sk-secret/token123?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/reset/[redacted]/[redacted]?...")
        );
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("token123"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_value_after_secret_url_path_label() {
        let display = shell_command_display_text(
            "curl https://example.test/reset/token/opaque-value/credential/other-value?debug=true",
        );

        assert!(display.text.contains(
            "https://example.test/reset/[redacted]/[redacted]/[redacted]/[redacted]?..."
        ));
        assert!(!display.text.contains("opaque-value"));
        assert!(!display.text.contains("other-value"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_common_token_url_path_labels() {
        let display = shell_command_display_text(
            "curl https://example.test/oauth/access-token/opaque-value/refresh_token/other-value?debug=true",
        );

        assert!(display.text.contains(
            "https://example.test/oauth/[redacted]/[redacted]/[redacted]/[redacted]?..."
        ));
        assert!(!display.text.contains("opaque-value"));
        assert!(!display.text.contains("other-value"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_percent_encoded_secret_url_path_segments() {
        let display = shell_command_display_text(
            "curl https://example.test/reset/sk%2Dsecret/token%31%32%33?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/reset/[redacted]/[redacted]?...")
        );
        assert!(!display.text.contains("sk%2Dsecret"));
        assert!(!display.text.contains("token%31%32%33"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_percent_encoded_secret_url_path_separators() {
        let display = shell_command_display_text(
            "curl https://example.test/reset%2Fsk-secret/public?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/[redacted]/public?...")
        );
        assert!(!display.text.contains("reset%2Fsk-secret"));
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_percent_encoded_backslash_path_separators() {
        let display = shell_command_display_text(
            "curl https://example.test/reset%5Csk-secret/public https://example.test/reset%5cghp_secret/public",
        );

        assert!(
            display
                .text
                .contains("https://example.test/[redacted]/public")
        );
        assert!(!display.text.contains("reset%5Csk-secret"));
        assert!(!display.text.contains("reset%5cghp_secret"));
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("ghp_secret"));
    }

    #[test]
    fn shell_command_display_text_redacts_wrapped_encoded_secret_url_path_separators() {
        let display = shell_command_display_text(
            "curl https://example.test/reset%2F(token123)/public?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/[redacted]/public?...")
        );
        assert!(!display.text.contains("reset%2F(token123)"));
        assert!(!display.text.contains("token123"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_encoded_backslash_secret_url_path_separators() {
        let upper = shell_command_display_text(
            "curl https://example.test/reset%5Csk-secret/public?debug=true",
        );
        let lower = shell_command_display_text(
            "curl https://example.test/reset%5csk-secret/public?debug=true",
        );

        for display in [upper, lower] {
            assert!(
                display
                    .text
                    .contains("https://example.test/[redacted]/public?...")
            );
            assert!(!display.text.contains("reset%5Csk-secret"));
            assert!(!display.text.contains("reset%5csk-secret"));
            assert!(!display.text.contains("sk-secret"));
            assert!(!display.text.contains("debug=true"));
        }
    }

    #[test]
    fn shell_command_display_text_redacts_value_after_percent_encoded_secret_label() {
        let display = shell_command_display_text(
            "curl https://example.test/reset%2Ftoken/opaque-value?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/[redacted]/[redacted]?...")
        );
        assert!(!display.text.contains("reset%2Ftoken"));
        assert!(!display.text.contains("opaque-value"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_value_after_trailing_encoded_secret_label() {
        let display = shell_command_display_text(
            "curl https://example.test/reset/token%2F/opaque-value?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/reset/[redacted]/[redacted]?...")
        );
        assert!(!display.text.contains("token%2F"));
        assert!(!display.text.contains("opaque-value"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_keeps_benign_headers_visible() {
        let display = shell_command_display_text(
            "curl -H 'Accept: application/json' -H 'Authorization: Bearer sk-secret' https://example.test",
        );

        assert!(display.text.contains("-H 'Accept: application/json'"));
        assert!(display.text.contains("-H 'Authorization: [redacted]'"));
        assert!(!display.text.contains("sk-secret"));
    }

    #[test]
    fn sanitize_display_text_redacts_spaced_credential_key_values() {
        let sanitized = sanitize_display_text(
            "token = opaque-token api_key = opaque-api-key password : opaque-password",
        );

        assert!(sanitized.contains("token = [redacted]"));
        assert!(sanitized.contains("api_key = [redacted]"));
        assert!(sanitized.contains("password : [redacted]"));
        assert!(!sanitized.contains("opaque-token"));
        assert!(!sanitized.contains("opaque-api-key"));
        assert!(!sanitized.contains("opaque-password"));
    }

    #[test]
    fn shell_command_display_text_redacts_spaced_credential_values() {
        let display = shell_command_display_text(
            "env token = opaque-value api_key = other-value password : third-value",
        );

        assert!(display.text.contains("token = [redacted]"));
        assert!(display.text.contains("api_key = [redacted]"));
        assert!(display.text.contains("password : [redacted]"));
        assert!(!display.text.contains("opaque-value"));
        assert!(!display.text.contains("other-value"));
        assert!(!display.text.contains("third-value"));
    }

    #[test]
    fn sanitize_display_text_redacts_half_spaced_credential_key_values() {
        let sanitized = sanitize_display_text("token =opaque-token password :opaque-password");

        assert!(sanitized.contains("token =[redacted]"));
        assert!(sanitized.contains("password :[redacted]"));
        assert!(!sanitized.contains("opaque-token"));
        assert!(!sanitized.contains("opaque-password"));
    }

    #[test]
    fn sanitize_display_text_redacts_wrapped_secret_url_paths() {
        let sanitized = sanitize_display_text(
            "see (https://example.test/reset/sk-secret) and <https://example.test/reset/token/opaque-value>",
        );

        assert!(sanitized.contains("(https://example.test/reset/[redacted])"));
        assert!(sanitized.contains("<https://example.test/reset/[redacted]/[redacted]>"));
        assert!(!sanitized.contains("sk-secret"));
        assert!(!sanitized.contains("opaque-value"));
    }

    #[test]
    fn sanitize_display_text_redacts_url_and_auth_header_substrings() {
        let sanitized = sanitize_display_text(
            "url=https://example.test/reset/token/opaque-value Authorization: Bearer opaque-token",
        );

        assert!(sanitized.contains("[redacted] Authorization: [redacted]"));
        assert!(sanitized.contains("Authorization: [redacted]"));
        assert!(!sanitized.contains("opaque-value"));
        assert!(!sanitized.contains("opaque-token"));
    }

    #[test]
    fn sanitize_display_text_redacts_common_sensitive_key_values() {
        let sanitized = sanitize_display_text(
            "access_token: access-value refresh-token = refresh-value credential: credential-value",
        );

        assert!(sanitized.contains("access_token: [redacted]"));
        assert!(sanitized.contains("refresh-token = [redacted]"));
        assert!(sanitized.contains("credential: [redacted]"));
        assert!(!sanitized.contains("access-value"));
        assert!(!sanitized.contains("refresh-value"));
        assert!(!sanitized.contains("credential-value"));
    }

    #[test]
    fn sanitize_url_for_display_redacts_file_urls() {
        let sanitized = super::sanitize_url_for_display("file:///Users/alice/.ssh/id_rsa");

        assert_eq!(sanitized, "[redacted]");
        assert!(!sanitized.contains("/Users/alice"));
    }
}
