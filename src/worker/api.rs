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
    /// Skill permissions to enforce in the worker.
    /// Missing field deserializes to empty vec (backward compat with older orchestrators).
    #[serde(default)]
    pub skill_permissions: Vec<crate::skills::enforcer::SerializedToolPermission>,
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

/// Payload sent to the orchestrator for each job event (shared by worker and Claude Code bridge).
#[derive(Debug, Serialize, Deserialize)]
pub struct JobEventPayload {
    pub event_type: String,
    pub data: serde_json::Value,
}

/// Response from the prompt polling endpoint.
#[derive(Debug, Deserialize)]
pub struct PromptResponse {
    pub content: String,
    #[serde(default)]
    pub done: bool,
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

    /// Get the base orchestrator URL.
    pub fn orchestrator_url(&self) -> &str {
        &self.orchestrator_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}/worker/{}/{}", self.orchestrator_url, self.job_id, path)
    }

    /// Send a GET request, check the status, and deserialize the JSON body.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        context: &str,
    ) -> Result<T, WorkerError> {
        let resp = self
            .client
            .get(self.url(path))
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
                reason: format!("{} returned {}", context, resp.status()),
            });
        }

        resp.json().await.map_err(|e| WorkerError::LlmProxyFailed {
            reason: format!("{}: failed to parse response: {}", context, e),
        })
    }

    /// Send a POST request with a JSON body, check the status, and deserialize the response.
    async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        context: &str,
    ) -> Result<T, WorkerError> {
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| WorkerError::LlmProxyFailed {
                reason: format!("{}: {}", context, e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkerError::LlmProxyFailed {
                reason: format!("{}: orchestrator returned {}: {}", context, status, body),
            });
        }

        resp.json().await.map_err(|e| WorkerError::LlmProxyFailed {
            reason: format!("{}: failed to parse response: {}", context, e),
        })
    }

    /// Fetch the job description from the orchestrator.
    pub async fn get_job(&self) -> Result<JobDescription, WorkerError> {
        self.get_json("job", "GET /job").await
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

        let proxy_resp: ProxyCompletionResponse = self
            .post_json("llm/complete", &proxy_req, "LLM complete")
            .await?;

        Ok(CompletionResponse {
            content: proxy_resp.content,
            input_tokens: proxy_resp.input_tokens,
            output_tokens: proxy_resp.output_tokens,
            finish_reason: parse_finish_reason(&proxy_resp.finish_reason),
            response_id: None,
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

        let proxy_resp: ProxyToolCompletionResponse = self
            .post_json("llm/complete_with_tools", &proxy_req, "LLM tool complete")
            .await?;

        Ok(ToolCompletionResponse {
            content: proxy_resp.content,
            tool_calls: proxy_resp.tool_calls,
            input_tokens: proxy_resp.input_tokens,
            output_tokens: proxy_resp.output_tokens,
            finish_reason: parse_finish_reason(&proxy_resp.finish_reason),
            response_id: None,
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

    /// Post a job event to the orchestrator (fire-and-forget style, logs on failure).
    pub async fn post_event(&self, payload: &JobEventPayload) {
        let resp = self
            .client
            .post(self.url("event"))
            .bearer_auth(&self.token)
            .json(payload)
            .send()
            .await;

        match resp {
            Ok(r) if !r.status().is_success() => {
                tracing::debug!(
                    job_id = %self.job_id,
                    event_type = %payload.event_type,
                    status = %r.status(),
                    "Job event POST rejected"
                );
            }
            Err(e) => {
                tracing::debug!(
                    job_id = %self.job_id,
                    event_type = %payload.event_type,
                    "Job event POST failed: {}", e
                );
            }
            _ => {}
        }
    }

    /// Poll the orchestrator for a follow-up prompt.
    ///
    /// Returns `None` if no prompt is available (204 No Content).
    pub async fn poll_prompt(&self) -> Result<Option<PromptResponse>, WorkerError> {
        let resp = self
            .client
            .get(self.url("prompt"))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.to_string(),
            })?;

        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(WorkerError::OrchestratorRejected {
                job_id: self.job_id,
                reason: format!("prompt endpoint returned {}", resp.status()),
            });
        }

        let prompt: PromptResponse =
            resp.json().await.map_err(|e| WorkerError::LlmProxyFailed {
                reason: format!("failed to parse prompt response: {}", e),
            })?;

        Ok(Some(prompt))
    }

    /// Signal job completion to the orchestrator.
    pub async fn report_complete(&self, report: &CompletionReport) -> Result<(), WorkerError> {
        let _: serde_json::Value = self
            .post_json("complete", report, "report complete")
            .await?;
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

    #[test]
    fn test_job_description_without_permissions() {
        // Backward compat: missing skill_permissions field deserializes to empty vec
        let json = r#"{"title":"Test","description":"desc","project_dir":null}"#;
        let job: JobDescription = serde_json::from_str(json).unwrap();
        assert_eq!(job.title, "Test");
        assert!(job.skill_permissions.is_empty());
    }

    #[test]
    fn test_job_description_with_permissions() {
        let json = r#"{
            "title": "Test",
            "description": "desc",
            "project_dir": null,
            "skill_permissions": [
                {
                    "tool_name": "shell",
                    "trust": "verified",
                    "patterns": [
                        {"kind": "Shell", "command": "cargo *"}
                    ]
                }
            ]
        }"#;
        let job: JobDescription = serde_json::from_str(json).unwrap();
        assert_eq!(job.skill_permissions.len(), 1);
        assert_eq!(job.skill_permissions[0].tool_name, "shell");
        assert_eq!(job.skill_permissions[0].patterns.len(), 1);
    }
}
