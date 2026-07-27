use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum length for bounded references, measured in bytes.
const MAX_BOUNDED_REF_LEN: usize = 256;

/// Validates that a bounded reference is non-empty, fits within the maximum
/// length in bytes, and contains no control characters.
fn validate_bounded_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > MAX_BOUNDED_REF_LEN {
        return Err(format!(
            "{kind} must be at most {MAX_BOUNDED_REF_LEN} bytes"
        ));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    Ok(())
}

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
bounded_ref!(TriggerOriginRef, "trigger_origin_ref");
bounded_ref!(TriggerFireSlot, "trigger_fire_slot");

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

    /// Derive the stable idempotency identity for one policy-authorized
    /// delivery fact. Attempt time is deliberately excluded: replaying the
    /// same projection for the same actor/target must address the same durable
    /// attempt, including after a process restart.
    pub(crate) fn for_policy_request(
        request: &crate::PrepareOutboundDeliveryRequest,
    ) -> Result<Self, crate::OutboundError> {
        #[derive(Serialize)]
        struct PolicyDeliveryIdentity<'a> {
            scope: &'a ironclaw_turns::TurnScope,
            actor: &'a ironclaw_turns::TurnActor,
            modality: crate::CommunicationModality,
            candidate: &'a crate::OutboundPushCandidate,
        }

        const POLICY_DELIVERY_NAMESPACE: Uuid =
            Uuid::from_u128(0x32bfed3f_94c7_5a74_89be_b38603aab29f);
        let identity = PolicyDeliveryIdentity {
            scope: &request.scope,
            actor: &request.actor,
            modality: request.modality,
            candidate: &request.candidate,
        };
        let serialized =
            serde_json::to_vec(&identity).map_err(|_| crate::OutboundError::Serialization)?;
        Ok(Self(Uuid::new_v5(&POLICY_DELIVERY_NAMESPACE, &serialized)))
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
    use serde_json::from_str;

    macro_rules! assert_invalid_inputs {
        ($ty:ty, $kind:literal) => {{
            let empty = "\"\"";
            let overlong = format!("\"{}\"", "x".repeat(MAX_BOUNDED_REF_LEN + 1));
            let control = "\"bad\\nvalue\"";

            assert!(
                <$ty>::new("").is_err(),
                concat!($kind, " should reject empty values")
            );
            assert!(
                from_str::<$ty>(empty).is_err(),
                concat!($kind, " should reject empty JSON input")
            );
            assert!(
                from_str::<$ty>(&overlong).is_err(),
                concat!($kind, " should reject overlong JSON input")
            );
            assert!(
                from_str::<$ty>(control).is_err(),
                concat!($kind, " should reject control characters")
            );
        }};
    }

    #[test]
    fn bounded_refs_reject_invalid_inputs() {
        assert_invalid_inputs!(ProjectionSubscriptionId, "projection_subscription_id");
        assert_invalid_inputs!(ProjectionUpdateRef, "projection_update_ref");
        assert_invalid_inputs!(TriggerOriginRef, "trigger_origin_ref");
        assert_invalid_inputs!(TriggerFireSlot, "trigger_fire_slot");
    }
}
