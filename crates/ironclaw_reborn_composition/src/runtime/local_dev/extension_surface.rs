use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, EffectKind, ExtensionId, GrantConstraints, MountView,
    NetworkPolicy, Principal,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

use crate::extension_lifecycle::{ActiveExtensionCapability, RebornLocalExtensionManagementPort};

#[derive(Clone, Default)]
pub(super) struct LocalDevExtensionSurfaceSource {
    extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
}

impl LocalDevExtensionSurfaceSource {
    pub(super) fn new(
        extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    ) -> Self {
        Self {
            extension_management,
        }
    }

    pub(super) fn snapshot(&self) -> LocalDevExtensionSurface {
        self.extension_management
            .as_deref()
            .map(LocalDevExtensionSurface::from_extension_management)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct LocalDevExtensionSurface {
    active_capabilities: Vec<ActiveExtensionCapability>,
}

impl LocalDevExtensionSurface {
    pub(super) fn from_extension_management(
        extension_management: &RebornLocalExtensionManagementPort,
    ) -> Self {
        Self {
            active_capabilities: extension_management.active_model_visible_capabilities(),
        }
    }

    pub(super) fn grants(&self, grantee: &ExtensionId) -> Vec<CapabilityGrant> {
        self.active_capabilities
            .iter()
            .map(|capability| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability.id.clone(),
                grantee: Principal::Extension(grantee.clone()),
                issued_by: Principal::HostRuntime,
                constraints: Self::grant_constraints(capability),
            })
            .collect()
    }

    pub(super) fn provider_trust(&self) -> BTreeMap<ExtensionId, TrustDecision> {
        let mut effects_by_provider: BTreeMap<ExtensionId, Vec<EffectKind>> = BTreeMap::new();
        for capability in &self.active_capabilities {
            let effects = effects_by_provider
                .entry(capability.provider.clone())
                .or_default();
            for effect in &capability.effects {
                if !effects.contains(effect) {
                    effects.push(*effect);
                }
            }
        }

        effects_by_provider
            .into_iter()
            .map(|(provider, allowed_effects)| {
                (
                    provider,
                    TrustDecision {
                        effective_trust: EffectiveTrustClass::user_trusted(),
                        authority_ceiling: AuthorityCeiling {
                            allowed_effects,
                            max_resource_ceiling: None,
                        },
                        provenance: TrustProvenance::AdminConfig,
                        evaluated_at: Utc::now(),
                    },
                )
            })
            .collect()
    }

    fn grant_constraints(capability: &ActiveExtensionCapability) -> GrantConstraints {
        GrantConstraints {
            allowed_effects: capability.effects.clone(),
            mounts: MountView::default(),
            network: NetworkPolicy {
                // Installed extensions get only their declared credential audiences as
                // egress targets; missing audiences intentionally fail closed.
                allowed_targets: capability.runtime_credentials.iter().fold(
                    Vec::new(),
                    |mut targets, credential| {
                        if !targets.contains(&credential.audience) {
                            targets.push(credential.audience.clone());
                        }
                        targets
                    },
                ),
                deny_private_ip_ranges: true,
                max_egress_bytes: None,
            },
            secrets: capability.runtime_credentials.iter().fold(
                Vec::new(),
                |mut handles, credential| {
                    if !handles.contains(&credential.handle) {
                        handles.push(credential.handle.clone());
                    }
                    handles
                },
            ),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        }
    }
}
