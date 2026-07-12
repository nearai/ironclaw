use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::{
    OpenAiCompatAuthenticatedCaller, OpenAiCompatHttpError, OpenAiCompatIdempotencyKey,
    OpenAiCompatRouteSurface, OpenAiCompatRouterState, OpenAiModelListResponse, OpenAiResponseId,
    OpenAiResponseObject,
};

pub async fn chat_completions(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, OpenAiCompatHttpError> {
    let Some(Extension(caller)) = caller else {
        return Err(OpenAiCompatHttpError::from_kind(
            401,
            false,
            crate::OpenAiCompatErrorKind::Authentication,
            None,
        ));
    };
    let Some(workflow) = state.chat_completions() else {
        return Err(OpenAiCompatHttpError::not_wired());
    };
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let request = crate::chat_workflow::parse_chat_request(&body)?;
    workflow
        .handle_chat_request(caller, request, &body, idempotency_key)
        .await
}

/// Handles `GET /v1/models` and its `/api/v1/models` alias.
///
/// Authentication is enforced first (mirroring `chat_completions`): a missing
/// caller fails closed with `401` before the catalog is consulted. When no
/// catalog is wired, the route fails closed with `501`.
pub async fn models_list(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
) -> Result<Json<OpenAiModelListResponse>, OpenAiCompatHttpError> {
    let caller = require_caller(caller)?;
    let Some(catalog) = state.models() else {
        return Err(OpenAiCompatHttpError::not_wired());
    };
    let entries = catalog.list_models(&caller).await?;
    Ok(Json(crate::models_catalog::model_list_response(entries)))
}

pub async fn responses_api_create(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, OpenAiCompatHttpError> {
    create_response(
        state,
        caller,
        headers,
        body,
        OpenAiCompatRouteSurface::ResponsesApi,
    )
    .await
}

pub async fn responses_v1_create(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, OpenAiCompatHttpError> {
    create_response(
        state,
        caller,
        headers,
        body,
        OpenAiCompatRouteSurface::ResponsesApi,
    )
    .await
}

pub async fn responses_api_retrieve(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    Path(response_id): Path<String>,
) -> Result<Json<OpenAiResponseObject>, OpenAiCompatHttpError> {
    retrieve_response(state, caller, response_id).await
}

pub async fn responses_v1_retrieve(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    Path(response_id): Path<String>,
) -> Result<Json<OpenAiResponseObject>, OpenAiCompatHttpError> {
    retrieve_response(state, caller, response_id).await
}

pub async fn responses_api_cancel(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    Path(response_id): Path<String>,
) -> Result<Json<OpenAiResponseObject>, OpenAiCompatHttpError> {
    cancel_response(state, caller, response_id).await
}

pub async fn responses_v1_cancel(
    State(state): State<OpenAiCompatRouterState>,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    Path(response_id): Path<String>,
) -> Result<Json<OpenAiResponseObject>, OpenAiCompatHttpError> {
    cancel_response(state, caller, response_id).await
}

fn require_caller(
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
) -> Result<OpenAiCompatAuthenticatedCaller, OpenAiCompatHttpError> {
    caller.map(|Extension(caller)| caller).ok_or_else(|| {
        OpenAiCompatHttpError::from_kind(
            401,
            false,
            crate::OpenAiCompatErrorKind::Authentication,
            None,
        )
    })
}

async fn create_response(
    state: OpenAiCompatRouterState,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    headers: HeaderMap,
    body: Bytes,
    surface: OpenAiCompatRouteSurface,
) -> Result<Response, OpenAiCompatHttpError> {
    let Some(workflow) = state.responses() else {
        return Err(OpenAiCompatHttpError::not_wired());
    };
    let caller = require_caller(caller)?;
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let request = crate::responses_workflow::parse_response_create_request(&body)?;
    workflow
        .handle_response_create_request(caller, request, &body, idempotency_key, surface)
        .await
}

async fn retrieve_response(
    state: OpenAiCompatRouterState,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    response_id: String,
) -> Result<Json<OpenAiResponseObject>, OpenAiCompatHttpError> {
    let Some(workflow) = state.responses() else {
        return Err(OpenAiCompatHttpError::not_wired());
    };
    let caller = require_caller(caller)?;
    let response_id = OpenAiResponseId::new(response_id)
        .map_err(|_| OpenAiCompatHttpError::not_found(Some("response_id".to_string())))?;
    workflow
        .retrieve_response(caller, response_id)
        .await
        .map(Json)
}

async fn cancel_response(
    state: OpenAiCompatRouterState,
    caller: Option<Extension<OpenAiCompatAuthenticatedCaller>>,
    response_id: String,
) -> Result<Json<OpenAiResponseObject>, OpenAiCompatHttpError> {
    let Some(workflow) = state.responses() else {
        return Err(OpenAiCompatHttpError::not_wired());
    };
    let caller = require_caller(caller)?;
    let response_id = OpenAiResponseId::new(response_id)
        .map_err(|_| OpenAiCompatHttpError::not_found(Some("response_id".to_string())))?;
    workflow
        .cancel_response(caller, response_id)
        .await
        .map(Json)
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
    OpenAiCompatIdempotencyKey::new(value)
        .map(Some)
        .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("idempotency_key".to_string())))
}
