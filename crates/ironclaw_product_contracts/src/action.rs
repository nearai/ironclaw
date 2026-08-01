//! Inbound-action identity and the bounded tokens product DTOs key on
//! (PROPOSAL §6.1.3).
//!
//! A single mutating action accepted at the product boundary is identified by
//! a [`ProductActionId`] and deduplicated by an [`ActionFingerprintKey`]:
//! tenant-scoped installation + external actor + source binding + external
//! event id. The durable ledger record and its saga phases are product
//! workflow state and stay in `ironclaw_product`; what crosses the boundary is
//! the identity and the fingerprint, because every caller that builds a
//! command context — including `ironclaw_extension_host`'s channel hosts —
//! must speak them.
//!
//! Never here: the ledger, its store, or the action saga.

use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalEventId};
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a product inbound action ledger entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProductActionId(Uuid);

impl ProductActionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ProductActionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ProductActionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

const SOURCE_BINDING_KEY_MAX_BYTES: usize = 2_048;
const PRODUCT_COMMAND_NAME_MAX_BYTES: usize = 256;
const INTERACTION_REF_MAX_BYTES: usize = 512;

fn validate_typed_token(kind: &'static str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{kind} exceeds {max_bytes}-byte limit"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} contains unsupported control characters"));
    }
    Ok(())
}

macro_rules! typed_token {
    ($name:ident, $kind:literal, $max_bytes:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_typed_token($kind, &value, $max_bytes)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

typed_token!(
    SourceBindingKey,
    "source binding key",
    SOURCE_BINDING_KEY_MAX_BYTES
);
typed_token!(
    ProductCommandName,
    "product command name",
    PRODUCT_COMMAND_NAME_MAX_BYTES
);
typed_token!(
    AuthRequestRef,
    "auth request ref",
    INTERACTION_REF_MAX_BYTES
);
typed_token!(
    LinkedThreadActionId,
    "linked thread action id",
    INTERACTION_REF_MAX_BYTES
);

/// Composite deduplication key for inbound actions. Two envelopes with the same
/// fingerprint are considered duplicates and the second will replay the first
/// outcome.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionFingerprintKey {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub source_binding_key: SourceBindingKey,
    pub external_event_id: ExternalEventId,
}

impl ActionFingerprintKey {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        source_binding_key: SourceBindingKey,
        external_event_id: ExternalEventId,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            external_actor_ref,
            source_binding_key,
            external_event_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalEventId};
    use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};

    #[test]
    fn typed_tokens_reject_empty_oversized_and_control_values() {
        assert!(SourceBindingKey::new("").is_err());
        assert!(ProductCommandName::new("x".repeat(PRODUCT_COMMAND_NAME_MAX_BYTES + 1)).is_err());
        assert!(AuthRequestRef::new("auth\nrequest").is_err());
        // NUL is rejected by its own arm of the guard, not by `is_control()`:
        // a token that reaches a path or a header with an embedded NUL
        // truncates at the C boundary, so both arms must be live.
        assert!(SourceBindingKey::new("space:0:;conv\0ersation").is_err());

        let linked = LinkedThreadActionId::new("open-thread").expect("valid action id");
        assert_eq!(linked.as_str(), "open-thread");
        assert_eq!(linked.clone().into_inner(), "open-thread");
        assert_eq!(String::from(linked), "open-thread");
    }

    #[test]
    fn typed_tokens_round_trip_through_serde_as_ref_and_display() {
        // The bounded-token template generates `TryFrom<String>` (the serde
        // entry point), `AsRef<str>`, and `Display` for every token; the
        // validating constructor alone leaves all three unexercised, and a
        // token that deserializes without validating is the defect the
        // `try_from` attribute exists to prevent.
        let key: SourceBindingKey =
            serde_json::from_value(serde_json::json!("space:0:;conversation:2:C1;topic:0:;"))
                .expect("valid token deserializes");
        assert_eq!(key.as_ref(), "space:0:;conversation:2:C1;topic:0:;");
        assert_eq!(key.to_string(), "space:0:;conversation:2:C1;topic:0:;");
        assert_eq!(
            serde_json::to_value(&key).expect("serialize"),
            serde_json::json!("space:0:;conversation:2:C1;topic:0:;")
        );

        let rejected = serde_json::from_value::<SourceBindingKey>(serde_json::json!(""));
        assert!(
            rejected.is_err(),
            "deserialization must run the same validation as the constructor"
        );
        assert!(
            serde_json::from_value::<AuthRequestRef>(serde_json::json!("auth\u{7}ref")).is_err(),
            "control characters are rejected through serde too"
        );
    }

    #[test]
    fn product_action_id_round_trips_display_and_uuid() {
        let action_id = ProductActionId::new();
        assert_eq!(action_id.to_string(), action_id.as_uuid().to_string());
        assert_ne!(ProductActionId::default().as_uuid(), action_id.as_uuid());
    }

    #[test]
    fn action_fingerprint_key_carries_every_dedup_component() {
        let key = ActionFingerprintKey::new(
            ProductAdapterId::new("test_adapter").expect("valid adapter"),
            AdapterInstallationId::new("install_alpha").expect("valid installation"),
            ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor"),
            SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
                .expect("valid source binding"),
            ExternalEventId::new("evt:action").expect("valid event"),
        );
        assert_eq!(key.adapter_id.as_str(), "test_adapter");
        assert_eq!(
            key.source_binding_key.as_str(),
            "space:0:;conversation:5:conv1;topic:0:;"
        );
        assert_eq!(key, key.clone());
    }
}
