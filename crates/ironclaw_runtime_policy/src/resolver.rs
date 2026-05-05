//! Resolver: `(DeploymentMode, RuntimeProfile, OrgPolicyConstraints) → EffectiveRuntimePolicy`.

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tenant/org-level constraint set passed alongside the requested profile.
///
/// Empty by default — meaning no extra ceiling beyond what the deployment
/// mode itself enforces. Populated by the settings/blueprint layer when the
/// tenant/org policy narrows authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgPolicyConstraints {
    /// Tenant/org-imposed profile ceiling. If set, the requested profile is
    /// narrowed to this value when more permissive *within the same family*.
    /// A `max_profile` from a different family than the requested profile is
    /// a configuration error and will fail resolution.
    pub max_profile: Option<RuntimeProfile>,

    /// Whether the org admin has explicitly approved
    /// [`RuntimeProfile::EnterpriseYoloDedicated`]. Without this,
    /// `EnterpriseYoloDedicated` resolution fails closed.
    pub admin_approves_dedicated_yolo: bool,
}

/// Caller request for runtime profile resolution.
///
/// `Serialize`/`Deserialize` are derived so the settings/blueprint
/// layer can persist a full request shape (deployment + requested
/// profile + org constraints + yolo disclosure ack) and replay it
/// through the resolver during reload, rather than re-deriving each
/// field from scratch each time. The wire shape is locked in by
/// `resolve_request_round_trips_through_serde`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub deployment: DeploymentMode,
    pub requested_profile: RuntimeProfile,
    pub org_policy: OrgPolicyConstraints,
    /// Caller-supplied disclosure acknowledgement for `*Yolo*` profiles. The
    /// CLI / settings / blueprint surface must explicitly capture this from
    /// the operator (CLI flag, settings opt-in, blueprint field). The
    /// resolver only enforces that it was provided — never sets it itself.
    pub yolo_disclosure_acknowledged: bool,
}

impl ResolveRequest {
    /// Convenience constructor with empty `OrgPolicyConstraints` and no yolo disclosure.
    /// The caller must layer in tenant/org policy and disclosure separately.
    pub fn new(deployment: DeploymentMode, requested_profile: RuntimeProfile) -> Self {
        Self {
            deployment,
            requested_profile,
            org_policy: OrgPolicyConstraints::default(),
            yolo_disclosure_acknowledged: false,
        }
    }
}

/// Reasons resolution may fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResolveError {
    /// Requested profile is not valid for the given deployment mode.
    #[error("runtime profile `{profile}` is not allowed under deployment mode `{deployment}`")]
    IncompatibleDeployment {
        deployment: DeploymentMode,
        profile: RuntimeProfile,
    },

    /// A `*Yolo*` profile was requested without
    /// [`ResolveRequest::yolo_disclosure_acknowledged`] set. Yolo profiles
    /// require explicit caller-supplied disclosure (CLI flag, settings
    /// opt-in, blueprint field).
    #[error(
        "runtime profile `{profile}` requires explicit disclosure acknowledgement \
         (CLI flag / settings / blueprint must capture operator confirmation)"
    )]
    YoloRequiresDisclosure { profile: RuntimeProfile },

    /// `EnterpriseYoloDedicated` was requested but the org admin policy did
    /// not approve dedicated yolo mode.
    #[error(
        "runtime profile `enterprise_yolo_dedicated` requires \
         `org_policy.admin_approves_dedicated_yolo = true`"
    )]
    DedicatedYoloRequiresOrgAdminApproval,

    /// `OrgPolicyConstraints::max_profile` references a profile from a different family
    /// than the requested profile (e.g. tenant ceiling is `HostedSafe` but
    /// the request is `LocalDev`). The settings/blueprint layer should have
    /// caught this at write time; surfacing it here is a fail-closed safety
    /// net.
    #[error(
        "org policy ceiling `{ceiling}` belongs to a different profile family \
         than the requested profile `{requested}`"
    )]
    OrgPolicyCeilingFamilyMismatch {
        requested: RuntimeProfile,
        ceiling: RuntimeProfile,
    },
}

/// Resolve `(deployment, requested_profile, org_policy)` into an
/// [`EffectiveRuntimePolicy`].
///
/// Returns the resolved policy on success. On failure, returns a typed
/// [`ResolveError`] — the caller is expected to surface this to the operator
/// (CLI error, settings rejection, blueprint apply error) and offer a
/// compatible alternative. The resolver never silently downgrades to a
/// less-privileged profile; narrowing only happens via an explicit
/// `OrgPolicyConstraints::max_profile` ceiling within the same family.
pub fn resolve(req: ResolveRequest) -> Result<EffectiveRuntimePolicy, ResolveError> {
    if !is_compatible(req.deployment, req.requested_profile) {
        return Err(ResolveError::IncompatibleDeployment {
            deployment: req.deployment,
            profile: req.requested_profile,
        });
    }

    if req.requested_profile.is_yolo() && !req.yolo_disclosure_acknowledged {
        return Err(ResolveError::YoloRequiresDisclosure {
            profile: req.requested_profile,
        });
    }

    if req.requested_profile == RuntimeProfile::EnterpriseYoloDedicated
        && !req.org_policy.admin_approves_dedicated_yolo
    {
        return Err(ResolveError::DedicatedYoloRequiresOrgAdminApproval);
    }

    let resolved_profile = if let Some(ceiling) = req.org_policy.max_profile {
        narrow_to_ceiling(req.requested_profile, ceiling)?
    } else {
        req.requested_profile
    };

    Ok(map_to_effective(
        req.deployment,
        req.requested_profile,
        resolved_profile,
    ))
}

/// `(deployment, profile)` compatibility matrix.
///
/// `SecureDefault`, `Sandboxed`, and `Experiment` are deployment-agnostic.
/// Family-specific profiles only resolve under their matching deployment.
const fn is_compatible(deployment: DeploymentMode, profile: RuntimeProfile) -> bool {
    match (deployment, profile) {
        // Deployment-agnostic profiles.
        (_, RuntimeProfile::SecureDefault)
        | (_, RuntimeProfile::Sandboxed)
        | (_, RuntimeProfile::Experiment) => true,

        // Local family — local single-user only.
        (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalSafe)
        | (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalDev)
        | (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalYolo) => true,

        // Hosted family — hosted multi-tenant only.
        (DeploymentMode::HostedMultiTenant, RuntimeProfile::HostedSafe)
        | (DeploymentMode::HostedMultiTenant, RuntimeProfile::HostedDev)
        | (DeploymentMode::HostedMultiTenant, RuntimeProfile::HostedYoloTenantScoped) => true,

        // Enterprise family — enterprise dedicated only.
        (DeploymentMode::EnterpriseDedicated, RuntimeProfile::EnterpriseSafe)
        | (DeploymentMode::EnterpriseDedicated, RuntimeProfile::EnterpriseDev)
        | (DeploymentMode::EnterpriseDedicated, RuntimeProfile::EnterpriseYoloDedicated) => true,

        // All other combinations fail closed.
        _ => false,
    }
}

/// Within-family narrowing rank. Higher = more permissive. Cross-family
/// comparisons return `None` so the caller can surface a typed error rather
/// than producing a nonsensical narrowing.
const fn family_rank(profile: RuntimeProfile) -> Option<(ProfileFamily, u8)> {
    match profile {
        RuntimeProfile::LocalSafe => Some((ProfileFamily::Local, 1)),
        RuntimeProfile::LocalDev => Some((ProfileFamily::Local, 2)),
        RuntimeProfile::LocalYolo => Some((ProfileFamily::Local, 3)),
        RuntimeProfile::HostedSafe => Some((ProfileFamily::Hosted, 1)),
        RuntimeProfile::HostedDev => Some((ProfileFamily::Hosted, 2)),
        RuntimeProfile::HostedYoloTenantScoped => Some((ProfileFamily::Hosted, 3)),
        RuntimeProfile::EnterpriseSafe => Some((ProfileFamily::Enterprise, 1)),
        RuntimeProfile::EnterpriseDev => Some((ProfileFamily::Enterprise, 2)),
        RuntimeProfile::EnterpriseYoloDedicated => Some((ProfileFamily::Enterprise, 3)),
        // SecureDefault / Sandboxed / Experiment have no family ordering —
        // they're deployment-agnostic helpers and cannot be ceiling targets.
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileFamily {
    Local,
    Hosted,
    Enterprise,
}

fn narrow_to_ceiling(
    requested: RuntimeProfile,
    ceiling: RuntimeProfile,
) -> Result<RuntimeProfile, ResolveError> {
    let (req_family, req_rank) = family_rank(requested)
        .ok_or(ResolveError::OrgPolicyCeilingFamilyMismatch { requested, ceiling })?;
    let (ceil_family, ceil_rank) = family_rank(ceiling)
        .ok_or(ResolveError::OrgPolicyCeilingFamilyMismatch { requested, ceiling })?;

    if req_family != ceil_family {
        return Err(ResolveError::OrgPolicyCeilingFamilyMismatch { requested, ceiling });
    }

    // Resolved profile is `min(requested, ceiling)` within the family.
    // - ceiling has headroom (`ceil_rank > req_rank`): keep `requested`
    //   — the operator already chose less authority than the ceiling allows.
    // - ceiling narrows (`ceil_rank < req_rank`): return `ceiling`
    //   — tenant/org policy reduces authority below what was requested.
    // - equal: either is correct; return `requested` for stable identity.
    if ceil_rank < req_rank {
        Ok(ceiling)
    } else {
        Ok(requested)
    }
}

/// Map the resolved profile to concrete backend/mode choices. The deployment
/// mode also disambiguates a few cases (e.g. `Sandboxed` filesystem backend
/// uses `TenantWorkspace` under hosted, `OrgDedicatedWorkspace` under
/// enterprise, `ScopedVirtual` under local).
fn map_to_effective(
    deployment: DeploymentMode,
    requested_profile: RuntimeProfile,
    resolved_profile: RuntimeProfile,
) -> EffectiveRuntimePolicy {
    let (
        filesystem_backend,
        process_backend,
        network_mode,
        secret_mode,
        approval_policy,
        audit_mode,
    ) = backends_for(deployment, resolved_profile);
    EffectiveRuntimePolicy {
        deployment,
        requested_profile,
        resolved_profile,
        filesystem_backend,
        process_backend,
        network_mode,
        secret_mode,
        approval_policy,
        audit_mode,
    }
}

/// Per-profile backend mapping. Centralised so the matrix is reviewable in
/// one place rather than scattered across the resolver.
fn backends_for(
    deployment: DeploymentMode,
    profile: RuntimeProfile,
) -> (
    FilesystemBackendKind,
    ProcessBackendKind,
    NetworkMode,
    SecretMode,
    ApprovalPolicy,
    AuditMode,
) {
    use RuntimeProfile::*;
    match profile {
        SecureDefault => (
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Brokered,
            SecretMode::BrokeredHandles,
            ApprovalPolicy::AskAlways,
            audit_for(deployment),
        ),

        LocalSafe => (
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::Brokered,
            SecretMode::ScrubbedEnv,
            ApprovalPolicy::AskWrites,
            AuditMode::LocalMinimal,
        ),
        LocalDev => (
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
            ApprovalPolicy::AskDestructive,
            AuditMode::LocalMinimal,
        ),
        LocalYolo => (
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::Direct,
            SecretMode::InheritedEnv,
            ApprovalPolicy::Minimal,
            AuditMode::LocalMinimal,
        ),

        HostedSafe => (
            FilesystemBackendKind::TenantWorkspace,
            ProcessBackendKind::TenantSandbox,
            NetworkMode::Brokered,
            SecretMode::TenantBroker,
            ApprovalPolicy::AskWrites,
            AuditMode::Standard,
        ),
        HostedDev => (
            FilesystemBackendKind::TenantWorkspace,
            ProcessBackendKind::TenantSandbox,
            NetworkMode::Allowlist,
            SecretMode::TenantBroker,
            ApprovalPolicy::AskDestructive,
            AuditMode::Standard,
        ),
        HostedYoloTenantScoped => (
            FilesystemBackendKind::TenantWorkspace,
            ProcessBackendKind::TenantSandbox,
            NetworkMode::Allowlist,
            SecretMode::TenantBroker,
            ApprovalPolicy::Minimal,
            AuditMode::Standard,
        ),

        EnterpriseSafe => (
            FilesystemBackendKind::OrgDedicatedWorkspace,
            ProcessBackendKind::OrgDedicatedRunner,
            NetworkMode::Brokered,
            SecretMode::OrgBroker,
            ApprovalPolicy::OrgPolicy,
            AuditMode::OrgPolicy,
        ),
        EnterpriseDev => (
            FilesystemBackendKind::OrgDedicatedWorkspace,
            ProcessBackendKind::OrgDedicatedRunner,
            NetworkMode::Allowlist,
            SecretMode::OrgBroker,
            ApprovalPolicy::OrgPolicy,
            AuditMode::OrgPolicy,
        ),
        EnterpriseYoloDedicated => (
            FilesystemBackendKind::OrgDedicatedWorkspace,
            ProcessBackendKind::OrgDedicatedRunner,
            NetworkMode::DirectLogged,
            SecretMode::OrgBroker,
            // **Approval-policy variance for the *Yolo* family**: this is
            // the only `*Yolo*` profile that does NOT map to
            // `ApprovalPolicy::Minimal`. Local and hosted yolo run inside
            // a single boundary (the user's laptop, a tenant's sandbox)
            // where the operator can credibly accept "no prompts."
            // Enterprise dedicated yolo runs against a *shared org*
            // boundary, so the approval ceiling is org-policy-driven —
            // orgs decide whether their dedicated yolo really means "no
            // prompts" or "minimal prompts." A consumer that assumes
            // "yolo ⇒ Minimal" must special-case this variant; the
            // `enterprise_yolo_dedicated_uses_org_policy_approvals_not_minimal`
            // test locks the variance in.
            ApprovalPolicy::OrgPolicy,
            AuditMode::OrgPolicy,
        ),

        Sandboxed => (
            // Sandboxed reuses the filesystem appropriate for the deployment
            // (no provider-host paths under hosted multi-tenant).
            sandboxed_filesystem_for(deployment),
            // Default sandboxed process backend is SRT — Docker/SmolVm are
            // valid alternatives the host runtime planner may select based
            // on availability; the resolver picks a reasonable default.
            ProcessBackendKind::Srt,
            NetworkMode::Brokered,
            sandboxed_secret_for(deployment),
            ApprovalPolicy::AskWrites,
            audit_for(deployment),
        ),
        Experiment => (
            sandboxed_filesystem_for(deployment),
            ProcessBackendKind::SmolVm,
            NetworkMode::Allowlist,
            sandboxed_secret_for(deployment),
            ApprovalPolicy::AskDestructive,
            audit_for(deployment),
        ),
        // `RuntimeProfile` is `#[non_exhaustive]` so this wildcard is
        // required to compile from a downstream crate. Reaching it means a
        // new variant was added to `ironclaw_host_api::RuntimeProfile`
        // without a corresponding case here — fail closed loudly so the
        // gap surfaces in development rather than as a silent default.
        _ => panic!(
            "backends_for_profile: unhandled RuntimeProfile variant — update the resolver before adding new variants"
        ),
    }
}

// `DeploymentMode` is `#[non_exhaustive]` so each match below must include
// a wildcard arm. We panic in the wildcard so a new `DeploymentMode`
// variant added without updating these helpers fails loudly instead of
// returning a silent default.
fn sandboxed_filesystem_for(deployment: DeploymentMode) -> FilesystemBackendKind {
    match deployment {
        DeploymentMode::LocalSingleUser => FilesystemBackendKind::ScopedVirtual,
        DeploymentMode::HostedMultiTenant => FilesystemBackendKind::TenantWorkspace,
        DeploymentMode::EnterpriseDedicated => FilesystemBackendKind::OrgDedicatedWorkspace,
        _ => panic!("sandboxed_filesystem_for: unhandled DeploymentMode variant"),
    }
}

fn sandboxed_secret_for(deployment: DeploymentMode) -> SecretMode {
    match deployment {
        DeploymentMode::LocalSingleUser => SecretMode::BrokeredHandles,
        DeploymentMode::HostedMultiTenant => SecretMode::TenantBroker,
        DeploymentMode::EnterpriseDedicated => SecretMode::OrgBroker,
        _ => panic!("sandboxed_secret_for: unhandled DeploymentMode variant"),
    }
}

fn audit_for(deployment: DeploymentMode) -> AuditMode {
    match deployment {
        DeploymentMode::LocalSingleUser => AuditMode::LocalMinimal,
        DeploymentMode::HostedMultiTenant => AuditMode::Standard,
        DeploymentMode::EnterpriseDedicated => AuditMode::OrgPolicy,
        _ => panic!("audit_for: unhandled DeploymentMode variant"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(deployment: DeploymentMode, profile: RuntimeProfile) -> ResolveRequest {
        ResolveRequest::new(deployment, profile)
    }

    fn req_yolo(deployment: DeploymentMode, profile: RuntimeProfile) -> ResolveRequest {
        ResolveRequest {
            yolo_disclosure_acknowledged: true,
            ..ResolveRequest::new(deployment, profile)
        }
    }

    // --- compatibility matrix ---------------------------------------------

    #[test]
    fn local_single_user_accepts_local_family_secure_default_sandboxed_experiment() {
        for profile in [
            RuntimeProfile::SecureDefault,
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::Sandboxed,
            RuntimeProfile::Experiment,
        ] {
            let policy = resolve(req(DeploymentMode::LocalSingleUser, profile))
                .unwrap_or_else(|e| panic!("expected {profile:?} to resolve: {e}"));
            assert_eq!(policy.requested_profile, profile);
            assert_eq!(policy.resolved_profile, profile);
            assert!(!policy.was_reduced());
        }
    }

    #[test]
    fn hosted_multi_tenant_rejects_every_local_profile() {
        for profile in [
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::LocalYolo,
        ] {
            let result = resolve(req(DeploymentMode::HostedMultiTenant, profile));
            match result {
                Err(ResolveError::IncompatibleDeployment {
                    deployment: DeploymentMode::HostedMultiTenant,
                    profile: rejected,
                }) => assert_eq!(rejected, profile),
                other => panic!("expected IncompatibleDeployment for {profile:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn hosted_multi_tenant_rejects_every_enterprise_profile() {
        for profile in [
            RuntimeProfile::EnterpriseSafe,
            RuntimeProfile::EnterpriseDev,
            RuntimeProfile::EnterpriseYoloDedicated,
        ] {
            let result = resolve(req(DeploymentMode::HostedMultiTenant, profile));
            assert!(matches!(
                result,
                Err(ResolveError::IncompatibleDeployment { .. })
            ));
        }
    }

    #[test]
    fn enterprise_dedicated_rejects_local_and_hosted_families() {
        for profile in [
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::LocalYolo,
            RuntimeProfile::HostedSafe,
            RuntimeProfile::HostedDev,
            RuntimeProfile::HostedYoloTenantScoped,
        ] {
            assert!(matches!(
                resolve(req(DeploymentMode::EnterpriseDedicated, profile)),
                Err(ResolveError::IncompatibleDeployment { .. })
            ));
        }
    }

    // --- yolo disclosure ---------------------------------------------------

    #[test]
    fn yolo_profiles_require_disclosure_acknowledgement() {
        for (deployment, profile) in [
            (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalYolo),
            (
                DeploymentMode::HostedMultiTenant,
                RuntimeProfile::HostedYoloTenantScoped,
            ),
        ] {
            let result = resolve(req(deployment, profile));
            assert!(
                matches!(
                    result,
                    Err(ResolveError::YoloRequiresDisclosure { profile: p }) if p == profile
                ),
                "expected YoloRequiresDisclosure for {profile:?}, got {result:?}"
            );

            // With acknowledgement, resolution succeeds.
            let policy = resolve(req_yolo(deployment, profile)).unwrap();
            assert_eq!(policy.resolved_profile, profile);
        }
    }

    #[test]
    fn enterprise_yolo_dedicated_requires_disclosure_and_org_admin_approval() {
        // Without disclosure: YoloRequiresDisclosure (checked first).
        let r = resolve(req(
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseYoloDedicated,
        ));
        assert!(matches!(
            r,
            Err(ResolveError::YoloRequiresDisclosure { .. })
        ));

        // With disclosure but without org admin approval:
        // DedicatedYoloRequiresOrgAdminApproval.
        let r = resolve(req_yolo(
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseYoloDedicated,
        ));
        assert!(matches!(
            r,
            Err(ResolveError::DedicatedYoloRequiresOrgAdminApproval)
        ));

        // With disclosure AND org admin approval: resolves.
        let request = ResolveRequest {
            yolo_disclosure_acknowledged: true,
            org_policy: OrgPolicyConstraints {
                admin_approves_dedicated_yolo: true,
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::EnterpriseYoloDedicated,
            )
        };
        let policy = resolve(request).unwrap();
        assert_eq!(
            policy.resolved_profile,
            RuntimeProfile::EnterpriseYoloDedicated
        );
    }

    #[test]
    fn enterprise_yolo_dedicated_uses_org_policy_approvals_not_minimal() {
        // Locks in the "yolo ⇒ Minimal approvals" exception zmanian
        // flagged: every other `*Yolo*` profile maps to
        // `ApprovalPolicy::Minimal`, but `EnterpriseYoloDedicated` runs
        // inside a *shared org boundary* and defers the approval ceiling
        // to org policy. A consumer that assumes "yolo means Minimal"
        // will be wrong here. If a future variant changes this mapping,
        // the assertion below is the canary.
        let request = ResolveRequest {
            yolo_disclosure_acknowledged: true,
            org_policy: OrgPolicyConstraints {
                admin_approves_dedicated_yolo: true,
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::EnterpriseYoloDedicated,
            )
        };
        let policy = resolve(request).unwrap();
        assert_eq!(
            policy.approval_policy,
            ApprovalPolicy::OrgPolicy,
            "EnterpriseYoloDedicated must defer approvals to org policy, not collapse to Minimal"
        );
        // For comparison: Local/Hosted yolo *do* collapse to Minimal,
        // confirming this is a per-variant decision, not an accident.
        for (deployment, profile) in [
            (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalYolo),
            (
                DeploymentMode::HostedMultiTenant,
                RuntimeProfile::HostedYoloTenantScoped,
            ),
        ] {
            let policy = resolve(req_yolo(deployment, profile)).unwrap();
            assert_eq!(policy.approval_policy, ApprovalPolicy::Minimal);
        }
    }

    // --- backend mapping ---------------------------------------------------

    #[test]
    fn local_family_maps_to_host_workspace_and_local_host_shell() {
        for profile in [RuntimeProfile::LocalSafe, RuntimeProfile::LocalDev] {
            let policy = resolve(req(DeploymentMode::LocalSingleUser, profile)).unwrap();
            assert_eq!(
                policy.filesystem_backend,
                FilesystemBackendKind::HostWorkspace
            );
            assert_eq!(policy.process_backend, ProcessBackendKind::LocalHost);
            assert_eq!(policy.audit_mode, AuditMode::LocalMinimal);
        }
    }

    #[test]
    fn hosted_family_never_resolves_to_provider_host_filesystem_or_shell() {
        // Critical security property: hosted multi-tenant never reaches
        // provider-host filesystem/shell, including via Sandboxed/Experiment.
        for profile in [
            RuntimeProfile::SecureDefault,
            RuntimeProfile::HostedSafe,
            RuntimeProfile::HostedDev,
            RuntimeProfile::Sandboxed,
        ] {
            let request = if profile.is_yolo() {
                req_yolo(DeploymentMode::HostedMultiTenant, profile)
            } else {
                req(DeploymentMode::HostedMultiTenant, profile)
            };
            let policy = resolve(request).unwrap_or_else(|e| panic!("{profile:?}: {e}"));
            assert_ne!(
                policy.filesystem_backend,
                FilesystemBackendKind::HostWorkspace,
                "hosted multi-tenant must never produce HostWorkspace; profile={profile:?}"
            );
            assert_ne!(
                policy.process_backend,
                ProcessBackendKind::LocalHost,
                "hosted multi-tenant must never produce LocalHost; profile={profile:?}"
            );
        }
        // Including the yolo case under disclosure.
        let policy = resolve(req_yolo(
            DeploymentMode::HostedMultiTenant,
            RuntimeProfile::HostedYoloTenantScoped,
        ))
        .unwrap();
        assert_ne!(
            policy.filesystem_backend,
            FilesystemBackendKind::HostWorkspace
        );
        assert_ne!(policy.process_backend, ProcessBackendKind::LocalHost);
    }

    #[test]
    fn enterprise_family_maps_to_org_dedicated_workspace_and_runner() {
        let policy = resolve(req(
            DeploymentMode::EnterpriseDedicated,
            RuntimeProfile::EnterpriseDev,
        ))
        .unwrap();
        assert_eq!(
            policy.filesystem_backend,
            FilesystemBackendKind::OrgDedicatedWorkspace
        );
        assert_eq!(
            policy.process_backend,
            ProcessBackendKind::OrgDedicatedRunner
        );
    }

    // --- org policy narrowing ---------------------------------------------

    #[test]
    fn org_policy_ceiling_within_family_narrows_resolved_profile() {
        // Tenant ceiling LocalSafe + requested LocalDev → resolved LocalSafe.
        let request = ResolveRequest {
            org_policy: OrgPolicyConstraints {
                max_profile: Some(RuntimeProfile::LocalSafe),
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalDev)
        };
        let policy = resolve(request).unwrap();
        assert_eq!(policy.requested_profile, RuntimeProfile::LocalDev);
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalSafe);
        assert!(policy.was_reduced());
        // Resolved policy reflects the narrowed profile's backends.
        assert_eq!(policy.approval_policy, ApprovalPolicy::AskWrites);
    }

    #[test]
    fn org_policy_ceiling_at_or_below_request_keeps_request() {
        // Ceiling LocalDev + requested LocalSafe → keeps LocalSafe (already at/below ceiling).
        let request = ResolveRequest {
            org_policy: OrgPolicyConstraints {
                max_profile: Some(RuntimeProfile::LocalDev),
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalSafe)
        };
        let policy = resolve(request).unwrap();
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalSafe);
        assert!(!policy.was_reduced());
    }

    #[test]
    fn org_policy_ceiling_above_request_in_same_family_keeps_request() {
        // Belt-and-suspenders for the monotonic-safety rule: when the
        // ceiling has headroom (ceiling rank > request rank), the resolved
        // profile must equal the requested profile, never the ceiling.
        // Ceilings can only reduce authority; they cannot raise it.
        let request = ResolveRequest {
            org_policy: OrgPolicyConstraints {
                max_profile: Some(RuntimeProfile::LocalYolo),
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalSafe)
        };
        let policy = resolve(request).unwrap();
        assert_eq!(policy.requested_profile, RuntimeProfile::LocalSafe);
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalSafe);
        assert!(!policy.was_reduced());
    }

    #[test]
    fn org_policy_ceiling_in_different_family_is_rejected() {
        // Hosted ceiling on a local request → family mismatch.
        let request = ResolveRequest {
            org_policy: OrgPolicyConstraints {
                max_profile: Some(RuntimeProfile::HostedSafe),
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalDev)
        };
        let r = resolve(request);
        assert!(matches!(
            r,
            Err(ResolveError::OrgPolicyCeilingFamilyMismatch {
                requested: RuntimeProfile::LocalDev,
                ceiling: RuntimeProfile::HostedSafe,
            })
        ));
    }

    #[test]
    fn org_policy_ceiling_to_deployment_agnostic_profile_is_rejected_as_family_mismatch() {
        // SecureDefault has no family, so it can't be a ceiling target.
        let request = ResolveRequest {
            org_policy: OrgPolicyConstraints {
                max_profile: Some(RuntimeProfile::SecureDefault),
                ..OrgPolicyConstraints::default()
            },
            ..ResolveRequest::new(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalDev)
        };
        assert!(matches!(
            resolve(request),
            Err(ResolveError::OrgPolicyCeilingFamilyMismatch { .. })
        ));
    }

    // --- determinism / serialization --------------------------------------

    #[test]
    fn resolution_is_deterministic_for_equal_inputs() {
        let request = req(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalDev);
        let a = resolve(request.clone()).unwrap();
        let b = resolve(request).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn resolved_policy_round_trips_through_serde() {
        let policy = resolve(req(
            DeploymentMode::LocalSingleUser,
            RuntimeProfile::LocalDev,
        ))
        .unwrap();
        let json = serde_json::to_string(&policy).unwrap();
        let parsed: EffectiveRuntimePolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, policy);
    }

    #[test]
    fn org_policy_constraints_round_trips_through_serde() {
        // zmanian test gap #4: the settings/blueprint layer persists
        // org constraints; the wire shape must round-trip cleanly so a
        // reload reproduces the same input to the resolver.
        let original = OrgPolicyConstraints {
            max_profile: Some(RuntimeProfile::LocalDev),
            admin_approves_dedicated_yolo: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: OrgPolicyConstraints = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);

        // Default round-trip (`max_profile = None`, `admin_approves =
        // false`) must also work — the empty case is the most common
        // shape from the settings layer.
        let empty = OrgPolicyConstraints::default();
        let json = serde_json::to_string(&empty).unwrap();
        let parsed: OrgPolicyConstraints = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, empty);
    }

    #[test]
    fn resolve_request_round_trips_through_serde() {
        // zmanian test gap #4: the full resolver input must serde
        // round-trip so settings/blueprint can persist it. Drive every
        // field, including the yolo disclosure ack and a non-default
        // org policy ceiling, so a regression that breaks any one
        // field's serde derive is caught.
        let original = ResolveRequest {
            deployment: DeploymentMode::EnterpriseDedicated,
            requested_profile: RuntimeProfile::EnterpriseYoloDedicated,
            org_policy: OrgPolicyConstraints {
                max_profile: Some(RuntimeProfile::EnterpriseDev),
                admin_approves_dedicated_yolo: true,
            },
            yolo_disclosure_acknowledged: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: ResolveRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn every_valid_deployment_profile_pair_resolves() {
        // Locks in the compatibility matrix as a single readable assertion.
        let valid_pairs = [
            // Deployment-agnostic profiles work everywhere.
            (
                DeploymentMode::LocalSingleUser,
                RuntimeProfile::SecureDefault,
            ),
            (
                DeploymentMode::HostedMultiTenant,
                RuntimeProfile::SecureDefault,
            ),
            (
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::SecureDefault,
            ),
            (DeploymentMode::LocalSingleUser, RuntimeProfile::Sandboxed),
            (DeploymentMode::HostedMultiTenant, RuntimeProfile::Sandboxed),
            (
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::Sandboxed,
            ),
            (DeploymentMode::LocalSingleUser, RuntimeProfile::Experiment),
            (
                DeploymentMode::HostedMultiTenant,
                RuntimeProfile::Experiment,
            ),
            (
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::Experiment,
            ),
            // Family-specific profiles in their matching deployment.
            (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalSafe),
            (DeploymentMode::LocalSingleUser, RuntimeProfile::LocalDev),
            (
                DeploymentMode::HostedMultiTenant,
                RuntimeProfile::HostedSafe,
            ),
            (DeploymentMode::HostedMultiTenant, RuntimeProfile::HostedDev),
            (
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::EnterpriseSafe,
            ),
            (
                DeploymentMode::EnterpriseDedicated,
                RuntimeProfile::EnterpriseDev,
            ),
        ];
        for (deployment, profile) in valid_pairs {
            let policy = resolve(req(deployment, profile))
                .unwrap_or_else(|e| panic!("({deployment:?}, {profile:?}) failed: {e}"));
            assert_eq!(policy.deployment, deployment);
            assert_eq!(policy.requested_profile, profile);
            assert_eq!(policy.resolved_profile, profile);
        }
    }
}
