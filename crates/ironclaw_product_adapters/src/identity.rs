//! Product-adapter identity types.

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::ProductAdapterError;

const MAX_ID_LEN: usize = 256;

fn validate_id(kind: &'static str, value: &str) -> Result<(), ProductAdapterError> {
    if value.is_empty() {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind,
            reason: "must not be empty".into(),
        });
    }
    if value.len() > MAX_ID_LEN {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind,
            reason: format!("must be at most {MAX_ID_LEN} bytes"),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind,
            reason: "must not contain control characters".into(),
        });
    }
    Ok(())
}

macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
                let value = value.into();
                validate_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(ProductAdapterId, "product_adapter_id");
string_id!(AdapterInstallationId, "adapter_installation_id");

/// Surface kind a product adapter exposes. Used by the workflow layer to pick
/// safe presentation defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurfaceKind {
    /// External chat channel (Telegram, Slack, Discord, ...).
    ExternalChannel,
    /// Browser web gateway.
    Web,
    /// Local CLI/TUI.
    Cli,
    /// Synchronous API surface (OpenAI-compatible, etc.).
    SynchronousApi,
}

impl ProductSurfaceKind {
    pub fn uses_push_delivery(self) -> bool {
        matches!(self, Self::ExternalChannel)
    }

    pub fn supports_synchronous_response(self) -> bool {
        matches!(self, Self::Web | Self::Cli | Self::SynchronousApi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_identifier() {
        assert!(matches!(
            ProductAdapterId::new(""),
            Err(ProductAdapterError::InvalidIdentifier { .. })
        ));
    }

    #[test]
    fn rejects_overlong_identifier() {
        let long = "a".repeat(MAX_ID_LEN + 1);
        assert!(ProductAdapterId::new(long).is_err());
    }

    #[test]
    fn rejects_control_characters() {
        assert!(ProductAdapterId::new("hello\0world").is_err());
        assert!(ProductAdapterId::new("hello\nworld").is_err());
    }

    #[test]
    fn serde_rejects_invalid_identifier() {
        assert!(serde_json::from_str::<ProductAdapterId>("\"hello\\nworld\"").is_err());
    }

    #[test]
    fn round_trips_valid_identifier() {
        let id = ProductAdapterId::new("telegram_v2").expect("valid");
        assert_eq!(id.as_str(), "telegram_v2");
        let json = serde_json::to_string(&id).expect("serialize");
        let parsed: ProductAdapterId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, id);
    }
}
