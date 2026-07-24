//! Static per-provider "did the operator configure this instance's backend at
//! all" readiness map.
//!
//! This is a third readiness axis alongside static package requirements and
//! per-user account setup. It answers whether the host can offer a provider
//! flow at all; it does not answer whether a package needs credentials or a
//! user has connected an account.
//!
//! The unified extension runtime keeps extension-owned setup in manifests and
//! account-setup descriptors. Concrete-provider detection and remediation are
//! supplied by the composition root; this module only applies the generic
//! configured-or-remediate rule.

use std::collections::BTreeMap;

use ironclaw_host_api::VendorId;

/// One build-time host-owned signal used for provider-instance readiness.
pub struct ProviderInstanceReadinessInput {
    pub provider: VendorId,
    pub configured: bool,
    pub remediation: String,
}

/// Return remediation for providers whose host-level configuration is absent.
pub fn provider_instance_readiness_map(
    inputs: impl IntoIterator<Item = ProviderInstanceReadinessInput>,
) -> BTreeMap<VendorId, String> {
    let mut map = BTreeMap::new();
    for input in inputs {
        if !input.configured {
            map.insert(input.provider, input.remediation);
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
            remediation: "configure provider A".to_string(),
        }]);
        assert_eq!(
            map.get(&provider()).map(String::as_str),
            Some("configure provider A")
        );
    }

    #[test]
    fn entry_absent_when_configured() {
        let map = provider_instance_readiness_map([ProviderInstanceReadinessInput {
            provider: provider(),
            configured: true,
            remediation: "configure provider A".to_string(),
        }]);
        assert!(!map.contains_key(&provider()));
    }
}
