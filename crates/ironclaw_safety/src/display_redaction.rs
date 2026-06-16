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
    static PATTERNS: OnceLock<[Regex; 4]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            Regex::new(
                r#"(?i)(-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)(["'])[^"']*(["'])"#,
            )
            .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(
                r#"(?i)(-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)([^\s"'][^\s]*)"#,
            )
            .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(
                r#"(?i)(Authorization|X-Api-Key|X-Auth-Token|Bearer)\s*:\s*(?:[a-zA-Z]+\s+[^\s"']+|[^\s"']+)"#,
            )
                .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(r#"[a-zA-Z][a-zA-Z0-9+.\-]*://[^\s"']+"#)
                .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
        ]
    });
    let mut out = patterns[0]
        .replace_all(cmd, "$1$2$3[redacted]$4")
        .into_owned();
    out = patterns[1].replace_all(&out, "$1$2[redacted]").into_owned();
    out = patterns[2].replace_all(&out, "$1: [redacted]").into_owned();
    out = patterns[3]
        .replace_all(&out, |captures: &regex::Captures<'_>| {
            sanitize_url_for_display(&captures[0])
        })
        .into_owned();
    sanitize_display_text(&out)
}

pub fn sanitize_display_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut redact_next_value = false;
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        if trimmed.is_empty() {
            push_safe_text(&mut out, token);
            continue;
        }
        let suffix = &token[trimmed.len()..];
        if is_url_like(trimmed) {
            out.push_str(&sanitize_url_for_display(trimmed));
            push_safe_text(&mut out, suffix);
            redact_next_value = false;
            continue;
        }
        let redact_current =
            redact_next_value || is_secret_like(trimmed) || is_unsafe_path_like(trimmed);
        if redact_current {
            out.push_str("[redacted]");
            push_safe_text(&mut out, suffix);
        } else {
            push_safe_text(&mut out, token);
        }
        redact_next_value = credential_key_expects_value(trimmed) && !suffix.is_empty();
    }
    out
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

pub fn sanitize_url_for_display(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return sanitize_display_text(url);
    };
    let (scheme, rest) = url.split_at(scheme_end + 3);
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
    let mut parts = vec![segment.trim_matches(token_boundary_punctuation).to_string()];
    if let Ok(decoded) = urlencoding::decode(segment)
        && decoded.as_ref() != segment
    {
        parts.extend(
            decoded
                .trim_matches(token_boundary_punctuation)
                .split('/')
                .map(|part| part.trim_matches(token_boundary_punctuation).to_string()),
        );
    }
    parts
}

fn is_secret_url_path_segment_value(segment: &str) -> bool {
    is_secret_like(segment) || is_secret_url_path_segment_label_prefix(segment)
}

fn is_secret_url_path_segment_label_prefix(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower.starts_with("secret")
        || lower.starts_with("token")
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
            | "access-tokens"
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
            | "secret:"
            | "secret="
            | "password:"
            | "password="
            | "token:"
            | "token="
    )
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
    use super::shell_command_display_text;

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
}
