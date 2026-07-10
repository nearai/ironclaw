use std::{borrow::Cow, sync::Arc};

use async_trait::async_trait;
use ironclaw_approvals::{ToolPermissionOverride, permission_mode_allows_persistent_approval};
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityDescriptor, CapabilityGrant,
    CapabilityId, Decision, DenyReason, EffectKind, ExecutionContext, Principal, ResourceEstimate,
    ResourceScope, Timestamp, runtime_policy::ApprovalPolicy,
};
use ironclaw_trust::TrustDecision;

pub(crate) trait ProfileApprovalGatePolicy: Send + Sync {
    fn capability_exempt_from_approval(&self, _capability: &CapabilityId) -> bool {
        false
    }

    fn effects_require_approval(
        &self,
        approval_policy: ApprovalPolicy,
        effects: &[EffectKind],
    ) -> bool;

    /// Hard floor (#4776/#4959): effects that ALWAYS require an explicit
    /// approval gate and can never be auto-approved or satisfied by a stored
    /// always-allow grant, regardless of `ApprovalPolicy` or the global
    /// auto-approve setting. The reborn equivalent of v1's
    /// `ApprovalRequirement::Always`, expressed per-call over the invocation's
    /// effects. Defaults to "no floor".
    fn effects_force_approval(&self, _effects: &[EffectKind]) -> bool {
        false
    }
}

/// Resolves approval settings for one dispatch. Implementations read the
/// durable per-user stores; the authorizer queries these after the base grant
/// decision allows the candidate so settings apply without process restart
/// while non-runnable candidates do not spend approval-store reads.
#[async_trait]
pub(crate) trait ApprovalSettingsProvider: Send + Sync {
    async fn tool_override(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<ToolPermissionOverride>;

    async fn global_auto_approve(&self, scope: &ResourceScope) -> bool;

    async fn tool_always_allow(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        grantee: &Principal,
    ) -> bool;
}

/// No stored overrides and global auto-approve off: the gate behaves exactly as
/// it did before #4959. Test-only — production wires
/// `StoreApprovalSettingsProvider`.
#[cfg(test)]
pub(crate) struct EmptyApprovalSettingsProvider;

#[cfg(test)]
#[async_trait]
impl ApprovalSettingsProvider for EmptyApprovalSettingsProvider {
    async fn tool_override(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
    ) -> Option<ToolPermissionOverride> {
        None
    }

    async fn global_auto_approve(&self, _scope: &ResourceScope) -> bool {
        false
    }

    async fn tool_always_allow(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
        _grantee: &Principal,
    ) -> bool {
        false
    }
}

pub(crate) fn profile_approval_authorizer(
    approval_policy: ApprovalPolicy,
    gate_policy: Arc<dyn ProfileApprovalGatePolicy>,
    settings: Arc<dyn ApprovalSettingsProvider>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    Arc::new(ProfileApprovalPolicyAuthorizer::new(
        approval_policy,
        gate_policy,
        settings,
    ))
}

struct ProfileApprovalPolicyAuthorizer {
    inner: GrantAuthorizer,
    approval_policy: ApprovalPolicy,
    gate_policy: Arc<dyn ProfileApprovalGatePolicy>,
    settings: Arc<dyn ApprovalSettingsProvider>,
}

impl ProfileApprovalPolicyAuthorizer {
    fn new(
        approval_policy: ApprovalPolicy,
        gate_policy: Arc<dyn ProfileApprovalGatePolicy>,
        settings: Arc<dyn ApprovalSettingsProvider>,
    ) -> Self {
        Self {
            inner: GrantAuthorizer::new(),
            approval_policy,
            gate_policy,
            settings,
        }
    }
}

#[async_trait::async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ProfileApprovalPolicyAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        let decision = self
            .inner
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await;
        require_approval_for_profile_policy(
            decision,
            context,
            descriptor,
            estimate,
            ProfileApprovalActionKind::Dispatch,
            self.approval_policy,
            self.gate_policy.as_ref(),
            self.settings.as_ref(),
        )
        .await
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        let decision = self
            .inner
            .authorize_spawn_with_trust(context, descriptor, estimate, trust_decision)
            .await;
        require_approval_for_profile_policy(
            decision,
            context,
            descriptor,
            estimate,
            ProfileApprovalActionKind::SpawnCapability,
            self.approval_policy,
            self.gate_policy.as_ref(),
            self.settings.as_ref(),
        )
        .await
    }
}

#[derive(Clone, Copy, Debug)]
enum ProfileApprovalActionKind {
    Dispatch,
    SpawnCapability,
}

#[allow(clippy::too_many_arguments)]
// arch-exempt: too_many_args, gate decision needs context+descriptor+estimate+policy+gate+settings, plan #4776
async fn require_approval_for_profile_policy(
    decision: Decision,
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: ProfileApprovalActionKind,
    approval_policy: ApprovalPolicy,
    gate_policy: &dyn ProfileApprovalGatePolicy,
    settings: &dyn ApprovalSettingsProvider,
) -> Decision {
    // The profile approval gate only ever upgrades an underlying `Allow`; a
    // `Deny` / `RequireApproval` from the grant authorizer passes through
    // unchanged.
    let Decision::Allow { .. } = &decision else {
        return decision;
    };

    // A spawn exercises SpawnProcess even when the capability's own descriptor
    // does not declare it: the underlying GrantAuthorizer authorizes spawns
    // against `spawn_descriptor`, which adds EffectKind::SpawnProcess. Evaluate
    // the approval gate against the same elevated effect set so a dispatch-only
    // capability cannot be spawned as a live process without an approval gate.
    let gate_effects = approval_gate_effects(action_kind, descriptor);
    let profile_requires_approval =
        gate_policy.effects_require_approval(approval_policy, &gate_effects);

    let require_approval = || Decision::RequireApproval {
        request: approval_request(context, descriptor, estimate, action_kind),
    };

    // Decision precedence (high → low), #4776:
    // 1. Explicit per-tool `disabled` → deny outright (strongest user intent).
    let tool_override = settings
        .tool_override(&context.resource_scope, &descriptor.id)
        .await;
    if matches!(tool_override, Some(ToolPermissionOverride::Disabled)) {
        return Decision::Deny {
            reason: DenyReason::PolicyDenied,
        };
    }
    // 2. A matching one-shot approval lease satisfies the current resume. This
    //    must beat explicit ask_each_time and hard-floor gates: those settings
    //    require a fresh human approval for each new invocation, but the lease
    //    is the durable proof that this invocation was just approved.
    //
    // Fingerprinted approval leases are not ambient grants: CapabilityHost's
    // `resume_json` path excludes them from normal grant loading, then validates
    // the blocked run's approval_request_id, approval request metadata, and
    // invocation fingerprint before injecting the matching lease grant into this
    // resume context. This predicate therefore recognizes the already selected
    // resume lease; it is not the lease lookup boundary.
    if has_matching_one_shot_approval_grant(context, descriptor, &gate_effects) {
        return decision;
    }
    // 3. Hard floor: never auto-approve / never satisfiable by a stored grant.
    if gate_policy.effects_force_approval(&gate_effects) {
        return require_approval();
    }
    // 4. Explicit per-tool `ask_each_time` → always gate fresh invocations,
    //    ignoring the global auto-approve setting and any stored always-allow
    //    grant.
    if matches!(tool_override, Some(ToolPermissionOverride::AskEachTime)) {
        return require_approval();
    }
    // 5. Capability deliberately exempt from the gate (in-turn consent).
    if gate_policy.capability_exempt_from_approval(&descriptor.id) {
        return decision;
    }
    let expected_grantee = Principal::Extension(descriptor.provider.clone());
    let durable_auto_approval_eligible =
        permission_mode_allows_persistent_approval(descriptor.default_permission);
    // 6. Global auto-approve bypasses an otherwise-gated eligible tool. Check
    // this before per-tool always-allow because it is the default-on common
    // path for new users; if enabled, both branches return the same Allow.
    if durable_auto_approval_eligible && settings.global_auto_approve(&context.resource_scope).await
    {
        return decision;
    }
    // 7. A settings-scope per-tool always-allow policy satisfies the gate.
    //    The provider verifies the active settings-page persistent policy
    //    directly, keyed to the capability provider rather than the caller
    //    extension, so this does not depend on whether that policy was also
    //    preloaded into this run's grants. Legacy prompt-created persistent
    //    grants are deliberately not enough when the tool row is "Follow
    //    global" and the global switch is off.
    if durable_auto_approval_eligible
        && settings
            .tool_always_allow(&context.resource_scope, &descriptor.id, &expected_grantee)
            .await
    {
        return decision;
    }
    // 8. With the global switch off, eligible tools default to asking even when
    //    the runtime profile would otherwise allow low-risk effects. This makes
    //    the Tools page switch authoritative for #4776; explicit always-allow
    //    grants above remain the per-tool opt-in.
    if durable_auto_approval_eligible {
        return require_approval();
    }
    // 9. Policy does not require a gate for this effect set.
    if !profile_requires_approval {
        return decision;
    }
    require_approval()
}

/// Effects the profile approval gate evaluates for `action_kind`.
///
/// Mirrors `ironclaw_authorization::spawn_descriptor`: a spawn always exercises
/// `SpawnProcess`, so it is added to the capability's declared effects when
/// gating a spawn. Dispatch evaluates the declared effects unchanged.
fn approval_gate_effects(
    action_kind: ProfileApprovalActionKind,
    descriptor: &CapabilityDescriptor,
) -> Cow<'_, [EffectKind]> {
    match action_kind {
        ProfileApprovalActionKind::Dispatch => Cow::Borrowed(descriptor.effects.as_slice()),
        ProfileApprovalActionKind::SpawnCapability => {
            if descriptor.effects.contains(&EffectKind::SpawnProcess) {
                Cow::Borrowed(descriptor.effects.as_slice())
            } else {
                let mut effects = descriptor.effects.clone();
                effects.push(EffectKind::SpawnProcess);
                Cow::Owned(effects)
            }
        }
    }
}

fn has_matching_one_shot_approval_grant(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    gate_effects: &[EffectKind],
) -> bool {
    let expected_grantee = Principal::Extension(context.extension_id.clone());
    let expected_user_approver = Principal::User(context.user_id.clone());
    let now = chrono::Utc::now();
    context.grants.grants.iter().any(|grant| {
        let grant_unexpired = grant_is_unexpired(grant, &now);
        let one_shot_approval_grant = grant.constraints.max_invocations == Some(1)
            && (grant.issued_by == Principal::HostRuntime
                || grant.issued_by == expected_user_approver)
            && grant_unexpired;
        grant.capability == descriptor.id
            && one_shot_approval_grant
            && grant.grantee == expected_grantee
            // Match against the spawn-elevated effect set so a one-shot lease
            // that does not cover SpawnProcess cannot satisfy a spawn gate.
            && gate_effects
                .iter()
                .all(|effect| grant.constraints.allowed_effects.contains(effect))
    })
}

fn grant_is_unexpired(grant: &CapabilityGrant, now: &Timestamp) -> bool {
    grant
        .constraints
        .expires_at
        .as_ref()
        .is_none_or(|expires_at| expires_at > now)
}

fn approval_request(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: ProfileApprovalActionKind,
) -> ApprovalRequest {
    let action = match action_kind {
        ProfileApprovalActionKind::Dispatch => Action::Dispatch {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
        ProfileApprovalActionKind::SpawnCapability => Action::SpawnCapability {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
    };
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: context.correlation_id,
        requested_by: Principal::Extension(context.extension_id.clone()),
        action: Box::new(action),
        invocation_fingerprint: None,
        reason: format!(
            "approval required for {:?} of {}",
            action_kind,
            descriptor.id.as_str()
        ),
        reusable_scope: None,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_approvals::persistent_approval_grant_issuer;
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet,
        EffectKind, ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy,
        PermissionMode, Principal, ResourceEstimate, RuntimeKind, TrustClass,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::json;

    use super::*;

    #[derive(Debug)]
    struct TestGatePolicy;

    impl ProfileApprovalGatePolicy for TestGatePolicy {
        fn effects_require_approval(
            &self,
            approval_policy: ApprovalPolicy,
            effects: &[EffectKind],
        ) -> bool {
            match approval_policy {
                ApprovalPolicy::Minimal => false,
                ApprovalPolicy::AskAlways => !effects.is_empty(),
                ApprovalPolicy::AskWrites | ApprovalPolicy::AskDestructive => {
                    effects.contains(&EffectKind::SpawnProcess)
                }
                ApprovalPolicy::OrgPolicy => !effects.is_empty(),
                _ => !effects.is_empty(),
            }
        }

        fn effects_force_approval(&self, effects: &[EffectKind]) -> bool {
            effects.contains(&EffectKind::Financial)
        }
    }

    /// Returns fixed settings so the gate's per-turn resolution can be driven
    /// deterministically (#4959).
    struct StubSettingsProvider {
        tool_override: Option<ToolPermissionOverride>,
        global_auto_approve: bool,
        tool_always_allow: bool,
    }

    #[async_trait]
    impl ApprovalSettingsProvider for StubSettingsProvider {
        async fn tool_override(
            &self,
            _scope: &ResourceScope,
            _capability_id: &CapabilityId,
        ) -> Option<ToolPermissionOverride> {
            self.tool_override
        }

        async fn global_auto_approve(&self, _scope: &ResourceScope) -> bool {
            self.global_auto_approve
        }

        async fn tool_always_allow(
            &self,
            _scope: &ResourceScope,
            _capability_id: &CapabilityId,
            _grantee: &Principal,
        ) -> bool {
            self.tool_always_allow
        }
    }

    /// Dispatch a `builtin.shell` capability carrying `effects`, with a granting
    /// lease and trust ceiling that make the underlying decision `Allow`, under
    /// the given approval policy + resolved settings.
    async fn dispatch_decision(
        approval_policy: ApprovalPolicy,
        effects: Vec<EffectKind>,
        settings: StubSettingsProvider,
    ) -> Decision {
        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), effects.clone());
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: effects.clone(),
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let trust = TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: effects,
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: chrono::Utc::now(),
        };
        profile_approval_authorizer(
            approval_policy,
            Arc::new(TestGatePolicy),
            Arc::new(settings),
        )
        .authorize_dispatch_with_trust(&ctx, &descriptor, &ResourceEstimate::default(), &trust)
        .await
    }

    #[tokio::test]
    async fn global_auto_approve_skips_gate_for_eligible_tool() {
        let decision = dispatch_decision(
            ApprovalPolicy::AskDestructive,
            vec![EffectKind::SpawnProcess],
            StubSettingsProvider {
                tool_override: None,
                global_auto_approve: true,
                tool_always_allow: false,
            },
        )
        .await;
        assert!(
            matches!(decision, Decision::Allow { .. }),
            "global auto-approve should skip the gate for an eligible tool, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn explicit_ask_each_time_overrides_global_auto_approve() {
        let decision = dispatch_decision(
            ApprovalPolicy::AskDestructive,
            vec![EffectKind::SpawnProcess],
            StubSettingsProvider {
                tool_override: Some(ToolPermissionOverride::AskEachTime),
                global_auto_approve: true,
                tool_always_allow: false,
            },
        )
        .await;
        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "explicit ask_each_time must gate even with global auto-approve on, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn explicit_disabled_denies_dispatch() {
        let decision = dispatch_decision(
            ApprovalPolicy::AskDestructive,
            vec![EffectKind::SpawnProcess],
            StubSettingsProvider {
                tool_override: Some(ToolPermissionOverride::Disabled),
                global_auto_approve: true,
                tool_always_allow: false,
            },
        )
        .await;
        assert!(
            matches!(
                decision,
                Decision::Deny {
                    reason: DenyReason::PolicyDenied
                }
            ),
            "explicit disabled must deny, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn hard_floor_requires_approval_even_with_global_auto_approve() {
        let decision = dispatch_decision(
            ApprovalPolicy::AskDestructive,
            vec![EffectKind::Financial],
            StubSettingsProvider {
                tool_override: None,
                global_auto_approve: true,
                tool_always_allow: false,
            },
        )
        .await;
        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "hard-floor (Financial) must gate even with global auto-approve on, got {decision:?}"
        );
    }

    fn test_descriptor(effects: Vec<EffectKind>) -> CapabilityDescriptor {
        test_descriptor_with_id(CapabilityId::new("builtin.shell").unwrap(), effects)
    }

    fn test_descriptor_with_id(id: CapabilityId, effects: Vec<EffectKind>) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id,
            provider: ExtensionId::new("builtin").unwrap(),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test".to_string(),
            parameters_schema: json!({}),
            effects,
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            resource_profile: None,
        }
    }

    fn test_context(grants: CapabilitySet) -> ExecutionContext {
        let ctx = ExecutionContext::local_default(
            ironclaw_host_api::UserId::new("test-user").unwrap(),
            ExtensionId::new("builtin").unwrap(),
            RuntimeKind::FirstParty,
            TrustClass::UserTrusted,
            grants,
            MountView::default(),
        )
        .unwrap();
        ctx.validate().unwrap();
        ctx
    }

    fn test_trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![EffectKind::SpawnProcess, EffectKind::DispatchCapability],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: chrono::Utc::now(),
        }
    }

    fn test_authorizer(
        approval_policy: ApprovalPolicy,
    ) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
        profile_approval_authorizer(
            approval_policy,
            Arc::new(TestGatePolicy),
            Arc::new(EmptyApprovalSettingsProvider),
        )
    }

    fn test_authorizer_with_settings(
        approval_policy: ApprovalPolicy,
        settings: StubSettingsProvider,
    ) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
        profile_approval_authorizer(
            approval_policy,
            Arc::new(TestGatePolicy),
            Arc::new(settings),
        )
    }

    #[tokio::test]
    async fn dispatch_with_destructive_effect_requires_approval() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "destructive dispatch should require approval, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn spawn_with_dispatch_only_capability_requires_approval() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let echo_id = CapabilityId::new("builtin.echo").unwrap();
        let descriptor =
            test_descriptor_with_id(echo_id.clone(), vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: echo_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_spawn_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "spawn of dispatch-only capability should require approval via SpawnProcess elevation, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn default_allow_dispatch_requires_approval_when_global_auto_approve_is_off() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let echo_id = CapabilityId::new("builtin.echo").unwrap();
        let descriptor =
            test_descriptor_with_id(echo_id.clone(), vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: echo_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "default-allow dispatch should still ask when global auto-approve is off, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn settings_always_allow_skips_default_allow_dispatch_gate_without_loaded_grant() {
        let authorizer = test_authorizer_with_settings(
            ApprovalPolicy::AskDestructive,
            StubSettingsProvider {
                tool_override: None,
                global_auto_approve: false,
                tool_always_allow: true,
            },
        );

        let echo_id = CapabilityId::new("builtin.echo").unwrap();
        let descriptor =
            test_descriptor_with_id(echo_id.clone(), vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: echo_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::Allow { .. }),
            "settings always-allow should skip the default-allow dispatch gate without a preloaded persistent grant, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn global_auto_approve_does_not_skip_manifest_ineligible_tool() {
        let authorizer = test_authorizer_with_settings(
            ApprovalPolicy::AskDestructive,
            StubSettingsProvider {
                tool_override: None,
                global_auto_approve: true,
                tool_always_allow: false,
            },
        );

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let mut descriptor =
            test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        descriptor.default_permission = PermissionMode::Deny;
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "global auto-approve must not bypass a tool whose manifest no longer permits durable approval, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn settings_persistent_grant_does_not_skip_manifest_ineligible_tool() {
        let authorizer = test_authorizer_with_settings(
            ApprovalPolicy::AskDestructive,
            StubSettingsProvider {
                tool_override: None,
                global_auto_approve: false,
                tool_always_allow: true,
            },
        );

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let mut descriptor =
            test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        descriptor.default_permission = PermissionMode::Deny;
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "settings persistent grant must not bypass a tool whose manifest no longer permits durable approval, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn legacy_persistent_grant_does_not_override_global_off() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let echo_id = CapabilityId::new("builtin.echo").unwrap();
        let provider = ExtensionId::new("builtin").unwrap();
        let descriptor =
            test_descriptor_with_id(echo_id.clone(), vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: echo_id,
                grantee: Principal::Extension(provider),
                issued_by: persistent_approval_grant_issuer(),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "legacy persistent grant must not bypass Follow global when global auto-approve is off, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn minimal_policy_still_asks_when_global_auto_approve_is_off() {
        let authorizer = test_authorizer(ApprovalPolicy::Minimal);

        let descriptor = test_descriptor(vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new("builtin.shell").unwrap(),
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "Minimal policy should still ask when global auto-approve is off, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn user_issued_one_shot_approval_grant_allows_resume() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let base_ctx = test_context(CapabilitySet { grants: vec![] });
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(base_ctx.extension_id.clone()),
                issued_by: Principal::User(base_ctx.user_id.clone()),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::Allow { .. }),
            "same-user one-shot approval lease should satisfy the local-dev gate, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn persistent_approval_grant_allows_reuse() {
        let authorizer = test_authorizer_with_settings(
            ApprovalPolicy::AskDestructive,
            StubSettingsProvider {
                tool_override: None,
                global_auto_approve: false,
                tool_always_allow: true,
            },
        );

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let base_ctx = test_context(CapabilitySet { grants: vec![] });
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(base_ctx.extension_id.clone()),
                issued_by: persistent_approval_grant_issuer(),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::Allow { .. }),
            "settings persistent approval grant should satisfy the local-dev gate, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn user_issued_persistent_like_grant_does_not_allow_reuse() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let base_ctx = test_context(CapabilitySet { grants: vec![] });
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(base_ctx.extension_id.clone()),
                issued_by: Principal::User(base_ctx.user_id.clone()),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "standing user grant must not impersonate persistent approval replay, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn other_user_issued_persistent_like_grant_does_not_allow_reuse() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::User(ironclaw_host_api::UserId::new("other-user").unwrap()),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "different-user standing grant must not impersonate persistent approval replay, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn expired_persistent_approval_grant_does_not_allow_reuse() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let base_ctx = test_context(CapabilitySet { grants: vec![] });
        let ctx = test_context(CapabilitySet {
            grants: vec![
                CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: shell_id.clone(),
                    grantee: Principal::Extension(base_ctx.extension_id.clone()),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: vec![EffectKind::SpawnProcess],
                        mounts: MountView::default(),
                        network: NetworkPolicy::default(),
                        secrets: Vec::new(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                },
                CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: shell_id,
                    grantee: Principal::Extension(base_ctx.extension_id.clone()),
                    issued_by: persistent_approval_grant_issuer(),
                    constraints: GrantConstraints {
                        allowed_effects: vec![EffectKind::SpawnProcess],
                        mounts: MountView::default(),
                        network: NetworkPolicy::default(),
                        secrets: Vec::new(),
                        resource_ceiling: None,
                        expires_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
                        max_invocations: None,
                    },
                },
            ],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "expired persistent approval grant must not satisfy the local-dev gate, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn other_user_issued_approval_grant_does_not_allow_resume() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::User(ironclaw_host_api::UserId::new("other-user").unwrap()),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "different-user approval lease must not satisfy the local-dev gate, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn deny_decision_passes_through_unchanged() {
        let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

        let descriptor = test_descriptor(vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet { grants: vec![] });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::Deny { .. }),
            "ungranted capability should return Deny unchanged, got {decision:?}"
        );
    }

    #[test]
    fn approval_request_reason_includes_capability_id() {
        let descriptor = test_descriptor(vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet { grants: vec![] });
        let req = approval_request(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            ProfileApprovalActionKind::Dispatch,
        );

        assert!(
            req.reason.contains("builtin.shell"),
            "reason should contain capability id, got: {:?}",
            req.reason
        );
        assert!(
            req.reason.contains("Dispatch"),
            "reason should contain action kind, got: {:?}",
            req.reason
        );
    }
}
