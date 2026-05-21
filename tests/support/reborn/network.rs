use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ironclaw_host_api::NetworkMethod;
use ironclaw_network::{
    NetworkHttpError, NetworkHttpResponse, NetworkHttpTransport, NetworkTransportRequest,
    PolicyNetworkHttpEgress,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct RecordingNetworkHttpTransport {
    inner: Arc<Mutex<RecordingNetworkState>>,
}

#[derive(Debug, Default)]
struct RecordingNetworkState {
    recorded: Vec<SanitizedNetworkTransportRequest>,
    scripted: VecDeque<Result<NetworkHttpResponse, NetworkHttpError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedNetworkTransportRequest {
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_len: usize,
    pub body_sha256: String,
}

impl RecordingNetworkHttpTransport {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingNetworkState::default())),
        }
    }

    pub fn push_response(&self, response: NetworkHttpResponse) {
        self.push_result(Ok(response));
    }

    pub fn push_error(&self, error: NetworkHttpError) {
        self.push_result(Err(error));
    }

    pub fn push_result(&self, result: Result<NetworkHttpResponse, NetworkHttpError>) {
        self.inner
            .lock()
            .expect("network transport lock poisoned")
            .scripted
            .push_back(result);
    }

    pub fn requests(&self) -> Vec<SanitizedNetworkTransportRequest> {
        self.inner
            .lock()
            .expect("network transport lock poisoned")
            .recorded
            .clone()
    }

    pub fn policy_egress(&self) -> PolicyNetworkHttpEgress<Self> {
        PolicyNetworkHttpEgress::new(self.clone())
    }
}

impl Default for RecordingNetworkHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkHttpTransport for RecordingNetworkHttpTransport {
    fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let mut state = self.inner.lock().map_err(|_| NetworkHttpError::Transport {
            reason: "network transport lock poisoned".to_string(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })?;
        state.recorded.push(sanitize_request(&request));
        state.scripted.pop_front().unwrap_or_else(|| {
            Err(NetworkHttpError::Transport {
                reason: "unexpected HTTP request".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            })
        })
    }
}

fn sanitize_request(request: &NetworkTransportRequest) -> SanitizedNetworkTransportRequest {
    SanitizedNetworkTransportRequest {
        method: request.method,
        url: sanitize_url(&request.url),
        headers: request
            .headers
            .iter()
            .map(|(name, value)| {
                if is_sensitive_header(name) {
                    (name.clone(), "<redacted>".to_string())
                } else {
                    (name.clone(), value.clone())
                }
            })
            .collect(),
        body_len: request.body.len(),
        body_sha256: hex::encode(Sha256::digest(&request.body)),
    }
}

fn sanitize_url(url: &str) -> String {
    url.split_once('?')
        .map_or_else(|| url.to_string(), |(base, _)| format!("{base}?<redacted>"))
}

fn is_sensitive_header(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "authorization"
            | "cookie"
            | "set-cookie"
            | "proxy-authorization"
            | "api-key"
            | "apikey"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
            | "x-csrf-token"
            | "csrf-token"
            | "secret"
            | "x-secret"
            | "client-secret"
            | "x-client-secret"
    )
}
