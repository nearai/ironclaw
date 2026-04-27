use std::process::Command;

#[test]
fn doctor_reports_bootstrap_errors_when_default_config_toml_is_malformed() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{
  "sandbox": {
    "container_runtime": "kubernetes",
    "k8s_namespace": "doctor-test"
  }
}"#,
    )
    .expect("write settings.json");
    std::fs::write(
        dir.path().join("config.toml"),
        "[sandbox\ncontainer_runtime = ",
    )
    .expect("write config.toml");

    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
        .arg("doctor")
        .env("IRONCLAW_BASE_DIR", dir.path())
        .env("NO_COLOR", "1")
        .env_remove("CONTAINER_RUNTIME")
        .env_remove("IRONCLAW_K8S_NAMESPACE")
        .output()
        .expect("run ironclaw doctor");

    assert!(
        output.status.success(),
        "doctor command should exit successfully even when checks fail: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "bootstrap config error: Failed to parse configuration: Failed to load config file"
        ),
        "expected doctor to surface malformed default config.toml, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("runtime resolution error"),
        "doctor should not continue with stale runtime settings after bootstrap failure:\n{stdout}"
    );
    assert!(
        !stdout.contains("kubernetes is the selected runtime"),
        "doctor should not read runtime selection from settings.json after bootstrap failure:\n{stdout}"
    );
    assert!(
        !stdout.contains("doctor-test"),
        "doctor should not read namespace from settings.json after bootstrap failure:\n{stdout}"
    );
}

#[test]
fn doctor_does_not_let_acp_agents_bypass_malformed_default_config_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{
  "sandbox": {
    "container_runtime": "kubernetes",
    "k8s_namespace": "doctor-test"
  }
}"#,
    )
    .expect("write settings.json");
    std::fs::write(
        dir.path().join("config.toml"),
        "[sandbox\ncontainer_runtime = ",
    )
    .expect("write config.toml");
    std::fs::write(
        dir.path().join("acp-agents.json"),
        r#"{
  "agents": [
    {
      "name": "codex",
      "command": "codex",
      "args": ["--help"],
      "enabled": true
    }
  ],
  "schema_version": 1
}"#,
    )
    .expect("write acp-agents.json");

    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
        .arg("doctor")
        .env("IRONCLAW_BASE_DIR", dir.path())
        .env("NO_COLOR", "1")
        .env_remove("CONTAINER_RUNTIME")
        .env_remove("IRONCLAW_K8S_NAMESPACE")
        .output()
        .expect("run ironclaw doctor");

    assert!(
        output.status.success(),
        "doctor command should exit successfully even when checks fail: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ACP agents"),
        "expected doctor output to include the ACP agents check, got:\n{stdout}"
    );
    assert!(
        stdout.contains(
            "ACP agents          bootstrap config error: Failed to parse configuration: Failed to load config file"
        ),
        "expected ACP agents check to honor bootstrap failure, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("1 agent(s) configured, all valid"),
        "ACP agents check should not bypass malformed config.toml via disk fallback:\n{stdout}"
    );
}
