use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, EffectKind, ExtensionId, GrantConstraints,
    MountView, NetworkPolicy, Principal,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

use crate::extension_host::{
    extension_lifecycle::ActiveExtensionCapability, lifecycle::LifecycleFacade,
};
use ironclaw_product::{LifecycleProductContext, ProductWorkflowError};

#[derive(Clone, Default)]
pub(in crate::runtime) struct ExtensionCapabilitySurfaceSource {
    readiness_source: Option<Arc<LifecycleFacade>>,
    #[cfg(test)]
    static_surface: Option<ExtensionCapabilitySurface>,
}

impl ExtensionCapabilitySurfaceSource {
    pub(in crate::runtime) fn new(readiness_source: Option<Arc<LifecycleFacade>>) -> Self {
        Self {
            readiness_source,
            #[cfg(test)]
            static_surface: None,
        }
    }

    #[cfg(test)]
    pub(in crate::runtime) fn from_surface(surface: ExtensionCapabilitySurface) -> Self {
        Self {
            readiness_source: None,
            static_surface: Some(surface),
        }
    }

    pub(in crate::runtime) async fn snapshot(
        &self,
        caller: LifecycleProductContext,
    ) -> Result<ExtensionCapabilitySurface, ProductWorkflowError> {
        #[cfg(test)]
        if let Some(surface) = &self.static_surface {
            return Ok(surface.clone());
        }
        let Some(readiness_source) = self.readiness_source.as_deref() else {
            return Ok(ExtensionCapabilitySurface::default());
        };
        ExtensionCapabilitySurface::from_readiness_source_for_caller(readiness_source, caller).await
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

    pub(super) async fn from_readiness_source_for_caller(
        readiness_source: &LifecycleFacade,
        caller: LifecycleProductContext,
    ) -> Result<Self, ProductWorkflowError> {
        Ok(Self {
            active_capabilities: readiness_source
                .caller_active_model_visible_capabilities(&caller)
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

pub(crate) fn extension_network_policy(capability: &ActiveExtensionCapability) -> NetworkPolicy {
    let mut targets = Vec::new();
    // Manifest-declared egress allowlist — the keyless-but-networked path. A
    // capability declares its egress hosts directly, without a credential
    // (and therefore without forcing a secret injection). This is the single
    // generic source of the allowlist: gsuite (the five Google API hosts) and
    // web-access (the Exa MCP host) declare theirs in their manifests like
    // every other networked capability — no per-provider special-case here.
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
    // Only mark the policy "constrained" (via `deny_private_ip_ranges` or a
    // `max_egress_bytes` cap) when the capability actually declares egress
    // targets. `network_policy_is_constrained` treats each of those as a
    // constraint, so setting either on a no-egress tool (no `network` effect,
    // no `network_targets`, no credential) would give it a non-empty-looking
    // policy → a spurious `ApplyNetworkPolicy` obligation with an empty
    // allowlist that fails `validate_network_policy_metadata`. A tool with no
    // egress targets gets an unconstrained empty policy → no network obligation
    // at all. Networked tools keep the private-IP SSRF guard plus their
    // manifest-declared egress cap on their declared targets. (A tool that
    // declares the `network` effect but no targets is still caught by the
    // effect-based obligation gate and fails as misconfigured.)
    let has_egress_targets = !targets.is_empty();
    NetworkPolicy {
        allowed_targets: targets,
        deny_private_ip_ranges: has_egress_targets,
        max_egress_bytes: capability.max_egress_bytes.filter(|_| has_egress_targets),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        CapabilityId, NetworkScheme, NetworkTargetPattern, PermissionMode, UserId,
    };

    const EXA_MCP_HOST: &str = "mcp.exa.ai";
    const WEB_ACCESS_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;
    const GSUITE_EGRESS_LIMIT: u64 = 10 * 1024 * 1024;

    fn https(host_pattern: &str) -> NetworkTargetPattern {
        NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: host_pattern.to_string(),
            port: None,
        }
    }

    /// The five Google API hosts every gsuite capability's manifest declares,
    /// in manifest order. This mirrors the manifest-declared allowlist the
    /// generic grant-minting path now folds in (no per-provider special-case).
    fn google_api_network_targets() -> Vec<NetworkTargetPattern> {
        vec![
            https("www.googleapis.com"),
            https("gmail.googleapis.com"),
            https("calendar.googleapis.com"),
            https("oauth2.googleapis.com"),
            https("accounts.google.com"),
        ]
    }

    /// The exact `NetworkPolicy` gsuite capabilities used to receive from the
    /// removed `gsuite_network_policy_for` special-case, now reproduced purely
    /// from the manifest-declared `network_targets` + `max_egress_bytes`.
    fn google_api_network_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: google_api_network_targets(),
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(GSUITE_EGRESS_LIMIT),
        }
    }

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
            max_egress_bytes: None,
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
    fn web_access_search_gets_exa_mcp_network_target_from_manifest() {
        // web-access declares its Exa MCP host + egress cap in its manifest
        // (`network_targets` + `max_egress_bytes`); the generic path folds them
        // in exactly as the removed special-case used to synthesize them.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("web-access.search").unwrap(),
            provider: ExtensionId::new("web-access").unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: vec![https(EXA_MCP_HOST)],
            max_egress_bytes: Some(WEB_ACCESS_EGRESS_LIMIT),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy.allowed_targets, vec![https(EXA_MCP_HOST)]);
        assert!(policy.deny_private_ip_ranges);
        assert_eq!(policy.max_egress_bytes, Some(WEB_ACCESS_EGRESS_LIMIT));
    }

    #[test]
    fn web_access_get_content_gets_exa_mcp_network_target_from_manifest() {
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("web-access.get_content").unwrap(),
            provider: ExtensionId::new("web-access").unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: vec![https(EXA_MCP_HOST)],
            max_egress_bytes: Some(WEB_ACCESS_EGRESS_LIMIT),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy.allowed_targets, vec![https(EXA_MCP_HOST)]);
        assert!(policy.deny_private_ip_ranges);
        assert_eq!(policy.max_egress_bytes, Some(WEB_ACCESS_EGRESS_LIMIT));
    }

    #[test]
    fn keyless_no_network_capability_yields_unconstrained_empty_policy() {
        // #5459: a pure-compute tool (no `network` effect, no `network_targets`,
        // no credential) must NOT be "constrained". `network_policy_is_constrained`
        // treats `deny_private_ip_ranges` AND `max_egress_bytes` as constraints, so
        // leaving either set here would emit a spurious empty `ApplyNetworkPolicy`
        // obligation that fails `validate_network_policy_metadata`. It must resolve
        // to an empty, unconstrained policy so no network obligation is emitted at
        // all — even if a manifest cap were present, it is dropped with no targets.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("ascii-renderer.draw").unwrap(),
            provider: ExtensionId::new("ascii-renderer").unwrap(),
            effects: vec![EffectKind::DispatchCapability],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            network_targets: Vec::new(),
            max_egress_bytes: Some(WEB_ACCESS_EGRESS_LIMIT),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert!(policy.allowed_targets.is_empty());
        assert!(
            !policy.deny_private_ip_ranges,
            "a no-egress policy must be unconstrained so no ApplyNetworkPolicy is emitted"
        );
        assert_eq!(
            policy.max_egress_bytes, None,
            "a manifest egress cap is dropped when there are no egress targets"
        );
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
            network_targets: vec![https("news.ycombinator.com")],
            max_egress_bytes: None,
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy.allowed_targets, vec![https("news.ycombinator.com")]);
        assert!(
            policy.deny_private_ip_ranges,
            "a tool with declared egress keeps the private-IP SSRF guard"
        );
        assert_eq!(policy.max_egress_bytes, None);
    }

    #[test]
    fn manifest_network_target_deduplicates_matching_credential_audience() {
        // A host declared in `network_targets` that also appears as a credential
        // audience is folded to a single allowlist entry, and the manifest egress
        // cap rides through.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("web-access.search").unwrap(),
            provider: ExtensionId::new("web-access").unwrap(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            runtime_credentials: vec![ironclaw_host_api::RuntimeCredentialRequirement {
                handle: ironclaw_host_api::SecretHandle::new("exa_mcp_token").unwrap(),
                source: ironclaw_host_api::RuntimeCredentialRequirementSource::SecretHandle,
                provider_scopes: Vec::new(),
                audience: https(EXA_MCP_HOST),
                target: ironclaw_host_api::RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            network_targets: vec![https(EXA_MCP_HOST)],
            max_egress_bytes: Some(WEB_ACCESS_EGRESS_LIMIT),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy.allowed_targets, vec![https(EXA_MCP_HOST)]);
        assert_eq!(policy.max_egress_bytes, Some(WEB_ACCESS_EGRESS_LIMIT));
    }

    #[test]
    fn gsuite_capabilities_get_google_api_network_policy() {
        // Gmail's manifest declares the five Google API hosts + the 10 MiB cap;
        // the credential audience (`gmail.googleapis.com`) dedupes against the
        // declared list, so the minted policy equals the historical gsuite one.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("gmail.list_messages").unwrap(),
            provider: ExtensionId::new("gmail").unwrap(),
            effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            default_permission: PermissionMode::Allow,
            runtime_credentials: vec![ironclaw_host_api::RuntimeCredentialRequirement {
                handle: ironclaw_host_api::SecretHandle::new("gmail_account").unwrap(),
                source: ironclaw_host_api::RuntimeCredentialRequirementSource::SecretHandle,
                provider_scopes: Vec::new(),
                audience: https("gmail.googleapis.com"),
                target: ironclaw_host_api::RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            network_targets: google_api_network_targets(),
            max_egress_bytes: Some(GSUITE_EGRESS_LIMIT),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy, google_api_network_policy());
    }

    #[test]
    fn calendar_capability_gets_google_api_network_policy() {
        // Calendar's credential audience (`www.googleapis.com`) is the first of
        // the five declared hosts, so the union is exactly the five-host policy.
        let capability = ActiveExtensionCapability {
            id: CapabilityId::new("google-calendar.list_events").unwrap(),
            provider: ExtensionId::new("google-calendar").unwrap(),
            effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            default_permission: PermissionMode::Allow,
            runtime_credentials: vec![ironclaw_host_api::RuntimeCredentialRequirement {
                handle: ironclaw_host_api::SecretHandle::new("google_calendar_account").unwrap(),
                source: ironclaw_host_api::RuntimeCredentialRequirementSource::SecretHandle,
                provider_scopes: Vec::new(),
                audience: https("www.googleapis.com"),
                target: ironclaw_host_api::RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            network_targets: google_api_network_targets(),
            max_egress_bytes: Some(GSUITE_EGRESS_LIMIT),
            owner: ironclaw_extensions::InstallationOwner::Tenant,
        };

        let policy = extension_network_policy(&capability);

        assert_eq!(policy, google_api_network_policy());
    }
}
