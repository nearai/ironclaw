use axum::Router;
use axum::routing::{get, post};

use crate::descriptors::{
    OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS, OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE,
    OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM, OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE, OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL,
};
use crate::handlers;

pub fn openai_compat_router() -> Router {
    Router::new()
        .route(
            OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS,
            post(handlers::chat_completions),
        )
        .route(
            OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE,
            post(handlers::responses_api_create),
        )
        .route(
            OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE,
            post(handlers::responses_v1_create),
        )
        .route(
            OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM,
            get(handlers::responses_api_retrieve),
        )
        .route(
            OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM,
            get(handlers::responses_v1_retrieve),
        )
        .route(
            OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL,
            post(handlers::responses_api_cancel),
        )
        .route(
            OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL,
            post(handlers::responses_v1_cancel),
        )
}
