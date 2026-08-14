//! Full-turn ACP harness proof using a deterministic Docker adapter.

#[allow(dead_code)]
#[path = "support/docker_gate.rs"]
mod docker_gate;
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::{collections::HashSet, fs, path::Path, sync::Arc, time::Duration};

use ironclaw_turn_runner::{
    agent_placement::{DockerAgentPlacement, HostAgentPlacement},
    harness_turn_run_executor::HarnessTurnRunConfig,
};
use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

const IMAGE: &str = "ironclaw-acp-fake:latest";
const PROFILE: &str = "reborn-planned-default";

#[tokio::test]
async fn unselected_profile_keeps_the_canonical_executor() {
    let workspace_root = tempfile::tempdir().expect("workspace root");
    let config = HarnessTurnRunConfig {
        run_profile_ids: HashSet::from(["not-selected".to_string()]),
        timeout: Duration::from_secs(1),
        max_update_bytes: 1024,
        placement: Arc::new(
            HostAgentPlacement::new(
                workspace_root.path().to_path_buf(),
                "command-that-must-not-start".into(),
                Vec::new(),
                Vec::new(),
                1024,
            )
            .expect("valid unused host placement"),
        ),
    };
    let harness = RebornIntegrationHarness::test_default()
        .with_acp_harness_for_test(config)
        .script([RebornScriptedReply::text("canonical reply")])
        .build()
        .await
        .expect("harness builds");

    harness.submit_turn("hello").await.expect("turn completes");
    harness
        .assert_reply_contains("canonical reply")
        .await
        .expect("canonical executor reply persisted");
}

#[tokio::test]
async fn host_placement_reuses_session_and_auto_approves_permissions() {
    let workspace_root = tempfile::tempdir().expect("workspace root");
    let config = host_config(workspace_root.path(), Duration::from_secs(10));
    let harness = RebornIntegrationHarness::test_default()
        .with_acp_harness_for_test(config)
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("first")
        .await
        .expect("first turn completes");
    harness
        .assert_reply_contains("fake ACP reply 1")
        .await
        .expect("first ACP reply persisted");
    harness
        .submit_turn("second")
        .await
        .expect("second turn completes");
    harness
        .assert_reply_contains("fake ACP reply 2")
        .await
        .expect("second ACP reply persisted");

    let events = find_file(workspace_root.path(), "fake-events.log")
        .and_then(|path| fs::read_to_string(path).ok())
        .expect("fake adapter event log");
    assert!(events.contains("new:fake-session"), "{events}");
    assert!(events.contains("load:fake-session"), "{events}");
    assert!(!events.contains("new-cwd:/workspace"), "{events}");
    assert!(events.contains("env:HOME,PATH"), "{events}");
    assert!(events.matches("permission:").count() >= 2, "{events}");
    assert!(events.contains("allow-once"), "{events}");
}

#[tokio::test]
async fn acp_timeout_is_terminal_and_releases_the_thread_for_another_turn() {
    let workspace_root = tempfile::tempdir().expect("workspace root");
    let config = host_config(workspace_root.path(), Duration::from_millis(200));
    let harness = RebornIntegrationHarness::test_default()
        .with_acp_harness_for_test(config)
        .build()
        .await
        .expect("harness builds");

    let failed_run = harness
        .submit_turn_async("hang")
        .await
        .expect("turn submits");
    let state = harness
        .wait_for_status(failed_run, TurnStatus::Failed)
        .await
        .expect("timeout is terminal");
    assert_eq!(
        state.failure.as_ref().map(|failure| failure.category()),
        Some("interrupted_unexpectedly")
    );

    harness
        .submit_turn("recover")
        .await
        .expect("a later turn can run after timeout cleanup");
    harness
        .assert_reply_contains("fake ACP reply 1")
        .await
        .expect("later reply persisted");
}

#[tokio::test]
async fn host_process_death_is_terminal_and_does_not_requeue() {
    let workspace_root = tempfile::tempdir().expect("workspace root");
    let config = host_config(workspace_root.path(), Duration::from_secs(10));
    let harness = RebornIntegrationHarness::test_default()
        .with_acp_harness_for_test(config)
        .build()
        .await
        .expect("harness builds");

    let failed_run = harness
        .submit_turn_async("crash")
        .await
        .expect("turn submits");
    let state = harness
        .wait_for_status(failed_run, TurnStatus::Failed)
        .await
        .expect("process death is terminal");
    assert_eq!(
        state.failure.as_ref().map(|failure| failure.category()),
        Some("driver_protocol_violation")
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = find_file(workspace_root.path(), "fake-events.log")
        .and_then(|path| fs::read_to_string(path).ok())
        .expect("fake adapter event log");
    assert_eq!(events.matches("crash").count(), 1, "{events}");

    harness
        .submit_turn("recover")
        .await
        .expect("a later turn can run after process death");
    harness
        .assert_reply_contains("fake ACP reply 1")
        .await
        .expect("later reply persisted");
}

#[tokio::test]
async fn docker_placement_matches_host_reply_and_session_behavior() {
    if !docker_gate::docker_available().await {
        eprintln!("SKIP: ACP harness Docker parity requires a Docker daemon");
        return;
    }
    if !docker_gate::docker_image_available(IMAGE).await {
        eprintln!("SKIP: ACP fake image {IMAGE:?} is not built");
        return;
    }

    let workspace_root = tempfile::tempdir().expect("workspace root");
    let config = HarnessTurnRunConfig {
        run_profile_ids: HashSet::from([PROFILE.to_string()]),
        timeout: Duration::from_secs(10),
        max_update_bytes: 16 * 1024,
        placement: Arc::new(
            DockerAgentPlacement::new(
                workspace_root.path().to_path_buf(),
                IMAGE.to_string(),
                Vec::new(),
                16 * 1024,
            )
            .expect("valid Docker placement"),
        ),
    };
    let harness = RebornIntegrationHarness::test_default()
        .with_acp_harness_for_test(config)
        .build()
        .await
        .expect("harness builds");

    harness.submit_turn("first").await.expect("first turn");
    harness.submit_turn("second").await.expect("second turn");
    harness
        .assert_reply_contains("fake ACP reply 2")
        .await
        .expect("Docker reply persisted");

    let failed_run = harness
        .submit_turn_async("crash")
        .await
        .expect("Docker crash turn submits");
    let state = harness
        .wait_for_status(failed_run, TurnStatus::Failed)
        .await
        .expect("Docker process death is terminal");
    assert_eq!(
        state.failure.as_ref().map(|failure| failure.category()),
        Some("driver_protocol_violation")
    );

    harness
        .submit_turn("recover")
        .await
        .expect("Docker placement releases the thread after process death");
    harness
        .assert_reply_contains("fake ACP reply 3")
        .await
        .expect("Docker recovery reply persisted");

    let events = find_file(workspace_root.path(), "fake-events.log")
        .and_then(|path| fs::read_to_string(path).ok())
        .expect("fake adapter event log");
    assert!(events.contains("new:fake-session"), "{events}");
    assert!(events.contains("load:fake-session"), "{events}");
    assert!(events.contains("new-cwd:/workspace"), "{events}");
    assert_eq!(events.matches("crash").count(), 1, "{events}");
    assert!(events.matches("permission:").count() >= 3, "{events}");
}

fn host_config(workspace_root: &Path, timeout: Duration) -> HarnessTurnRunConfig {
    let fake_agent =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acp-fake/agent.mjs");
    HarnessTurnRunConfig {
        run_profile_ids: HashSet::from([PROFILE.to_string()]),
        timeout,
        max_update_bytes: 16 * 1024,
        placement: Arc::new(
            HostAgentPlacement::new(
                workspace_root.to_path_buf(),
                "node".into(),
                vec![fake_agent.into_os_string()],
                Vec::new(),
                16 * 1024,
            )
            .expect("valid host placement"),
        ),
    }
}

fn find_file(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    for entry in fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.file_name().and_then(|value| value.to_str()) == Some(name) {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find_file(&path, name)
        {
            return Some(found);
        }
    }
    None
}
