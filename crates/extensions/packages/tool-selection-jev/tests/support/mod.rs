//! A loopback stub of a Jev decisions endpoint, and request fixtures.
//!
//! The stub speaks just enough HTTP/1.1 for the host egress: it reads one
//! request per connection (headers, then `content-length` bytes), records
//! it, and answers with whatever the test's handler returns.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    ids::CapabilityId,
};
use ironclaw_loop_contracts::{
    ConversationContext, ProviderToolDefinition, ToolSelectionCandidate, ToolSelectionRequest,
};
use ironclaw_tool_selection_jev::{
    DEFAULT_JEV_MODEL, JevApiKey, JevEndpoint, JevToolClassifier, with_stub_endpoint,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub const API_KEY: &str = "jev-KEY-CANARY-0123456789";

/// The path the stub serves on: not any provider's, so a test proves the
/// classifier posts to the configured URL as given.
pub const STUB_PATH: &str = "/stub/jev/decisions";

/// One request the stub received.
#[derive(Debug, Clone)]
pub struct Recorded {
    /// The request line's path, e.g. [`STUB_PATH`].
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The question ids, in body order (sorted by key).
    pub fn question_ids(&self) -> Vec<String> {
        self.body["questions"]
            .as_object()
            .map(|questions| questions.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// What the stub sends back for one request.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A status and a raw body.
    Status(u16, String),
    /// A status with a `retry-after` of this many whole seconds.
    RetryAfter(u16, u64),
    /// Never answer.
    Hang,
    /// Close the connection without a response.
    Close,
}

type Handler = dyn Fn(usize, &Recorded) -> Reply + Send + Sync;

pub struct StubServer {
    pub url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl StubServer {
    /// Serve with `handler`, which sees the 0-based request number and the
    /// request. With `hold_until`, no request is answered before that many
    /// have arrived: a test of concurrency, since sequential requests would
    /// never get there.
    pub async fn start(
        hold_until: Option<usize>,
        handler: impl Fn(usize, &Recorded) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let requests: Arc<Mutex<Vec<Recorded>>> = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let requests = Arc::clone(&recorded);
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let Some(request) = read_request(&mut stream).await else {
                        return;
                    };
                    let number = {
                        let mut requests = requests.lock().expect("requests");
                        requests.push(request.clone());
                        requests.len() - 1
                    };
                    if let Some(expected) = hold_until {
                        for _ in 0..400 {
                            if requests.lock().expect("requests").len() >= expected {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    }
                    match handler(number, &request) {
                        Reply::Status(status, body) => {
                            let response = format!(
                                "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n\
                                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        }
                        Reply::RetryAfter(status, seconds) => {
                            let response = format!(
                                "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n\
                                 retry-after: {seconds}\r\ncontent-length: 2\r\n\
                                 connection: close\r\n\r\n{{}}"
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        }
                        Reply::Hang => tokio::time::sleep(Duration::from_secs(30)).await,
                        Reply::Close => drop(stream),
                    }
                });
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}{STUB_PATH}"),
            requests,
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("requests").clone()
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string();
    let headers: Vec<(String, String)> = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&buffer[header_end..header_end + length]).ok()?;
    Some(Recorded {
        path,
        headers,
        body,
    })
}

/// A 200 answering every question asked with `probabilities` (default 0.1),
/// reporting 100 input tokens and `jev-1.13.0` as the model that answered
/// (what the `jev-latest` alias resolved to).
pub fn answer_all(request: &Recorded, probabilities: &BTreeMap<&str, f32>) -> Reply {
    let answers: serde_json::Map<String, Value> = request
        .question_ids()
        .into_iter()
        .map(|id| {
            let probability = probabilities.get(id.as_str()).copied().unwrap_or(0.1);
            (id, json!({"type": "noul", "noul": probability}))
        })
        .collect();
    Reply::Status(
        200,
        json!({
            "model": "jev-1.13.0",
            "answers": answers,
            "usage": {"input_tokens": 100, "output_tokens": 4}
        })
        .to_string(),
    )
}

pub fn loopback_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: None,
    }
}

pub fn classifier(url: &str, timeout_ms: u64) -> JevToolClassifier {
    with_stub_endpoint(
        JevToolClassifier::new(
            JevEndpoint::default(),
            DEFAULT_JEV_MODEL,
            JevApiKey::new(API_KEY).expect("key"),
            Duration::from_millis(timeout_ms),
        )
        .expect("classifier"),
        url,
        loopback_policy(),
    )
}

pub fn candidate(name: &str, description: &str, tokens: u32) -> ToolSelectionCandidate {
    ToolSelectionCandidate {
        definition: ProviderToolDefinition::from_parts(
            CapabilityId::new(name.replace("__", ".")).expect("capability id"),
            name,
            description,
            json!({
                "type": "object",
                "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}
            }),
        )
        .expect("definition"),
        est_schema_tokens: tokens,
    }
}

/// `count` tools named `tool_000__run` … in catalog order.
pub fn catalog(count: usize) -> Vec<ToolSelectionCandidate> {
    (0..count)
        .map(|index| {
            candidate(
                &format!("tool_{index:03}__run"),
                &format!("Run task number {index} for the user."),
                50,
            )
        })
        .collect()
}

pub fn request(
    text: &str,
    candidates: Vec<ToolSelectionCandidate>,
    pinned: &[&str],
    max_tools: usize,
) -> ToolSelectionRequest {
    ToolSelectionRequest {
        context: ConversationContext::new(vec![text.to_string()]),
        called_tools: Vec::new(),
        candidates,
        pinned: pinned.iter().map(|name| name.to_string()).collect(),
        max_tools,
        token_budget: 100_000,
        reserved_tools: 3 + pinned.len(),
        reserved_tokens: 300,
    }
}
