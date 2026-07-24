//! Static per-provider "did the operator configure this instance's backend at
//! all" readiness map.
//!
//! This is a third readiness axis alongside static package requirements and
//! per-user account setup. It answers whether the host can offer a provider
//! flow at all; it does not answer whether a package needs credentials or a
//! user has connected an account.
//!
//! The unified extension runtime keeps extension-owned setup in manifests and
//! account-setup descriptors. The composition root supplies only the
//! configured/not-configured signal; administrator fields and remediation stay
//! behind the authorized Admin Configuration surface.

use std::collections::BTreeSet;

use ironclaw_host_api::VendorId;

/// One build-time host-owned signal used for provider-instance readiness.
pub(crate) struct ProviderInstanceReadinessInput {
    pub(crate) provider: VendorId,
    pub(crate) configured: bool,
}

/// Return providers whose host-level configuration is absent. Administrator
/// field metadata and remediation never enter the caller lifecycle domain.
pub(crate) fn provider_instance_readiness_map(
    inputs: impl IntoIterator<Item = ProviderInstanceReadinessInput>,
) -> BTreeSet<VendorId> {
    let mut map = BTreeSet::new();
    for input in inputs {
        if !input.configured {
            map.insert(input.provider);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> VendorId {
        VendorId::new("provider-a").expect("test provider id is valid")
    }

    #[test]
    fn entry_present_when_not_configured() {
        let map = provider_instance_readiness_map([ProviderInstanceReadinessInput {
            provider: provider(),
            configured: false,
        }]);
        assert!(map.contains(&provider()));
    }

    #[test]
    fn entry_absent_when_configured() {
        let map = provider_instance_readiness_map([ProviderInstanceReadinessInput {
            provider: provider(),
            configured: true,
        }]);
        assert!(!map.contains(&provider()));
    }
}
