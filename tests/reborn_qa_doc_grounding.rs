//! QA use-case coverage for document-grounded answers:
//!
//! - "use the NEAR AI Strategy doc in my Google Drive as your knowledge
//!   base for answering strategy questions" → IronClaw references the doc
//!   and confirms it can answer from it.
//! - "For my next meeting, find information about the company that I am
//!   meeting with from my Google Docs and find the latest news." →
//!   the reply references a doc and the latest news.
//!
//! Drive documents are modeled as workspace files served through the real
//! `builtin.read_file` capability; "latest news" is fetched through the
//! real `builtin.http` capability against a live loopback server.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    routing::get,
};
use ironclaw_host_api::{CapabilityId, NetworkPolicy, NetworkScheme, NetworkTargetPattern};
use ironclaw_host_runtime::{HTTP_CAPABILITY_ID, READ_FILE_CAPABILITY_ID};
use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const STRATEGY_DOC_CONTENT: &str = "NEAR AI Strategy: user-owned agents are the core pillar; users keep custody of credentials and data.";
const COMPANY_DOC_CONTENT: &str = "PepsiCo brief: meeting about agent-assisted supply chain pilots; key contact in platform team.";

#[tokio::test]
async fn reborn_qa_strategy_doc_becomes_knowledge_base_for_answers() {
    const REPLY: &str = "I read the NEAR AI Strategy doc: user-owned agents are the core pillar. I can answer strategy questions from it.";

    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_strategy_doc",
                serde_json::json!({"path": "/workspace/drive/near-ai-strategy.md"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(REPLY),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-qa-strategy-knowledge-base",
        model_gateway,
    )
    .await
    .expect("harness");
    seed_drive_doc(&harness, "drive/near-ai-strategy.md", STRATEGY_DOC_CONTENT);
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-strategy-knowledge-base",
            "use the NEAR AI Strategy doc in my Google Drive as your knowledge base for answering strategy questions",
        )
        .await
        .expect("submit knowledge base request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("knowledge base confirmation reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, read_file);

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    let doc_result = serde_json::to_string(&results[0].output).expect("doc result json");
    assert!(
        doc_result.contains("user-owned agents are the core pillar"),
        "the strategy doc content must reach the model as a tool result, got {doc_result}"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_meeting_prep_references_company_doc_and_latest_news() {
    const REPLY: &str = "Your next meeting is with PepsiCo: the PepsiCo brief covers supply chain pilots, and the latest news is 'PepsiCo expands AI logistics program'.";

    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server =
        LiveNewsHttpServer::start(Router::new().route("/news/pepsico", get(company_news))).await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                RebornScriptedProviderToolCall::new(
                    read_file.clone(),
                    "call_read_company_doc",
                    serde_json::json!({"path": "/workspace/drive/companies/pepsico.md"}),
                ),
                RebornScriptedProviderToolCall::new(
                    http.clone(),
                    "call_fetch_company_news",
                    serde_json::json!({
                        "url": server.url("/news/pepsico"),
                        "timeout_ms": 2500,
                    }),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(REPLY),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-meeting-prep",
            model_gateway,
            loopback_policy(server.port),
        )
        .await
        .expect("harness");
    seed_drive_doc(&harness, "drive/companies/pepsico.md", COMPANY_DOC_CONTENT);
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-meeting-prep",
            "For my next meeting, find information about the company that I am meeting with from my Google Docs and find the latest news.",
        )
        .await
        .expect("submit meeting prep request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("meeting prep reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].capability_id, read_file);
    assert_eq!(invocations[1].capability_id, http);

    let results = harness.capability_results();
    assert_eq!(results.len(), 2);
    let doc_result = serde_json::to_string(&results[0].output).expect("doc result json");
    assert!(
        doc_result.contains("PepsiCo brief"),
        "the company doc must reach the model as a tool result, got {doc_result}"
    );
    assert_eq!(results[1].output["status"], serde_json::json!(200));
    let news_body = results[1].output["body_text"]
        .as_str()
        .expect("news body text");
    assert!(
        news_body.contains("PepsiCo expands AI logistics program"),
        "the latest news must reach the model as a tool result, got {news_body}"
    );

    assert_eq!(server.requests(), vec!["/news/pepsico".to_string()]);
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

fn seed_drive_doc(harness: &RebornBinaryE2EHarness, relative: &str, content: &str) {
    let path = harness
        .host_workspace_file_path(relative)
        .expect("drive doc path");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create drive doc dir");
    }
    std::fs::write(path, content).expect("write drive doc");
}

async fn company_news(State(state): State<LiveNewsHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "articles": [
                {"title": "PepsiCo expands AI logistics program", "published": "2026-06-10"},
            ],
        })),
    )
        .into_response()
}

#[derive(Clone)]
struct LiveNewsHttpState {
    requests: Arc<Mutex<Vec<String>>>,
}

impl LiveNewsHttpState {
    fn record(&self, uri: &Uri) {
        self.requests
            .lock()
            .expect("live news request log lock poisoned")
            .push(
                uri.path_and_query()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| uri.path().to_string()),
            );
    }
}

struct LiveNewsHttpServer {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl LiveNewsHttpServer {
    async fn start(routes: Router<LiveNewsHttpState>) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind live news test server");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = routes.with_state(LiveNewsHttpState {
            requests: Arc::clone(&requests),
        });
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            port,
            requests,
            task,
        }
    }

    fn url(&self, path_and_query: &str) -> String {
        format!("http://127.0.0.1:{}{path_and_query}", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("live news request log lock poisoned")
            .clone()
    }
}

impl Drop for LiveNewsHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn loopback_policy(port: u16) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: Some(port),
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(10_000),
    }
}
