use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_AUTOMATION_NAME_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct AutomationName(String);

impl AutomationName {
    fn validate(value: &str) -> Result<(), AutomationNameError> {
        if value.is_empty() {
            return Err(AutomationNameError::Empty);
        }
        if value.len() > MAX_AUTOMATION_NAME_BYTES {
            return Err(AutomationNameError::TooLong);
        }
        Ok(())
    }

    pub fn new(raw: impl Into<String>) -> Result<Self, AutomationNameError> {
        let raw = raw.into();
        let trimmed = raw.trim();
        Self::validate(trimmed)?;
        let value = if trimmed.len() == raw.len() {
            raw
        } else {
            trimmed.to_string()
        };
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for AutomationName {
    type Error = AutomationNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for AutomationName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for AutomationName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<AutomationName> for String {
    fn from(name: AutomationName) -> Self {
        name.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AutomationNameError {
    #[error("automation name must not be empty")]
    Empty,
    #[error("automation name must be at most {MAX_AUTOMATION_NAME_BYTES} bytes")]
    TooLong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_name_trims_and_validates() {
        let name = AutomationName::new("  Daily status  ").expect("valid name");
        assert_eq!(name.as_str(), "Daily status");
    }

    #[test]
    fn automation_name_rejects_blank_and_too_long() {
        assert_eq!(AutomationName::new("  "), Err(AutomationNameError::Empty));
        assert_eq!(
            AutomationName::new("x".repeat(MAX_AUTOMATION_NAME_BYTES + 1)),
            Err(AutomationNameError::TooLong)
        );
    }

    #[test]
    fn automation_name_deserializes_through_validation() {
        let name: AutomationName =
            serde_json::from_str("\"  Daily status  \"").expect("valid serialized automation name");
        assert_eq!(name.as_str(), "Daily status");
        assert!(serde_json::from_str::<AutomationName>("\"   \"").is_err());
    }
}
