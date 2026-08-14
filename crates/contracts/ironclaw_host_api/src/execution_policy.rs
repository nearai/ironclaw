//! Persistable, provider-neutral execution restrictions for one turn.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{error::HostApiError, ids::CapabilityId};

const MAX_REQUIRED_SKILL_NAME_BYTES: usize = 64;

/// Exact skill name that must be activated before execution begins.
///
/// Skill source scopes are intentionally not persisted here: the activation
/// catalog already owns visibility and rejects ambiguous names. Persisting a
/// second source taxonomy would let this neutral contract drift from that
/// catalog.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RequiredSkill(String);

impl RequiredSkill {
    fn validate(value: &str) -> Result<(), HostApiError> {
        if value.trim() != value || value.is_empty() {
            return Err(HostApiError::invalid_id(
                "required_skill",
                value,
                "must be non-empty and have no surrounding whitespace",
            ));
        }
        if value.len() > MAX_REQUIRED_SKILL_NAME_BYTES {
            return Err(HostApiError::invalid_id(
                "required_skill",
                value,
                "must be at most 64 bytes",
            ));
        }
        let mut characters = value.chars();
        let Some(first) = characters.next() else {
            return Err(HostApiError::invalid_id(
                "required_skill",
                value,
                "must be non-empty",
            ));
        };
        if !first.is_ascii_alphanumeric()
            || !characters.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
        {
            return Err(HostApiError::invalid_id(
                "required_skill",
                value,
                "must match the skill-name grammar: an ASCII alphanumeric followed by ASCII alphanumerics, dots, underscores, or hyphens",
            ));
        }
        Ok(())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RequiredSkill {
    type Error = HostApiError;

    fn try_from(value: String) -> Result<Self, HostApiError> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for RequiredSkill {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequiredSkill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<RequiredSkill> for String {
    fn from(skill: RequiredSkill) -> Self {
        skill.0
    }
}

/// Restrictions attached to a turn by a trusted host-owned caller.
///
/// Unknown fields are rejected: a misspelled restriction key must fail
/// deserialization loudly instead of silently widening the surface back to the
/// caller's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnExecutionPolicy {
    /// `None` preserves the caller's normal surface; `Some([])` exposes no
    /// capabilities; a non-empty value only narrows the existing surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_capability_ids: Option<Vec<CapabilityId>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_skills: Vec<RequiredSkill>,
}

#[cfg(test)]
mod tests {
    use super::{RequiredSkill, TurnExecutionPolicy};

    #[test]
    fn required_skill_uses_the_catalog_name_grammar() {
        assert!(RequiredSkill::new("code-review.v2").is_ok());
        assert!(RequiredSkill::new("-code-review").is_err());
        assert!(RequiredSkill::new("code/review").is_err());
        assert!(RequiredSkill::new("éxample").is_err());
        assert!(RequiredSkill::new("a".repeat(65)).is_err());
    }

    #[test]
    fn required_skill_round_trips_as_a_plain_json_string() {
        let skill = RequiredSkill::new("code-review.v2").expect("valid skill name");
        let wire = serde_json::to_string(&skill).expect("serialize");
        assert_eq!(wire, "\"code-review.v2\"");
        let back: RequiredSkill = serde_json::from_str(&wire).expect("deserialize");
        assert_eq!(back, skill);
    }

    #[test]
    fn required_skill_rejects_invalid_wire_values() {
        for wire in ["\"\"", "\" padded \"", "\"code/review\"", "\"-leading\""] {
            assert!(
                serde_json::from_str::<RequiredSkill>(wire).is_err(),
                "wire value {wire} must fail validation on deserialize"
            );
        }
    }

    #[test]
    fn policy_rejects_unknown_fields_instead_of_dropping_restrictions() {
        // A typoed restriction key must not deserialize into the unrestricted
        // default policy — that would silently remove the capability allowlist.
        let wire = r#"{"allowed_capability_ids_typo":[]}"#;
        assert!(serde_json::from_str::<TurnExecutionPolicy>(wire).is_err());

        let valid = r#"{"allowed_capability_ids":[],"required_skills":["triage"]}"#;
        let policy: TurnExecutionPolicy = serde_json::from_str(valid).expect("valid policy");
        assert_eq!(policy.allowed_capability_ids, Some(Vec::new()));
        assert_eq!(policy.required_skills.len(), 1);
    }
}
