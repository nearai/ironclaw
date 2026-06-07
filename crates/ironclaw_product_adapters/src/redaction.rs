//! Redaction helpers for product-adapter DTOs.
//!
//! Adapter-facing DTOs and errors must never carry raw secrets, bot tokens,
//! host paths, raw prompts, raw tool input, provider/runtime internals, or
//! backend diagnostics. [`RedactedString`] wraps any value that originates
//! from a protocol payload but should not survive into adapter-visible
//! output. Its `Debug`/`Display`/`Serialize` impls all emit
//! [`REDACTED_PLACEHOLDER`].
//!
//! The internal value is reachable only through [`RedactedString::expose`],
//! which is private to this crate; only well-reviewed code paths inside
//! `ironclaw_product_adapters` may consult the underlying string. WASM
//! components and downstream adapter implementations cannot construct or
//! observe the underlying value because the type is `Clone` but its
//! fields are not exposed and `expose` is `pub(crate)`.

use serde::{Deserialize, Serialize, Serializer};
use std::fmt;

pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

/// Wrapper that renders as `<redacted>` everywhere a public consumer can see
/// it. Use for any value that originated from a protocol payload, secret
/// store, or backend error text.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedString(String);

impl RedactedString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the redacted placeholder; the caller never sees the inner
    /// value through this method.
    pub fn placeholder() -> &'static str {
        REDACTED_PLACEHOLDER
    }

    #[allow(dead_code)]
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_PLACEHOLDER)
    }
}

impl fmt::Display for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_PLACEHOLDER)
    }
}

impl Serialize for RedactedString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED_PLACEHOLDER)
    }
}

impl<'de> Deserialize<'de> for RedactedString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::new(value))
    }
}

/// Helper trait to assert that a value's `Debug` representation does not
/// contain a particular substring. Used by tests to prove redaction holds.
pub trait RedactedDebug {
    fn debug_does_not_contain(&self, needle: &str) -> bool;
}

impl<T: fmt::Debug> RedactedDebug for T {
    fn debug_does_not_contain(&self, needle: &str) -> bool {
        !format!("{self:?}").contains(needle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_inner_value() {
        let value = RedactedString::new("super-secret-token");
        let rendered = format!("{value:?}");
        assert_eq!(rendered, REDACTED_PLACEHOLDER);
        assert!(!rendered.contains("super-secret-token"));
    }

    #[test]
    fn display_does_not_leak_inner_value() {
        let value = RedactedString::new("super-secret-token");
        let rendered = value.to_string();
        assert_eq!(rendered, REDACTED_PLACEHOLDER);
    }

    #[test]
    fn serialize_emits_placeholder() {
        let value = RedactedString::new("super-secret-token");
        let json = serde_json::to_string(&value).expect("serialize");
        assert_eq!(json, "\"<redacted>\"");
    }

    #[test]
    fn debug_does_not_contain_helper_works() {
        let value = RedactedString::new("super-secret-token");
        assert!(value.debug_does_not_contain("super-secret-token"));
        assert!(!value.debug_does_not_contain(REDACTED_PLACEHOLDER));
    }
}
