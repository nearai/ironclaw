//! A loopback stub of a Jev decisions endpoint, and request fixtures.
//!
//! The stub speaks just enough HTTP/1.1 for the host egress: it reads one
//! request per connection, records it, and answers with whatever the test's
//! handler returns.

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
    ConversationContext, ProviderToolDefinition, ToolSelection, ToolSelectionCandidate,
    ToolSelectionClassifier, ToolSelectionRequest,
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

/// One request the stub received: the request line's path, the headers
/// (names in lower case) and the JSON body.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl Recorded {
    /// The question ids, in body order (sorted by key).
    pub fn question_ids(&self) -> Vec<String> {
        let questions = self.body["questions"].as_object();
        questions
            .into_iter()
            .flat_map(|map| map.keys().cloned())
            .collect()
    }
}

pub struct StubServer {
    pub url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl StubServer {
    /// Serve with `handler`, which returns a status and a body, or `None`
    /// to never answer. No request is answered before `hold_until` requests
    /// have arrived: with more than one, a test of concurrency, since
    /// sequential requests would never get there.
    pub async fn start(
        hold_until: usize,
        handler: impl Fn(&Recorded) -> Option<(u16, String)> + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let requests: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let handler = Arc::new(handler);
        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let requests = Arc::clone(&recorded);
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let Some(request) = read_request(&mut stream).await else {
                        return;
                    };
                    requests.lock().expect("requests").push(request.clone());
                    while requests.lock().expect("requests").len() < hold_until {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let Some((status, body)) = handler(&request) else {
                        return tokio::time::sleep(Duration::from_secs(30)).await;
                    };
                    let response = format!(
                        "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        let url = format!("http://127.0.0.1:{port}{STUB_PATH}");
        Self { url, requests }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("requests").clone()
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..end]).to_string();
        let mut lines = head.lines();
        let path = lines.next()?.split_whitespace().nth(1)?.to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
            .collect();
        let (_, length) = headers.iter().find(|(name, _)| name == "content-length")?;
        let body_end = end + 4 + length.parse::<usize>().ok()?;
        if buffer.len() >= body_end {
            let body = serde_json::from_slice(&buffer[end + 4..body_end]).ok()?;
            return Some(Recorded {
                path,
                headers,
                body,
            });
        }
    }
}

/// A 200 answering every question asked with `probability(id)`.
pub fn answer_with(request: &Recorded, probability: impl Fn(&str) -> f32) -> Option<(u16, String)> {
    let answers: BTreeMap<String, Value> = request
        .question_ids()
        .into_iter()
        .map(|id| {
            let answer = json!({"type": "noul", "noul": probability(&id)});
            (id, answer)
        })
        .collect();
    let usage = json!({"input_tokens": 100, "output_tokens": 4});
    Some((
        200,
        json!({"model": "jev-1.13.0", "answers": answers, "usage": usage}).to_string(),
    ))
}

/// A classifier that posts to `url`, allowed plain HTTP to `host` only.
pub fn classifier_for(url: &str, timeout_ms: u64, host: &str) -> JevToolClassifier {
    let policy = NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: host.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: None,
    };
    let key = JevApiKey::new(API_KEY).expect("key");
    let timeout = Duration::from_millis(timeout_ms);
    let classifier =
        JevToolClassifier::new(JevEndpoint::default(), DEFAULT_JEV_MODEL, key, timeout);
    with_stub_endpoint(classifier.expect("classifier"), url, policy)
}

/// A classifier pointed at the stub, allowed plain HTTP to loopback only.
pub fn classifier(url: &str, timeout_ms: u64) -> JevToolClassifier {
    classifier_for(url, timeout_ms, "127.0.0.1")
}

/// Classify `request` at the stub; a failure is its error's kind label.
pub async fn classify(
    url: &str,
    timeout_ms: u64,
    request: ToolSelectionRequest,
) -> Result<ToolSelection, &'static str> {
    let outcome = classifier(url, timeout_ms).classify(&request).await;
    outcome.map_err(|error| error.kind_label())
}

pub fn candidate(name: &str, description: &str, tokens: u32) -> ToolSelectionCandidate {
    let schema = json!({
        "type": "object",
        "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}
    });
    let id = CapabilityId::new(name.replace("__", ".")).expect("capability id");
    ToolSelectionCandidate {
        definition: ProviderToolDefinition::from_parts(id, name, description, schema)
            .expect("definition"),
        est_schema_tokens: tokens,
    }
}

pub fn request(
    text: &str,
    candidates: Vec<ToolSelectionCandidate>,
    max_tools: usize,
) -> ToolSelectionRequest {
    ToolSelectionRequest {
        context: ConversationContext::new(vec![text.to_string()]),
        candidates,
        max_tools,
        token_budget: 100_000,
    }
}
