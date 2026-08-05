//! Full-turn proof that the sandbox profile routes `builtin.shell` into Docker.

#[path = "support/docker_gate.rs"]
mod docker_gate;
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const CONTAINER_MARKER: &str = "SANDBOX_SHELL_IN_CONTAINER";
const PERSISTENCE_MARKER: &str = "SANDBOX_WORKSPACE_PERSISTED";
const EGRESS_MARKER: &str = "SANDBOX_DIRECT_EGRESS_OK";

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

        let harness = RebornIntegrationHarness::test_default()
            .with_sandbox_shell_tools()
            .script([
                RebornScriptedReply::tool_call(
                    "builtin.shell",
                    json!({
                        "command": format!(
                            "test -f /.dockerenv && printf '{PERSISTENCE_MARKER}' > /workspace/persistence-marker.txt && python -c \"import urllib.request; response = urllib.request.urlopen('https://example.com', timeout=15); assert response.status == 200; response.close()\" && echo {CONTAINER_MARKER} {EGRESS_MARKER}"
                        )
                    }),
                ),
                RebornScriptedReply::tool_call(
                    "builtin.shell",
                    json!({"command": "cat /workspace/persistence-marker.txt; uid=$(id -u); test \"$uid\" -ne 0; echo NON_ROOT_UID_OK"}),
                ),
                RebornScriptedReply::text("ran in the sandbox"),
            ])
            .build()
            .await
            .expect("sandbox-shell harness builds");

        harness
            .submit_turn("run a sandboxed shell command")
            .await
            .expect("turn completes");
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
            .assert_tool_result_contains(EGRESS_MARKER)
            .await
            .expect("sandbox profile enabled direct HTTPS egress");
        harness
            .assert_tool_result_contains("NON_ROOT_UID_OK")
            .await
            .expect("command ran as a non-root sandbox uid");
        harness
            .assert_tool_result_contains(PERSISTENCE_MARKER)
            .await
            .expect("workspace persisted across shell calls");
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
