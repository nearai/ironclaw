//! Reject inline secret material at parse time.
//!
//! Mirrors the epic [#3036](https://github.com/nearai/ironclaw/issues/3036)
//! invariant for blueprints — `docs/reborn/contracts/secrets.md` requires
//! that values containing secret material are rejected at *write* time, and
//! the only legitimate way to reference a secret in declarative config is
//! through an opaque handle (env-var name, secret-store key, etc.).
//!
//! The boot-config file applies the same rule even though it's not a
//! blueprint: an operator who *would* paste a raw API key into
//! `~/.ironclaw/reborn/config.toml` instead of pointing at
//! `OPENAI_API_KEY` should be told no, loudly, on the very first
//! `ironclaw-reborn run`. That way the muscle-memory carries straight
//! into blueprint authoring later.
//!
//! The detection is pattern-based on the known high-signal shapes that
//! show up in operator paste-mistakes. False positives (legit non-secret
//! strings that happen to match a shape) are deliberately preferred over
//! silently accepting a secret — the worst failure mode is a noisy
//! parse error pointing at the offending key, never a leaked credential.

use std::borrow::Cow;
use std::fmt;

use thiserror::Error;

/// Patterns we treat as "looks like a secret". The list is intentionally
/// conservative — better a false positive caught by the parser than a
/// real secret silently round-tripped through a TOML file checked into
/// git.
const SECRET_PREFIXES: &[&str] = &[
    // OpenAI direct + OpenAI-compat. Anthropic direct API keys also
    // share this prefix (`sk-ant-...`).
    "sk-",
    // Stripe-style scoped keys. Conservative; included because operator
    // pastes from a billing console aren't impossible.
    "sk_live_",
    "sk_test_",
    // Slack bot / user / app tokens.
    "xoxb-",
    "xoxp-",
    "xapp-",
    // GitHub tokens, all stable prefixes per GitHub's docs.
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    // AWS. `AKIA` / `ASIA` are long enough to be high-signal even as
    // a substring of a legitimate identifier.
    "AKIA",
    "ASIA",
    // Google API keys (`AIza...`) + OAuth bearer (`ya29.`).
    "AIza",
    "ya29.",
    // HuggingFace.
    "hf_",
    // NEAR AI session token. (NEAR AI typically uses `nai-sess-...`
    // for session shapes — included for symmetry with the v1
    // session.json contents.)
    "nai-sess-",
];

/// Heuristic JWT shape (`xxx.yyy.zzz` of base64-url chars). Very few
/// configuration values would legitimately match this.
fn looks_like_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let is_base64url = |segment: &str| -> bool {
        !segment.is_empty()
            && segment.len() > 8
            && segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
    };
    parts.iter().all(|part| is_base64url(part))
}

/// Long string of hex characters. A real provider id, model id, or env
/// var name is essentially never this shape.
fn looks_like_long_hex(value: &str) -> bool {
    if value.len() < 32 {
        return false;
    }
    value.chars().all(|character| character.is_ascii_hexdigit())
}

/// Returns `Err` if `value` looks like inline secret material that
/// must not appear in a declarative config file.
///
/// Prefix markers are matched inside values, but short markers require
/// token-like boundaries so ordinary identifiers such as `risk-mitigation`
/// do not trip the guard. Embedded credentials in URLs still fail closed.
pub fn reject_inline_secret(
    label: impl Into<Cow<'static, str>>,
    value: &str,
) -> Result<(), InlineSecretError> {
    let label = label.into();
    let value = value.trim();
    // Empty / very short values can't carry secrets meaningfully — and a
    // legitimate value like `model = "gpt-4o-mini"` would otherwise trip
    // a careless prefix match. Count characters, not bytes, so Unicode
    // values follow the same intuitive threshold as ASCII values.
    if value.is_empty() || value.chars().count() < 12 {
        return Ok(());
    }
    for prefix in SECRET_PREFIXES {
        if contains_secret_prefix(value, prefix) {
            return Err(InlineSecretError {
                label: label.clone(),
                pattern: SecretPattern::Prefix(prefix),
            });
        }
    }
    if looks_like_jwt(value) {
        return Err(InlineSecretError {
            label: label.clone(),
            pattern: SecretPattern::Jwt,
        });
    }
    if looks_like_long_hex(value) {
        return Err(InlineSecretError {
            label,
            pattern: SecretPattern::LongHex,
        });
    }
    Ok(())
}

fn contains_secret_prefix(value: &str, prefix: &'static str) -> bool {
    if prefix.chars().count() <= 4 {
        contains_short_prefix_at_boundary(value, prefix)
    } else {
        value.contains(prefix)
    }
}

fn contains_short_prefix_at_boundary(value: &str, prefix: &'static str) -> bool {
    let value_chars = value.chars().collect::<Vec<_>>();
    let prefix_chars = prefix.chars().collect::<Vec<_>>();

    value_chars
        .windows(prefix_chars.len())
        .enumerate()
        .any(|(index, window)| {
            window == prefix_chars.as_slice()
                && (index == 0 || !is_identifier_character(value_chars[index - 1]))
        })
}

fn is_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretPattern {
    Prefix(&'static str),
    Jwt,
    LongHex,
}

impl fmt::Display for SecretPattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prefix(prefix) => write!(formatter, "value contains secret marker `{prefix}`"),
            Self::Jwt => formatter.write_str("value is JWT-shaped (`<hdr>.<payload>.<sig>`)"),
            Self::LongHex => formatter.write_str("value is a long hex run (>= 32 hex chars)"),
        }
    }
}

#[derive(Debug, Clone, Error)]
#[error(
    "config field `{label}` looks like inline secret material ({pattern}); \
     this file must reference secrets by env-var name (e.g. `api_key_env = \"OPENAI_API_KEY\"`), \
     never paste the value directly (see docs/reborn/contracts/secrets.md, epic #3036)"
)]
pub struct InlineSecretError {
    pub(crate) label: Cow<'static, str>,
    pub(crate) pattern: SecretPattern,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_openai_sk_prefix() {
        let value = "sk-proj-abcdef1234567890abcdef1234";
        let err = reject_inline_secret("llm.default.api_key", value).expect_err("must reject");
        assert!(matches!(err.pattern, SecretPattern::Prefix("sk-")));
    }

    #[test]
    fn rejects_slack_bot_token() {
        // Synthesized at runtime so the literal in source doesn't match
        // upstream secret-scanners (GitHub push protection trips on a
        // direct `xoxb-...` literal even when it is obviously a test
        // fixture). The composed string has the same shape the
        // production rejection rule catches.
        let value = format!(
            "{}{}",
            "xo", "xb-1234567890123-1234567890123-aBcDeFgHiJkLmNoPqRsTuVwX"
        );
        assert!(reject_inline_secret("any", &value).is_err());
    }

    #[test]
    fn rejects_jwt_shape() {
        let value =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let err = reject_inline_secret("any", value).expect_err("jwt must reject");
        assert_eq!(err.pattern, SecretPattern::Jwt);
    }

    #[test]
    fn rejects_long_hex() {
        let value = "deadbeefcafebabe0123456789abcdef0123456789abcdef";
        let err = reject_inline_secret("any", value).expect_err("long hex must reject");
        assert_eq!(err.pattern, SecretPattern::LongHex);
    }

    #[test]
    fn rejects_secret_with_surrounding_whitespace() {
        let value = " sk-proj-1234567890abcdef12345678 ";
        let err = reject_inline_secret("any", value).expect_err("trimmed secret must reject");
        assert_eq!(err.pattern, SecretPattern::Prefix("sk-"));
    }

    #[test]
    fn rejects_embedded_secret_prefix() {
        let value = "https://proxy.example/v1?key=sk-proj-1234567890abcdef12345678";
        let err = reject_inline_secret("llm.default.base_url", value)
            .expect_err("embedded secret must reject");
        assert_eq!(err.pattern, SecretPattern::Prefix("sk-"));
    }

    #[test]
    fn allows_short_prefixes_inside_identifiers() {
        for ok in ["shelf_layout", "risk-mitigation", "task-master-skill"] {
            reject_inline_secret("any", ok).expect_err_or_pass(ok);
        }
    }

    #[test]
    fn counts_characters_not_utf8_bytes_for_length_floor() {
        let value = "密密密密sk-";
        reject_inline_secret("any", value).expect_err_or_pass(value);
    }

    #[test]
    fn allows_env_ref_strings() {
        for ok in ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "REBORN_TEST_LLM_KEY"] {
            reject_inline_secret("llm.default.api_key_env", ok).expect("env-ref must pass");
        }
    }

    #[test]
    fn allows_normal_config_values() {
        for ok in [
            "openai",
            "gpt-4o-mini",
            "https://api.openai.com/v1",
            "local-dev",
            "acme",
            "acme-bot",
            "reborn-cli",
            "https://example.com/some/long/path/with/segments",
            // model strings can be long but aren't hex
            "anthropic/claude-3.5-sonnet-20250620",
        ] {
            reject_inline_secret("any", ok).expect_err_or_pass(ok);
        }
    }

    // Tiny helper so the per-string assertion error message names the
    // offender. Newtype trait, only used by the test above.
    trait OkOrPanic {
        fn expect_err_or_pass(self, value: &str);
    }
    impl OkOrPanic for Result<(), InlineSecretError> {
        fn expect_err_or_pass(self, value: &str) {
            if let Err(error) = self {
                panic!("expected `{value}` to pass; got {error}");
            }
        }
    }
}
