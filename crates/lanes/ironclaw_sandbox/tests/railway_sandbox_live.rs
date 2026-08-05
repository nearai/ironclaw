//! Manual authenticated canary for Railway sandbox persistence and isolation.
//!
//! Run explicitly with one Railway token plus project configuration:
//! `cargo test -p ironclaw_sandbox --test railway_sandbox_live -- --ignored --nocapture`.

use std::{collections::HashMap, process::Command};

use ironclaw_host_api::{
    ids::{AgentId, InvocationId, TenantId, UserId},
    process::{CommandExecutionRequest, SandboxCommandTransport},
    resource::ResourceScope,
};
use ironclaw_sandbox::{
    RailwayPreviewSandboxConfig, RailwayPreviewSandboxTransport, RebornSandboxUserKey,
};

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for the Railway canary"))
}

fn request(scope: &ResourceScope, command: impl Into<String>) -> CommandExecutionRequest {
    CommandExecutionRequest {
        scope: scope.clone(),
        mounts: None,
        command: command.into(),
        workdir: Some("/workspace".to_string()),
        timeout_secs: Some(300),
        extra_env: HashMap::new(),
    }
}

fn checkpoint_name(scope: &ResourceScope) -> String {
    format!(
        "{}-checkpoint",
        RebornSandboxUserKey::from_scope(scope).container_name()
    )
}

struct CheckpointCleanup {
    cli: String,
    project_id: String,
    environment_id: String,
    names: Vec<String>,
}

impl Drop for CheckpointCleanup {
    fn drop(&mut self) {
        for name in &self.names {
            let status = Command::new(&self.cli)
                .args([
                    "sandbox",
                    "checkpoint",
                    "delete",
                    name,
                    "--project",
                    &self.project_id,
                    "--environment",
                    &self.environment_id,
                ])
                .status();
            if !matches!(status, Ok(status) if status.success()) {
                eprintln!("warning: failed to delete Railway canary checkpoint {name}");
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Railway auth and creates billable preview sandbox resources"]
async fn railway_workspace_survives_transport_restart_without_credentials() {
    let project_id = required_env("IRONCLAW_REBORN_RAILWAY_PROJECT_ID");
    let environment_id = required_env("IRONCLAW_REBORN_RAILWAY_ENVIRONMENT_ID");
    let has_project_token = std::env::var_os("RAILWAY_TOKEN").is_some();
    let has_api_token = std::env::var_os("RAILWAY_API_TOKEN").is_some();
    assert_ne!(
        has_project_token, has_api_token,
        "exactly one of RAILWAY_TOKEN or RAILWAY_API_TOKEN is required"
    );

    let cli =
        std::env::var("IRONCLAW_REBORN_RAILWAY_CLI_PATH").unwrap_or_else(|_| "railway".to_string());
    let config = RailwayPreviewSandboxConfig::new(project_id.clone(), environment_id.clone())
        .expect("Railway canary configuration is valid")
        .with_cli_path(cli.clone())
        .with_idle_timeout_minutes(5)
        .expect("canary idle timeout is valid");
    let unique = InvocationId::new().to_string();
    let scope = ResourceScope {
        tenant_id: TenantId::new(format!("railway-canary-tenant-{unique}"))
            .expect("generated tenant id is valid"),
        user_id: UserId::new(format!("railway-canary-user-{unique}"))
            .expect("generated user id is valid"),
        agent_id: Some(AgentId::new("railway-canary").expect("static agent id is valid")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let scope_b = ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: UserId::new(format!("railway-canary-user-b-{unique}"))
            .expect("generated user id is valid"),
        agent_id: scope.agent_id.clone(),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let _cleanup = CheckpointCleanup {
        cli,
        project_id,
        environment_id,
        names: vec![checkpoint_name(&scope), checkpoint_name(&scope_b)],
    };
    let marker = format!("ironclaw-railway-persistence-{unique}");
    let replacement = format!("ironclaw-railway-persistence-newest-{unique}");
    let marker_b = format!("ironclaw-railway-isolation-b-{unique}");

    let first = RailwayPreviewSandboxTransport::new(config.clone());
    let write = first
        .run_command(request(
            &scope,
            format!(
                "python - <<'PY'\n\
                 import os\n\
                 from pathlib import Path\n\
                 assert os.getuid() == 1000\n\
                 assert Path('/.dockerenv').is_file()\n\
                 assert not Path('/var/run/docker.sock').exists()\n\
                 root = next(line.split() for line in Path('/proc/mounts').read_text().splitlines() if line.split()[1] == '/')\n\
                 assert 'ro' in root[3].split(',')\n\
                 # Validate the inner Docker worker's --network none posture;\n\
                 # Railway's outer ISOLATED sandbox may still have NAT egress.\n\
                 routes = [line.split() for line in Path('/proc/net/route').read_text().splitlines()[1:]]\n\
                 assert not any(route[1] == '00000000' for route in routes)\n\
                 Path('/tmp/ironclaw-write-probe').write_text('tmpfs-ok')\n\
                 Path('state.txt').write_text('{marker}')\n\
                 print('IRONCLAW_RAILWAY_SANDBOX_ISOLATION_OK')\n\
                 PY"
            ),
        ))
        .await
        .expect("Railway worker writes persistent state");
    assert_eq!(write.exit_code, 0, "write failed: {}", write.output);
    assert!(
        write
            .output
            .contains("IRONCLAW_RAILWAY_SANDBOX_ISOLATION_OK")
    );
    assert!(write.sandboxed);

    let write_b = first
        .run_command(request(
            &scope_b,
            format!(
                "python - <<'PY'\n\
                 from pathlib import Path\n\
                 assert not Path('state.txt').exists()\n\
                 Path('state.txt').write_text('{marker_b}')\n\
                 print('IRONCLAW_RAILWAY_SECOND_USER_ISOLATED')\n\
                 PY"
            ),
        ))
        .await
        .expect("second user receives an isolated Railway workspace");
    assert_eq!(write_b.exit_code, 0, "write failed: {}", write_b.output);
    drop(first);

    let restarted = RailwayPreviewSandboxTransport::new(config.clone());
    let read = restarted
        .run_command(request(
            &scope,
            "python -c 'from pathlib import Path; print(Path.cwd()); print(Path(\"state.txt\").read_text())' && env",
        ))
        .await
        .expect("Railway worker restores checkpoint after transport restart");
    assert_eq!(read.exit_code, 0, "read failed: {}", read.output);
    assert!(read.output.contains("/workspace"));
    assert!(read.output.contains(&marker));
    for forbidden in [
        "RAILWAY_TOKEN=",
        "RAILWAY_API_TOKEN=",
        "NEARAI_API_KEY=",
        "OPENAI_API_KEY=",
        "ANTHROPIC_API_KEY=",
    ] {
        assert!(
            !read.output.contains(forbidden),
            "worker leaked {forbidden}"
        );
    }

    let read_b = restarted
        .run_command(request(
            &scope_b,
            "python -c 'from pathlib import Path; print(Path(\"state.txt\").read_text())'",
        ))
        .await
        .expect("second user checkpoint restores independently");
    assert_eq!(read_b.exit_code, 0, "read failed: {}", read_b.output);
    assert!(read_b.output.contains(&marker_b));
    assert!(!read_b.output.contains(&marker));

    let replace = restarted
        .run_command(request(
            &scope,
            format!(
                "python -c 'from pathlib import Path; Path(\"state.txt\").write_text(\"{replacement}\"); print(\"CHECKPOINT_REPLACED\")'"
            ),
        ))
        .await
        .expect("first user's deterministic checkpoint is updated");
    assert_eq!(replace.exit_code, 0, "write failed: {}", replace.output);
    drop(restarted);

    let restarted_again = RailwayPreviewSandboxTransport::new(config);
    let newest = restarted_again
        .run_command(request(
            &scope,
            "python -c 'from pathlib import Path; print(Path(\"state.txt\").read_text())'",
        ))
        .await
        .expect("newest checkpoint restores after another transport restart");
    assert_eq!(newest.exit_code, 0, "read failed: {}", newest.output);
    assert!(newest.output.contains(&replacement));
    assert!(!newest.output.contains(&marker));

    let still_isolated = restarted_again
        .run_command(request(
            &scope_b,
            "python -c 'from pathlib import Path; print(Path(\"state.txt\").read_text())'",
        ))
        .await
        .expect("second user's checkpoint remains independent");
    assert_eq!(still_isolated.exit_code, 0);
    assert!(still_isolated.output.contains(&marker_b));
    assert!(!still_isolated.output.contains(&replacement));
}
