#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::db_write_measurement::{
    CanonicalDbWriteMeasurement, MeasuredStorageBackend, measure_db_writes,
};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const WORKLOAD: &str = "canonical-agent-turn";
const TOOL_CALLS: usize = 10;
const MODEL_ATTEMPTS: usize = TOOL_CALLS + 1;
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(250);
const MODEL_CALL_DELAY: Duration = Duration::from_millis(120);
const MINIMUM_HEARTBEATS: usize = 3;
const FINAL_REPLY: &str = "canonical measurement complete";

#[tokio::test]
async fn canonical_agent_turn_db_writes_libsql()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_canonical_agent_turn(StorageMode::LibSql, MeasuredStorageBackend::Libsql).await
}

#[tokio::test]
async fn canonical_agent_turn_db_writes_postgres()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_canonical_agent_turn(StorageMode::Postgres, MeasuredStorageBackend::Postgres).await
}
#[tokio::test]
async fn durable_milestones_reject_custom_actor_group_threads()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let group = RebornIntegrationGroup::builder()
        .storage(StorageMode::LibSql)
        .with_durable_milestone_event_store_for_test()
        .builtin_tools()
        .await?;
    let result = group
        .thread("conv-db-write-custom-actor")
        .with_actor_id("db-write-custom-actor")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await;

    let error = match result {
        Ok(_) => return Err("custom actor unexpectedly used canonical durable milestones".into()),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("custom-actor group threads cannot use")
    );
    Ok(())
}

async fn run_canonical_agent_turn(
    storage: StorageMode,
    backend: MeasuredStorageBackend,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut script = (0..TOOL_CALLS)
        .map(|attempt| {
            RebornScriptedReply::tool_call(
                "builtin.http",
                json!({"url": format!("https://measurement.invalid/step/{attempt}")}),
            )
        })
        .collect::<Vec<_>>();
    script.push(RebornScriptedReply::text(FINAL_REPLY));

    let harness =
        RebornIntegrationHarness::builder(format!("conv-db-write-canonical-{}", backend.as_str()))
            .storage(storage)
            .with_builtin_http_tools()
            .with_durable_milestone_event_store_for_test()
            .with_runner_heartbeat_interval_for_test(HEARTBEAT_INTERVAL)
            .with_model_call_delay_for_test(MODEL_CALL_DELAY)
            .record_model_calls_for_test()
            .script(script)
            .build()
            .await?;
    let config = harness.db_probe_config(true)?;

    let (measurement, run_id) = measure_db_writes(
        &config,
        CanonicalDbWriteMeasurement::new(WORKLOAD, TOOL_CALLS),
        || async {
            harness
                .submit_turn("run the canonical measured tool flow")
                .await
        },
    )
    .await?;

    harness.assert_reply_contains(FINAL_REPLY).await?;
    harness
        .assert_interactive_model_provider_call_count(MODEL_ATTEMPTS)
        .await?;
    harness
        .assert_tool_invocation_count("builtin.http", TOOL_CALLS)
        .await?;
    harness
        .assert_capability_result_count("builtin.http", TOOL_CALLS)
        .await?;
    harness.assert_no_process_heartbeat_entries(run_id).await?;
    harness.assert_durable_event_count_at_least(1).await?;

    measurement.assert_minimum_duration(HEARTBEAT_INTERVAL * MINIMUM_HEARTBEATS as u32)?;
    measurement.assert_nonzero_root_filesystem_families()?;
    println!("{}", serde_json::to_string(&measurement)?);
    Ok(())
}
