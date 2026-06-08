use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;

const MAX_IDEMPOTENCY_KEY_LEN: usize = 256;

use crate::{
    OpenAiChatCompletionResponse, OpenAiCompatAuthenticatedCaller, OpenAiCompatHttpError,
    OpenAiCompatIdempotencyKey, OpenAiCompatRouterState,
};

pub async fn chat_completions(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<OpenAiChatCompletionResponse>, OpenAiCompatHttpError> {
    let Some(workflow) = state.chat_completions() else {
        return Err(OpenAiCompatHttpError::not_wired());
    };
    let Some(Extension(caller)) = caller else {
        return Err(OpenAiCompatHttpError::from_kind(
            401,
            false,
            crate::OpenAiCompatErrorKind::Authentication,
            None,
        ));
    };
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    workflow
        .complete_chat(caller, &body, idempotency_key)
        .await
        .map(Json)
}

pub async fn responses_api_create(
    State(_state): State<OpenAiCompatRouterState>,
) -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::not_wired()
}

pub async fn responses_v1_create(
    State(_state): State<OpenAiCompatRouterState>,
) -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::not_wired()
}

pub async fn responses_api_retrieve(
    State(_state): State<OpenAiCompatRouterState>,
) -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::not_wired()
}

pub async fn responses_v1_retrieve(
    State(_state): State<OpenAiCompatRouterState>,
) -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::not_wired()
}

pub async fn responses_api_cancel(
    State(_state): State<OpenAiCompatRouterState>,
) -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::not_wired()
}

pub async fn responses_v1_cancel(
    State(_state): State<OpenAiCompatRouterState>,
) -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::not_wired()
}

fn idempotency_key_from_headers(
    headers: &HeaderMap,
) -> Result<Option<OpenAiCompatIdempotencyKey>, OpenAiCompatHttpError> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("idempotency_key".to_string())))?;
    if value.len() > MAX_IDEMPOTENCY_KEY_LEN || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "idempotency_key".to_string(),
        )));
    }
    OpenAiCompatIdempotencyKey::new(value)
        .map(Some)
        .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("idempotency_key".to_string())))
}
