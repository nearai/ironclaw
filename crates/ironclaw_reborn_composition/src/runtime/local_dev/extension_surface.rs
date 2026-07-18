use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, EffectKind, ExtensionId, GrantConstraints,
    MountView, NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

use crate::extension_host::extension_lifecycle::{
    ActiveExtensionCapability, RebornLocalExtensionManagementPort,
};
use ironclaw_first_party_extensions::{
    EXA_MCP_HOST, NETWORK_EGRESS_LIMIT, WEB_ACCESS_EXTENSION_ID, WEB_GET_CONTENT_CAPABILITY_ID,
    WEB_SEARCH_CAPABILITY_ID, gsuite_network_policy_for,
};
use ironclaw_product_workflow::ProductWorkflowError;

#[derive(Clone, Default)]
pub(in crate::runtime) struct ExtensionCapabilitySurfaceSource {
    extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    #[cfg(test)]
    static_surface: Option<ExtensionCapabilitySurface>,
}

impl ExtensionCapabilitySurfaceSource {
    pub(in crate::runtime) fn new(
        extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    ) -> Self {
        Self {
            extension_management,
            #[cfg(test)]
            static_surface: None,
        }
    }

    #[cfg(test)]
    pub(in crate::runtime) fn from_surface(surface: ExtensionCapabilitySurface) -> Self {
        Self {
            extension_management: None,
            static_surface: Some(surface),
        }
    }

    pub(in crate::runtime) async fn snapshot(
        &self,
    ) -> Result<ExtensionCapabilitySurface, ProductWorkflowError> {
        #[cfg(test)]
        if let Some(surface) = &self.static_surface {
            return Ok(surface.clone());
        }
        let Some(extension_management) = self.extension_management.as_deref() else {
            return Ok(ExtensionCapabilitySurface::default());
        };
        ExtensionCapabilitySurface::from_extension_management(extension_management).await
    }
}

#[derive(Debug, Clone, Default)]
pub(in crate::runtime) struct ExtensionCapabilitySurface {
    active_capabilities: Vec<ActiveExtensionCapability>,
}

impl ExtensionCapabilitySurface {
    #[cfg(test)]
    pub(in crate::runtime) fn from_active_capabilities(
        active_capabilities: Vec<ActiveExtensionCapability>,
    ) -> Self {
        Self {
            active_capabilities,
        }
    }

    pub(super) async fn from_extension_management(
        extension_management: &RebornLocalExtensionManagementPort,
    ) -> Result<Self, ProductWorkflowError> {
        Ok(Self {
            active_capabilities: extension_management
                .active_model_visible_capabilities()
                .await?,
        })
    }

    /// Mint capability grants for one request (#5459 P1: filtered to the
    /// CALLER — tenant-owned capabilities grant to everyone, user-private ones
    /// only to their owner). This is the single choke point: dispatch
    /// authorization reuses the grants minted here, so a capability filtered
    /// out is both invisible in the surface AND denied at dispatch — grant
    /// absence fails closed with no separate preflight.
    pub(in crate::runtime) fn grants(
        &self,
        grantee: &ExtensionId,
        caller: &ironclaw_host_api::UserId,
    ) -> Vec<CapabilityGrant> {
        self.active_capabilities
            .iter()
            .filter(|capability| capability.owner.visible_to(caller))
            .map(|capability| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability.id.clone(),
                grantee: Principal::Extension(grantee.clone()),
                issued_by: Principal::HostRuntime,
                constraints: Self::grant_constraints(capability),
            })
            .collect()
    }

    pub(in crate::runtime) fn capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&ActiveExtensionCapability> {
        self.active_capabilities
            .iter()
            .find(|capability| capability.id == *capability_id)
    }

    /// Provider trust for the same request; filtered by the same owner rule as
    /// [`Self::grants`] so a user-private extension's provider is not even
    /// advertised to other users' surfaces.
    pub(super) fn provider_trust(
        &self,
        caller: &ironclaw_host_api::UserId,
    ) -> BTreeMap<ExtensionId, TrustDecision> {
        let mut effects_by_provider: BTreeMap<ExtensionId, Vec<EffectKind>> = BTreeMap::new();
        for capability in &self.active_capabilities {
            if !capability.owner.visible_to(caller) {
                continue;
            }
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
            network: extension_network_policy(capability),
            secrets: {
                let mut handles = Vec::new();
                for credential in &capability.runtime_credentials {
                    if !handles.contains(&credential.handle) {
                        handles.push(credential.handle.clone());
                    }
                }
                handles
            },
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        }
    }
}

fn extension_network_policy(capability: &ActiveExtensionCapability) -> NetworkPolicy {
    if let Some(policy) = gsuite_network_policy_for(&capability.provider) {
        return policy;
    }

    let mut targets = Vec::new();
    // Manifest-declared egress allowlist — the keyless-but-networked path. A
    // capability declares its egress hosts directly, without a credential
    // (and therefore without forcing a secret injection).
    for target in &capability.network_targets {
        if !targets.contains(target) {
            targets.push(target.clone());
        }
    }
    // Credential audiences are folded in on top: a credentialed egress host is
    // reachable whether or not it was also listed in `network_targets`.
    for credential in &capability.runtime_credentials {
        if !targets.contains(&credential.audience) {
            targets.push(credential.audience.clone());
        }
    }
    let is_web_access_exa_mcp = capability.provider.as_str() == WEB_ACCESS_EXTENSION_ID
        && matches!(
            capability.id.as_str(),
            WEB_SEARCH_CAPABILITY_ID | WEB_GET_CONTENT_CAPABILITY_ID
        );
    if is_web_access_exa_mcp
        && !targets
            .iter()
            .any(|target| target.host_pattern == EXA_MCP_HOST)
    {
        targets.push(NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: EXA_MCP_HOST.to_string(),
            port: None,
        });
    }
    // Only mark the policy "constrained" (via `deny_private_ip_ranges`) when the
    // capability actually declares egress targets. `network_policy_is_constrained`
    // treats `deny_private_ip_ranges` as a constraint, so an unconditional `true`
    // would give even a no-network tool (no `network` effect, no `network_targets`,
    // no credential) a non-empty-looking policy → a spurious `ApplyNetworkPolicy`
    // obligation with an empty allowlist that fails `validate_network_policy_metadata`.
    // A tool with no egress targets gets an unconstrained empty policy → no network
    // obligation at all. Networked tools keep the private-IP SSRF guard on their
    // declared targets. (A tool that declares the `network` effect but no targets is
    // still caught by the effect-based obligation gate and fails as misconfigured.)
    let has_egress_targets = !targets.is_empty();
    NetworkPolicy {
        allowed_targets: targets,
        deny_private_ip_ranges: has_egress_targets,
        max_egress_bytes: is_web_access_exa_mcp.then_some(NETWORK_EGRESS_LIMIT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_first_party_extensions::google_api_network_policy;
    use ironclaw_host_api::{CapabilityId, PermissionMode, UserId};

    /// #5459 P1: the grant-minting choke point — a member-held extension's
    /// capabilities mint grants (and advertise provider trust) ONLY for its
    /// members (every member of the set, not just one); tenant-owned ones
    /// mint for everyone. Grant absence is what makes an un-held tool both
    /// invisible in the surface and denied at dispatch, so this filter IS
    /// the enforcement.
    #[test]
    fn grants_and_provider_trust_filter_member_held_capabilities_to_their_members() {
        let alice = UserId::new("alice").unwrap();
        let bob = UserId::new("bob").unwrap();
        let carol = UserId::new("carol").unwrap();
        let grantee = ExtensionId::new("caller").unwrap();
        let capability = |id: &str, provider: &str, owner| ActiveExtensionCapability {
            id: CapabilityId::new(id).unwrap(),
            provider: ExtensionId::new(provider).unwrap(),
            effects: vec![EffectKind::DispatchCapability],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            owner,
        };
        let surface = ExtensionCapabilitySurface::from_active_capabilities(vec![
            capability(
                "market-data.snp500",
                "market-data",
                ironclaw_extensions::InstallationOwner::users(
                    [alice.clone(), bob.clone()].into_iter().collect(),
                )
                .expect("member set"),
            ),
            capability(
                "hacker-news.top_stories",
                "hacker-news",
                ironclaw_extensions::InstallationOwner::Tenant,
            ),
        ]);

        for member in [&alice, &bob] {
            let member_capabilities: Vec<_> = surface
                .grants(&grantee, member)
                .into_iter()
                .map(|grant| grant.capability.as_str().to_string())
                .collect();
            assert!(
                member_capabilities.contains(&"market-data.snp500".to_string()),
                "every member of the set gets the grant"
            );
            assert!(member_capabilities.contains(&"hacker-news.top_stories".to_string()));
        }

        let carol_capabilities: Vec<_> = surface
            .grants(&grantee, &carol)
            .into_iter()
            .map(|grant| grant.capability.as_str().to_string())
            .collect();
        assert!(
            !carol_capabilities.contains(&"market-data.snp500".to_string()),
            "a member-held capability must not mint a grant for a non-member"
        );
        assert!(carol_capabilities.contains(&"hacker-news.top_stories".to_string()));

        for member in [&alice, &bob] {
            let member_trust = surface.provider_trust(member);
            assert!(member_trust.contains_key(&ExtensionId::new("market-data").unwrap()));
        }
        let carol_trust = surface.provider_trust(&carol);
        assert!(
            !carol_trust.contains_key(&ExtensionId::new("market-data").unwrap()),
            "a member-held provider must not be advertised to a non-member"
        );
        assert!(carol_trust.contains_key(&ExtensionId::new("hacker-news").unwrap()));
    }

    #[test]
    fn web_access_search_gets_exa_mcp_network_target_without_credentials() {
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new(WEB_SEARCH_CAPABILITY_ID).unwrap(),
            provider: ExtensionId::new(WEB_ACCESS_EXTENSION_ID).unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(
            policy.allowed_targets,
            vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: EXA_MCP_HOST.to_string(),
                port: None,
            }]
        );
        assert!(policy.deny_private_ip_ranges);
        assert_eq!(policy.max_egress_bytes, Some(NETWORK_EGRESS_LIMIT));
    }

    #[test]
    fn web_access_get_content_gets_exa_mcp_network_target_without_credentials() {
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID).unwrap(),
            provider: ExtensionId::new(WEB_ACCESS_EXTENSION_ID).unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(
            policy.allowed_targets,
            vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: EXA_MCP_HOST.to_string(),
                port: None,
            }]
        );
        assert!(policy.deny_private_ip_ranges);
        assert_eq!(policy.max_egress_bytes, Some(NETWORK_EGRESS_LIMIT));
    }

    #[test]
    fn keyless_no_network_capability_yields_unconstrained_empty_policy() {
        // #5459: a pure-compute tool (no `network` effect, no `network_targets`,
        // no credential) must NOT be "constrained". `network_policy_is_constrained`
        // treats `deny_private_ip_ranges` as a constraint, so leaving it `true` here
        // would emit a spurious empty `ApplyNetworkPolicy` obligation that fails
        // `validate_network_policy_metadata`. It must resolve to an empty,
        // unconstrained policy so no network obligation is emitted at all.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("ascii-renderer.draw").unwrap(),
            provider: ExtensionId::new("ascii-renderer").unwrap(),
            effects: vec![EffectKind::DispatchCapability],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert!(policy.allowed_targets.is_empty());
        assert!(
            !policy.deny_private_ip_ranges,
            "a no-egress policy must be unconstrained so no ApplyNetworkPolicy is emitted"
        );
        assert_eq!(policy.max_egress_bytes, None);
    }

    #[test]
    fn manifest_network_targets_populate_allowlist_without_credential() {
        // #5459 "network + no key": a tool declares its egress host via
        // `network_targets` (no credential) and gets a constrained allowlist —
        // an `ApplyNetworkPolicy` scoped to that host, no secret injection.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("hacker-news.top_stories").unwrap(),
            provider: ExtensionId::new("hacker-news").unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "news.ycombinator.com".to_string(),
                port: None,
            }],
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(
            policy.allowed_targets,
            vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "news.ycombinator.com".to_string(),
                port: None,
            }]
        );
        assert!(
            policy.deny_private_ip_ranges,
            "a tool with declared egress keeps the private-IP SSRF guard"
        );
    }

    #[test]
    fn web_access_search_deduplicates_existing_exa_mcp_network_target() {
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new(WEB_SEARCH_CAPABILITY_ID).unwrap(),
            provider: ExtensionId::new(WEB_ACCESS_EXTENSION_ID).unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: vec![ironclaw_host_api::RuntimeCredentialRequirement {
                handle: ironclaw_host_api::SecretHandle::new("exa_mcp_token").unwrap(),
                source: ironclaw_host_api::RuntimeCredentialRequirementSource::SecretHandle,
                provider_scopes: Vec::new(),
                audience: NetworkTargetPattern {
                    scheme: Some(NetworkScheme::Https),
                    host_pattern: EXA_MCP_HOST.to_string(),
                    port: None,
                },
                target: ironclaw_host_api::RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            network_targets: Vec::new(),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy.allowed_targets.len(), 1);
        assert_eq!(policy.max_egress_bytes, Some(NETWORK_EGRESS_LIMIT));
    }

    #[test]
    fn gsuite_capabilities_get_google_api_network_policy() {
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("gmail.list_messages").unwrap(),
            provider: ExtensionId::new(ironclaw_first_party_extensions::GMAIL_EXTENSION_ID)
                .unwrap(),
            effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy, google_api_network_policy());
    }

    #[test]
    fn calendar_capability_gets_google_api_network_policy() {
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("google-calendar.list_events").unwrap(),
            provider: ExtensionId::new(ironclaw_first_party_extensions::CALENDAR_EXTENSION_ID)
                .unwrap(),
            effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy, google_api_network_policy());
    }
}
