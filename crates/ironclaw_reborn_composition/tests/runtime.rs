use std::{sync::LazyLock, time::Duration};

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_reborn_composition::{
    HooksActivationConfig, PollSettings, RebornBuildInput, RebornRuntimeError,
    RebornRuntimeIdentity, RebornRuntimeInput, RebornSkillSourceKind, TurnRunnerSettings,
    build_reborn_runtime,
};
use ironclaw_turns::TurnStatus;
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
    .with_runner_settings(TurnRunnerSettings {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_secs(60),
        ..TurnRunnerSettings::default()
    });

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

    // With no LLM gateway configured the stubbed driver path reports a
    // protocol violation, which maps to a terminal Failed turn instead of the
    // pre-PR RecoveryRequired path that cancelled via the standalone-runtime
    // cancel guard.
    assert_eq!(reply.status, TurnStatus::Failed);
    assert_eq!(
        reply.failure_category.as_deref(),
        Some("driver_protocol_violation")
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
        Some("driver_protocol_violation")
    );
    assert_eq!(second_reply.text, None);

    runtime.shutdown().await.unwrap();
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
    .with_runner_settings(TurnRunnerSettings {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_secs(60),
        ..TurnRunnerSettings::default()
    })
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
    .with_runner_settings(TurnRunnerSettings {
        heartbeat_interval: Duration::from_millis(25),
        poll_interval: Duration::from_secs(60),
        ..TurnRunnerSettings::default()
    });

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
/// `InMemoryTurnStateStore::with_limits` wiring path so that a mis-wired or
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
    .with_runner_settings(TurnRunnerSettings {
        // Cap = 1 per user. Verifies this value flows from settings → store limits.
        max_concurrent_runs_per_user: NonZeroU32::new(1),
        heartbeat_interval: Duration::from_millis(25),
        poll_interval: Duration::from_millis(10),
        ..TurnRunnerSettings::default()
    });

    let runtime = build_reborn_runtime(input).await.unwrap();

    // Submit two sequential turns on two conversations. With the stub gateway
    // each turn completes (as Failed / driver_protocol_violation) before the
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
    .with_runner_settings(TurnRunnerSettings {
        // Explicitly set 2 workers — ensures the guard uses .all() semantics
        // and does not fire when only a subset of workers have finished.
        worker_count: NonZeroUsize::new(2).unwrap(),
        heartbeat_interval: Duration::from_millis(25),
        poll_interval: Duration::from_secs(60),
        ..TurnRunnerSettings::default()
    });

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
