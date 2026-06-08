#![forbid(unsafe_code)]

//! Reborn-native OpenAI-compatible API contract surface.
//!
//! The crate owns DTOs, route descriptors, and a sanitized error envelope for
//! the OpenAI-compatible Chat Completions and Responses surfaces. The optional
//! `openai-compat-beta` feature exposes fail-closed axum handlers so host
//! composition can mount the route fragment without routing through the v1
//! gateway. Real ProductWorkflow wiring lands in later slices.

mod chat;
mod descriptors;
mod error;
#[cfg(feature = "openai-compat-beta")]
mod handlers;
mod refs;
mod responses;
#[cfg(feature = "openai-compat-beta")]
mod router;

pub use chat::{
    OpenAiChatChoice, OpenAiChatCompletionChunk, OpenAiChatCompletionRequest,
    OpenAiChatCompletionResponse, OpenAiChatDelta, OpenAiChatFinishReason, OpenAiChatFunction,
    OpenAiChatMessage, OpenAiChatMessageRole, OpenAiChatStreamChoice, OpenAiChatTool,
    OpenAiChatToolCall, OpenAiChatToolCallDelta, OpenAiChatToolCallFunction,
    OpenAiChatToolCallFunctionDelta, OpenAiChatToolKind, OpenAiUsage,
};
pub use descriptors::{
    OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS, OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE,
    OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM, OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE, OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL, OPENAI_COMPAT_ROUTE_CHAT_COMPLETIONS,
    OPENAI_COMPAT_ROUTE_RESPONSES_API_CANCEL, OPENAI_COMPAT_ROUTE_RESPONSES_API_CREATE,
    OPENAI_COMPAT_ROUTE_RESPONSES_API_RETRIEVE, OPENAI_COMPAT_ROUTE_RESPONSES_V1_CANCEL,
    OPENAI_COMPAT_ROUTE_RESPONSES_V1_CREATE, OPENAI_COMPAT_ROUTE_RESPONSES_V1_RETRIEVE,
    openai_compat_routes,
};
pub use error::{
    OpenAiCompatError, OpenAiCompatErrorCode, OpenAiCompatErrorKind, OpenAiCompatErrorResponse,
    OpenAiCompatErrorType, OpenAiCompatHttpError,
};
#[cfg(feature = "openai-compat-beta")]
pub use handlers::{
    chat_completions, responses_api_cancel, responses_api_create, responses_api_retrieve,
    responses_v1_cancel, responses_v1_create, responses_v1_retrieve,
};
pub use refs::{
    InMemoryOpenAiCompatRefStore, OpenAiChatCompletionId, OpenAiCompatActorScope,
    OpenAiCompatBindInternalRefs, OpenAiCompatIdempotencyConflict, OpenAiCompatIdempotencyKey,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatPublicId, OpenAiCompatRefError, OpenAiCompatRefLookup, OpenAiCompatRefOperation,
    OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore,
    OpenAiCompatRequestFingerprint, OpenAiCompatResourceBinding, OpenAiCompatResourceKind,
    OpenAiCompatResourceMapping, OpenAiCompatRouteSurface, OpenAiCompatTurnRunRef,
    OpenAiResponseId,
};
pub use responses::{
    OpenAiResponseErrorObject, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseStatus, OpenAiResponseUsage,
    OpenAiResponsesCreateRequest, OpenAiResponsesInput, OpenAiResponsesInputItem,
    OpenAiResponsesMessageRole,
};
#[cfg(feature = "openai-compat-beta")]
pub use router::openai_compat_router;
