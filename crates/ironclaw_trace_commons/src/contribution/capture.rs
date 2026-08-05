//! Capture: turning conversation messages and recorded LLM traces into a
//! `RawTraceContribution`, then into a redacted envelope.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use ironclaw_llm::recording::{TraceFile, TraceResponse};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContribution {
    pub trace_id: Uuid,
    pub submission_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub ironclaw: IronclawTraceMetadata,
    pub consent: ConsentMetadata,
    pub contributor: ContributorMetadata,
    pub events: Vec<RawTraceContributionEvent>,
    pub outcome: OutcomeMetadata,
    pub replay: ReplayMetadata,
    pub embedding_analysis: Option<EmbeddingAnalysisMetadata>,
    pub value: ValueMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContributionEvent {
    pub event_id: Uuid,
    pub event_type: TraceContributionEventType,
    pub timestamp: DateTime<Utc>,
    pub content: Option<String>,
    pub structured_payload: Value,
    pub tool_name: Option<String>,
    pub latency_ms: Option<u64>,
    pub token_counts: Option<TokenCounts>,
    pub cost_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceCaptureTurn {
    pub user_input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RawTraceCaptureToolCall>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceCaptureToolCall {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

pub fn capture_turns_from_conversation_messages(
    messages: &[crate::ConversationMessage],
) -> Vec<RawTraceCaptureTurn> {
    let mut turns = Vec::new();
    let mut iter = messages.iter().peekable();

    while let Some(message) = iter.next() {
        if message.role == "user" {
            let mut turn = RawTraceCaptureTurn {
                user_input: message.content.clone(),
                response: None,
                tool_calls: Vec::new(),
                started_at: message.created_at,
                completed_at: None,
                state: Some("Completed".to_string()),
            };

            if let Some(next) = iter.peek()
                && next.role == "tool_calls"
                && let Some(tool_message) = iter.next()
            {
                turn.tool_calls = parse_capture_tool_calls(&tool_message.content);
            }

            if let Some(next) = iter.peek()
                && next.role == "assistant"
                && let Some(assistant_message) = iter.next()
            {
                turn.response = Some(assistant_message.content.clone());
                turn.completed_at = Some(assistant_message.created_at);
            }

            if turn.response.is_none() {
                turn.state = Some("Failed".to_string());
            }
            turns.push(turn);
        } else if message.role == "assistant" {
            turns.push(RawTraceCaptureTurn {
                user_input: String::new(),
                response: Some(message.content.clone()),
                tool_calls: Vec::new(),
                started_at: message.created_at,
                completed_at: Some(message.created_at),
                state: Some("Completed".to_string()),
            });
        }
    }

    turns
}

pub(crate) fn parse_capture_tool_calls(content: &str) -> Vec<RawTraceCaptureToolCall> {
    let value = match serde_json::from_str::<Value>(content) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let calls = match value {
        Value::Array(calls) => calls,
        Value::Object(mut obj) => match obj.remove("calls") {
            Some(Value::Array(calls)) => calls,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    calls
        .into_iter()
        .map(|call| RawTraceCaptureToolCall {
            name: call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            result_preview: call
                .get("result_preview")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            error: call
                .get("error")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            rationale: call
                .get("rationale")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct RecordedTraceContributionOptions {
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
    pub consent_scopes: Vec<ConsentScope>,
    pub channel: TraceChannel,
    pub engine_version: Option<String>,
    pub feature_flags: BTreeMap<String, String>,
    pub pseudonymous_contributor_id: Option<String>,
    pub tenant_scope_ref: Option<String>,
    pub credit_account_ref: Option<String>,
}

impl Default for RecordedTraceContributionOptions {
    fn default() -> Self {
        Self {
            include_message_text: false,
            include_tool_payloads: false,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            channel: TraceChannel::Cli,
            engine_version: None,
            feature_flags: BTreeMap::new(),
            pseudonymous_contributor_id: None,
            tenant_scope_ref: None,
            credit_account_ref: None,
        }
    }
}

impl RecordedTraceContributionOptions {
    pub fn set_include_message_text(mut self, include_message_text: bool) -> Self {
        self.include_message_text = include_message_text;
        self
    }

    pub fn set_include_tool_payloads(mut self, include_tool_payloads: bool) -> Self {
        self.include_tool_payloads = include_tool_payloads;
        self
    }

    pub fn set_consent_scopes(mut self, consent_scopes: Vec<ConsentScope>) -> Self {
        self.consent_scopes = consent_scopes;
        self
    }
}

impl RawTraceContribution {
    pub fn from_recorded_trace(
        trace: &TraceFile,
        options: RecordedTraceContributionOptions,
    ) -> Self {
        let created_at = Utc::now();
        let mut events = Vec::new();
        let mut required_tools = BTreeSet::new();

        for step in &trace.steps {
            match &step.response {
                TraceResponse::UserInput { content } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        event_type: TraceContributionEventType::UserMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        latency_ms: None,
                        token_counts: None,
                        cost_usd: None,
                    });
                }
                TraceResponse::Text {
                    content,
                    input_tokens,
                    output_tokens,
                } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        event_type: TraceContributionEventType::AssistantMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        latency_ms: None,
                        token_counts: Some(TokenCounts {
                            input_tokens: *input_tokens,
                            output_tokens: *output_tokens,
                        }),
                        cost_usd: None,
                    });
                }
                TraceResponse::ToolCalls {
                    tool_calls,
                    input_tokens,
                    output_tokens,
                } => {
                    for tool_call in tool_calls {
                        required_tools.insert(tool_call.name.clone());
                        let structured_payload = if options.include_tool_payloads {
                            serde_json::json!({
                                "tool_call_id": tool_call.id,
                                "arguments": tool_call.arguments,
                            })
                        } else {
                            serde_json::json!({
                                "tool_call_id": tool_call.id,
                            })
                        };

                        events.push(RawTraceContributionEvent {
                            event_id: Uuid::new_v4(),
                            event_type: TraceContributionEventType::ToolCall,
                            timestamp: created_at,
                            content: None,
                            structured_payload,
                            tool_name: Some(tool_call.name.clone()),
                            latency_ms: None,
                            token_counts: Some(TokenCounts {
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            }),
                            cost_usd: None,
                        });
                    }
                }
            }

            for expected in &step.expected_tool_results {
                required_tools.insert(expected.name.clone());
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::ToolResult,
                    timestamp: created_at,
                    content: options
                        .include_tool_payloads
                        .then(|| expected.content.clone()),
                    structured_payload: serde_json::json!({
                        "tool_call_id": expected.tool_call_id,
                    }),
                    tool_name: Some(expected.name.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }
        }

        for exchange in &trace.http_exchanges {
            let structured_payload = if options.include_tool_payloads {
                serde_json::json!({
                    "request": {
                        "method": exchange.request.method,
                        "url": exchange.request.url,
                        "headers": exchange.request.headers,
                        "body": exchange.request.body,
                    },
                    "response": {
                        "status": exchange.response.status,
                        "headers": exchange.response.headers,
                    },
                })
            } else {
                serde_json::json!({
                    "request": {
                        "method": exchange.request.method,
                    },
                    "response": {
                        "status": exchange.response.status,
                    },
                })
            };

            events.push(RawTraceContributionEvent {
                event_id: Uuid::new_v4(),
                event_type: TraceContributionEventType::HttpExchange,
                timestamp: created_at,
                content: options
                    .include_tool_payloads
                    .then(|| exchange.response.body.clone()),
                structured_payload,
                tool_name: Some("http".to_string()),
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
            });
        }

        let required_tools: Vec<String> = required_tools.into_iter().collect();

        Self {
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at,
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: options.engine_version,
                feature_flags: options.feature_flags,
                channel: options.channel,
                // Options carry no origin id today; the wire field stays
                // (compat-pinned) for envelope producers that stamp one.
                channel_origin: None,
                model_name: Some(trace.model_name.clone()),
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: options.consent_scopes,
                message_text_included: options.include_message_text,
                tool_payloads_included: options.include_tool_payloads,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: options.pseudonymous_contributor_id,
                tenant_scope_ref: options.tenant_scope_ref,
                credit_account_ref: options.credit_account_ref,
                revocation_handle: Uuid::new_v4(),
            },
            events,
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: !trace.steps.is_empty(),
                required_tools,
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
        }
    }

    pub fn from_capture_turns(
        turns: &[RawTraceCaptureTurn],
        options: RecordedTraceContributionOptions,
    ) -> Self {
        let created_at = Utc::now();
        let mut events = Vec::new();
        let mut required_tools = BTreeSet::new();
        let mut task_success = TaskSuccess::Unknown;

        for turn in turns {
            if !turn.user_input.is_empty() {
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::UserMessage,
                    timestamp: turn.started_at,
                    content: options
                        .include_message_text
                        .then(|| turn.user_input.clone()),
                    structured_payload: serde_json::json!({
                        "state": turn.state,
                    }),
                    tool_name: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            for tool_call in &turn.tool_calls {
                required_tools.insert(tool_call.name.clone());
                let structured_payload = if options.include_tool_payloads {
                    serde_json::json!({
                        "result_preview": tool_call.result_preview,
                        "error": tool_call.error,
                        "rationale": tool_call.rationale,
                    })
                } else {
                    serde_json::json!({
                        "has_result": tool_call.result_preview.is_some(),
                        "has_error": tool_call.error.is_some(),
                    })
                };
                let content = options
                    .include_tool_payloads
                    .then(|| {
                        tool_call
                            .result_preview
                            .as_deref()
                            .or(tool_call.error.as_deref())
                            .unwrap_or("")
                            .to_string()
                    })
                    .filter(|content| !content.is_empty());

                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::ToolCall,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content,
                    structured_payload,
                    tool_name: Some(tool_call.name.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            if let Some(response) = &turn.response {
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::AssistantMessage,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content: options.include_message_text.then(|| response.clone()),
                    structured_payload: Value::Null,
                    tool_name: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            if matches!(turn.state.as_deref(), Some("Failed" | "failed")) {
                task_success = TaskSuccess::Failure;
            } else if task_success == TaskSuccess::Unknown && turn.response.is_some() {
                task_success = TaskSuccess::Success;
            }
        }

        let required_tools: Vec<String> = required_tools.into_iter().collect();

        Self {
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at,
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: options.engine_version,
                feature_flags: options.feature_flags,
                channel: options.channel,
                // Options carry no origin id today; the wire field stays
                // (compat-pinned) for envelope producers that stamp one.
                channel_origin: None,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: options.consent_scopes,
                message_text_included: options.include_message_text,
                tool_payloads_included: options.include_tool_payloads,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: options.pseudonymous_contributor_id,
                tenant_scope_ref: options.tenant_scope_ref,
                credit_account_ref: options.credit_account_ref,
                revocation_handle: Uuid::new_v4(),
            },
            events,
            outcome: OutcomeMetadata::default().set_task_success(task_success),
            replay: ReplayMetadata {
                replayable: !turns.is_empty(),
                required_tools,
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: vec![
                    "Captured from web conversation history; exact tool arguments may be omitted by consent policy.".to_string(),
                ],
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
        }
    }
}
