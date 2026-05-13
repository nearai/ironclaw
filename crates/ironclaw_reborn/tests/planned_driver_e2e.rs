use std::collections::VecDeque;

use ironclaw_agent_loop::test_support::{
    MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedModelResponse,
};
use ironclaw_reborn::{PlannedDriver, build_loop_family_registry};
use ironclaw_turns::{
    AgentLoopDriverRunRequest, LoopExit, LoopMessageRef,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverError, AgentLoopHostErrorKind, LoopInput, LoopRunInfoPort,
    },
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
    let registry = build_loop_family_registry();
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(driver.descriptor().id.as_str(), "reborn:default-loop");
}

#[tokio::test]
async fn planned_driver_executor_error_maps_to_unavailable() {
    let registry = build_loop_family_registry();
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
    let registry = build_loop_family_registry();
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

#[tokio::test]
async fn planned_driver_consumes_steering_message_before_model_call() {
    let registry = build_loop_family_registry();
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let script = ScenarioScript {
        model_responses: VecDeque::from([ScriptedModelResponse::Reply {
            text: "hi".to_string(),
        }]),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::from([
            vec![LoopInput::Steering {
                message_ref: LoopMessageRef::new("msg:steering").unwrap(),
            }],
            Vec::new(),
        ]),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let calls = host.call_log();
    let first_prompt = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::BuildPromptBundle))
        .expect("prompt should be built");
    assert_eq!(calls[0], MockHostCall::PollInputs);
    assert_eq!(calls[1], MockHostCall::AckInputs);
    assert!(
        first_prompt > 1,
        "steering input must be acknowledged before the prompt/model path"
    );
}

#[tokio::test]
async fn planned_driver_followup_restarts_after_natural_stop() {
    let registry = build_loop_family_registry();
    let driver = PlannedDriver::default_from_registry(&registry).expect("driver should build");
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Reply {
                text: "first".to_string(),
            },
            ScriptedModelResponse::Reply {
                text: "second".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::from([
            Vec::new(),
            vec![LoopInput::FollowUp {
                message_ref: LoopMessageRef::new("msg:followup").unwrap(),
            }],
            Vec::new(),
            Vec::new(),
        ]),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();

    let exit = driver
        .run(run_request(&driver, &host), &host)
        .await
        .expect("planned driver run should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_call_count(), 2);
    assert!(
        host.call_log()
            .iter()
            .filter(|call| matches!(call, MockHostCall::AckInputs))
            .count()
            >= 1,
        "followup consumption should ack the advanced input cursor"
    );
}
