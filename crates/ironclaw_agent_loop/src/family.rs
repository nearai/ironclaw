use std::{borrow::Cow, collections::HashMap, fmt, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::planner::AgentLoopPlannerInternal;

/// Identity for a Builtin loop family.
///
/// Profile JSON serializes as a flat string. The registry is the authority on
/// whether a deserialized id is actually bound.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopFamilyId(Cow<'static, str>);

impl LoopFamilyId {
    pub const DEFAULT: Self = Self(Cow::Borrowed("default"));
    pub const SUBAGENT: Self = Self(Cow::Borrowed("subagent"));

    pub fn new(id: impl Into<Cow<'static, str>>) -> Result<Self, String> {
        let id = id.into();
        validate_loop_family_id(id.as_ref())?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Display for LoopFamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

impl Serialize for LoopFamilyId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LoopFamilyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn validate_loop_family_id(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("loop_family_id must not be empty".to_string());
    }
    if value.len() > 128 {
        return Err("loop_family_id must be at most 128 bytes".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' || c == ':')
    {
        return Err(
            "loop_family_id must contain only lowercase ASCII letters, digits, _, -, or :"
                .to_string(),
        );
    }
    Ok(())
}

/// Content digest for a component whose implementation affects replay safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentDigest(pub [u8; 32]);

impl ComponentDigest {
    pub fn from_blake3(bytes: impl AsRef<[u8]>) -> Self {
        Self(*blake3::hash(bytes.as_ref()).as_bytes())
    }
}

/// Content-addressed identity for a loop family, hook, skill snapshot, model
/// route, or other replay-relevant component.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComponentIdentity {
    pub id: Cow<'static, str>,
    pub digest: ComponentDigest,
}

impl ComponentIdentity {
    pub const fn from_static(id: &'static str, digest: ComponentDigest) -> Self {
        Self {
            id: Cow::Borrowed(id),
            digest,
        }
    }

    pub fn new(id: impl Into<Cow<'static, str>>, digest: ComponentDigest) -> Self {
        Self {
            id: id.into(),
            digest,
        }
    }
}

/// A Builtin loop family, opaque outside `ironclaw_agent_loop`.
///
/// Family factories are the only production constructors. Downstream crates can
/// resolve and hold a family, but cannot inspect or compose its planner slot.
pub struct LoopFamily {
    id: LoopFamilyId,
    version: ComponentIdentity,
    #[allow(dead_code)]
    planner: Arc<dyn AgentLoopPlannerInternal>,
}

impl LoopFamily {
    pub(crate) fn new(
        id: LoopFamilyId,
        version: ComponentIdentity,
        planner: Arc<dyn AgentLoopPlannerInternal>,
    ) -> Self {
        Self {
            id,
            version,
            planner,
        }
    }

    pub fn id(&self) -> &LoopFamilyId {
        &self.id
    }

    pub fn version(&self) -> &ComponentIdentity {
        &self.version
    }

    #[allow(dead_code)]
    pub(crate) fn planner(&self) -> &dyn AgentLoopPlannerInternal {
        self.planner.as_ref()
    }
}

/// Immutable singleton-style registry for Builtin loop families.
pub struct LoopFamilyRegistry {
    families: HashMap<LoopFamilyId, Arc<LoopFamily>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoopFamilyRegistryError {
    #[error("duplicate loop family id: {id}")]
    DuplicateFamilyId { id: LoopFamilyId },
}

impl LoopFamilyRegistry {
    pub fn get(&self, id: &LoopFamilyId) -> Option<Arc<LoopFamily>> {
        self.families.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &LoopFamilyId> {
        self.families.keys()
    }

    pub fn with_families(
        families: Vec<Arc<LoopFamily>>,
    ) -> Result<Arc<Self>, LoopFamilyRegistryError> {
        let mut map = HashMap::with_capacity(families.len());
        for family in families {
            let id = family.id().clone();
            if map.contains_key(&id) {
                return Err(LoopFamilyRegistryError::DuplicateFamilyId { id });
            }
            map.insert(id, family);
        }
        Ok(Arc::new(Self { families: map }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::default_planner::DefaultPlanner;

    use super::*;

    #[test]
    fn loop_family_id_default_is_flat_string() {
        assert_eq!(LoopFamilyId::DEFAULT.as_str(), "default");
        let json = serde_json::to_string(&LoopFamilyId::DEFAULT).expect("serialize id");
        assert_eq!(json, "\"default\"");
        let decoded: LoopFamilyId = serde_json::from_str(&json).expect("deserialize id");
        assert_eq!(decoded, LoopFamilyId::DEFAULT);
    }

    #[test]
    fn loop_family_id_subagent_is_flat_string() {
        assert_eq!(LoopFamilyId::SUBAGENT.as_str(), "subagent");
        let json = serde_json::to_string(&LoopFamilyId::SUBAGENT).expect("serialize id");
        assert_eq!(json, "\"subagent\"");
        let decoded: LoopFamilyId = serde_json::from_str(&json).expect("deserialize id");
        assert_eq!(decoded, LoopFamilyId::SUBAGENT);
    }

    #[test]
    fn loop_family_id_validates_construction_and_deserialization() {
        let valid = LoopFamilyId::new("default_family-1:stable").expect("valid id");
        assert_eq!(valid.as_str(), "default_family-1:stable");

        let too_long = "a".repeat(129);
        let invalid_values = [
            "",
            "Default",
            "with space",
            "bad\ncontrol",
            "path/ish",
            "path\\ish",
            too_long.as_str(),
        ];
        for value in invalid_values {
            assert!(
                LoopFamilyId::new(value.to_string()).is_err(),
                "expected invalid construction for {value:?}"
            );

            let json = serde_json::to_string(value).expect("serialize invalid id");
            assert!(
                serde_json::from_str::<LoopFamilyId>(&json).is_err(),
                "expected invalid deserialization for {value:?}"
            );
        }
    }

    #[test]
    fn component_identity_round_trips() {
        let identity = ComponentIdentity::from_static("default", ComponentDigest([7; 32]));
        let json = serde_json::to_string(&identity).expect("serialize identity");
        let decoded: ComponentIdentity = serde_json::from_str(&json).expect("deserialize identity");
        assert_eq!(decoded, identity);
    }

    #[test]
    fn registry_resolves_bound_family_only() {
        let family = Arc::new(LoopFamily::new(
            LoopFamilyId::DEFAULT,
            ComponentIdentity::from_static("default", ComponentDigest([0; 32])),
            Arc::new(DefaultPlanner::compose_default()),
        ));
        let registry = LoopFamilyRegistry::with_families(vec![family]).expect("valid registry");

        assert!(registry.get(&LoopFamilyId::DEFAULT).is_some());
        assert!(
            registry
                .get(&LoopFamilyId::new("unknown").expect("valid test id"))
                .is_none()
        );
        assert_eq!(registry.ids().count(), 1);
    }

    #[test]
    fn registry_rejects_duplicate_family_ids() {
        let family_a = Arc::new(LoopFamily::new(
            LoopFamilyId::DEFAULT,
            ComponentIdentity::from_static("default", ComponentDigest([1; 32])),
            Arc::new(DefaultPlanner::compose_default()),
        ));
        let family_b = Arc::new(LoopFamily::new(
            LoopFamilyId::DEFAULT,
            ComponentIdentity::from_static("default", ComponentDigest([2; 32])),
            Arc::new(DefaultPlanner::compose_default()),
        ));

        let error = match LoopFamilyRegistry::with_families(vec![family_a, family_b]) {
            Ok(_) => panic!("expected duplicate family id error"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            LoopFamilyRegistryError::DuplicateFamilyId {
                id: LoopFamilyId::DEFAULT
            }
        );
    }
}
