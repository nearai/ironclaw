//! Composition-tier proof that the `hosted-single-tenant-volume-sandboxed`
//! profile resolves a per-user tenant-sandbox scope identity, not a shared
//! singleton — the wiring half of the cross-tenant isolation invariant in
//! `.claude/rules/safety-and-sandbox.md` ("Process and shell execution: real
//! OS isolation, per tenant"). The companion real-Docker crate-tier test
//! (`ironclaw_host_runtime/tests/sandbox_cross_tenant_escape.rs`) proves the
//! actual host bind-mount containment; this test runs everywhere (no
//! Docker) and proves composition forwards each caller's own scope through
//! to the sandbox transport instead of collapsing it onto one shared owner
//! scope.
//!
//! Uses a recording fake `SandboxCommandTransport` (same pattern as
//! `ProductionReadySandboxTransport` in `facade_factory.rs`) wired through
//! `RebornRuntimeProcessBinding::tenant_sandbox`, so no container is ever
//! launched.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, MountView, Principal, ResourceEstimate, ResourceScope,
    RuntimeKind, TrustClass, UserId, runtime_policy::ApprovalPolicy,
};
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RebornSandboxScopeKey, RuntimeProcessError,
    SHELL_CAPABILITY_ID, SandboxCommandTransport, TenantSandboxProcessPort,
    sandbox_network_policy,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornRuntimeProcessBinding, build_reborn_services,
    hosted_single_tenant_volume_sandboxed_runtime_policy, local_runtime_build_input_with_options,
};

/// `hosted-single-tenant-volume-sandboxed` requires an externally-supplied
/// secrets master key (fail-closed by design — see
/// `deployment.rs::hosted_volume_secret_master_key`); this crate's tests
/// serialize mutation of that one env var behind a lock, mirroring the
/// `EnvVarGuard`/`SECRETS_MASTER_KEY_ENV_LOCK` pattern already used in
/// `facade_factory.rs`.
static SECRET_MASTER_KEY_ENV_LOCK: Mutex<()> = Mutex::new(());
const SECRET_MASTER_KEY_ENV: &str = "IRONCLAW_REBORN_SECRET_MASTER_KEY";

struct EnvVarGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(value: &str) -> Self {
        let lock = SECRET_MASTER_KEY_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = std::env::var_os(SECRET_MASTER_KEY_ENV);
        // SAFETY: mutation is serialized by `SECRET_MASTER_KEY_ENV_LOCK`
        // above, and the prior value is restored on drop before the lock is
        // released.
        unsafe {
            std::env::set_var(SECRET_MASTER_KEY_ENV, value);
        }
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: see `set` above — still holding the serializing lock.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(SECRET_MASTER_KEY_ENV, value),
                None => std::env::remove_var(SECRET_MASTER_KEY_ENV),
            }
        }
    }
}

/// Records every `CommandExecutionRequest.scope` it is handed instead of
/// touching Docker — lets this test observe exactly which scope composition
/// forwarded per invocation.
#[derive(Debug, Default)]
struct RecordingSandboxTransport {
    scopes: Mutex<Vec<ResourceScope>>,
}

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.scopes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(request.scope);
        Ok(CommandExecutionOutput {
            output: "ok".to_string(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: std::time::Duration::ZERO,
        })
    }
}

fn shell_execution_context(user: &str) -> ExecutionContext {
    let grants = CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
            grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![
                    EffectKind::DispatchCapability,
                    EffectKind::SpawnProcess,
                    EffectKind::ExecuteCode,
                    EffectKind::ReadFilesystem,
                    EffectKind::WriteFilesystem,
                    EffectKind::Network,
                ],
                mounts: MountView::default(),
                // The sandboxed profile carries a real, non-empty egress
                // allowlist (proxy-enforced, see `sandbox_boot.rs` and
                // `ironclaw_host_runtime::sandbox_process::network_allowlist`)
                // rather than `NetworkPolicy::default()`'s empty deny-all —
                // an empty allowlist fails
                // `validate_network_policy_metadata` and blocks every
                // `builtin.shell` invocation in this profile.
                network: sandbox_network_policy(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    };
    ExecutionContext::local_default(
        UserId::new(user).unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

#[tokio::test]
async fn hosted_single_tenant_volume_sandboxed_forwards_distinct_scope_per_user() {
    let _env_guard = EnvVarGuard::set("01234567890123456789012345678901");
    let dir = tempfile::tempdir().unwrap();

    let transport = Arc::new(RecordingSandboxTransport::default());
    let process_port = Arc::new(TenantSandboxProcessPort::new(transport.clone()));
    // Bypass the approval gate the sandboxed profile's real policy carries
    // (`AskWrites`) so the shell invocation reaches the process port
    // directly — the thing under test is scope forwarding through
    // composition's `TenantSandbox` binding, not the approval pipeline,
    // which is covered elsewhere. `process_backend` stays `TenantSandbox`,
    // the real value under test.
    let mut policy = hosted_single_tenant_volume_sandboxed_runtime_policy()
        .expect("sandboxed profile policy resolves");
    assert_eq!(
        policy.process_backend.as_str(),
        "tenant_sandbox",
        "sanity: the profile under test must resolve TenantSandbox process backend"
    );
    policy.approval_policy = ApprovalPolicy::Minimal;

    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::HostedSingleTenantVolumeSandboxed,
        "sandboxed-two-user-owner",
        dir.path().to_path_buf(),
        Default::default(),
    )
    .expect("sandboxed profile build input resolves with the master key env set")
    .with_runtime_policy(policy)
    .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(process_port));

    let services = build_reborn_services(input)
        .await
        .expect("sandboxed profile services build with a wired TenantSandbox binding");
    let runtime = services
        .host_runtime
        .as_deref()
        .expect("sandboxed profile composes a host runtime");

    for user in ["user-a", "user-b"] {
        let outcome = runtime
            .invoke_capability((
                shell_execution_context(user),
                CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
                ResourceEstimate::default(),
                serde_json::json!({"command": "true"}),
            ))
            .await
            .expect("shell invocation returns an outcome");
        assert!(
            matches!(
                outcome,
                ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(_)
            ),
            "expected user {user} shell invocation to complete via the recording sandbox \
             transport, got {outcome:?}"
        );
    }

    let scopes = transport
        .scopes
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(
        scopes.len(),
        2,
        "expected exactly one recorded request per user, got {scopes:?}"
    );
    let key_a = RebornSandboxScopeKey::from_scope(&scopes[0]);
    let key_b = RebornSandboxScopeKey::from_scope(&scopes[1]);
    assert_ne!(scopes[0].user_id, scopes[1].user_id);
    assert_ne!(
        key_a, key_b,
        "composition must forward each user's own scope through the shared TenantSandbox \
         binding, not collapse both users onto one shared sandbox scope identity: {:?} vs {:?}",
        scopes[0], scopes[1]
    );
}
