use std::sync::atomic::{AtomicUsize, Ordering};
use std::{sync::Arc, sync::LazyLock, time::Duration};

use async_trait::async_trait;
use ironclaw_approvals::AutoApproveSettingInput;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, Principal, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_product_workflow::{
    ApprovalInteractionDecision, ListPendingApprovalsRequest, ListPendingAuthInteractionsRequest,
    ResolveApprovalInteractionRequest,
};
use ironclaw_reborn_composition::{
    HooksActivationConfig, PollSettings, RebornBuildInput, RebornRuntimeError,
    RebornRuntimeIdentity, RebornRuntimeInput, RebornSkillSourceKind, RebornTurnDriveOutcome,
    TurnRunnerSettings, build_reborn_runtime,
};
#[cfg(feature = "libsql")]
use ironclaw_reborn_composition::{
    RebornCompositionProfile, local_runtime_build_input_with_options,
};
use ironclaw_turns::run_profile::{
    LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey, ResumeTurnRequest,
    ResumeTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator,
    TurnError, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const SEND_USER_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);
// These tests start full local-dev runtimes; with libsql enabled they contend
// enough under libtest parallelism to trip timeout-oriented assertions.
static RUNTIME_COMPOSITION_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

async fn runtime_composition_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    RUNTIME_COMPOSITION_TEST_LOCK.lock().await
}

#[tokio::test]
async fn runtime_rejects_disabled_profile_before_local_substrate_lookup() {
    let input =
        RebornRuntimeInput::from_services(RebornBuildInput::disabled("runtime-disabled-owner"));

    let error = match build_reborn_runtime(input).await {
        Ok(_) => panic!("disabled profile is not a runnable REPL runtime"),
        Err(error) => error,
    };

    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("profile=disabled must not start live Reborn runtime traffic"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn runtime_rejects_migration_dry_run_before_live_traffic() {
    let dir = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .unwrap(),
    );
    let input = RebornRuntimeInput::from_services(RebornBuildInput::libsql(
        ironclaw_reborn_composition::RebornCompositionProfile::MigrationDryRun,
        "runtime-migration-dry-run-owner",
        db,
        dir.path().join("events.db").to_string_lossy(),
        None,
        ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
    ));

    let error = match build_reborn_runtime(input).await {
        Ok(runtime) => {
            runtime.shutdown().await.expect("shutdown");
            panic!("migration-dry-run must validate only and never start live runtime traffic");
        }
        Err(error) => error,
    };

    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(
        reason.contains("profile=migration-dry-run")
            && reason.contains("must not start live Reborn runtime traffic"),
        "reason: {reason}"
    );
}

#[tokio::test]
async fn runtime_requires_resolved_runtime_policy_for_local_dev() {
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
        "runtime-policy-owner",
        root.path().join("local-dev"),
    ));

    let error = match build_reborn_runtime(input).await {
        Ok(_) => panic!("local-dev runtime should require a resolved runtime policy"),
        Err(error) => error,
    };

    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("resolved runtime policy"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn hosted_single_tenant_volume_builds_live_runtime() {
    // Regression for #5346: the runtime profile match was hardcoded after
    // #5259 added this live local-substrate profile, so startup reached the
    // "unsupported runtime profile checked above" unreachable arm.
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        local_runtime_build_input_with_options(
            RebornCompositionProfile::HostedSingleTenantVolume,
            "runtime-hosted-volume-owner",
            root.path().join("hosted-volume"),
            Default::default(),
        )
        .unwrap(),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-hosted-volume-tenant".to_string(),
        agent_id: "runtime-hosted-volume-agent".to_string(),
        source_binding_id: "runtime-hosted-volume-source".to_string(),
        reply_target_binding_id: "runtime-hosted-volume-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            .set_heartbeat_interval(Duration::from_secs(60))
            .set_poll_interval(Duration::from_secs(60)),
    );

    let runtime = build_reborn_runtime(input).await.unwrap();
    assert_eq!(runtime.default_run_profile_id(), "reborn-planned-default");

    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn stub_gateway_send_cancels_recovery_required_and_releases_conversation() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-test-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-test-tenant".to_string(),
        agent_id: "runtime-test-agent".to_string(),
        source_binding_id: "runtime-test-source".to_string(),
        reply_target_binding_id: "runtime-test-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            .set_heartbeat_interval(Duration::from_secs(60))
            .set_poll_interval(Duration::from_secs(60)),
    );

    let runtime = build_reborn_runtime(input).await.unwrap();
    assert_eq!(runtime.default_run_profile_id(), "reborn-planned-default");

    let conversation = runtime.new_conversation().await.unwrap();
    let reply = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conversation, "hello"),
    )
    .await
    .unwrap()
    .unwrap();

    // With no LLM gateway compiled in, the stub gateway reports a
    // configuration fault (CredentialUnavailable) that fails the run on
    // first sight — no availability retries — and verifies the final
    // checkpoint evidence, mapping to a terminal
    // model_credentials_unavailable failure instead of the pre-PR
    // RecoveryRequired path that cancelled via the standalone-runtime guard.
    assert_eq!(reply.status, TurnStatus::Failed);
    assert_eq!(
        reply.failure_category.as_deref(),
        Some("model_credentials_unavailable")
    );
    assert_eq!(reply.text, None);

    let second_reply = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conversation, "hello again"),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(second_reply.status, TurnStatus::Failed);
    assert_eq!(
        second_reply.failure_category.as_deref(),
        Some("model_credentials_unavailable")
    );
    assert_eq!(second_reply.text, None);

    runtime.shutdown().await.unwrap();
}

/// Minimal completing model gateway: every model call returns a plain assistant
/// reply, so a turn reaches `TurnStatus::Completed` without needing a real LLM.
#[derive(Default)]
struct AlwaysReplyGateway;

#[async_trait]
impl HostManagedModelGateway for AlwaysReplyGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply(
            "done".to_string(),
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        _request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply(
            "done".to_string(),
        ))
    }
}

/// #6263 Step 4/5b — production wiring at the composition seam.
/// `build_reborn_runtime` composes the durable turn-state ROW store
/// (`factory.rs`) unconditionally, replacing the former in-memory authority +
/// block-persistence snapshot. This drives a real turn end to end over that
/// store (submit → claim → terminal, through the production runtime), then
/// gracefully `shutdown()`s — which routes through `RebornRuntime::shutdown →
/// FilesystemTurnStateStoreKind::drain`, exercising the write-behind durable
/// tail drain for real: the test locks that composing the store, serving a
/// real turn over it, and draining on shutdown all succeed without
/// error/hang/panic.
///
/// Deeper durability is pinned one tier down, over the raw store where
/// scope/backend are controlled precisely: terminal/gate-park recovery across a
/// store reopen and the drain-flushes-the-tail contract in
/// `ironclaw_turns::row_store_crash_consistency` (incl.
/// `write_behind_drain_flushes_the_async_tail_for_graceful_restart`), and the
/// block-persistence→row migration in
/// `filesystem_turn_state_contract::filesystem_turn_state_row_store_migrates_block_persistence_gate_park_snapshot`.
#[tokio::test]
async fn inmemory_turn_state_row_store_serves_turn_and_drains_on_shutdown() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("wb-durable-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "wb-durable-tenant".to_string(),
        agent_id: "wb-durable-agent".to_string(),
        source_binding_id: "wb-durable-source".to_string(),
        reply_target_binding_id: "wb-durable-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            .set_heartbeat_interval(Duration::from_secs(60))
            .set_poll_interval(Duration::from_secs(60)),
    )
    .with_model_gateway_override(Arc::new(AlwaysReplyGateway));

    // Compose the durable row store via the production build path and drive a real
    // turn to Completed over it: proves the flipped store serves the full
    // submit → claim → terminal transition set through the production runtime.
    let runtime = build_reborn_runtime(input).await.unwrap();
    let conversation = runtime.new_conversation().await.unwrap();
    let reply = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conversation, "durable please"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        reply.status,
        TurnStatus::Completed,
        "turn must complete over the WriteBehind store, got {:?} ({:?})",
        reply.status,
        reply.failure_category
    );

    // Graceful shutdown drains the WriteBehind tail through
    // `FilesystemTurnStateStoreKind::drain`; a broken drain wiring surfaces here.
    runtime
        .shutdown()
        .await
        .expect("graceful shutdown drains the WriteBehind tail without error");
}

#[tokio::test]
async fn send_user_message_with_cancellation_cancels_submitted_run() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-cancel-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-cancel-tenant".to_string(),
        agent_id: "runtime-cancel-agent".to_string(),
        source_binding_id: "runtime-cancel-source".to_string(),
        reply_target_binding_id: "runtime-cancel-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            .set_heartbeat_interval(Duration::from_secs(60))
            .set_poll_interval(Duration::from_secs(60)),
    )
    .with_poll_settings(PollSettings {
        interval: Duration::from_secs(60),
        max_total: Duration::from_secs(180),
    });

    let runtime = build_reborn_runtime(input).await.unwrap();
    let conversation = runtime.new_conversation().await.unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.send_user_message_with_cancellation(&conversation, "cancel me", cancellation),
    )
    .await
    .unwrap()
    .unwrap_err();

    assert!(matches!(error, RebornRuntimeError::OperationCancelled));

    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn skill_execution_adapter_prepares_filesystem_bundles_end_to_end() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let storage_root = root.path().join("local-dev");
    let skill_root = storage_root
        .join("tenants/runtime-skill-execution-tenant/users/runtime-skill-execution-owner/skills/policy-helper");
    std::fs::create_dir_all(skill_root.join("references")).unwrap();
    std::fs::write(
        skill_root.join("SKILL.md"),
        skill_md("policy-helper", "policy-helper", "Use policy guidance."),
    )
    .unwrap();
    std::fs::write(skill_root.join("references/policy.md"), "filesystem policy").unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-skill-execution-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-skill-execution-tenant".to_string(),
        agent_id: "runtime-skill-execution-agent".to_string(),
        source_binding_id: "runtime-skill-execution-source".to_string(),
        reply_target_binding_id: "runtime-skill-execution-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(10),
    });

    let runtime = build_reborn_runtime(input).await.unwrap();
    let conversation = runtime.new_conversation().await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        runtime.execute_skill_message(&conversation, "$policy-helper"),
    )
    .await
    .unwrap()
    .unwrap();

    let policy_activations: Vec<_> = result
        .plan
        .activations()
        .iter()
        .filter(|activation| {
            activation.name == "policy-helper"
                && activation.source == Some(RebornSkillSourceKind::User)
        })
        .collect();
    assert_eq!(
        policy_activations.len(),
        1,
        "explicit user skill should activate exactly once"
    );
    // Runtime composition may add criteria-selected system skills; this guard is
    // specifically about the explicit filesystem-backed user skill.
    let policy_bundles: Vec<_> = result
        .plan
        .active_bundles()
        .iter()
        .filter(|bundle| {
            bundle.source == RebornSkillSourceKind::User && bundle.skill_name == "policy-helper"
        })
        .collect();
    assert_eq!(
        policy_bundles.len(),
        1,
        "explicit user skill bundle should be active exactly once"
    );
    let activation = policy_activations[0];
    let bundle = policy_bundles[0];
    assert_eq!(bundle.skill_name, activation.name);

    let asset = runtime
        .read_skill_execution_asset(
            &conversation,
            &result.plan,
            activation,
            "references/policy.md",
        )
        .await
        .unwrap();
    assert_eq!(asset.into_utf8().unwrap(), "filesystem policy");

    runtime.shutdown().await.unwrap();
}

/// Drives `build_reborn_runtime` through the third-party hook activation wiring
/// (runtime.rs: third-party discovery input + projection registry + tenant
/// threading) with BOTH flags on and a real `/system/extensions` manifest tree
/// on the local-dev host filesystem.
///
/// This is the only test that exercises the `build_reborn_runtime` third-party
/// path end-to-end: `tests/third_party_hook_projection.rs` calls
/// `build_hook_projection_registry` + `build_hook_dispatcher_builder_factory_for_tenant`
/// directly against a fake filesystem, and every other `build_reborn_runtime`
/// call here uses the default disabled `HooksActivationConfig`. A regression in
/// the wiring (dropped `hooks_config`, wrong `extension_filesystem`, mis-threaded
/// tenant) would surface here as a build/start failure rather than going
/// uncovered.
#[tokio::test]
async fn build_reborn_runtime_wires_third_party_hooks_when_enabled() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let storage_root = root.path().join("local-dev");

    // Plant a discoverable third-party extension carrying a `[[hooks]]` block at
    // the per-owner `/system/extensions` discovery root that local-dev mounts.
    // The third-party projection path must read this manifest; with the wiring
    // broken (e.g. `extension_filesystem` not threaded), the runtime would not
    // build/start cleanly through `build_default_planned_runtime`.
    let extension_dir = storage_root.join("system/extensions/example-hook-ext");
    std::fs::create_dir_all(&extension_dir).unwrap();
    std::fs::write(
        extension_dir.join("manifest.toml"),
        third_party_hook_manifest("example-hook-ext"),
    )
    .unwrap();

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-hooks-owner", storage_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-hooks-tenant".to_string(),
        agent_id: "runtime-hooks-agent".to_string(),
        source_binding_id: "runtime-hooks-source".to_string(),
        reply_target_binding_id: "runtime-hooks-reply".to_string(),
    })
    .with_hooks_config(HooksActivationConfig::enabled().with_third_party_enabled(true))
    .with_runner_settings(
        TurnRunnerSettings::default()
            .set_heartbeat_interval(Duration::from_millis(25))
            .set_poll_interval(Duration::from_secs(60)),
    );

    // Build succeeds: the third-party discovery + projection + dispatcher factory
    // composed into the planned runtime without error.
    let runtime = build_reborn_runtime(input).await.unwrap();
    assert_eq!(runtime.default_run_profile_id(), "reborn-planned-default");

    // Runtime starts: a conversation turn runs through the composed dispatcher
    // and reaches a terminal state without hanging.
    let conversation = runtime.new_conversation().await.unwrap();
    let reply = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conversation, "hello"),
    )
    .await
    .unwrap()
    .unwrap();
    // TODO(coverage gap, inherited from the removed test): the stub local-dev
    // gateway terminates the turn before any capability call dispatches, so this
    // asserts terminal progress rather than observing the projected `deny-run`
    // hook actually firing on `example-hook-ext.run`. The wiring (discovery +
    // projection + tenant threading) is exercised at build/start; end-to-end
    // hook *enforcement* through `build_reborn_runtime` still needs a harness
    // that drives a real capability call to completion.
    assert!(reply.status.is_terminal(), "got {:?}", reply.status);

    runtime.shutdown().await.unwrap();
}

/// A discoverable v2 installed-extension manifest carrying a single
/// `before_capability` hook over its own capability. Mirrors the canonical
/// shape in `tests/third_party_hook_projection.rs`.
fn third_party_hook_manifest(id: &str) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "{id} extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run {id}"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"
required_host_ports = ["host.runtime.http_egress"]

[[hooks]]
id = "deny-run"
kind = "before_capability"
scope = "own_capabilities"
body = {{ mode = "predicate", spec = {{ type = "deny_capability", reason = "blocked", when = {{ type = "name_equals", name = "{id}.run" }} }} }}
"#
    )
}

fn skill_md(name: &str, keyword: &str, prompt: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {name} description\nactivation:\n  keywords: [\"{keyword}\"]\n---\n\n{prompt}"
    )
}

/// Caller-level config-wiring test: `build_reborn_runtime` correctly threads
/// `TurnRunnerSettings::max_concurrent_runs_per_user` into the turn-state store.
///
/// Exercises the full `build_reborn_runtime` → `build_reborn_services` →
/// `FilesystemTurnStateRowStore::with_limits` wiring path so that a mis-wired or
/// accidentally-dropped limit is caught at the composition boundary, not just in
/// unit tests that hand-construct the store.
///
/// Per `.claude/rules/testing.md` ("Test Through the Caller, Not Just the Helper")
/// the store-level cap enforcement is tested in `concurrent_workers.rs`; this test
/// adds the missing caller-tier assertion that `build_reborn_runtime` propagates the
/// cap value from the settings struct into the live store.
///
/// The test uses a single-user runtime with `max_concurrent_runs_per_user = 1`.
/// It submits two sequential turns on distinct conversations and asserts neither
/// is rejected by a misconfiguration of the limits (e.g., a zero limit that would
/// refuse any run). Sequential submission is sufficient because: with the stub
/// gateway, turns complete synchronously (no LLM gateway configured → driver
/// protocol violation, which is a terminal failure that releases the slot); the
/// per-user cap only blocks a *second* concurrent turn while the first is Running.
/// A full concurrent-claim proof that two parallel tasks race for the slot is in
/// `config_wiring_per_user_cap_enforced_via_store_limits` (concurrent_workers.rs),
/// which mirrors the exact store construction `build_reborn_runtime` performs.
#[tokio::test]
async fn build_reborn_runtime_wires_per_user_cap_from_turn_runner_settings() {
    let _guard = runtime_composition_test_guard().await;
    use std::num::NonZeroU32;

    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("cap-wiring-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "cap-wiring-tenant".to_string(),
        agent_id: "cap-wiring-agent".to_string(),
        source_binding_id: "cap-wiring-source".to_string(),
        reply_target_binding_id: "cap-wiring-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            // Cap = 1 per user. Verifies this value flows from settings → store limits.
            .set_max_concurrent_runs_per_user(NonZeroU32::new(1).expect("nonzero cap"))
            .set_heartbeat_interval(Duration::from_millis(25))
            .set_poll_interval(Duration::from_millis(10)),
    );

    let runtime = build_reborn_runtime(input).await.unwrap();

    // Submit two sequential turns on two conversations. With the stub gateway
    // each turn completes (as Failed / model_credentials_unavailable) before the
    // next is submitted, so the per-user slot is always free and neither
    // submission should be rejected. If the cap was accidentally set to 0 (a
    // misconfiguration the wiring layer could introduce) the store would block
    // every claim and both turns would never be completed, causing a timeout.
    let conv_a = runtime.new_conversation().await.unwrap();
    let reply_a = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conv_a, "first message"),
    )
    .await
    .expect("first send timed out (cap wiring may have set limit to 0)");

    assert!(
        !matches!(reply_a, Err(RebornRuntimeError::WorkerStopped)),
        "first turn must not be rejected by a misconfigured zero-cap store; got: {reply_a:?}"
    );

    let conv_b = runtime.new_conversation().await.unwrap();
    let reply_b = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conv_b, "second message"),
    )
    .await
    .expect("second send timed out (cap wiring may have set limit to 0)");

    assert!(
        !matches!(reply_b, Err(RebornRuntimeError::WorkerStopped)),
        "second turn must not be rejected by a misconfigured zero-cap store; got: {reply_b:?}"
    );

    runtime.shutdown().await.unwrap();
}

/// Verifies the `all()`-not-`any()` worker-stopped guard semantics.
///
/// `RebornRuntime` starts N workers and returns `WorkerStopped` only when
/// *every* worker has exited. This test exercises the guard with `worker_count
/// = 2` to confirm that submissions succeed while all workers are alive.
///
/// Partial-crash testing (killing exactly one of N workers and asserting the
/// other N-1 still accept work) requires internal access to `worker_cancel` /
/// `worker_handles`, which are private fields. That path is covered by the
/// unit-level tests inside `runtime.rs` (module-internal `#[cfg(test)]`). What
/// this test contributes is the composition-level proof that `build_reborn_runtime`
/// with `worker_count > 1` does NOT spuriously raise `WorkerStopped` while all
/// workers are healthy — the bug the `all()` fix addresses.
#[tokio::test]
async fn multi_worker_runtime_does_not_raise_worker_stopped_while_workers_are_alive() {
    let _guard = runtime_composition_test_guard().await;
    use std::num::NonZeroUsize;

    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("multi-worker-guard-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "multi-worker-guard-tenant".to_string(),
        agent_id: "multi-worker-guard-agent".to_string(),
        source_binding_id: "multi-worker-guard-source".to_string(),
        reply_target_binding_id: "multi-worker-guard-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            // Explicitly set 2 workers — ensures the guard uses .all() semantics
            // and does not fire when only a subset of workers have finished.
            .set_worker_count(NonZeroUsize::new(2).expect("nonzero worker count"))
            .set_heartbeat_interval(Duration::from_millis(25))
            .set_poll_interval(Duration::from_secs(60)),
    );

    let runtime = build_reborn_runtime(input).await.unwrap();
    let conversation = runtime.new_conversation().await.unwrap();

    // Submit a turn: with 2 healthy workers the guard must NOT return WorkerStopped.
    let reply = tokio::time::timeout(
        SEND_USER_MESSAGE_TIMEOUT,
        runtime.send_user_message(&conversation, "hello from multi-worker test"),
    )
    .await
    .unwrap();

    assert!(
        !matches!(reply, Err(RebornRuntimeError::WorkerStopped)),
        "WorkerStopped must not be raised while all workers are running; got: {reply:?}"
    );

    runtime.shutdown().await.unwrap();
}

// W5-WEBUI-API-2 enabler smoke test: `local_dev_*_interaction_service_for_test`
// build real services (not `Rejecting*`/`Unavailable*` fallbacks) using the
// runtime's own live `TurnCoordinator`. Full RESOLVE_GATE scenario coverage is a later PR.
#[tokio::test]
async fn local_dev_test_support_interaction_service_accessors_build_real_services() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(
            "test-support-accessors-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "test-support-accessors-tenant".to_string(),
        agent_id: "test-support-accessors-agent".to_string(),
        source_binding_id: "test-support-accessors-source".to_string(),
        reply_target_binding_id: "test-support-accessors-reply".to_string(),
    })
    .with_runner_settings(
        TurnRunnerSettings::default()
            .set_heartbeat_interval(Duration::from_secs(60))
            .set_poll_interval(Duration::from_secs(60)),
    );

    let runtime = build_reborn_runtime(input).await.unwrap();
    let turn_coordinator = runtime
        .services()
        .turn_coordinator
        .clone()
        .expect("local-dev runtime should wire a turn coordinator");

    let approval_interaction_service = runtime
        .services()
        .local_dev_approval_interaction_service_for_test(turn_coordinator.clone())
        .expect("local-dev capability policy and grantee resolver should construct cleanly")
        .expect("local-dev runtime should support the approval interaction test accessor");
    let auth_interaction_service = runtime
        .services()
        .local_dev_auth_interaction_service_for_test(turn_coordinator)
        .expect("local-dev runtime should support the auth interaction test accessor");

    let scope = TurnScope::new(
        TenantId::new("test-support-accessors-tenant").expect("tenant id"),
        None,
        None,
        ThreadId::new("test-support-accessors-thread".to_string()).expect("thread id"),
    );
    let actor = TurnActor::new(UserId::new("test-support-accessors-user").expect("user id"));

    // Discriminating assertion: a real service answers `Ok` with an empty list;
    // the fail-closed `Rejecting*`/`Unavailable*` fallbacks always `Err`.
    let pending_approvals = approval_interaction_service
        .list_pending(ListPendingApprovalsRequest {
            scope: scope.clone(),
            actor: actor.clone(),
        })
        .await
        .expect("real approval interaction service must answer Ok, not fail closed");
    assert!(pending_approvals.approvals.is_empty());

    let pending_auth = auth_interaction_service
        .list_pending(ListPendingAuthInteractionsRequest { scope, actor })
        .await
        .expect("real auth interaction service must answer Ok, not fail closed");
    assert!(pending_auth.auth_interactions.is_empty());

    runtime.shutdown().await.unwrap();
}

/// Delegates every `TurnCoordinator` method to `inner`, counting `resume_turn`
/// calls. Proves `local_dev_approval_interaction_service_for_test` actually
/// wires the *caller-supplied* coordinator into resolve/resume, not the
/// runtime's own (henrypark133 review, PR #5654).
struct SpyTurnCoordinator {
    inner: Arc<dyn TurnCoordinator>,
    resume_calls: AtomicUsize,
}

impl SpyTurnCoordinator {
    fn new(inner: Arc<dyn TurnCoordinator>) -> Self {
        Self {
            inner,
            resume_calls: AtomicUsize::new(0),
        }
    }

    fn resume_calls(&self) -> usize {
        self.resume_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TurnCoordinator for SpyTurnCoordinator {
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        self.inner.prepare_turn(scope).await
    }

    async fn abort_prepared_turn(&self, run_id: TurnRunId) -> Result<(), TurnError> {
        self.inner.abort_prepared_turn(run_id).await
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.inner.submit_turn(request).await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.resume_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.resume_turn(request).await
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        self.inner.retry_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.inner.cancel_run(request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.inner.get_run_state(request).await
    }
}

/// Scripted gateway that dispatches one `builtin.write_file` call (default
/// permission `ask`, so it parks the turn on `TurnStatus::BlockedApproval`
/// instead of completing), then emits a final reply once resumed.
#[derive(Default)]
struct SingleWriteApprovalGateway {
    call_count: std::sync::Mutex<usize>,
}

#[async_trait]
impl HostManagedModelGateway for SingleWriteApprovalGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "SingleWriteApprovalGateway requires the capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        _request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut count = self
                .call_count
                .lock()
                .expect("write gateway call lock poisoned");
            let index = *count;
            *count += 1;
            index
        };
        if call_index > 0 {
            return Ok(HostManagedModelResponse::assistant_reply(
                "wrote the coordinator-spy file".to_string(),
            ));
        }

        let write_id = CapabilityId::new("builtin.write_file").expect("write_file capability id");
        let write_tool = capabilities
            .tool_definitions()
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("tool_definitions failed: {err}"),
                )
            })?
            .into_iter()
            .find(|def| def.capability_id == write_id)
            .expect("builtin.write_file must be visible in local-dev capability surface");
        let call = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "coordinator-spy-provider".to_string(),
                provider_model_id: "coordinator-spy-model".to_string(),
                turn_id: Some("coordinator-spy-turn".to_string()),
                id: "coordinator-spy-write".to_string(),
                name: write_tool.name,
                arguments: json!({"path": "/workspace/coordinator-spy.txt", "content": "spy write"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("register_provider_tool_call(write_file) failed: {err}"),
                )
            })?;
        Ok(HostManagedModelResponse::capability_calls(vec![call], ""))
    }
}

// W5-WEBUI-API-2 follow-up (henrypark133 review): the smoke test above only
// calls `list_pending`, so it can't prove the caller-supplied `TurnCoordinator`
// is actually the one driving resolve/resume. Drive a real approval gate to
// `BlockedApproval` and resolve it through a service built with a *spy*
// coordinator wrapping the runtime's own — only the spy's `resume_turn` may
// fire.
#[tokio::test]
async fn local_dev_test_support_interaction_services_use_supplied_turn_coordinator_on_resolve() {
    let _guard = runtime_composition_test_guard().await;
    let root = tempfile::tempdir().unwrap();
    let tag = "coordinator-spy";
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(format!("{tag}-owner"), root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: format!("{tag}-tenant"),
        agent_id: format!("{tag}-agent"),
        source_binding_id: format!("{tag}-source"),
        reply_target_binding_id: format!("{tag}-reply"),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(5),
    })
    .with_model_gateway_override(Arc::new(SingleWriteApprovalGateway::default()));

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");

    // `AUTO_APPROVE_DEFAULT_ENABLED` is `true` for a never-configured user
    // (ironclaw_approvals::auto_approve), so a fresh runtime auto-dispatches
    // `write_filesystem` capabilities instead of gating them. Disable it here
    // so the scripted write actually parks on `BlockedApproval`.
    runtime
        .services()
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev exposes auto-approve settings for test")
        .set(AutoApproveSettingInput {
            updated_by: Principal::User(UserId::new(format!("{tag}-owner")).expect("user")),
            scope: ResourceScope {
                tenant_id: TenantId::new(format!("{tag}-tenant")).expect("tenant"),
                user_id: UserId::new(format!("{tag}-owner")).expect("user"),
                agent_id: Some(AgentId::new(format!("{tag}-agent")).expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            enabled: false,
        })
        .await
        .expect("disable auto-approve for the coordinator-spy scope");

    let inner_coordinator = runtime
        .services()
        .turn_coordinator
        .clone()
        .expect("local-dev runtime should wire a turn coordinator");
    let spy = Arc::new(SpyTurnCoordinator::new(inner_coordinator));
    let spy_dyn: Arc<dyn TurnCoordinator> = spy.clone();

    let approval_interaction_service = runtime
        .services()
        .local_dev_approval_interaction_service_for_test(spy_dyn)
        .expect("local-dev capability policy and grantee resolver should construct cleanly")
        .expect("local-dev runtime should support the approval interaction test accessor");

    let conversation = runtime.new_conversation().await.expect("conversation");
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.send_user_message_until_gate(&conversation, "write the coordinator spy file"),
    )
    .await
    .expect("send finishes")
    .expect("send should block on the approval gate");

    let (run_id, gate_ref) = match outcome {
        RebornTurnDriveOutcome::BlockedOnGate {
            run_id,
            status,
            gate_ref,
            ..
        } => {
            assert_eq!(
                status,
                TurnStatus::BlockedApproval,
                "expected the write to block on an approval gate"
            );
            (run_id, gate_ref)
        }
        RebornTurnDriveOutcome::Terminal(reply) => {
            panic!(
                "expected the write to block on an approval gate; got terminal reply: {reply:?}"
            );
        }
    };

    let scope = TurnScope::new_with_owner(
        TenantId::new(format!("{tag}-tenant")).expect("tenant id"),
        Some(AgentId::new(format!("{tag}-agent")).expect("agent id")),
        None,
        conversation.0.clone(),
        Some(UserId::new(format!("{tag}-owner")).expect("user id")),
    );
    let actor = TurnActor::new(UserId::new(format!("{tag}-owner")).expect("user id"));

    assert_eq!(spy.resume_calls(), 0, "resume must not fire before resolve");

    approval_interaction_service
        .resolve(ResolveApprovalInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new(format!("{tag}-resolve"))
                .expect("idempotency key"),
        })
        .await
        .expect("resolve should approve and resume the blocked run");

    assert_eq!(
        spy.resume_calls(),
        1,
        "the supplied (spy) coordinator, not the runtime's own, must be the one used to resume the run"
    );

    runtime.shutdown().await.unwrap();
}

fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}
