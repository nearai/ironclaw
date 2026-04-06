//! 0G Labs integration — Compute (decentralized LLM inference) and
//! Storage (execution trace persistence).

use crate::config::AgentConfig;
use crate::types::{ExecutionTrace, InferenceRequest, InferenceResponse};
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};

// ── 0G Compute ────────────────────────────────────────────────────────────────

/// 0G Compute — decentralized LLM inference with TEE attestations.
pub struct ZeroGCompute {
    client: Client,
    endpoint: String,
    auth_token: Option<String>,
}

impl ZeroGCompute {
    pub async fn new(config: &AgentConfig) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            endpoint: config.zero_g_compute_endpoint.clone(),
            auth_token: config.zero_g_compute_auth_token.clone(),
        })
    }

    /// Send an inference request to 0G Compute.
    ///
    /// Uses the OpenAI-compatible `/v1/proxy/chat/completions` endpoint.
    /// On success, extracts a TEE attestation from the response if available.
    /// Falls back to a local SHA-256 content hash when no attestation is found.
    pub async fn inference(&self, request: &InferenceRequest) -> Result<InferenceResponse> {
        let url = format!("{}/v1/proxy/chat/completions", self.endpoint);
        let model = request.model.clone().unwrap_or_else(|| "qwen/qwen-2.5-7b-instruct".to_string());

        // Build OpenAI-compatible messages array
        let mut messages = Vec::with_capacity(2);
        if !request.system_prompt.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": request.system_prompt
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": request.user_prompt
        }));

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
        });

        tracing::debug!("0G Compute request to {url}: model={model}");

        let mut req_builder = self.client.post(&url).json(&body);

        if let Some(token) = &self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req_builder
            .send()
            .await
            .with_context(|| format!("Failed to reach 0G Compute at {url}"))?;

        let status = resp.status();
        let body_str = resp
            .text()
            .await
            .context("Failed to read 0G Compute response body")?;

        tracing::debug!("0G Compute response status={status}, body_len={}", body_str.len());

        if !status.is_success() {
            anyhow::bail!(
                "0G Compute returned HTTP {status} from {url}: {}",
                &body_str[..body_str.len().min(500)]
            );
        }

        // Parse OpenAI-style response: extract content from choices[0].message.content
        let content = parse_openai_content(&body_str)
            .unwrap_or_else(|| body_str.clone());

        // Extract TEE attestation, or fall back to content hash
        let attestation = parse_attestation(&body_str)
            .unwrap_or_else(|| {
                tracing::debug!(
                    "No attestation in 0G response; falling back to content hash"
                );
                let mut h = Sha256::new();
                h.update(content.as_bytes());
                format!("0x{}", hex::encode(h.finalize()))
            });

        Ok(InferenceResponse {
            content,
            attestation_signature: attestation,
            provider: "0g-compute".to_string(),
        })
    }
}

// ── 0G Storage ────────────────────────────────────────────────────────────────

/// 0G Storage — decentralized storage for execution traces.
pub struct ZeroGStorage {
    client: Client,
    indexer_rpc: String,
}

impl ZeroGStorage {
    pub async fn new(config: &AgentConfig) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            indexer_rpc: config.zero_g_indexer_rpc.clone(),
        })
    }

    /// Upload an execution trace to 0G Storage.
    ///
    /// Returns the storage root hash. Falls back to a local content hash
    /// if the 0G endpoint is unavailable.
    pub async fn store_trace(&self, trace: &ExecutionTrace) -> Result<String> {
        let data = serde_json::to_string(trace).context("Failed to serialize trace")?;

        let mut h = Sha256::new();
        h.update(data.as_bytes());
        let content_hash = format!("0x{}", hex::encode(h.finalize()));

        let url = format!("{}/upload", self.indexer_rpc);
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "data": data,
                "tags": {
                    "type": "execution-trace",
                    "agent": trace.agent_id,
                    "session": trace.session_id,
                }
            }))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                Ok(extract_root_hash(&body).unwrap_or(content_hash))
            }
            Ok(r) => {
                tracing::warn!(
                    "0G Storage returned {} — using content hash",
                    r.status()
                );
                Ok(content_hash)
            }
            Err(e) => {
                tracing::warn!("0G Storage upload failed ({e}) — using content hash");
                Ok(content_hash)
            }
        }
    }

    /// Retrieve an execution trace from 0G Storage by root hash.
    pub async fn retrieve_trace(&self, root_hash: &str) -> Result<ExecutionTrace> {
        let url = format!("{}/download", self.indexer_rpc);
        let resp = self
            .client
            .get(&url)
            .query(&[("root", root_hash)])
            .send()
            .await
            .with_context(|| format!("Failed to reach 0G Storage at {url}"))?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "0G Storage returned {} for trace {root_hash}",
                resp.status()
            );
        }

        let body = resp
            .text()
            .await
            .context("Failed to read 0G Storage response")?;

        let trace_str = if let Ok(parsed) = serde_json::from_str::<Value>(&body) {
            parsed.get("data").map(|d| d.to_string()).unwrap_or(body)
        } else {
            body
        };

        serde_json::from_str(&trace_str)
            .with_context(|| format!("Failed to deserialize trace {root_hash}"))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse content from an OpenAI-compatible chat completions response.
/// Looks for `choices[0].message.content`.
fn parse_openai_content(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    parsed
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()?
        .to_string()
        .into()
}

/// Extract a TEE attestation from a 0G Compute JSON response.
fn parse_attestation(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    ["attestation", "signature", "proof", "tee_attestation", "id"]
        .iter()
        .find_map(|k| parsed.get(k)?.as_str().map(String::from))
}

/// Extract a root/hash field from a JSON response body, or fall back to
/// a plain hex string that looks like a 32-byte hash.
fn extract_root_hash(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    ["root_hash", "hash", "root"]
        .iter()
        .find_map(|k| parsed.get(k)?.as_str().map(String::from))
        .or_else(|| {
            let t = body.trim();
            if t.starts_with("0x") && t.len() == 66 {
                Some(t.to_string())
            } else {
                None
            }
        })
}
