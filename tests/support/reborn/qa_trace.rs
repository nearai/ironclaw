//! Record/replay support for QA-phrase traces against the Reborn runtime.
//!
//! Recording wraps the real Anthropic provider in the existing
//! `ironclaw_llm::recording::RecordingLlm` (the same recorder v1 live tests
//! use — it sits at the `LlmProvider` seam, underneath Reborn's
//! `LlmProviderModelGateway`, so it is runtime-agnostic) and drives a
//! local-dev Reborn runtime with the production model-gateway conversion
//! layer. The flushed JSON is the recorded `LlmTrace` format that
//! `RebornTraceReplayModelGateway::from_trace` replays deterministically.
//!
//! Tool names recorded at this seam are the model-facing names the Reborn
//! gateway advertises, which equal capability ids (`builtin.trigger_create`)
//! for every first-party tool except `builtin.skill_activate` (advertised as
//! `builtin__skill_activate`); the QA phrases do not exercise that tool.

#![allow(dead_code)] // Shared by the QA recorder/replay test binaries only.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_llm::{
    LlmConfig, LlmProvider, NearAiConfig, ProviderProtocol, RegistryProviderConfig, SessionConfig,
    build_static_provider_chain, create_session_manager, recording::RecordingLlm,
};
use ironclaw_loop_support::HostManagedModelGateway;
use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_reborn_composition::{
    AssistantReply, RebornCompositionProfile, RebornLocalRuntimeProfileOptions, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, RebornTurnDriveOutcome, build_reborn_runtime,
    local_runtime_build_input_with_options,
};
use ironclaw_turns::run_profile::ModelProfileId;
use secrecy::SecretString;

use crate::support::trace_llm::LlmTrace;

pub const QA_RECORD_KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const QA_RECORD_MODEL_ENV: &str = "IRONCLAW_QA_RECORD_MODEL";
pub const QA_RECORD_DEFAULT_MODEL: &str = "claude-sonnet-4-6";

const QA_TENANT: &str = "qa-trace-tenant";
const QA_USER: &str = "qa-trace-owner";
const QA_AGENT: &str = "qa-trace-agent";

/// Tenant id the QA-trace runtime is composed with — replay assertions need
/// it to query tenant-scoped state (e.g. the trigger repository).
pub fn qa_trace_tenant_id() -> &'static str {
    QA_TENANT
}

/// The model profile id the composed Reborn runtime routes turns through;
/// must match `wrap_swappable_gateway` in `ironclaw_reborn_composition`.
const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

pub fn qa_fixture_path(fixture_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/llm_traces/reborn_qa")
        .join(format!("{fixture_name}.json"))
}

pub fn load_qa_trace(fixture_name: &str) -> LlmTrace {
    let path = qa_fixture_path(fixture_name);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "QA trace fixture {} is missing ({error}); record it with the \
             ignored recorder test for this phrase",
            path.display()
        )
    });
    serde_json::from_str(&json).expect("QA trace fixture parses as recorded LlmTrace JSON")
}

/// Clear `expected_tool_results` on every step so a runtime replay re-executes
/// the recorded capability calls against today's runtime without exact-matching
/// nondeterministic tool output (trigger ids, timestamps) captured at record
/// time. Tool-choice contracts are asserted on the raw fixture instead.
pub fn strip_expected_tool_results(trace: &mut LlmTrace) {
    for turn in &mut trace.turns {
        for step in &mut turn.steps {
            step.expected_tool_results.clear();
        }
    }
}

/// Build the local-dev yolo Reborn runtime the QA traces are recorded and
/// replayed against. No trigger poller: the phrases under test create
/// routines, they don't need them to fire.
pub async fn build_qa_trace_runtime(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
) -> RebornRuntime {
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        QA_USER,
        root.path().join("local-dev"),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);
    let input = RebornRuntimeInput::from_services(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: QA_TENANT.to_string(),
            agent_id: QA_AGENT.to_string(),
            source_binding_id: "qa-trace-source".to_string(),
            reply_target_binding_id: "qa-trace-reply".to_string(),
        })
        .with_model_gateway_override(gateway);
    build_reborn_runtime(input).await.expect("runtime builds")
}

/// Send one phrase through a fresh conversation and wait for the terminal
/// reply.
pub async fn send_qa_phrase(runtime: &RebornRuntime, phrase: &str) -> AssistantReply {
    let conversation = runtime
        .new_conversation()
        .await
        .expect("new QA conversation");
    runtime
        .send_user_message(&conversation, phrase)
        .await
        .expect("QA phrase turn reaches a terminal state")
}

/// Record one QA phrase against the live Anthropic API and flush the trace to
/// the fixture path. Panics with a clear message when the API key is absent —
/// recorder tests are `#[ignore]`d and only run when explicitly invoked.
pub async fn record_qa_phrase(fixture_name: &str, phrase: &str) {
    let api_key = std::env::var(QA_RECORD_KEY_ENV).unwrap_or_else(|_| {
        panic!("{QA_RECORD_KEY_ENV} must be set to record QA traces against the live API")
    });
    let model = std::env::var(QA_RECORD_MODEL_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| QA_RECORD_DEFAULT_MODEL.to_string());

    let config = anthropic_llm_config(api_key, &model);
    let session = create_session_manager(config.session.clone()).await;
    let provider = build_static_provider_chain(&config, session)
        .await
        .expect("anthropic provider chain builds");

    let fixture_path = qa_fixture_path(fixture_name);
    if let Some(parent) = fixture_path.parent() {
        std::fs::create_dir_all(parent).expect("fixture directory");
    }
    let recorder = Arc::new(RecordingLlm::new(
        provider,
        fixture_path.clone(),
        format!("recorded-qa-{fixture_name}"),
    ));

    let profile = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE).expect("model profile id");
    let policy = LlmModelProfilePolicy::new().allow_model_profile(profile, None);
    let gateway =
        LlmProviderModelGateway::new(Arc::clone(&recorder) as Arc<dyn LlmProvider>, policy);

    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_qa_trace_runtime(&root, Arc::new(gateway)).await;
    // Drive the phrase to a terminal status *or* the first gate it raises.
    // Using `send_user_message_until_gate` (not `send_user_message`) means an
    // OAuth/approval-gated phrase records the agent's decisions up to the gate
    // and reports the pause, instead of parking in the non-terminal
    // `BlockedAuth` state until `RunTimeout`. Resolving the gate to record the
    // post-auth turns is a deliberate follow-up that goes through the WebUI
    // facade with a seeded credential — not wired here.
    let conversation = runtime
        .new_conversation()
        .await
        .expect("new QA conversation");
    let outcome = runtime
        .send_user_message_until_gate(&conversation, phrase)
        .await
        .expect("QA phrase reaches a terminal status or a gate");
    runtime.shutdown().await.expect("runtime shutdown");

    recorder.flush().await.expect("flush recorded QA trace");
    match outcome {
        RebornTurnDriveOutcome::Terminal(reply) => {
            assert!(
                reply.is_successful_final_reply(),
                "recorded QA phrase {fixture_name:?} did not complete successfully \
                 (status {:?}); trace still flushed to {} for inspection — scrub and \
                 re-record before committing",
                reply.status,
                fixture_path.display()
            );
            println!(
                "recorded QA trace {} (reply: {})",
                fixture_path.display(),
                reply.text.as_deref().unwrap_or("<none>")
            );
        }
        RebornTurnDriveOutcome::BlockedOnGate {
            status,
            gate_ref,
            partial_text,
            ..
        } => {
            // A gate pause is the expected recordable outcome for phrases that
            // require interactive auth/approval (e.g. "connect to Gmail"): the
            // agent routed to the gate, which is exactly what the contract for
            // those phrases pins. The trace is flushed up to the gate.
            println!(
                "recorded QA trace {} (paused at gate: status {:?}, gate_ref {}, partial reply: {})",
                fixture_path.display(),
                status,
                gate_ref.as_str(),
                partial_text.as_deref().unwrap_or("<none>")
            );
        }
    }

    // Give the recording a 2s settle so background turn-state writes finish
    // before the tempdir drops.
    tokio::time::sleep(Duration::from_secs(2)).await;
}

fn anthropic_llm_config(api_key: String, model: &str) -> LlmConfig {
    LlmConfig {
        backend: "anthropic".to_string(),
        session: SessionConfig::default(),
        nearai: NearAiConfig {
            model: model.to_string(),
            cheap_model: None,
            base_url: "https://cloud-api.near.ai/v1".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 1,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: false,
        },
        provider: Some(RegistryProviderConfig::generic(
            ProviderProtocol::Anthropic,
            "anthropic",
            Some(SecretString::from(api_key)),
            "https://api.anthropic.com",
            model,
        )),
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 120,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 1,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    }
}
