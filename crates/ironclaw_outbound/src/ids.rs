use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! bounded_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_bounded_ref($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

bounded_ref!(ProjectionSubscriptionId, "projection_subscription_id");
bounded_ref!(ProjectionUpdateRef, "projection_update_ref");

fn validate_bounded_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{kind} must be at most 256 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutboundDeliveryId(Uuid);

impl OutboundDeliveryId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for OutboundDeliveryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OutboundDeliveryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_refs_reject_control_characters() {
        assert!(ProjectionSubscriptionId::new("sub\n1").is_err());
        assert!(ProjectionUpdateRef::new("update\0").is_err());
        assert!(serde_json::from_str::<ProjectionSubscriptionId>("\"sub\\n1\"").is_err());
        assert!(serde_json::from_str::<ProjectionUpdateRef>("\"\"").is_err());
    }
}
