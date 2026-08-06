//! API-visible first-party mutations for skill activation settings.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_assistant::{
    RebornSkillActionResponse, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
};
use ironclaw_extension_registry::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    capability::{EffectKind, OriginGateMatrix, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    ids::{CapabilityId, TenantId, UserId},
    resource::{ResourceEstimate, ResourceProfile, ResourceUsage},
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    let root = package
        .materialized_root()
        .map_err(|error| ExtensionError::InvalidManifest {
            reason: format!("built-in package requires a materialized root: {error}"),
        })?
        .clone();
    ExtensionPackage::from_manifest(package.manifest, root)
}

pub fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    auto_activate_learned: Arc<AtomicBool>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID)?,
        Arc::new(SetSkillAutoActivateLearnedHandler {
            auto_activate_learned,
            switch_owner: OnceLock::new(),
        }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID)?,
        description: "Set the learned-skill auto-activation default for this deployment."
            .to_string(),
        // A settings write. `EffectKind` has no settings variant, and the
        // per-user design this switch is waiting on (a durable per-user
        // record, read per turn by the activation source) *is* a filesystem
        // write, so the manifest keeps the stricter declaration rather than
        // under-declaring the effect. Over-declaring only tightens gating.
        effects: vec![EffectKind::WriteFilesystem],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        standard_op: None,
        input_schema_ref: CapabilityProfileSchemaRef::new(
            "schemas/builtin/skill_auto_activate_learned_set.input.v1.json",
        )?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(
            "schemas/builtin/skill_auto_activate_learned_set.output.v1.json",
        )?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(500)
                .set_output_bytes(1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(OriginGateMatrix::product_consent_only()),
    })
}

/// The identity a call to this capability writes on behalf of.
///
/// Only the tenant/user axes: `agent_id` and `project_id` vary between two
/// calls by the same person, and this switch is not scoped to either.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SwitchCaller {
    tenant_id: TenantId,
    user_id: UserId,
}

/// Writes the learned-skill auto-activation default.
///
/// **The switch is process-global, not per-user.** What the skill-activation
/// source reads (`ironclaw_loop_host::skill_activation::activation`) is this
/// one `AtomicBool`, shared by every turn of every user in the process; there
/// is no durable per-user record behind it and the read site takes no user.
/// So this handler does not pretend the setting is per-user: it binds the
/// switch to the first authenticated caller that sets it and denies every
/// other caller, rather than letting one user silently re-configure another
/// user's turns. The deny is deliberately the fail-closed direction — the
/// alternative is a cross-user write with no signal at either end.
///
/// Making the setting genuinely per-user is a change to the *read* side and
/// its wiring (a durable per-user record plus a per-turn lookup at the
/// activation source), neither of which this crate owns; the guard here is
/// what keeps the gap from being a silent one until then.
struct SetSkillAutoActivateLearnedHandler {
    auto_activate_learned: Arc<AtomicBool>,
    switch_owner: OnceLock<SwitchCaller>,
}

impl SetSkillAutoActivateLearnedHandler {
    /// Claim the process-global switch for `caller`, or reject a caller that
    /// is not the one holding it.
    fn claim_switch(
        &self,
        caller: SwitchCaller,
        started: Instant,
    ) -> Result<(), FirstPartyCapabilityError> {
        if self.switch_owner.get_or_init(|| caller.clone()) == &caller {
            return Ok(());
        }
        tracing::debug!(
            "denied a learned-skill auto-activation write from a caller that does not \
             hold the process-global switch"
        );
        Err(dispatch_error(
            RuntimeDispatchErrorKind::PolicyDenied,
            started,
        ))
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for SetSkillAutoActivateLearnedHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        ensure_declared(&request, started)?;
        let caller = authenticated_caller(&request, started)?;
        // Parse before claiming: a malformed payload must not take the switch
        // away from the caller that can actually use it.
        let enabled = parse_enabled(request.input, started)?;
        self.claim_switch(caller, started)?;
        self.auto_activate_learned.store(enabled, Ordering::Relaxed);
        let response = RebornSkillActionResponse {
            success: true,
            message: format!(
                "Default skill auto-activation {}",
                if enabled { "enabled" } else { "disabled" }
            ),
        };
        let output = serde_json::to_value(response)
            .map_err(|_| dispatch_error(RuntimeDispatchErrorKind::InvalidResult, started))?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            resource_usage(started),
        ))
    }
}

fn authenticated_caller(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<SwitchCaller, FirstPartyCapabilityError> {
    if request.authenticated_actor_user_id.as_ref() != Some(&request.scope.user_id) {
        return Err(dispatch_error(
            RuntimeDispatchErrorKind::PolicyDenied,
            started,
        ));
    }
    Ok(SwitchCaller {
        tenant_id: request.scope.tenant_id.clone(),
        user_id: request.scope.user_id.clone(),
    })
}

fn ensure_declared(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    if request.capability_id.as_str() == SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID {
        Ok(())
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::UndeclaredCapability,
            started,
        ))
    }
}

fn parse_enabled(
    input: serde_json::Value,
    started: Instant,
) -> Result<bool, FirstPartyCapabilityError> {
    let object = input
        .as_object()
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    let enabled = object
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    if object.len() == 1 {
        Ok(enabled)
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::InputEncode,
            started,
        ))
    }
}

fn dispatch_error(kind: RuntimeDispatchErrorKind, started: Instant) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(kind).with_usage(resource_usage(started))
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        resource::ResourceScope,
    };

    use super::*;

    fn handler(
        auto_activate_learned: Arc<AtomicBool>,
    ) -> Arc<dyn FirstPartyCapabilityHandler + 'static> {
        let mut registry = FirstPartyCapabilityRegistry::new();
        insert_handler(&mut registry, auto_activate_learned).expect("handler wiring");
        registry
            .get(
                &CapabilityId::new(SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID)
                    .expect("capability id"),
            )
            .expect("handler registered under its declared capability id")
    }

    /// One authenticated WebUI call, shaped the way the product surface
    /// stamps it: the scope's user is the verified actor.
    fn set_request(user: &str, enabled: bool) -> FirstPartyCapabilityRequest {
        raw_request(
            user,
            SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
            serde_json::json!({ "enabled": enabled }),
        )
    }

    /// The same authenticated shape as [`set_request`], but with the two
    /// fields the rejection taxonomy discriminates on -- the dispatched
    /// capability id and the raw input payload -- left to the caller.
    fn raw_request(
        user: &str,
        capability_id: &str,
        input: serde_json::Value,
    ) -> FirstPartyCapabilityRequest {
        let user_id = UserId::new(user).expect("user id");
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(capability_id).expect("capability id"),
            ResourceScope::local_default(user_id.clone(), InvocationId::new())
                .expect("resource scope"),
            input,
            None,
        );
        request.authenticated_actor_user_id = Some(user_id);
        request
    }

    #[test]
    fn capability_is_api_only_filesystem_write() {
        let manifest = manifest().expect("manifest");
        assert_eq!(manifest.visibility, CapabilityVisibility::Api);
        assert_eq!(manifest.default_permission, PermissionMode::Allow);
        assert_eq!(manifest.effects, vec![EffectKind::WriteFilesystem]);
    }

    /// The switch this capability writes is one process-wide flag, so a
    /// second user's call must be denied rather than silently re-configuring
    /// the first user's turns.
    ///
    /// Driven through the registry the way composition wires it, because the
    /// registration is what binds the handler to the id the product surface
    /// dispatches; a direct call on the struct would not prove that the
    /// authenticated caller ever reaches this handler.
    #[tokio::test]
    async fn a_second_user_cannot_move_the_process_global_default() {
        let auto_activate_learned = Arc::new(AtomicBool::new(true));
        let handler = handler(Arc::clone(&auto_activate_learned));

        handler
            .dispatch(set_request("alice", false))
            .await
            .expect("the owning caller sets the default");
        assert!(
            !auto_activate_learned.load(Ordering::Relaxed),
            "the owning caller's write lands"
        );

        let error = handler
            .dispatch(set_request("mallory", true))
            .await
            .expect_err("a different user must not write the process-global switch");
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::PolicyDenied),
            "the denial is a policy decision, not a malformed input"
        );
        assert!(
            !auto_activate_learned.load(Ordering::Relaxed),
            "the first caller's default survives another user's attempt"
        );
    }

    /// The fail-closed guard binds the switch to a caller, not to a single
    /// call: the owning caller keeps toggling it, including back to the value
    /// it started at.
    #[tokio::test]
    async fn the_owning_caller_keeps_writing_the_default() {
        let auto_activate_learned = Arc::new(AtomicBool::new(true));
        let handler = handler(Arc::clone(&auto_activate_learned));

        for enabled in [false, true, false] {
            handler
                .dispatch(set_request("alice", enabled))
                .await
                .expect("the owning caller keeps its write authority");
            assert_eq!(auto_activate_learned.load(Ordering::Relaxed), enabled);
        }
    }

    /// A caller whose verified actor does not match the scope it claims is
    /// denied before it can claim the switch — otherwise a spoofed scope
    /// would lock the real owner out of a setting it never set.
    #[tokio::test]
    async fn an_unverified_caller_neither_writes_nor_claims_the_switch() {
        let auto_activate_learned = Arc::new(AtomicBool::new(true));
        let handler = handler(Arc::clone(&auto_activate_learned));

        let mut spoofed = set_request("alice", false);
        spoofed.authenticated_actor_user_id = Some(UserId::new("mallory").expect("user id"));
        let error = handler
            .dispatch(spoofed)
            .await
            .expect_err("the verified actor must match the scope it writes");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::PolicyDenied));
        assert!(auto_activate_learned.load(Ordering::Relaxed));

        handler
            .dispatch(set_request("alice", false))
            .await
            .expect("the real owner still claims the switch afterwards");
        assert!(!auto_activate_learned.load(Ordering::Relaxed));
    }

    /// The rejection taxonomy of a *well-authenticated* caller: the two
    /// pre-write validators (`ensure_declared`, `parse_enabled`) must each
    /// answer with their own `RuntimeDispatchErrorKind`, and neither may let
    /// the write through.
    ///
    /// Both matter beyond tidiness. `ensure_declared` is what stops a handler
    /// registered under one id from servicing a dispatch for another, so a
    /// misrouted registry entry has to surface as `UndeclaredCapability`
    /// rather than silently writing the deployment default. `parse_enabled`
    /// rejects a payload carrying *extra* keys, not just a missing/ill-typed
    /// `enabled`: this capability's input schema is closed, and accepting an
    /// unknown sibling key is how a later schema revision starts silently
    /// ignoring a field callers believe they set.
    ///
    /// Driven through the registry-resolved handler with the *same* verified
    /// caller as the happy path, so the only thing that varies between the
    /// accepted call and each rejected one is the discriminating argument
    /// under test -- the dispatched capability id, or the input payload.
    #[tokio::test]
    async fn malformed_requests_are_rejected_by_kind_without_moving_the_switch() {
        let auto_activate_learned = Arc::new(AtomicBool::new(true));
        let handler = handler(Arc::clone(&auto_activate_learned));

        let rejections: Vec<(&str, FirstPartyCapabilityRequest, RuntimeDispatchErrorKind)> = vec![
            (
                "a dispatch for a capability this handler does not declare",
                raw_request(
                    "alice",
                    "ironclaw.skill.auto_activate_learned_unset",
                    serde_json::json!({ "enabled": false }),
                ),
                RuntimeDispatchErrorKind::UndeclaredCapability,
            ),
            (
                "a non-object payload",
                raw_request(
                    "alice",
                    SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
                    serde_json::json!(false),
                ),
                RuntimeDispatchErrorKind::InputEncode,
            ),
            (
                "an object with no `enabled` field",
                raw_request(
                    "alice",
                    SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
                    serde_json::json!({ "enable": false }),
                ),
                RuntimeDispatchErrorKind::InputEncode,
            ),
            (
                "a closed schema carrying an unknown sibling key",
                raw_request(
                    "alice",
                    SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
                    serde_json::json!({ "enabled": false, "scope": "everyone" }),
                ),
                RuntimeDispatchErrorKind::InputEncode,
            ),
        ];

        for (case, request, expected) in rejections {
            let Err(error) = handler.dispatch(request).await else {
                panic!("{case} must be rejected");
            };
            assert_eq!(
                error.kind(),
                Some(expected),
                "{case} reports the wrong kind"
            );
            assert!(
                auto_activate_learned.load(Ordering::Relaxed),
                "{case} must not reach the store"
            );
        }

        // The same caller still owns the switch afterwards: a rejected request
        // is not allowed to have claimed it on the way out.
        handler
            .dispatch(set_request("alice", false))
            .await
            .expect("a rejected request left the switch unclaimed");
        assert!(!auto_activate_learned.load(Ordering::Relaxed));
    }

    /// `extend_builtin_first_party_package` is only ever handed the
    /// host-bundled built-in package, which is materialized -- so the
    /// `materialized_root()` failure arm has never run. It is still the guard
    /// that keeps a rootless package from being published with this
    /// capability grafted onto it, and it must fail closed with a manifest
    /// error rather than panicking or silently dropping the root.
    #[test]
    fn a_rootless_package_cannot_be_extended_with_this_capability() {
        let manifest = ironclaw_extension_registry::ExtensionManifest::parse(
            VIRTUAL_MANIFEST,
            ironclaw_extension_registry::ManifestSource::UserRegistered,
            &ironclaw_host_api::host_port::HostPortCatalog::empty(),
            &{
                let mut contracts = ironclaw_extension_registry::HostApiContractRegistry::new();
                contracts
                    .register(Arc::new(
                        ironclaw_extension_registry::CapabilityProviderHostApiContract::new()
                            .expect("capability-provider contract"),
                    ))
                    .expect("register contract");
                contracts
            },
        )
        .expect("virtual manifest parses");
        // A virtual package carries its descriptors inline (there is no tree to
        // read `$ref` schemas from), so project them the way hosted-MCP
        // discovery does at `ironclaw_extension_host::hosted_mcp_manifest`.
        let capabilities = manifest
            .capabilities
            .iter()
            .map(
                |capability| ironclaw_host_api::capability::CapabilityDescriptor {
                    id: capability.id.clone(),
                    provider: manifest.id.clone(),
                    runtime: manifest.runtime.kind(),
                    trust_ceiling: manifest.descriptor_trust_default,
                    description: capability.description.clone(),
                    parameters_schema: serde_json::Value::Null,
                    effects: capability.effects.clone(),
                    default_permission: capability.default_permission,
                    runtime_credentials: capability.runtime_credentials.clone(),
                    network_targets: capability.network_targets.clone(),
                    max_egress_bytes: capability.max_egress_bytes,
                    resource_profile: capability.resource_profile.clone(),
                    origin_gate_matrix: capability.origin_gate_matrix.clone(),
                    standard_op: capability.standard_op,
                },
            )
            .collect();
        let package = ExtensionPackage::from_virtual_manifest(manifest, None, capabilities)
            .expect("remote-only package");

        let error = extend_builtin_first_party_package(package)
            .expect_err("a virtual package has no filesystem root to extend");
        assert!(
            matches!(
                &error,
                ExtensionError::InvalidManifest { reason }
                    if reason.contains("built-in package requires a materialized root")
            ),
            "unexpected error: {error:?}"
        );
    }

    /// A remote-only (virtual) package: an MCP-over-HTTP runtime, so it has no
    /// filesystem tree and therefore no materialized root.
    const VIRTUAL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "remote-tools"
name = "Remote Tools"
version = "0.1.0"
description = "Remote-only tool provider"
trust = "untrusted"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "remote-tools.invoke"
description = "Invoke a remote tool"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/remote-tools/invoke.input.v1.json"
"#;
}
