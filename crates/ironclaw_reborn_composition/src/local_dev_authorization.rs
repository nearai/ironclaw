use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{
    AutoApproveSettingStore, ToolPermissionOverrideKey, ToolPermissionOverrideStore,
};
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::{
    CapabilityId, EffectKind, ResourceScope,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy, RuntimeProfile},
};

use crate::{
    local_dev_capability_policy::LocalDevCapabilityPolicy,
    profile_approval_authorization::{
        ApprovalSettingsProvider, ProfileApprovalGatePolicy, ResolvedApprovalSettings,
        profile_approval_authorizer,
    },
    runtime_profile_approval_policy::RuntimeProfileApprovalGatePolicy,
};

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
    settings: Arc<dyn ApprovalSettingsProvider>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let (approval_policy, resolved_profile) = local_dev_approval_policy(runtime_policy);
    let gate_effects = capability_policy.approval_gate_effects();
    let exempt_capabilities = capability_policy.approval_gate_exempt_capabilities();
    let gate_policy: Arc<dyn ProfileApprovalGatePolicy> = Arc::new(
        RuntimeProfileApprovalGatePolicy::new(resolved_profile, gate_effects)
            .with_exempt_capabilities(exempt_capabilities),
    );
    profile_approval_authorizer(approval_policy, gate_policy, settings)
}

/// Live [`ApprovalSettingsProvider`] backed by the durable per-user approval
/// stores. Queried on every dispatch gate decision so a WebUI change takes
/// effect without a process restart (#4959).
pub(crate) struct StoreApprovalSettingsProvider {
    overrides: Arc<dyn ToolPermissionOverrideStore>,
    auto_approve: Arc<dyn AutoApproveSettingStore>,
}

impl StoreApprovalSettingsProvider {
    pub(crate) fn new(
        overrides: Arc<dyn ToolPermissionOverrideStore>,
        auto_approve: Arc<dyn AutoApproveSettingStore>,
    ) -> Self {
        Self {
            overrides,
            auto_approve,
        }
    }
}

#[async_trait]
impl ApprovalSettingsProvider for StoreApprovalSettingsProvider {
    async fn resolve(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> ResolvedApprovalSettings {
        // Fail safe: a store read error resolves to "no override, auto-approve
        // off" so the gate falls back to asking rather than silently
        // auto-approving or denying. The error is logged, not swallowed.
        let key = ToolPermissionOverrideKey::new(scope, capability_id.clone());
        let tool_override = match self.overrides.get(&key).await {
            Ok(record) => record.map(|record| record.state),
            Err(error) => {
                // silent-ok: fail-safe to "ask" on store read error; logged for observability.
                tracing::warn!(%error, "tool permission override lookup failed; defaulting to ask");
                None
            }
        };
        let global_auto_approve = match self.auto_approve.is_enabled(scope).await {
            Ok(enabled) => enabled,
            Err(error) => {
                // silent-ok: fail-safe to "ask" by disabling global auto-approve; logged for observability.
                tracing::warn!(%error, "auto-approve setting lookup failed; defaulting to off");
                false
            }
        };
        ResolvedApprovalSettings {
            tool_override,
            global_auto_approve,
        }
    }
}

pub(crate) fn local_dev_effects_require_approval(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: &LocalDevCapabilityPolicy,
    effects: &[EffectKind],
) -> bool {
    let (approval_policy, resolved_profile) = local_dev_approval_policy(runtime_policy);
    RuntimeProfileApprovalGatePolicy::new(
        resolved_profile,
        capability_policy.approval_gate_effects(),
    )
    .effects_require_approval(approval_policy, effects)
}

fn local_dev_approval_policy(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
) -> (ApprovalPolicy, RuntimeProfile) {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    let resolved_profile = runtime_policy
        .map(|policy| policy.resolved_profile)
        .unwrap_or(RuntimeProfile::LocalDev);
    (approval_policy, resolved_profile)
}

#[cfg(test)]
mod tests {
    use ironclaw_approvals::{
        AutoApproveSettingInput, InMemoryAutoApproveSettingStore,
        InMemoryToolPermissionOverrideStore,
    };
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, MountView, PermissionMode,
        Principal, ResourceEstimate, RuntimeKind, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        BUILTIN_FIRST_PARTY_PROVIDER, PROFILE_SET_CAPABILITY_ID,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::json;

    use super::*;
    use crate::local_dev_capability_policy::local_dev_capability_policy;

    async fn local_dev_shell_decision_with_authorizer(
        authorizer: &dyn TrustAwareCapabilityDispatchAuthorizer,
        scope_user: &UserId,
    ) -> ironclaw_host_api::Decision {
        let capability_id = CapabilityId::new("builtin.shell").expect("capability id");
        let effects = vec![EffectKind::SpawnProcess];
        let descriptor = CapabilityDescriptor {
            id: capability_id,
            provider: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id"),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test".to_string(),
            parameters_schema: json!({}),
            effects: effects.clone(),
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            resource_profile: None,
        };
        let provider_id = ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id");
        let policy = local_dev_capability_policy().expect("capability policy");
        let grants = policy.builtin_grants(
            &provider_id,
            &MountView::default(),
            &MountView::default(),
            &MountView::default(),
        );
        let context = ironclaw_host_api::ExecutionContext::local_default(
            scope_user.clone(),
            provider_id,
            RuntimeKind::FirstParty,
            TrustClass::UserTrusted,
            grants,
            MountView::default(),
        )
        .expect("execution context");
        let trust_decision = TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: effects,
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: chrono::Utc::now(),
        };

        authorizer
            .authorize_dispatch_with_trust(
                &context,
                &descriptor,
                &ResourceEstimate::default(),
                &trust_decision,
            )
            .await
    }

    /// Run the local-dev authorizer for a Trace Commons capability with the
    /// given descriptor `effects` and return its decision. Asserts up front that
    /// the effects WOULD require an approval gate without an exemption, so a
    /// "skips gate" assertion can't pass via a non-gating default policy.
    async fn trace_commons_authorize_decision(
        capability_id: &str,
        effects: Vec<EffectKind>,
    ) -> ironclaw_host_api::Decision {
        let capability_id = CapabilityId::new(capability_id).expect("capability id");
        let descriptor = CapabilityDescriptor {
            id: capability_id,
            provider: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id"),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test".to_string(),
            parameters_schema: json!({}),
            effects: effects.clone(),
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            resource_profile: None,
        };
        let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
        let provider_id = ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id");
        let grants = policy.builtin_grants(
            &provider_id,
            &MountView::default(),
            &MountView::default(),
            &MountView::default(),
        );
        let context = ironclaw_host_api::ExecutionContext::local_default(
            ironclaw_host_api::UserId::new("test-user").expect("user id"),
            provider_id,
            RuntimeKind::FirstParty,
            TrustClass::UserTrusted,
            grants,
            MountView::default(),
        )
        .expect("execution context");
        // These effects must be gate-worthy without an exemption, so the
        // skips-gate vs requires-gate distinction is driven by the exemption
        // list, not by a non-gating default policy.
        assert!(
            local_dev_effects_require_approval(None, policy.as_ref(), &effects),
            "test must use effects that require approval without the capability exemption"
        );
        let trust_decision = TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: effects,
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: chrono::Utc::now(),
        };
        let authorizer = local_dev_authorizer(
            None,
            policy,
            Arc::new(crate::profile_approval_authorization::EmptyApprovalSettingsProvider),
        );
        authorizer
            .authorize_dispatch_with_trust(
                &context,
                &descriptor,
                &ResourceEstimate::default(),
                &trust_decision,
            )
            .await
    }

    #[tokio::test]
    async fn local_dev_trace_commons_profile_set_requires_approval_gate() {
        // profile_set publishes a PUBLIC community profile and is deliberately
        // NOT on the approval-gate exemption list: a model-controlled
        // `confirmed=true` is not sufficient consent for a public external
        // write, so it must hit the runtime approval gate.
        let decision = trace_commons_authorize_decision(
            TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ],
        )
        .await;
        assert!(
            matches!(
                decision,
                ironclaw_host_api::Decision::RequireApproval { .. }
            ),
            "profile_set (public external write, not exempt) must require an approval gate, got {decision:?}"
        );
    }

    /// Surface-visibility regression test: `builtin.profile_set` must be
    /// Available (Allow) in the local-dev authorizer, not RequireApproval.
    ///
    /// This exercises the FULL authorizer path (grant lookup + effect-set
    /// check + exemption list) to guard against the MissingGrant regression
    /// that caused the capability to vanish from the model-visible surface.
    /// The effects used (ReadFilesystem + WriteFilesystem) are gate-worthy
    /// without an exemption (write_filesystem is in ask_writes), so the Allow
    /// decision can only come from the exemption list, not from a non-gating
    /// default policy.
    #[tokio::test]
    async fn local_dev_builtin_profile_set_skips_approval_gate() {
        let decision = trace_commons_authorize_decision(
            PROFILE_SET_CAPABILITY_ID,
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        )
        .await;
        assert!(
            matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
            "builtin.profile_set is a private local write (no network/external_write) and is \
             exempt from the approval gate; got {decision:?}"
        );
    }

    #[tokio::test]
    async fn local_dev_trace_commons_onboard_skips_approval_gate() {
        // onboard IS exempt (it runs its own in-turn confirmed=true consent
        // before the network POST). Cover it with its real
        // network + external_write + filesystem-write effects so dropping the
        // TOML exemption fails here.
        let decision = trace_commons_authorize_decision(
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ],
        )
        .await;
        assert!(
            matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
            "onboard is consented in-turn and exempt, so it should not require a REPL approval gate, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn local_dev_authorizer_reads_approval_settings_store_on_each_dispatch() {
        let user_id = UserId::new("test-user").expect("user id");
        let overrides = Arc::new(InMemoryToolPermissionOverrideStore::new());
        let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
        let settings = Arc::new(StoreApprovalSettingsProvider::new(
            overrides,
            auto_approve.clone(),
        ));
        let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
        let authorizer = local_dev_authorizer(None, policy, settings);

        let before = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
        assert!(
            matches!(before, ironclaw_host_api::Decision::RequireApproval { .. }),
            "default local-dev shell dispatch should gate before a settings update, got {before:?}"
        );

        let scope = ironclaw_host_api::ResourceScope::local_default(
            user_id.clone(),
            ironclaw_host_api::InvocationId::new(),
        )
        .expect("local resource scope");
        auto_approve
            .set(AutoApproveSettingInput {
                scope,
                enabled: true,
                updated_by: Principal::User(user_id.clone()),
            })
            .await
            .expect("auto-approve setting update");

        let after = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
        assert!(
            matches!(after, ironclaw_host_api::Decision::Allow { .. }),
            "same authorizer should observe the store update on the next dispatch, got {after:?}"
        );
    }
}
