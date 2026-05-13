use std::collections::BTreeSet;

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::LoopRunContext;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityAllowSet {
    All,
    Allowlist(BTreeSet<CapabilityId>),
}

impl CapabilityAllowSet {
    pub fn allowlist(ids: impl IntoIterator<Item = CapabilityId>) -> Self {
        Self::Allowlist(ids.into_iter().collect())
    }

    pub fn permits(&self, id: &CapabilityId) -> bool {
        match self {
            Self::All => true,
            Self::Allowlist(set) => set.contains(id),
        }
    }
}

#[derive(Debug, Error)]
pub enum CapabilityResolveError {
    #[error("capability surface profile is unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("capability surface profile could not be resolved: {reason}")]
    Internal { reason: String },
}

impl CapabilityResolveError {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub fn internal(reason: impl Into<String>) -> Self {
        Self::Internal {
            reason: reason.into(),
        }
    }
}

#[async_trait]
pub trait CapabilitySurfaceProfileResolver: Send + Sync {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("test capability id is valid")
    }

    #[test]
    fn all_permits_anything() {
        assert!(CapabilityAllowSet::All.permits(&capability_id("demo.any")));
    }

    #[test]
    fn allowlist_permits_listed() {
        let allowed = capability_id("demo.allowed");
        let denied = capability_id("demo.denied");
        let allow_set = CapabilityAllowSet::allowlist([allowed.clone()]);

        assert!(allow_set.permits(&allowed));
        assert!(!allow_set.permits(&denied));
    }
}
