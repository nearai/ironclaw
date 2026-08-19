//! Full-turn proof that the sandbox profile routes `builtin.shell` into Docker.

#[path = "support/docker_gate.rs"]
mod docker_gate;
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::{
    collections::{HashMap, HashSet},
    process::Command,
};

use ironclaw_host_api::ids::InvocationId;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const CONTAINER_PREFIX: &str = "ironclaw-reborn-sandbox-user-";
const LABEL_TENANT: &str = "ironclaw.tenant";
const LABEL_USER: &str = "ironclaw.user";
const CONTAINER_MARKER: &str = "SANDBOX_SHELL_IN_CONTAINER";
const EPHEMERAL_MARKER: &str = "SANDBOX_CONTAINER_STATE_PERSISTED";
const PERSISTENCE_MARKER: &str = "SANDBOX_WORKSPACE_PERSISTED";

#[derive(Clone)]
struct ContainerIdentity {
    tenant: String,
    user: String,
}

struct DockerCleanup {
    identity: Option<ContainerIdentity>,
    container_ids: HashSet<String>,
}

impl DockerCleanup {
    fn new() -> Self {
        Self {
            identity: None,
            container_ids: HashSet::new(),
        }
    }

    fn bind(&mut self, identity: ContainerIdentity) {
        self.identity = Some(identity);
    }

    fn capture(&mut self) -> ContainerSnapshot {
        let identity = self
            .identity
            .as_ref()
            .expect("Docker cleanup is bound to the harness identity");
        let matches = containers_matching_labels(&[
            (LABEL_TENANT, &identity.tenant),
            (LABEL_USER, &identity.user),
        ]);
        self.container_ids
            .extend(matches.iter().map(|container| container.id.clone()));
        assert_eq!(
            matches.len(),
            1,
            "full-turn sandbox must leave exactly one stable user container"
        );
        matches.into_iter().next().expect("length checked")
    }
}

impl Drop for DockerCleanup {
    fn drop(&mut self) {
        let Some(identity) = &self.identity else {
            return;
        };
        let tenant = format!("{LABEL_TENANT}={}", identity.tenant);
        let user = format!("{LABEL_USER}={}", identity.user);
        let tenant_filter = format!("label={tenant}");
        let user_filter = format!("label={user}");
        if let Ok(output) = Command::new("docker")
            .args([
                "container",
                "list",
                "--all",
                "--quiet",
                "--filter",
                tenant_filter.as_str(),
                "--filter",
                user_filter.as_str(),
            ])
            .output()
        {
            if output.status.success() {
                self.container_ids.extend(
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::to_string),
                );
            }
        }
        for id in &self.container_ids {
            let _ = Command::new("docker")
                .args(["container", "rm", "--force", id])
                .output();
        }
    }
}

struct ContainerSnapshot {
    id: String,
    name: String,
    running: bool,
    labels: HashMap<String, String>,
}

fn containers_matching_labels(labels: &[(&str, &str)]) -> Vec<ContainerSnapshot> {
    let mut command = Command::new("docker");
    command.args(["container", "list", "--all"]);
    for (key, value) in labels {
        command.arg("--filter").arg(format!("label={key}={value}"));
    }
    let output = command
        .args(["--format", "{{.ID}}"])
        .output()
        .expect("docker container list starts");
    assert!(
        output.status.success(),
        "docker container list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|id| inspect_container(id.trim()))
        .collect()
}

fn inspect_container(id: &str) -> ContainerSnapshot {
    let output = docker_command(&["container", "inspect", id]);
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("docker inspect returns JSON");
    let container = value
        .as_array()
        .and_then(|containers| containers.first())
        .expect("docker inspect returns one container");
    let labels = container["Config"]["Labels"]
        .as_object()
        .expect("sandbox container has labels")
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("label {key:?} is not a string"))
                    .to_string(),
            )
        })
        .collect();
    ContainerSnapshot {
        id: container["Id"]
            .as_str()
            .expect("container has an id")
            .to_string(),
        name: container["Name"]
            .as_str()
            .expect("container has a name")
            .trim_start_matches('/')
            .to_string(),
        running: container["State"]["Running"]
            .as_bool()
            .expect("container running state is boolean"),
        labels,
    }
}

fn docker_command(args: &[&str]) -> std::process::Output {
    let output = Command::new("docker")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("docker {args:?} could not start: {error}"));
    assert!(
        output.status.success(),
        "docker {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

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
                        "test -f /.dockerenv && printf '{PERSISTENCE_MARKER}' > /workspace/persistence-marker.txt && printf '{EPHEMERAL_MARKER}' > /tmp/container-marker.txt && hostname > /tmp/container-hostname.txt && echo {CONTAINER_MARKER}"
                    )
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.shell",
                json!({"command": "test \"$(cat /tmp/container-hostname.txt)\" = \"$(hostname)\" && cat /workspace/persistence-marker.txt /tmp/container-marker.txt && uid=$(id -u) && test \"$uid\" -ne 0 && echo NON_ROOT_UID_OK"}),
            ),
            RebornScriptedReply::text("ran in the sandbox"),
        ])
        .build()
        .await
        .expect("sandbox-shell harness builds");
        let expected_tenant = harness.binding.tenant_id.as_str().to_string();
        let expected_user = harness.binding.actor_user_id.as_str().to_string();
        cleanup.bind(ContainerIdentity {
            tenant: expected_tenant.clone(),
            user: expected_user.clone(),
        });

        harness
            .submit_turn("run two sandboxed shell commands")
            .await
            .expect("turn completes");
        let container = cleanup.capture();
        let digest = container
            .name
            .strip_prefix(CONTAINER_PREFIX)
            .expect("sandbox uses the stable user-container prefix");
        assert_eq!(digest.len(), 24);
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
            .expect("workspace persisted across shell calls");
        harness
            .assert_tool_result_contains(EPHEMERAL_MARKER)
            .await
            .expect("container-local state persisted across shell calls");
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
