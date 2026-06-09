use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::OpenAiChatCompletionsWorkflow;
use crate::descriptors::{
    OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS, OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE,
    OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM, OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE, OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL,
};
use crate::handlers;

#[derive(Clone, Default)]
pub struct OpenAiCompatRouterState {
    /// Wired by host composition when `openai-compat-beta` is active.
    /// When `None`, chat completions requests return 501 fail-closed.
    /// arch-exempt: optional Arc, genuinely optional by design; default
    /// fail-closed behavior is intentional until host composition wires #4444.
    chat_completions: Option<Arc<OpenAiChatCompletionsWorkflow>>,
}

impl OpenAiCompatRouterState {
    pub fn not_wired() -> Self {
        Self::default()
    }

    pub fn with_chat_completions(chat_completions: Arc<OpenAiChatCompletionsWorkflow>) -> Self {
        Self {
            chat_completions: Some(chat_completions),
        }
    }

    pub(crate) fn chat_completions(&self) -> Option<Arc<OpenAiChatCompletionsWorkflow>> {
        self.chat_completions.clone()
    }
}

pub fn openai_compat_router() -> Router {
    openai_compat_router_with_state(OpenAiCompatRouterState::not_wired())
}

pub fn openai_compat_router_with_state(state: OpenAiCompatRouterState) -> Router {
    openai_compat_routes().with_state(state)
}

fn openai_compat_routes() -> Router<OpenAiCompatRouterState> {
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
