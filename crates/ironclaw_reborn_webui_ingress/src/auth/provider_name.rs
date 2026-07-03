//! Validated newtype for OAuth provider identifiers.
//!
//! Per [`.claude/rules/types.md`], identifiers carried between
//! internal modules must be specialized types — never `String` /
//! `&str`. The OAuth flow stores the provider identity in three
//! places that all have to agree (the pending-flow record, the
//! `{provider}` URL segment on the callback, the `OAuthProvider`
//! impl's self-identifier); a string-typed value would let one
//! layer drift from another. This newtype makes that compile error.
//!
//! Validation rules: lowercase ASCII alphanumeric plus underscore,
//! 1..=32 characters. Restrictive on purpose — provider names show
//! up in URL paths, log lines, and the `OAUTH_PROVIDER_ORDER` SPA
//! list; a permissive grammar would invite surprises.

use std::fmt;

use thiserror::Error;

/// Provider identifier — e.g. `google`, `github`, `near`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OAuthProviderName(String);

/// Errors raised by [`OAuthProviderName::new`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OAuthProviderNameError {
    #[error("provider name is empty")]
    Empty,
    #[error("provider name exceeds 32 characters: {0} chars")]
    TooLong(usize),
    #[error(
        "provider name contains disallowed character {0:?}; only lowercase \
         ASCII alphanumerics and underscore are accepted"
    )]
    InvalidChar(char),
}

impl OAuthProviderName {
    fn validate(raw: &str) -> Result<(), OAuthProviderNameError> {
        if raw.is_empty() {
            return Err(OAuthProviderNameError::Empty);
        }
        let chars = raw.chars().count();
        if chars > 32 {
            return Err(OAuthProviderNameError::TooLong(chars));
        }
        for ch in raw.chars() {
            if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
                return Err(OAuthProviderNameError::InvalidChar(ch));
            }
        }
        Ok(())
    }

    /// Construct a validated provider name. Used at the route
    /// boundary (URL `{provider}` segment) and inside provider
    /// constructors.
    pub fn new(raw: impl Into<String>) -> Result<Self, OAuthProviderNameError> {
        let s = raw.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for OAuthProviderName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OAuthProviderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// Deliberately no `From<String>` / `From<&str>` — infallible
// conversion would silently bypass validation. Use `new`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_lowercase_alphanumeric() {
        OAuthProviderName::new("google").expect("ok");
        OAuthProviderName::new("github").expect("ok");
        OAuthProviderName::new("near_wallet").expect("ok");
        OAuthProviderName::new("g1").expect("ok");
        OAuthProviderName::new("a").expect("ok");
    }

    #[test]
    fn rejects_uppercase() {
        assert_eq!(
            OAuthProviderName::new("Google"),
            Err(OAuthProviderNameError::InvalidChar('G')),
        );
    }

    #[test]
    fn rejects_punctuation_and_path_separators() {
        for raw in ["good/bad", "good.bad", "good-bad", "go od", "good\\bad"] {
            assert!(OAuthProviderName::new(raw).is_err(), "{raw:?} must reject",);
        }
    }

    #[test]
    fn rejects_empty_and_too_long() {
        assert_eq!(
            OAuthProviderName::new(""),
            Err(OAuthProviderNameError::Empty)
        );
        let too_long = "g".repeat(33);
        assert_eq!(
            OAuthProviderName::new(&too_long),
            Err(OAuthProviderNameError::TooLong(33)),
        );
    }

    #[test]
    fn rejects_non_ascii() {
        // Unicode letters are not in the grammar.
        assert!(OAuthProviderName::new("goöogle").is_err());
        assert!(OAuthProviderName::new("中文").is_err());
    }

    #[test]
    fn equality_is_by_value() {
        assert_eq!(
            OAuthProviderName::new("google").unwrap(),
            OAuthProviderName::new("google").unwrap(),
        );
        assert_ne!(
            OAuthProviderName::new("google").unwrap(),
            OAuthProviderName::new("github").unwrap(),
        );
    }
}
