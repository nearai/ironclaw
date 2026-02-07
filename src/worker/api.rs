//! HTTP client for worker-to-orchestrator communication.
//!
//! Every request includes a bearer token from `IRONCLAW_WORKER_TOKEN` env var.
//! The orchestrator validates this token is scoped to the correct job.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::WorkerError;
use crate::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// HTTP client that a container worker uses to talk to the orchestrator.
pub struct WorkerHttpClient {
    client: reqwest::Client,
    orchestrator_url: String,
    job_id: Uuid,
    token: String,
}

/// Status update sent from worker to orchestrator.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub state: String,
    pub message: Option<String>,
    pub iteration: u32,
}

/// Job description fetched from orchestrator.
#[derive(Debug, Serialize, Deserialize)]
pub struct JobDescription {
    pub title: String,
    pub description: String,
    pub project_dir: Option<String>,
}

/// Completion result from the orchestrator (proxied from the real LLM).
#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyCompletionResponse {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyToolCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tool_choice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyToolCompletionResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: String,
}

/// Completion result for the worker to report when done.
#[derive(Debug, Serialize, Deserialize)]
pub struct CompletionReport {
    pub success: bool,
    pub message: Option<String>,
    pub iterations: u32,
}

impl WorkerHttpClient {
    /// Create a new client from environment.
    ///
    /// Reads `IRONCLAW_WORKER_TOKEN` from the environment.
    pub fn from_env(orchestrator_url: String, job_id: Uuid) -> Result<Self, WorkerError> {
        let token =
            std::env::var("IRONCLAW_WORKER_TOKEN").map_err(|_| WorkerError::MissingToken)?;

        Ok(Self {
            client: reqwest::Client::new(),
            orchestrator_url: orchestrator_url.trim_end_matches('/').to_string(),
            job_id,
            token,
        })
    }

    /// Create with an explicit token (for testing).
    pub fn new(orchestrator_url: String, job_id: Uuid, token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            orchestrator_url: orchestrator_url.trim_end_matches('/').to_string(),
            job_id,
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/worker/{}/{}", self.orchestrator_url, self.job_id, path)
    }

    /// Fetch the job description from the orchestrator.
    pub async fn get_job(&self) -> Result<JobDescription, WorkerError> {
        let resp = self
            .client
            .get(self.url("job"))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(WorkerError::OrchestratorRejected {
                job_id: self.job_id,
                reason: format!("GET /job returned {}", resp.status()),
            });
        }

        resp.json().await.map_err(|e| WorkerError::LlmProxyFailed {
            reason: format!("failed to parse job description: {}", e),
        })
    }

    /// Proxy an LLM completion request through the orchestrator.
    pub async fn llm_complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, WorkerError> {
        let proxy_req = ProxyCompletionRequest {
            messages: request.messages.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stop_sequences: request.stop_sequences.clone(),
        };

        let resp = self
            .client
            .post(self.url("llm/complete"))
            .bearer_auth(&self.token)
            .json(&proxy_req)
            .send()
            .await
            .map_err(|e| WorkerError::LlmProxyFailed {
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::LlmProxyFailed {
                reason: format!("orchestrator returned {}: {}", status, body),
            });
        }

        let proxy_resp: ProxyCompletionResponse =
            resp.json().await.map_err(|e| WorkerError::LlmProxyFailed {
                reason: format!("failed to parse LLM response: {}", e),
            })?;

        Ok(CompletionResponse {
            content: proxy_resp.content,
            input_tokens: proxy_resp.input_tokens,
            output_tokens: proxy_resp.output_tokens,
            finish_reason: parse_finish_reason(&proxy_resp.finish_reason),
        })
    }

    /// Proxy an LLM tool completion request through the orchestrator.
    pub async fn llm_complete_with_tools(
        &self,
        request: &ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, WorkerError> {
        let proxy_req = ProxyToolCompletionRequest {
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tool_choice: request.tool_choice.clone(),
        };

        let resp = self
            .client
            .post(self.url("llm/complete_with_tools"))
            .bearer_auth(&self.token)
            .json(&proxy_req)
            .send()
            .await
            .map_err(|e| WorkerError::LlmProxyFailed {
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::LlmProxyFailed {
                reason: format!("orchestrator returned {}: {}", status, body),
            });
        }

        let proxy_resp: ProxyToolCompletionResponse =
            resp.json().await.map_err(|e| WorkerError::LlmProxyFailed {
                reason: format!("failed to parse tool completion response: {}", e),
            })?;

        Ok(ToolCompletionResponse {
            content: proxy_resp.content,
            tool_calls: proxy_resp.tool_calls,
            input_tokens: proxy_resp.input_tokens,
            output_tokens: proxy_resp.output_tokens,
            finish_reason: parse_finish_reason(&proxy_resp.finish_reason),
        })
    }

    /// Report status to the orchestrator.
    pub async fn report_status(&self, update: &StatusUpdate) -> Result<(), WorkerError> {
        let resp = self
            .client
            .post(self.url("status"))
            .bearer_auth(&self.token)
            .json(update)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            tracing::warn!(
                "Status report failed with {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }

        Ok(())
    }

    /// Signal job completion to the orchestrator.
    pub async fn report_complete(&self, report: &CompletionReport) -> Result<(), WorkerError> {
        let resp = self
            .client
            .post(self.url("complete"))
            .bearer_auth(&self.token)
            .json(report)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(WorkerError::OrchestratorRejected {
                job_id: self.job_id,
                reason: format!("completion report rejected: {}", resp.status()),
            });
        }

        Ok(())
    }
}

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_use" | "tool_calls" => FinishReason::ToolUse,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let client = WorkerHttpClient::new(
            "http://host.docker.internal:50051".to_string(),
            Uuid::nil(),
            "test-token".to_string(),
        );

        assert_eq!(
            client.url("llm/complete"),
            format!(
                "http://host.docker.internal:50051/worker/{}/llm/complete",
                Uuid::nil()
            )
        );
    }

    #[test]
    fn test_parse_finish_reason() {
        assert_eq!(parse_finish_reason("stop"), FinishReason::Stop);
        assert_eq!(parse_finish_reason("tool_use"), FinishReason::ToolUse);
        assert_eq!(parse_finish_reason("unknown"), FinishReason::Unknown);
    }
}
