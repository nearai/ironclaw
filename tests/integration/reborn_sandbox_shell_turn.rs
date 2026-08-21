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

use ironclaw_host_api::ids::{InvocationId, TenantId, TenantUserWorkspaceKey, UserId};
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;
use user_sandbox_live::{
    CONTAINER_DIGEST_HEX_LEN, CONTAINER_PREFIX, ContainerIdentity, DockerCleanup, LABEL_TENANT,
    LABEL_USER,
};

const CONTAINER_MARKER: &str = "SANDBOX_SHELL_IN_CONTAINER";
const EPHEMERAL_MARKER: &str = "SANDBOX_CONTAINER_STATE_PERSISTED";
const LEAF_ONLY_MARKER: &str = "SANDBOX_CANONICAL_LEAF_ONLY";
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
                        r#"python - <<'PY'
import os
from pathlib import Path

assert Path('/.dockerenv').is_file()
assert Path('/workspace/selected-leaf-sentinel.txt').read_text() == 'host-selected-leaf'
for relative in [
    'reborn-home-sentinel.txt',
    'state/reborn-state-sentinel.txt',
    'state/.reborn-secrets-master-key',
    'state/provider-credential-sentinel.txt',
    'system/system-sentinel.txt',
    'users',
]:
    assert not (Path('/workspace') / relative).exists(), relative
forbidden_env = [
    'IRONCLAW_REBORN_HOME', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY',
    'NEARAI_API_KEY', 'GITHUB_TOKEN', 'GH_TOKEN', 'RAILWAY_TOKEN',
    'RAILWAY_API_TOKEN', 'AWS_ACCESS_KEY_ID', 'AWS_SECRET_ACCESS_KEY',
]
assert all(name not in os.environ for name in forbidden_env)
Path('/workspace/container-write.txt').write_text('container-owned-leaf')
Path('/workspace/persistence-marker.txt').write_text('{PERSISTENCE_MARKER}')
Path('/tmp/container-marker.txt').write_text('{EPHEMERAL_MARKER}')
print('{LEAF_ONLY_MARKER}')
print('{CONTAINER_MARKER}')
PY"#
                    )
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.shell",
                json!({
                    "command": "cat /workspace/persistence-marker.txt /tmp/container-marker.txt && uid=$(id -u) && test \"$uid\" -ne 0 && echo NON_ROOT_UID_OK"
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

        let installation_home = harness.installation_home();
        let caller_key =
            TenantUserWorkspaceKey::from_scope(&harness.turn_scope.to_resource_scope());
        let sibling_key = TenantUserWorkspaceKey::from_tenant_user(
            &TenantId::new(&expected_tenant).expect("sandbox tenant id"),
            &UserId::new(format!("sandbox-sibling-{}", InvocationId::new()))
                .expect("sandbox sibling id"),
        );
        let selected_leaf = installation_home
            .join("workspaces")
            .join("users")
            .join(caller_key.digest_segment());
        let sibling_leaf = installation_home
            .join("workspaces")
            .join("users")
            .join(sibling_key.digest_segment());
        std::fs::create_dir_all(&selected_leaf).expect("selected workspace leaf");
        std::fs::create_dir_all(&sibling_leaf).expect("sibling workspace leaf");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let workspace_root = installation_home.join("workspaces");
            for private_namespace in [&workspace_root, &workspace_root.join("users")] {
                std::fs::set_permissions(private_namespace, std::fs::Permissions::from_mode(0o700))
                    .expect("private workspace namespace");
            }
        }
        std::fs::create_dir_all(installation_home.join("state")).expect("canonical state root");
        std::fs::create_dir_all(installation_home.join("system")).expect("canonical system root");
        for (path, value) in [
            (
                selected_leaf.join("selected-leaf-sentinel.txt"),
                "host-selected-leaf",
            ),
            (
                sibling_leaf.join("sibling-sentinel.txt"),
                "host-sibling-only",
            ),
            (
                installation_home.join("reborn-home-sentinel.txt"),
                "host-reborn-home-only",
            ),
            (
                installation_home.join("state/reborn-state-sentinel.txt"),
                "host-state-only",
            ),
            (
                installation_home.join("state/.reborn-secrets-master-key"),
                "host-master-key-only",
            ),
            (
                installation_home.join("state/provider-credential-sentinel.txt"),
                "host-provider-credential-only",
            ),
            (
                installation_home.join("system/system-sentinel.txt"),
                "host-system-only",
            ),
        ] {
            std::fs::write(path, value).expect("sandbox isolation sentinel");
        }

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
            .assert_tool_result_contains(LEAF_ONLY_MARKER)
            .await
            .expect("only the canonical caller leaf was visible");
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
        assert_eq!(
            std::fs::read_to_string(selected_leaf.join("container-write.txt"))
                .expect("container write remains in selected leaf"),
            "container-owned-leaf"
        );
        assert_eq!(
            std::fs::read_to_string(sibling_leaf.join("sibling-sentinel.txt"))
                .expect("sibling sentinel remains host-only"),
            "host-sibling-only"
        );
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
