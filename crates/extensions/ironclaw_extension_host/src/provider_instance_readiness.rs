//! Per-provider "did the operator configure this instance's backend at all"
//! readiness.
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
//!
//! Readiness is resolved through [`ProviderInstanceReadinessPort`] **per
//! activation**, not snapshotted at composition time. A deployment whose
//! operator supplies vendor client credentials through administrator
//! configuration writes them long after boot, and the OAuth engine already
//! resolves that source per request — a boot-time snapshot would answer
//! "unconfigured" for a vendor the engine can, right now, start a flow for.
//! [`StaticProviderInstanceReadiness`] remains the shape for hosts whose
//! configuration genuinely is fixed at build time.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use ironclaw_host_api::ids::VendorId;

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

/// Resolve whether a vendor's host-level instance configuration exists,
/// returning the operator remediation when it does not.
///
/// `None` means configured: the host can offer this vendor's flow now.
#[async_trait]
pub trait ProviderInstanceReadinessPort: Send + Sync + fmt::Debug {
    async fn remediation_for(&self, provider: &VendorId) -> Option<String>;
}

/// Readiness fixed at composition time, for hosts whose provider
/// configuration cannot change while the process runs.
#[derive(Debug, Clone, Default)]
pub struct StaticProviderInstanceReadiness {
    unconfigured: BTreeMap<VendorId, String>,
}

impl StaticProviderInstanceReadiness {
    pub fn new(unconfigured: BTreeMap<VendorId, String>) -> Self {
        Self { unconfigured }
    }
}

impl From<BTreeMap<VendorId, String>> for StaticProviderInstanceReadiness {
    fn from(unconfigured: BTreeMap<VendorId, String>) -> Self {
        Self::new(unconfigured)
    }
}

#[async_trait]
impl ProviderInstanceReadinessPort for StaticProviderInstanceReadiness {
    async fn remediation_for(&self, provider: &VendorId) -> Option<String> {
        self.unconfigured.get(provider).cloned()
    }
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

    #[tokio::test]
    async fn static_readiness_reports_the_unconfigured_provider_remediation() {
        let readiness = StaticProviderInstanceReadiness::from(provider_instance_readiness_map([
            ProviderInstanceReadinessInput {
                provider: provider(),
                configured: false,
                remediation: "configure provider A".to_string(),
            },
        ]));
        assert_eq!(
            readiness.remediation_for(&provider()).await,
            Some("configure provider A".to_string())
        );
    }

    #[tokio::test]
    async fn static_readiness_stays_silent_for_a_configured_provider() {
        let readiness = StaticProviderInstanceReadiness::from(provider_instance_readiness_map([
            ProviderInstanceReadinessInput {
                provider: provider(),
                configured: true,
                remediation: "configure provider A".to_string(),
            },
        ]));
        assert_eq!(readiness.remediation_for(&provider()).await, None);
    }
}
