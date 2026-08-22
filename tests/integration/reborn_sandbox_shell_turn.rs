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
    LABEL_USER,
};

const CONTAINER_MARKER: &str = "SANDBOX_SHELL_IN_CONTAINER";
const EPHEMERAL_MARKER: &str = "SANDBOX_CONTAINER_STATE_PERSISTED";
const PERSISTENCE_MARKER: &str = "SANDBOX_WORKSPACE_PERSISTED";

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
                "builtin.shell",
                json!({
                    "command": format!(
                        "test -f /.dockerenv && printf '{PERSISTENCE_MARKER}' > /workspace/persistence-marker.txt && printf '{EPHEMERAL_MARKER}' > /tmp/container-marker.txt && cat /workspace/persistence-marker.txt /tmp/container-marker.txt && uid=$(id -u) && test \"$uid\" -ne 0 && echo NON_ROOT_UID_OK && echo {CONTAINER_MARKER}"
                    )
                }),
            ),
            RebornScriptedReply::text("ran in the sandbox"),
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
        harness
            .assert_tool_invoked("builtin.shell")
            .await
            .expect("shell dispatches");
        harness
            .assert_tool_result_contains(CONTAINER_MARKER)
            .await
            .expect("command ran in Docker");
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
            .assert_reply_contains("ran in the sandbox")
            .await
            .expect("turn finalized");
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
