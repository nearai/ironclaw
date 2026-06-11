use std::time::Duration;

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
    assert!(reason.contains("profile=disabled is not yet wired end-to-end"));
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
        heartbeat_interval: Duration::from_millis(25),
        poll_interval: Duration::from_secs(60),
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
    let root = tempfile::tempdir().unwrap();
    let storage_root = root.path().join("local-dev");
    let skill_root = storage_root
        .join("tenants/runtime-skill-execution-tenant/users/runtime-skill-execution-owner/skills/filesystem-review");
    std::fs::create_dir_all(skill_root.join("references")).unwrap();
    std::fs::write(
        skill_root.join("SKILL.md"),
        skill_md(
            "filesystem-review",
            "filesystem-review",
            "Use filesystem-backed review guidance.",
        ),
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
        runtime.execute_skill_message(&conversation, "$filesystem-review"),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(result.plan.activations().len(), 1);
    assert_eq!(result.plan.activations()[0].name, "filesystem-review");
    assert_eq!(result.plan.active_bundles().len(), 1);
    assert_eq!(
        result.plan.active_bundles()[0].source,
        RebornSkillSourceKind::User
    );
    assert_eq!(
        result.plan.active_bundles()[0].skill_name,
        "filesystem-review"
    );

    let asset = runtime
        .read_skill_execution_asset(
            &conversation,
            &result.plan,
            &result.plan.activations()[0],
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
