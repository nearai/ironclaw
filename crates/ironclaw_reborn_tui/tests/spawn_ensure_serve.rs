//! Integration tests for `ironclaw_reborn_tui::spawn::ensure_serve` and
//! `ServeHandle`'s kill-on-drop behavior (B2.10), driven through the `pub`
//! API only — reusing the shared `tests/support::MockServer` fixture. The
//! readiness-poll retry/timeout/early-exit branches (`poll_until_ready`)
//! are `pub(crate)` and therefore not reachable from this external
//! integration-test binary; those three tests live inline in
//! `src/spawn/mod.rs`'s `#[cfg(test)] mod tests` instead.
//!
//! Documented gap: a full end-to-end "tui subcommand spawns the real
//! `ironclaw-reborn serve` binary" smoke test is deferred to a follow-up
//! after B2.12 lands the subcommand wiring (see B2.10's plan section).

mod support;

use ironclaw_reborn_tui::client::ApiClient;
use ironclaw_reborn_tui::spawn::{ServeHandle, SpawnError, ensure_serve};
use support::{MockServer, ScriptedResponse};

#[tokio::test]
async fn ensure_serve_returns_external_when_already_healthy() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/session",
        ScriptedResponse::ok(serde_json::json!({
            "tenant_id": "tenant-1",
            "user_id": "user-1",
        })),
    );

    let client = server.client();
    let handle = ensure_serve(&client, None).await.expect("ensure_serve");

    assert!(matches!(handle, ServeHandle::External));
}

#[tokio::test]
async fn ensure_serve_without_spawn_and_unhealthy_returns_no_serve_available() {
    // No mock server started, no spawn configured: the session probe fails
    // immediately and there is nothing to fall back to spawning.
    let client = ApiClient::new("http://127.0.0.1:1".to_string(), "token".to_string());

    let error = ensure_serve(&client, None)
        .await
        .expect_err("expected NoServeAvailable");

    assert!(matches!(error, SpawnError::NoServeAvailable));
}

#[tokio::test]
async fn serve_handle_spawned_kills_child_on_drop() {
    let child = tokio::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .kill_on_drop(true)
        .spawn()
        .expect("spawn long-lived child");
    let pid = child.id().expect("child pid").to_string();

    let handle = ServeHandle::Spawned(child);
    drop(handle);

    let mut gone = false;
    for _ in 0..40 {
        let status = tokio::process::Command::new("kill")
            .args(["-0", &pid])
            .status()
            .await
            .expect("run kill -0");
        if !status.success() {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert!(gone, "child process {pid} still alive after handle drop");
}
