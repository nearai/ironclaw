use std::sync::Arc;

use axum::{
    Json,
    http::{HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};

use crate::llm::{CompletionRequest, FinishReason, ToolCompletionRequest};

use super::handlers::{map_llm_error, openai_error};
use super::translate::{
    apply_named_tool_choice, convert_messages, convert_tools, finish_reason_str,
    normalize_tool_choice, parse_stop,
};
use super::types::{
    OpenAiChatChunk, OpenAiChatRequest, OpenAiChunkChoice, OpenAiDelta, OpenAiErrorResponse,
    OpenAiToolCallDelta, OpenAiToolCallFunctionDelta,
};

/// Handle streaming responses.
///
/// The current `LlmProvider` returns complete responses (no streaming method).
/// We execute the LLM call first, then simulate chunked delivery by splitting
/// the response into word-boundary chunks. This ensures LLM failures return
/// proper HTTP errors instead of SSE error events. True token streaming can be
/// added later by extending `LlmProvider` with a `complete_stream()` method.
pub(crate) async fn handle_streaming(
    llm: Arc<dyn crate::llm::LlmProvider>,
    req: OpenAiChatRequest,
    has_tools: bool,
) -> Result<Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    let messages = convert_messages(&req.messages)
        .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;

    let requested_model = req.model.clone();
    let id = chat_completion_id();
    let created = unix_timestamp();

    // Execute the LLM call before starting the SSE stream.
    // Since streaming is simulated (LlmProvider returns complete responses),
    // this lets us return proper HTTP errors on failure.
    enum LlmResult {
        Simple(crate::llm::CompletionResponse),
        WithTools(crate::llm::ToolCompletionResponse),
    }

    let llm_result = if has_tools {
        let tools = convert_tools(req.tools.as_deref().unwrap_or(&[]));
        let parsed_choice = req
            .tool_choice
            .as_ref()
            .map(normalize_tool_choice)
            .transpose()
            .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?
            .flatten();
        let (tools, final_choice) = apply_named_tool_choice(tools, parsed_choice)
            .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;
        let mut tool_req = ToolCompletionRequest::new(messages, tools).with_model(req.model);
        if let Some(t) = req.temperature {
            tool_req = tool_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            tool_req = tool_req.with_max_tokens(mt);
        }
        if let Some(choice) = final_choice {
            tool_req = tool_req.with_tool_choice(choice);
        }
        LlmResult::WithTools(
            llm.complete_with_tools(tool_req)
                .await
                .map_err(map_llm_error)?,
        )
    } else {
        let mut comp_req = CompletionRequest::new(messages).with_model(req.model);
        if let Some(t) = req.temperature {
            comp_req = comp_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            comp_req = comp_req.with_max_tokens(mt);
        }
        if let Some(ref stop_val) = req.stop {
            comp_req.stop_sequences = parse_stop(stop_val);
        }
        LlmResult::Simple(llm.complete(comp_req).await.map_err(map_llm_error)?)
    };
    let model_name = llm.effective_model_name(Some(requested_model.as_str()));

    // LLM succeeded — emit the response as SSE chunks
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);

    tokio::spawn(async move {
        // Send initial chunk with role
        let role_chunk = OpenAiChatChunk {
            id: id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model_name.clone(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        };
        let data = serde_json::to_string(&role_chunk).unwrap_or_default();
        let _ = tx.send(Ok(Event::default().data(data))).await;

        match llm_result {
            LlmResult::WithTools(resp) => {
                // Stream content chunks
                if let Some(ref content) = resp.content {
                    stream_content_chunks(&tx, &id, created, &model_name, content).await;
                }

                // Stream tool calls
                if !resp.tool_calls.is_empty() {
                    let deltas: Vec<OpenAiToolCallDelta> = resp
                        .tool_calls
                        .iter()
                        .enumerate()
                        .map(|(i, tc)| OpenAiToolCallDelta {
                            index: i as u32,
                            id: Some(tc.id.clone()),
                            call_type: Some("function".to_string()),
                            function: Some(OpenAiToolCallFunctionDelta {
                                name: Some(tc.name.clone()),
                                arguments: Some(
                                    serde_json::to_string(&tc.arguments).unwrap_or_default(),
                                ),
                            }),
                        })
                        .collect();

                    let chunk = OpenAiChatChunk {
                        id: id.clone(),
                        object: "chat.completion.chunk",
                        created,
                        model: model_name.clone(),
                        choices: vec![OpenAiChunkChoice {
                            index: 0,
                            delta: OpenAiDelta {
                                role: None,
                                content: None,
                                tool_calls: Some(deltas),
                            },
                            finish_reason: None,
                        }],
                    };
                    let data = serde_json::to_string(&chunk).unwrap_or_default();
                    let _ = tx.send(Ok(Event::default().data(data))).await;
                }

                // Final chunk with finish_reason
                send_finish_chunk(&tx, &id, created, &model_name, resp.finish_reason).await;
            }
            LlmResult::Simple(resp) => {
                stream_content_chunks(&tx, &id, created, &model_name, &resp.content).await;
                send_finish_chunk(&tx, &id, created, &model_name, resp.finish_reason).await;
            }
        }

        // Send [DONE] sentinel
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let sse = Sse::new(stream).keep_alive(KeepAlive::new().text(""));
    let mut response = sse.into_response();
    response.headers_mut().insert(
        "x-ironclaw-streaming",
        HeaderValue::from_static("simulated"),
    );
    Ok(response)
}

/// Split content into word-boundary chunks and send as SSE events.
async fn stream_content_chunks(
    tx: &tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
    id: &str,
    created: u64,
    model: &str,
    content: &str,
) {
    // Split on word boundaries, grouping ~20 chars per chunk
    let mut buf = String::new();
    for word in content.split_inclusive(char::is_whitespace) {
        buf.push_str(word);
        if buf.len() >= 20 {
            let chunk = OpenAiChatChunk {
                id: id.to_string(),
                object: "chat.completion.chunk",
                created,
                model: model.to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiDelta {
                        role: None,
                        content: Some(buf.clone()),
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
            };
            let data = serde_json::to_string(&chunk).unwrap_or_default();
            if tx.send(Ok(Event::default().data(data))).await.is_err() {
                return;
            }
            buf.clear();
        }
    }
    // Flush remaining
    if !buf.is_empty() {
        let chunk = OpenAiChatChunk {
            id: id.to_string(),
            object: "chat.completion.chunk",
            created,
            model: model.to_string(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: Some(buf),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        };
        let data = serde_json::to_string(&chunk).unwrap_or_default();
        let _ = tx.send(Ok(Event::default().data(data))).await;
    }
}

async fn send_finish_chunk(
    tx: &tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
    id: &str,
    created: u64,
    model: &str,
    reason: FinishReason,
) {
    let chunk = OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some(finish_reason_str(reason)),
        }],
    };
    let data = serde_json::to_string(&chunk).unwrap_or_default();
    let _ = tx.send(Ok(Event::default().data(data))).await;
}

fn chat_completion_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4().simple())
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
