use std::{borrow::Cow, collections::HashMap, fmt, sync::Arc};

use serde::{Deserialize, Serialize};

/// Identity for a Builtin loop family.
///
/// Profile JSON serializes as a flat string. The registry is the authority on
/// whether a deserialized id is actually bound.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoopFamilyId(pub Cow<'static, str>);

impl LoopFamilyId {
    pub const DEFAULT: Self = Self(Cow::Borrowed("default"));

    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for LoopFamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

/// Content digest for a component whose implementation affects replay safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentDigest(pub [u8; 32]);

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

pub(crate) trait LoopFamilyPlanner: Send + Sync {}

/// A Builtin loop family, opaque outside `ironclaw_agent_loop`.
///
/// Family factories are the only production constructors. Downstream crates can
/// resolve and hold a family, but cannot inspect or compose its planner slot.
pub struct LoopFamily {
    id: LoopFamilyId,
    version: ComponentIdentity,
    _planner: Arc<dyn LoopFamilyPlanner>,
}

impl LoopFamily {
    pub(crate) fn new(
        id: LoopFamilyId,
        version: ComponentIdentity,
        planner: Arc<dyn LoopFamilyPlanner>,
    ) -> Self {
        Self {
            id,
            version,
            _planner: planner,
        }
    }

    pub fn id(&self) -> &LoopFamilyId {
        &self.id
    }

    pub fn version(&self) -> &ComponentIdentity {
        &self.version
    }
}

/// Immutable singleton-style registry for Builtin loop families.
pub struct LoopFamilyRegistry {
    families: HashMap<LoopFamilyId, Arc<LoopFamily>>,
}

impl LoopFamilyRegistry {
    pub fn get(&self, id: &LoopFamilyId) -> Option<Arc<LoopFamily>> {
        self.families.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &LoopFamilyId> {
        self.families.keys()
    }

    pub fn with_families(families: Vec<Arc<LoopFamily>>) -> Arc<Self> {
        let mut map = HashMap::with_capacity(families.len());
        for family in families {
            map.insert(family.id().clone(), family);
        }
        Arc::new(Self { families: map })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct TestPlanner;

    impl LoopFamilyPlanner for TestPlanner {}

    #[test]
    fn loop_family_id_default_is_flat_string() {
        assert_eq!(LoopFamilyId::DEFAULT.0, "default");
        let json = serde_json::to_string(&LoopFamilyId::DEFAULT).expect("serialize id");
        assert_eq!(json, "\"default\"");
        let decoded: LoopFamilyId = serde_json::from_str(&json).expect("deserialize id");
        assert_eq!(decoded, LoopFamilyId::DEFAULT);
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
            Arc::new(TestPlanner),
        ));
        let registry = LoopFamilyRegistry::with_families(vec![family]);

        assert!(registry.get(&LoopFamilyId::DEFAULT).is_some());
        assert!(registry.get(&LoopFamilyId::new("unknown")).is_none());
        assert_eq!(registry.ids().count(), 1);
    }
}
