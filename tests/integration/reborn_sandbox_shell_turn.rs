//! Full-turn proof that the sandbox profile routes `builtin.shell` into Docker.

#[path = "support/docker_gate.rs"]
mod docker_gate;
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

#[allow(dead_code)]
#[path = "../../crates/lanes/ironclaw_sandbox/tests/support/user_sandbox_live.rs"]
mod user_sandbox_live;

use ironclaw_host_api::ids::InvocationId;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;
use user_sandbox_live::{
    CONTAINER_DIGEST_HEX_LEN, CONTAINER_PREFIX, ContainerIdentity, DockerCleanup, LABEL_TENANT,
    LABEL_USER, containers_for_identity,
};

const CONTAINER_MARKER: &str = "SANDBOX_SHELL_IN_CONTAINER";
const EPHEMERAL_MARKER: &str = "SANDBOX_CONTAINER_STATE_PERSISTED";
const PERSISTENCE_MARKER: &str = "SANDBOX_WORKSPACE_PERSISTED";
const LOOP_WORKER_MARKER: &str = "CANONICAL_LOOP_WORKER_ACTIVE";
const FILE_TOOL_TO_SHELL_MARKER: &str = "FILE_TOOL_TO_SHELL_VISIBLE";
const FILE_TOOL_TO_SHELL_PATCHED: &str = "FILE_TOOL_TO_SHELL_PATCHED";
const SHELL_TO_FILE_TOOL_MARKER: &str = "SHELL_TO_FILE_TOOL_VISIBLE";
const TRIGGER_IDLE_USER_MARKER: &str = "TRIGGER_STARTED_IDLE_USER_LOOP";

#[test]
fn sandbox_shell_turn_executes_in_a_real_container() {
    run_with_larger_stack(async {
        if !docker_gate::docker_available().await {
            eprintln!("SKIP: sandbox shell turn requires a Docker daemon");
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image).await {
            eprintln!("SKIP: sandbox worker image {image:?} is not built");
            return;
        }

        let mut cleanup = DockerCleanup::new();
        let harness = RebornIntegrationHarness::builder(format!(
            "sandbox-shell-{}",
            InvocationId::new()
        ))
        .with_sandbox_shell_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({
                    "path": "/workspace/coding-probe.txt",
                    "content": FILE_TOOL_TO_SHELL_MARKER,
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.apply_patch",
                json!({
                    "path": "/workspace/coding-probe.txt",
                    "old_string": FILE_TOOL_TO_SHELL_MARKER,
                    "new_string": FILE_TOOL_TO_SHELL_PATCHED,
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.shell",
                json!({
                    "command": format!(
                        "set -eu; test -f /.dockerenv; test \"$(cat /workspace/coding-probe.txt)\" = '{FILE_TOOL_TO_SHELL_PATCHED}'; echo {FILE_TOOL_TO_SHELL_MARKER}; printf '{SHELL_TO_FILE_TOOL_MARKER}' > /workspace/shell-created.txt; found=0; for exe in /proc/[0-9]*/exe; do target=$(readlink \"$exe\" 2>/dev/null || true); if [ \"$target\" = '/usr/local/bin/ironclaw-loop-worker' ]; then found=1; break; fi; done; test \"$found\" -eq 1; echo {LOOP_WORKER_MARKER}; printf '{PERSISTENCE_MARKER}' > /workspace/persistence-marker.txt; printf '{EPHEMERAL_MARKER}' > /tmp/container-marker.txt; cat /workspace/persistence-marker.txt /tmp/container-marker.txt; uid=$(id -u); test \"$uid\" -ne 0; echo NON_ROOT_UID_OK; echo {CONTAINER_MARKER}"
                    ),
                    "credential_contexts": [],
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.read_file",
                json!({
                    "path": "/workspace/shell-created.txt",
                }),
            ),
            RebornScriptedReply::text("ran coding tools in the sandbox"),
        ])
        .build()
        .await
        .expect("sandbox-shell harness builds");
        let expected_tenant = harness.binding.tenant_id.as_str().to_string();
        let expected_user = harness.binding.actor_user_id.as_str().to_string();
        let identity = ContainerIdentity {
            tenant: expected_tenant.clone(),
            user: expected_user.clone(),
        };
        cleanup.track_identity(identity.clone());
        assert!(
            containers_for_identity(&expected_tenant, &expected_user).is_empty(),
            "idle trigger owner must not have a sandbox before the fire"
        );
        let triggered = harness
            .submit_triggered_turn_scripted(
                "run the scheduled sandbox check",
                [
                    RebornScriptedReply::tool_call(
                        "builtin.shell",
                        json!({
                            "command": format!(
                                "set -eu; test -f /.dockerenv; found=0; for exe in /proc/[0-9]*/exe; do target=$(readlink \"$exe\" 2>/dev/null || true); if [ \"$target\" = '/usr/local/bin/ironclaw-loop-worker' ]; then found=1; break; fi; done; test \"$found\" -eq 1; echo {LOOP_WORKER_MARKER}; echo {TRIGGER_IDLE_USER_MARKER}"
                            ),
                            "credential_contexts": [],
                        }),
                    ),
                    RebornScriptedReply::text("scheduled sandbox check complete"),
                ],
            )
            .await
            .expect("triggered turn accepted");
        harness
            .wait_for_status_in_scope(
                &triggered.turn_scope,
                triggered.run_id,
                ironclaw_turns::TurnStatus::Completed,
            )
            .await
            .expect("triggered sandbox run completes");
        assert!(cleanup.capture_identity(&identity).running);
        harness
            .assert_tool_result_contains(TRIGGER_IDLE_USER_MARKER)
            .await
            .expect("trigger fire started the idle user's sandbox loop");
        harness
            .thread_harness
            .assert_final_reply(
                triggered.turn_scope.thread_id,
                "scheduled sandbox check complete",
            )
            .await
            .expect("triggered final reply persists in the trigger thread");

        harness
            .submit_turn("run a sandboxed shell command")
            .await
            .expect("turn completes");
        let container = cleanup.capture_identity(&identity);
        let digest = container
            .name
            .strip_prefix(CONTAINER_PREFIX)
            .expect("sandbox uses the stable user-container prefix");
        assert_eq!(digest.len(), CONTAINER_DIGEST_HEX_LEN);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(container.running);
        assert_eq!(
            container.labels.get(LABEL_TENANT).map(String::as_str),
            Some(expected_tenant.as_str()),
            "full-turn container carries its dispatch actor tenant identity"
        );
        assert_eq!(
            container.labels.get(LABEL_USER).map(String::as_str),
            Some(expected_user.as_str()),
            "full-turn container carries its dispatch actor user identity"
        );
        harness
            .assert_model_tools_contains("builtin__shell")
            .await
            .expect("shell is model-visible");
        for capability in [
            "builtin__write_file",
            "builtin__apply_patch",
            "builtin__read_file",
        ] {
            harness
                .assert_model_tools_contains(capability)
                .await
                .expect("coding tool is model-visible");
        }
        harness
            .assert_tool_invoked("builtin.shell")
            .await
            .expect("shell dispatches");
        for capability in [
            "builtin.write_file",
            "builtin.apply_patch",
            "builtin.read_file",
        ] {
            harness
                .assert_tool_invoked(capability)
                .await
                .expect("coding tool dispatches");
        }
        harness
            .assert_tool_result_contains(CONTAINER_MARKER)
            .await
            .expect("command ran in Docker");
        harness
            .assert_tool_result_contains(LOOP_WORKER_MARKER)
            .await
            .expect("canonical loop worker was active in the same user container");
        harness
            .assert_tool_result_contains(FILE_TOOL_TO_SHELL_MARKER)
            .await
            .expect("sandbox shell reads the file-tool workspace");
        harness
            .assert_tool_result_contains(SHELL_TO_FILE_TOOL_MARKER)
            .await
            .expect("file tools read the sandbox shell workspace");
        harness
            .assert_tool_result_contains("NON_ROOT_UID_OK")
            .await
            .expect("command ran as a non-root sandbox uid");
        harness
            .assert_tool_result_contains(PERSISTENCE_MARKER)
            .await
            .expect("sandbox workspace path is writable");
        harness
            .assert_tool_result_contains(EPHEMERAL_MARKER)
            .await
            .expect("sandbox temporary path is writable");
        harness
            .assert_reply_contains("ran coding tools in the sandbox")
            .await
            .expect("turn finalized");
    });
}

const PI_LOOP_WORKER_MARKER: &str = "PI_LOOP_WORKER_ACTIVE";

/// Pi loop-worker lane: the same sandbox membrane launches
/// `/usr/local/bin/ironclaw-pi-worker` (content-`Resolved` wire v2) instead of
/// the canonical Rust worker when
/// `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND=pi`. Gated on
/// `IRONCLAW_REQUIRE_DOCKER_TESTS` like the Rust lane above.
#[test]
fn sandbox_shell_turn_runs_the_pi_loop_worker_in_a_real_container() {
    run_with_larger_stack(async {
        if !docker_gate::docker_available().await {
            eprintln!("SKIP: pi loop-worker lane requires a Docker daemon");
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image).await {
            eprintln!("SKIP: sandbox worker image {image:?} is not built");
            return;
        }

        let mut cleanup = DockerCleanup::new();
        let harness = RebornIntegrationHarness::builder(format!(
            "sandbox-pi-worker-{}",
            InvocationId::new()
        ))
        .with_sandbox_shell_tools()
        .with_sandbox_loop_worker_kind(Default::default())
        .script([
            RebornScriptedReply::tool_call(
                "builtin.shell",
                json!({
                    "command": format!(
                        "set -eu; test -f /.dockerenv; found=0; for exe in /proc/[0-9]*/exe; do target=$(readlink \"$exe\" 2>/dev/null || true); if [ \"$target\" = '/usr/local/bin/ironclaw-pi-worker' ]; then found=1; break; fi; done; test \"$found\" -eq 1; echo {PI_LOOP_WORKER_MARKER}"
                    ),
                    "credential_contexts": [],
                }),
            ),
            RebornScriptedReply::text("pi loop worker finished the turn"),
        ])
        .build()
        .await
        .expect("pi-loop-worker harness builds");
        let identity = ContainerIdentity {
            tenant: harness.binding.tenant_id.as_str().to_string(),
            user: harness.binding.actor_user_id.as_str().to_string(),
        };
        cleanup.track_identity(identity);

        harness
            .submit_turn("run a turn driven by the pi loop worker")
            .await
            .expect("pi lane turn completes");
        harness
            .assert_tool_invoked("builtin.shell")
            .await
            .expect("shell dispatches in the pi lane");
        harness
            .assert_tool_result_contains(PI_LOOP_WORKER_MARKER)
            .await
            .expect("the pi loop worker process was active in the container");
        harness
            .assert_reply_contains("pi loop worker finished the turn")
            .await
            .expect("the pi-driven turn finalizes its reply");
    });
}

fn run_with_larger_stack<F>(test: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name("sandbox-shell-turn".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test);
        })
        .expect("spawn sandbox shell test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
