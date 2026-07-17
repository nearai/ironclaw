use std::process::Command;

#[test]
fn status_lists_enabled_wasm_channel_names() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let base_dir = tempdir.path();
    let channels_dir = base_dir.join("channels");
    std::fs::create_dir_all(&channels_dir).expect("create channels dir");
    std::fs::File::create(channels_dir.join("telegram.wasm")).expect("write wasm");
    std::fs::write(
        base_dir.join("config.toml"),
        "[channels]\nwasm_channels_enabled = true\nwasm_channels = [\"telegram\"]\n",
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw-v1"))
        .arg("status")
        .env("IRONCLAW_BASE_DIR", base_dir)
        .current_dir(base_dir)
        .output()
        .expect("run ironclaw status");

    assert!(
        output.status.success(),
        "status command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Channels") && stdout.contains("telegram"),
        "status output did not include enabled WASM channel names:\n{}",
        stdout
    );
}

#[test]
fn retained_v1_help_and_completions_use_the_isolated_command_name() {
    let binary = env!("CARGO_BIN_EXE_ironclaw-v1");

    let help = Command::new(binary)
        .arg("--help")
        .output()
        .expect("run ironclaw-v1 --help");
    assert!(help.status.success(), "legacy help command must succeed");
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_stdout.contains("Usage: ironclaw-v1") && !help_stdout.contains("Usage: ironclaw "),
        "retained v1 help must not claim the canonical command name:\n{help_stdout}"
    );

    let completion = Command::new(binary)
        .args(["completion", "--shell", "bash"])
        .output()
        .expect("run ironclaw-v1 completion");
    assert!(
        completion.status.success(),
        "legacy completion command must succeed"
    );
    let completion_stdout = String::from_utf8_lossy(&completion.stdout);
    assert!(
        completion_stdout.contains("ironclaw-v1")
            && !completion_stdout.contains("complete -F _ironclaw ironclaw"),
        "retained v1 completion must not register the canonical command:\n{completion_stdout}"
    );
}
