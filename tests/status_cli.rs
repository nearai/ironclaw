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

    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
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
