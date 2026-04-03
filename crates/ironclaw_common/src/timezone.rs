//! Validated IANA timezone type.

use serde::{Deserialize, Serialize};

/// A validated IANA timezone.
///
/// Wraps `chrono_tz::Tz` and guarantees the timezone string was valid at
/// construction time. Use `ValidTimezone::parse()` to create — it returns
/// `None` for empty or unrecognized timezone strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidTimezone(chrono_tz::Tz);

impl ValidTimezone {
    /// Parse an IANA timezone string. Returns `None` for empty or invalid input.
    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        trimmed.parse::<chrono_tz::Tz>().ok().map(Self)
    }

    /// The underlying `chrono_tz::Tz` value.
    pub fn tz(&self) -> chrono_tz::Tz {
        self.0
    }

    /// The IANA name (e.g. "America/New_York").
    pub fn name(&self) -> &str {
        self.0.name()
    }
}

impl std::fmt::Display for ValidTimezone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for ValidTimezone {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.name().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ValidTimezone {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid IANA timezone: '{s}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_timezone() {
        let tz = ValidTimezone::parse("America/New_York").unwrap();
        assert_eq!(tz.name(), "America/New_York");
    }

    #[test]
    fn parse_with_whitespace() {
        let tz = ValidTimezone::parse("  Europe/London  ").unwrap();
        assert_eq!(tz.name(), "Europe/London");
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(ValidTimezone::parse("").is_none());
        assert!(ValidTimezone::parse("   ").is_none());
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(ValidTimezone::parse("NotATimezone").is_none());
        assert!(ValidTimezone::parse("US/FakeCity").is_none());
    }

    #[test]
    fn serde_roundtrip() {
        let tz = ValidTimezone::parse("Asia/Tokyo").unwrap();
        let json = serde_json::to_string(&tz).unwrap();
        assert_eq!(json, "\"Asia/Tokyo\"");
        let back: ValidTimezone = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tz);
    }

    #[test]
    fn deserialize_invalid_fails() {
        let result: Result<ValidTimezone, _> = serde_json::from_str("\"NotReal\"");
        assert!(result.is_err());
    }
}
