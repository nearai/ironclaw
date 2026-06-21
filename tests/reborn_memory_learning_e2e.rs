#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::{sync::LazyLock, time::Duration};

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
};
use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_memory::stable_learning_document_relative_path;
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::{HarnessWaitConfig, RebornBinaryE2EHarness},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const IRONCLAW_LEARNING_ENABLED_ENV: &str = "IRONCLAW_LEARNING_ENABLED";
const LEARNING_SCHEMA_FIELDS: &[&str] = &["key", "category", "confidence", "created_at", "source"];

static LEARNING_ENV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: tests that mutate IRONCLAW_LEARNING_ENABLED hold
        // LEARNING_ENV_LOCK for the full mutation scope.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    fn clear(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: tests that mutate IRONCLAW_LEARNING_ENABLED hold
        // LEARNING_ENV_LOCK for the full mutation scope.
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: guard restoration happens while the creating test still
        // holds LEARNING_ENV_LOCK.
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

#[tokio::test]
async fn reborn_trace_memory_learning_schema_is_hidden_when_disabled() {
    let _lock = LEARNING_ENV_LOCK.lock().await;
    let _learning_env = EnvGuard::clear(IRONCLAW_LEARNING_ENABLED_ENV);

    run_schema_e2e(
        "room-trace-memory-learning-disabled-schema",
        "event-trace-memory-learning-disabled-schema",
        "verify learning memory schema is disabled",
        "memory learning disabled schema complete",
        &["content", "target", "append", "metadata"],
        LEARNING_SCHEMA_FIELDS,
    )
    .await;
}

#[tokio::test]
async fn reborn_trace_memory_learning_schema_is_visible_when_enabled() {
    let _lock = LEARNING_ENV_LOCK.lock().await;
    let _learning_env = EnvGuard::set(IRONCLAW_LEARNING_ENABLED_ENV, "true");

    run_schema_e2e(
        "room-trace-memory-learning-enabled-schema",
        "event-trace-memory-learning-enabled-schema",
        "verify learning memory schema is enabled",
        "memory learning enabled schema complete",
        &[
            "content",
            "target",
            "append",
            "metadata",
            "key",
            "category",
            "confidence",
            "created_at",
            "source",
        ],
        &[],
    )
    .await;
}

#[tokio::test]
async fn reborn_trace_memory_learning_keyed_write_search_read_e2e() {
    let _lock = LEARNING_ENV_LOCK.lock().await;
    let _learning_env = EnvGuard::set(IRONCLAW_LEARNING_ENABLED_ENV, "true");
    let memory_write = capability_id(MEMORY_WRITE_CAPABILITY_ID);
    let memory_read = capability_id(MEMORY_READ_CAPABILITY_ID);
    let memory_search = capability_id(MEMORY_SEARCH_CAPABILITY_ID);
    let learning_key = "editor preference";
    let learning_category = "preference";
    let expected_path = stable_learning_document_relative_path(learning_category, learning_key)
        .expect("stable learning path");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_write.clone(),
                "call_memory_learning_write_old",
                serde_json::json!({
                    "key": learning_key,
                    "category": learning_category,
                    "confidence": 2,
                    "created_at": "2026-06-14T00:00:00Z",
                    "source": "reborn-e2e",
                    "content": "learning e2e old_marker prefers nano"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_write.clone(),
                "call_memory_learning_write_new",
                serde_json::json!({
                    "key": learning_key,
                    "category": learning_category,
                    "confidence": 9,
                    "created_at": "2026-06-14T00:01:00Z",
                    "source": "reborn-e2e",
                    "content": "learning e2e new_marker prefers helix region=us-east-1"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_search.clone(),
                "call_memory_learning_search_old",
                serde_json::json!({"query": "learning e2e old_marker", "limit": 5}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_search.clone(),
                "call_memory_learning_search_new",
                serde_json::json!({"query": "learning e2e new_marker", "limit": 5}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_read.clone(),
                "call_memory_learning_read",
                serde_json::json!({"path": expected_path}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "memory learning e2e trace complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-memory-learning-keyed-e2e",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    submit_and_expect_reply(
        &mut harness,
        "event-trace-memory-learning-keyed-e2e",
        "exercise learning memory keyed write search read",
        "memory learning e2e trace complete",
    )
    .await;

    let results = harness.capability_results();
    assert_eq!(results.len(), 5, "results: {results:?}");
    assert_eq!(results[0].capability_id, memory_write);
    assert_eq!(results[1].capability_id, memory_write);
    assert_eq!(results[2].capability_id, memory_search);
    assert_eq!(results[3].capability_id, memory_search);
    assert_eq!(results[4].capability_id, memory_read);
    assert_eq!(results[0].output["path"], serde_json::json!(expected_path));
    assert_eq!(results[1].output["path"], serde_json::json!(expected_path));
    assert_eq!(results[1].output["append"], serde_json::json!(false));
    assert_stable_learning_path_is_hashed(results[1].output["path"].as_str().unwrap());

    assert_eq!(results[2].output["result_count"], serde_json::json!(0));
    assert_eq!(results[3].output["result_count"], serde_json::json!(1));
    let search_result = &results[3].output["results"][0];
    assert_eq!(search_result["confidence"], serde_json::json!(9));
    assert_eq!(search_result["is_stale"], serde_json::json!(false));
    assert_output_contains_new_learning_only(
        search_result["content"]
            .as_str()
            .expect("search result content"),
    );
    assert_eq!(search_result["key"], serde_json::json!(learning_key));
    assert_eq!(
        search_result["category"],
        serde_json::json!(learning_category)
    );
    assert_eq!(search_result["source"], serde_json::json!("reborn-e2e"));
    assert_eq!(
        search_result["created_at"],
        serde_json::json!("2026-06-14T00:01:00Z")
    );
    assert_output_contains_new_learning_only(
        results[4].output["content"]
            .as_str()
            .expect("memory_read content"),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

async fn run_schema_e2e(
    room_id: &'static str,
    event_id: &'static str,
    user_message: &'static str,
    final_reply: &'static str,
    required_properties: &'static [&'static str],
    forbidden_properties: &'static [&'static str],
) {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolSchemaThenResponse {
            capability_id: capability_id(MEMORY_WRITE_CAPABILITY_ID),
            required_properties: required_properties.to_vec(),
            forbidden_properties: forbidden_properties.to_vec(),
            response: HostManagedModelResponse::assistant_reply(final_reply),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(room_id, model_gateway)
            .await
            .expect("harness");
    harness.start();

    submit_and_expect_reply(&mut harness, event_id, user_message, final_reply).await;

    assert!(
        harness.capability_invocations().is_empty(),
        "schema-only check must not invoke memory_write"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

async fn submit_and_expect_reply(
    harness: &mut RebornBinaryE2EHarness,
    event_id: &'static str,
    user_message: &'static str,
    final_reply: &'static str,
) {
    let submitted = harness
        .submit_text(event_id, user_message)
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            host_runtime_tool_wait(),
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply(final_reply)
        .await
        .expect("final reply");
}

fn host_runtime_tool_wait() -> HarnessWaitConfig {
    HarnessWaitConfig {
        timeout: Duration::from_secs(10),
        poll_interval: Duration::from_millis(10),
    }
}

fn capability_id(raw: &str) -> CapabilityId {
    CapabilityId::new(raw).expect("valid capability id")
}

fn assert_stable_learning_path_is_hashed(path: &str) {
    let Some(rest) = path.strip_prefix("keyed/") else {
        panic!("stable learning path must use keyed prefix: {path}");
    };
    let Some((category_hash, key_file_name)) = rest.split_once('/') else {
        panic!("stable learning path must include category and key hash segments: {path}");
    };
    let Some(key_hash) = key_file_name.strip_suffix(".md") else {
        panic!("stable learning key hash must end in .md: {path}");
    };
    assert_lowercase_sha256(category_hash);
    assert_lowercase_sha256(key_hash);
    for leaked in ["editor", "preference"] {
        assert!(
            !path.contains(leaked),
            "stable learning path leaked raw input fragment {leaked}: {path}"
        );
    }
}

fn assert_lowercase_sha256(value: &str) {
    assert_eq!(value.len(), 64, "expected sha256 hex segment: {value}");
    assert!(
        value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
        "expected lowercase sha256 hex segment: {value}"
    );
}

fn assert_output_contains_new_learning_only(output: &str) {
    assert!(!output.contains("old_marker"));
    assert!(output.contains("new_marker"));
    assert!(output.contains("region=us-east-1"));
}
