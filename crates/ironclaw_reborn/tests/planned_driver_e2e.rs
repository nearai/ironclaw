use ironclaw_agent_loop::test_support::{MockAgentLoopDriverHost, ScenarioScript};
use ironclaw_reborn::{PlannedDriver, build_loop_family_registry};
use ironclaw_turns::{
    AgentLoopDriverRunRequest, LoopExit,
    run_profile::{AgentLoopDriver, AgentLoopDriverError, AgentLoopHostErrorKind, LoopRunInfoPort},
};

fn run_request(
    driver: &PlannedDriver,
    host: &MockAgentLoopDriverHost,
) -> AgentLoopDriverRunRequest {
    let mut profile = host.run_context().resolved_run_profile.clone();
    let descriptor = driver.descriptor();
    profile.loop_driver = descriptor.clone();
    profile.checkpoint_schema_id = descriptor
        .checkpoint_schema_id
        .clone()
        .expect("planned driver descriptor should carry checkpoint schema");
    profile.checkpoint_schema_version = descriptor
        .checkpoint_schema_version
        .expect("planned driver descriptor should carry checkpoint version");
    AgentLoopDriverRunRequest {
        turn_id: host.run_context().turn_id,
        run_id: host.run_context().run_id,
        resolved_run_profile: profile,
    }
}

#[tokio::test]
async fn default_planned_driver_smoke() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(driver.descriptor().id.as_str(), "reborn:planned-default");
}

#[tokio::test]
async fn planned_driver_executor_error_maps_to_unavailable() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .fail_prompt_with(AgentLoopHostErrorKind::Unavailable)
        .build();

    let error = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect_err("model unavailability should map to driver error");

    assert_eq!(
        error,
        AgentLoopDriverError::Unavailable {
            reason: "Prompt: unavailable".to_string()
        }
    );
    let debug = format!("{error:?}");
    assert!(!debug.contains("sk-fake"));
    assert!(!debug.contains("/host/path"));
}

#[tokio::test]
async fn planned_driver_rejects_mismatched_profile_assignment() {
    let registry = build_loop_family_registry().expect("registry should build");
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .build();
    let mut request = run_request(&driver, &host);
    request.resolved_run_profile.loop_driver.version = ironclaw_turns::RunProfileVersion::new(99);

    let error = driver
        .run(request, &host)
        .await
        .expect_err("mismatched descriptor should be rejected");

    assert!(matches!(error, AgentLoopDriverError::InvalidRequest { .. }));
}
