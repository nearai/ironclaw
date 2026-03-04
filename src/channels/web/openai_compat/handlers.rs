use std::sync::Arc;

use axum::{Json, body::Bytes, extract::State, http::StatusCode, response::IntoResponse};

use crate::llm::{CompletionRequest, ToolCompletionRequest};

use super::stream::handle_streaming;
use super::translate::{
    apply_named_tool_choice, convert_messages, convert_tool_calls_to_openai, convert_tools,
    finish_reason_str, normalize_tool_choice, parse_stop, validate_model_name,
};
use super::types::{
    OpenAiChatRequest, OpenAiChatResponse, OpenAiChoice, OpenAiContent, OpenAiErrorDetail,
    OpenAiErrorResponse, OpenAiMessage, OpenAiUsage,
};
use crate::channels::web::server::GatewayState;

pub(crate) fn map_llm_error(
    err: crate::error::LlmError,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    let (status, error_type, code) = match &err {
        crate::error::LlmError::AuthFailed { .. }
        | crate::error::LlmError::SessionExpired { .. } => (
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "auth_error",
        ),
        crate::error::LlmError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            "rate_limit",
        ),
        crate::error::LlmError::ContextLengthExceeded { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "context_length_exceeded",
        ),
        crate::error::LlmError::ModelNotAvailable { .. } => (
            StatusCode::NOT_FOUND,
            "invalid_request_error",
            "model_not_found",
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "internal_error",
        ),
    };

    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: err.to_string(),
                error_type: error_type.to_string(),
                param: None,
                code: Some(code.to_string()),
            },
        }),
    )
}

pub(crate) fn openai_error(
    status: StatusCode,
    message: impl Into<String>,
    error_type: &str,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: message.into(),
                error_type: error_type.to_string(),
                param: None,
                code: None,
            },
        }),
    )
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

pub async fn chat_completions_handler(
    State(state): State<Arc<GatewayState>>,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, Json<OpenAiErrorResponse>)> {
    let req: OpenAiChatRequest = serde_json::from_slice(&body).map_err(|e| {
        openai_error(
            StatusCode::BAD_REQUEST,
            format!("Invalid JSON body: {}", e),
            "invalid_request_error",
        )
    })?;

    if !state.chat_rate_limiter.check() {
        return Err(openai_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Please try again later.",
            "rate_limit_error",
        ));
    }

    let llm = state.llm_provider.as_ref().ok_or_else(|| {
        openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM provider not configured",
            "server_error",
        )
    })?;

    if req.messages.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "messages must not be empty",
            "invalid_request_error",
        ));
    }
    if let Err(e) = validate_model_name(&req.model) {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            e,
            "invalid_request_error",
        ));
    }

    let has_tools = req.tools.as_ref().is_some_and(|t| !t.is_empty());
    let stream = req.stream.unwrap_or(false);
    let requested_model = req.model.clone();

    if stream {
        return handle_streaming(llm.clone(), req, has_tools)
            .await
            .map(IntoResponse::into_response);
    }

    // --- Non-streaming path ---

    let messages = convert_messages(&req.messages)
        .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;
    let id = chat_completion_id();
    let created = unix_timestamp();

    if has_tools {
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

        let resp = llm
            .complete_with_tools(tool_req)
            .await
            .map_err(map_llm_error)?;
        let model_name = llm.effective_model_name(Some(requested_model.as_str()));

        let tool_calls_openai = if resp.tool_calls.is_empty() {
            None
        } else {
            Some(convert_tool_calls_to_openai(&resp.tool_calls))
        };

        let response = OpenAiChatResponse {
            id,
            object: "chat.completion",
            created,
            model: model_name,
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: resp.content.clone().map(OpenAiContent::Text),
                    name: None,
                    tool_call_id: None,
                    tool_calls: tool_calls_openai,
                },
                finish_reason: finish_reason_str(resp.finish_reason),
            }],
            usage: OpenAiUsage {
                prompt_tokens: resp.input_tokens,
                completion_tokens: resp.output_tokens,
                total_tokens: resp.input_tokens + resp.output_tokens,
            },
        };

        Ok(Json(response).into_response())
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

        let resp = llm.complete(comp_req).await.map_err(map_llm_error)?;
        let model_name = llm.effective_model_name(Some(requested_model.as_str()));

        let response = OpenAiChatResponse {
            id,
            object: "chat.completion",
            created,
            model: model_name,
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: Some(OpenAiContent::Text(resp.content)),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                finish_reason: finish_reason_str(resp.finish_reason),
            }],
            usage: OpenAiUsage {
                prompt_tokens: resp.input_tokens,
                completion_tokens: resp.output_tokens,
                total_tokens: resp.input_tokens + resp.output_tokens,
            },
        };

        Ok(Json(response).into_response())
    }
}

pub async fn models_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let llm = state.llm_provider.as_ref().ok_or_else(|| {
        openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM provider not configured",
            "server_error",
        )
    })?;

    let model_name = llm.active_model_name();
    let created = unix_timestamp();

    // Try to fetch available models from the provider
    let models = match llm.list_models().await {
        Ok(names) if !names.is_empty() => names
            .into_iter()
            .map(|name| {
                serde_json::json!({
                    "id": name,
                    "object": "model",
                    "created": created,
                    "owned_by": "ironclaw"
                })
            })
            .collect(),
        Ok(_) => {
            // Empty list: fall back to active model
            vec![serde_json::json!({
                "id": model_name,
                "object": "model",
                "created": created,
                "owned_by": "ironclaw"
            })]
        }
        Err(e) => return Err(map_llm_error(e)),
    };

    Ok(Json(serde_json::json!({
        "object": "list",
        "data": models
    })))
}
