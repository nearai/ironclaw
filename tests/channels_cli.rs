use std::process::Command;

#[test]
fn channels_list_http_entry_reports_unified_webhook_listener_address() {
    let home = tempfile::tempdir().expect("temp home");
    let libsql_path = home.path().join("ironclaw.db");
    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
        .args(["channels", "list", "--verbose"])
        .env_clear()
        .env("HOME", home.path())
        .env("DATABASE_BACKEND", "libsql")
        .env("LIBSQL_PATH", &libsql_path)
        .env("HTTP_ENABLED", "true")
        .env("HTTP_HOST", "0.0.0.0")
        .env("HTTP_PORT", "8089")
        .env("WEBHOOK_HOST", "127.0.0.1")
        .env("WEBHOOK_PORT", "9091")
        .output()
        .expect("run ironclaw channels list");

    assert!(
        output.status.success(),
        "channels list should succeed: status={:?}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("  http [enabled] (built-in)"),
        "expected enabled HTTP channel entry in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("host: 127.0.0.1"),
        "HTTP channel entry must report the unified listener host:\n{stdout}"
    );
    assert!(
        stdout.contains("port: 9091"),
        "HTTP channel entry must report the unified listener port:\n{stdout}"
    );
}
