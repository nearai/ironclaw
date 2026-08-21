//! Shared Docker lifecycle helpers for the local user-sandbox acceptance suites.

use std::{
    collections::{HashMap, HashSet},
    process::Command,
    time::{Duration, Instant},
};

use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId},
    process::CommandExecutionRequest,
    resource::ResourceScope,
};

pub(crate) use ironclaw_sandbox::sandbox_process::{
    USER_SANDBOX_CONTAINER_DIGEST_HEX_LEN as CONTAINER_DIGEST_HEX_LEN,
    USER_SANDBOX_CONTAINER_NAME_PREFIX as CONTAINER_PREFIX,
    USER_SANDBOX_LABEL_IMAGE as LABEL_IMAGE,
    USER_SANDBOX_LABEL_SECURITY_POSTURE as LABEL_SECURITY_POSTURE,
    USER_SANDBOX_LABEL_TENANT as LABEL_TENANT, USER_SANDBOX_LABEL_USER as LABEL_USER,
    USER_SANDBOX_NETWORK_LABEL_TENANT as NETWORK_LABEL_TENANT,
    USER_SANDBOX_NETWORK_LABEL_USER as NETWORK_LABEL_USER,
    USER_SANDBOX_PROXY_LABEL_TENANT as PROXY_LABEL_TENANT,
    USER_SANDBOX_PROXY_LABEL_USER as PROXY_LABEL_USER,
};

pub(crate) fn scope(tenant: &str, user: &str, project: &str, thread: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).expect("tenant id"),
        user_id: UserId::new(user).expect("user id"),
        agent_id: Some(AgentId::new("docker-live-agent").expect("agent id")),
        project_id: Some(ProjectId::new(project).expect("project id")),
        mission_id: None,
        thread_id: Some(ThreadId::new(thread).expect("thread id")),
        invocation_id: InvocationId::new(),
    }
}

pub(crate) fn request(scope: ResourceScope, command: impl Into<String>) -> CommandExecutionRequest {
    CommandExecutionRequest {
        scope,
        mounts: None,
        command: command.into(),
        workdir: Some("/workspace".to_string()),
        timeout_secs: Some(60),
        extra_env: HashMap::new(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TestScope {
    pub(crate) tenant: String,
    pub(crate) user: String,
    pub(crate) project: String,
    pub(crate) thread: String,
}

impl TestScope {
    pub(crate) fn unique(label: &str) -> Self {
        let suffix = InvocationId::new();
        Self {
            tenant: format!("{label}-tenant-{suffix}"),
            user: format!("{label}-user-{suffix}"),
            project: format!("{label}-project-{suffix}"),
            thread: format!("{label}-thread-{suffix}"),
        }
    }

    pub(crate) fn resource_scope(&self) -> ResourceScope {
        scope(&self.tenant, &self.user, &self.project, &self.thread)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ContainerIdentity {
    pub(crate) tenant: String,
    pub(crate) user: String,
}

impl From<&TestScope> for ContainerIdentity {
    fn from(scope: &TestScope) -> Self {
        Self {
            tenant: scope.tenant.clone(),
            user: scope.user.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ContainerSnapshot {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) hostname: String,
    pub(crate) image: String,
    pub(crate) running: bool,
    pub(crate) labels: HashMap<String, String>,
}

#[derive(Default)]
pub(crate) struct DockerCleanup {
    identities: Vec<ContainerIdentity>,
    pub(crate) container_ids: HashSet<String>,
    image_tags: Vec<String>,
    network_ids: HashSet<String>,
}

impl DockerCleanup {
    #[allow(dead_code)] // used by the root full-turn integration consumer
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_scopes(scopes: impl IntoIterator<Item = TestScope>) -> Self {
        Self {
            identities: scopes
                .into_iter()
                .map(|scope| ContainerIdentity::from(&scope))
                .collect(),
            container_ids: HashSet::new(),
            image_tags: Vec::new(),
            network_ids: HashSet::new(),
        }
    }

    #[allow(dead_code)] // used by the root full-turn integration consumer
    pub(crate) fn track_identity(&mut self, identity: ContainerIdentity) {
        self.identities.push(identity);
    }

    pub(crate) fn capture(&mut self, scope: &TestScope) -> ContainerSnapshot {
        self.capture_identity(&ContainerIdentity::from(scope))
    }

    pub(crate) fn capture_identity(&mut self, identity: &ContainerIdentity) -> ContainerSnapshot {
        let matches = containers_for_identity(&identity.tenant, &identity.user);
        self.container_ids
            .extend(matches.iter().map(|container| container.id.clone()));
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one container for tenant {:?} and user {:?}, found {matches:?}",
            identity.tenant,
            identity.user,
        );
        matches.into_iter().next().expect("length checked")
    }

    pub(crate) fn track_image(&mut self, image: String) {
        self.image_tags.push(image);
    }
}

impl Drop for DockerCleanup {
    fn drop(&mut self) {
        for identity in &self.identities {
            self.container_ids.extend(docker_resource_ids(
                &["container", "list", "--all"],
                LABEL_TENANT,
                LABEL_USER,
                identity,
            ));
            self.container_ids.extend(docker_resource_ids(
                &["container", "list", "--all"],
                PROXY_LABEL_TENANT,
                PROXY_LABEL_USER,
                identity,
            ));
            self.network_ids.extend(docker_resource_ids(
                &["network", "list"],
                NETWORK_LABEL_TENANT,
                NETWORK_LABEL_USER,
                identity,
            ));
        }
        for id in &self.container_ids {
            let _ = Command::new("docker")
                .args(["container", "rm", "--force", id])
                .output();
        }
        for id in &self.network_ids {
            let _ = Command::new("docker").args(["network", "rm", id]).output();
        }
        for image in &self.image_tags {
            let _ = Command::new("docker")
                .args(["image", "rm", "--force", image])
                .output();
        }
    }
}

fn docker_resource_ids(
    resource_args: &[&str],
    tenant_label: &str,
    user_label: &str,
    identity: &ContainerIdentity,
) -> Vec<String> {
    let tenant_filter = format!("label={tenant_label}={}", identity.tenant);
    let user_filter = format!("label={user_label}={}", identity.user);
    let Ok(output) = Command::new("docker")
        .args(resource_args)
        .args([
            "--quiet",
            "--filter",
            tenant_filter.as_str(),
            "--filter",
            user_filter.as_str(),
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

pub(crate) fn docker_command(args: &[&str]) -> std::process::Output {
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

pub(crate) fn containers_for_user(scope: &TestScope) -> Vec<ContainerSnapshot> {
    containers_for_identity(&scope.tenant, &scope.user)
}

pub(crate) fn containers_for_identity(tenant: &str, user: &str) -> Vec<ContainerSnapshot> {
    let tenant_filter = format!("label={LABEL_TENANT}={tenant}");
    let user_filter = format!("label={LABEL_USER}={user}");
    let output = Command::new("docker")
        .args(["container", "list", "--all", "--filter"])
        .arg(&tenant_filter)
        .arg("--filter")
        .arg(&user_filter)
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

pub(crate) fn inspect_container(id: &str) -> ContainerSnapshot {
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
        hostname: container["Config"]["Hostname"]
            .as_str()
            .expect("container has a hostname")
            .to_string(),
        image: container["Config"]["Image"]
            .as_str()
            .expect("container has an image")
            .to_string(),
        running: container["State"]["Running"]
            .as_bool()
            .expect("container running state is boolean"),
        labels,
    }
}

pub(crate) fn assert_stable_identity(container: &ContainerSnapshot, scope: &TestScope) {
    let suffix = container
        .name
        .strip_prefix(CONTAINER_PREFIX)
        .unwrap_or_else(|| panic!("unexpected sandbox container name: {}", container.name));
    assert_eq!(
        suffix.len(),
        CONTAINER_DIGEST_HEX_LEN,
        "stable name uses a 96-bit hex digest"
    );
    assert!(
        suffix.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "stable name digest is hexadecimal: {}",
        container.name
    );
    assert_eq!(container.labels.get(LABEL_TENANT), Some(&scope.tenant));
    assert_eq!(container.labels.get(LABEL_USER), Some(&scope.user));
    assert!(
        container
            .labels
            .get(LABEL_IMAGE)
            .is_some_and(|value| !value.is_empty()),
        "container records image identity"
    );
    assert!(
        container
            .labels
            .get(LABEL_SECURITY_POSTURE)
            .is_some_and(|value| !value.is_empty()),
        "container records the security-posture stamp"
    );
}

pub(crate) async fn wait_for_running_state(
    id: &str,
    running: bool,
    timeout: Duration,
) -> ContainerSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = inspect_container(id);
        if snapshot.running == running {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "container {id} did not reach running={running} within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn wait_for_container_absent(id: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let id_filter = format!("id={id}");
        let output = tokio::process::Command::new("docker")
            .args(["container", "list", "--all", "--quiet", "--filter"])
            .arg(&id_filter)
            .output()
            .await
            .expect("docker container list starts");
        assert!(
            output.status.success(),
            "docker container list failed while waiting for {id} removal: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "container {id} was not reclaimed within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn wait_for_user_container(
    scope: &TestScope,
    timeout: Duration,
) -> ContainerSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let mut matches = containers_for_user(scope);
        assert!(
            matches.len() <= 1,
            "expected at most one container for tenant {:?} and user {:?}, found {matches:?}",
            scope.tenant,
            scope.user,
        );
        if let Some(container) = matches.pop() {
            return container;
        }
        assert!(
            Instant::now() < deadline,
            "user container did not appear within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn wait_for_container_file(id: &str, path: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let output = Command::new("docker")
            .args(["container", "exec", id, "test", "-e", path])
            .output()
            .expect("docker container exec starts");
        if output.status.success() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "container {id} did not create {path:?} within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn docker_visible_tempdir() -> tempfile::TempDir {
    // Colima only bind-mounts configured host roots into its Docker VM.
    tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("Docker-visible sandbox workspace tempdir")
}
