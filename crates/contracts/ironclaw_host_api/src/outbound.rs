//! Provider-neutral outbound routing vocabulary.
//!
//! Target identifiers cross extension inventory, trigger persistence, and
//! mediated capability boundaries. Their shape and validation therefore live
//! here rather than in any one outbound consumer.

use serde::{Deserialize, Serialize};

const OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES: usize = 512;

/// Opaque identifier returned by an outbound target registry.
///
/// Provider-specific structure is intentionally not exposed. Callers may only
/// persist or select values returned by the authoritative registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct OutboundDeliveryTargetId(String);

impl OutboundDeliveryTargetId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_target_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for OutboundDeliveryTargetId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for OutboundDeliveryTargetId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for OutboundDeliveryTargetId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<OutboundDeliveryTargetId> for String {
    fn from(value: OutboundDeliveryTargetId) -> Self {
        value.0
    }
}

fn validate_target_id(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("outbound delivery target id must not be empty".to_string());
    }
    if value.len() > OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES {
        return Err(format!(
            "outbound delivery target id must be at most {OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES} bytes"
        ));
    }
    if value.trim() != value {
        return Err("outbound delivery target id must not have surrounding whitespace".to_string());
    }
    if value.chars().any(char::is_control) {
        return Err("outbound delivery target id must not contain control characters".to_string());
    }
    if has_unsafe_unicode_format_character(value) {
        return Err(
            "outbound delivery target id must not contain unsafe Unicode formatting characters"
                .to_string(),
        );
    }
    if value
        .chars()
        .any(|character| matches!(character, '\u{2028}' | '\u{2029}'))
    {
        return Err(
            "outbound delivery target id must not contain line or paragraph separators".to_string(),
        );
    }
    Ok(())
}

fn has_unsafe_unicode_format_character(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{00ad}'
                | '\u{034f}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200d}'
                | '\u{2060}'
                | '\u{feff}'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::OutboundDeliveryTargetId;

    #[test]
    fn target_id_validation_and_serde_are_one_neutral_contract() {
        let value = format!("provider:{}", "x".repeat(503));
        let target = OutboundDeliveryTargetId::new(value.clone()).expect("512-byte target");
        let encoded = serde_json::to_value(&target).expect("serialize target");
        assert_eq!(encoded, serde_json::json!(value));
        assert_eq!(
            serde_json::from_value::<OutboundDeliveryTargetId>(encoded)
                .expect("deserialize target"),
            target
        );

        assert!(OutboundDeliveryTargetId::new("x".repeat(513)).is_err());
        assert!(OutboundDeliveryTargetId::new(" target").is_err());
        assert!(OutboundDeliveryTargetId::new("target\n").is_err());
        assert!(OutboundDeliveryTargetId::new("target:\u{200b}hidden").is_err());
    }
}
