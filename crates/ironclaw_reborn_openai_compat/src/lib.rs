#![forbid(unsafe_code)]

//! Reborn-native OpenAI-compatible API contract surface.
//!
//! The crate owns DTOs, route descriptors, and a sanitized error envelope for
//! the OpenAI-compatible Chat Completions and Responses surfaces. The optional
//! `openai-compat-beta` feature exposes axum route fragments for host
//! composition without routing through the v1 gateway. By default the router is
//! fail-closed; host composition can inject ProductWorkflow-backed Chat,
//! Responses, and projection-streaming services for the wired Reborn slices.

#[cfg(feature = "openai-compat-beta")]
mod ack_helpers;
mod chat;
#[cfg(feature = "openai-compat-beta")]
mod chat_workflow;
#[cfg(feature = "openai-compat-beta")]
mod content_parts;
mod descriptors;
mod error;
#[cfg(feature = "openai-compat-beta")]
mod handlers;
#[cfg(feature = "openai-compat-beta")]
mod identity;
#[cfg(feature = "openai-compat-beta")]
mod projection_helpers;
mod refs;
mod responses;
#[cfg(feature = "openai-compat-beta")]
mod responses_workflow;
#[cfg(feature = "openai-compat-beta")]
mod router;
#[cfg(feature = "openai-compat-beta")]
mod streaming;

pub use chat::{
    OpenAiChatChoice, OpenAiChatCompletionChunk, OpenAiChatCompletionRequest,
    OpenAiChatCompletionResponse, OpenAiChatDelta, OpenAiChatFinishReason, OpenAiChatFunction,
    OpenAiChatMessage, OpenAiChatMessageRole, OpenAiChatStreamChoice, OpenAiChatTool,
    OpenAiChatToolCall, OpenAiChatToolCallDelta, OpenAiChatToolCallFunction,
    OpenAiChatToolCallFunctionDelta, OpenAiChatToolKind, OpenAiUsage,
};
#[cfg(feature = "openai-compat-beta")]
pub use chat_workflow::{
    OPENAI_COMPAT_CONVERSATION_PREFIX, OpenAiChatCompletionProjection,
    OpenAiChatCompletionProjectionReader, OpenAiChatCompletionProjectionRequest,
    OpenAiChatCompletionsWorkflow, OpenAiChatModelOnlyTools, OpenAiCompatAuthenticatedCaller,
    OpenAiCompatInboundAttachmentSubmit,
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
#[cfg(feature = "openai-compat-beta")]
pub use identity::{
    OPENAI_COMPAT_ACTOR_KIND, OPENAI_COMPAT_ADAPTER_ID, OPENAI_COMPAT_INSTALLATION_ID,
};
pub use refs::{
    InMemoryOpenAiCompatRefStore, OpenAiChatCompletionId, OpenAiCompatActorScope,
    OpenAiCompatBindInternalRefs, OpenAiCompatIdempotencyConflict, OpenAiCompatIdempotencyKey,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatPublicId, OpenAiCompatRecordAcceptedAck, OpenAiCompatRefError,
    OpenAiCompatRefLookup, OpenAiCompatRefOperation, OpenAiCompatRefReservation,
    OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore, OpenAiCompatRequestFingerprint,
    OpenAiCompatResourceBinding, OpenAiCompatResourceKind, OpenAiCompatResourceMapping,
    OpenAiCompatRouteSurface, OpenAiCompatTurnRunRef, OpenAiResponseId, unix_timestamp_now,
};
pub use responses::{
    OpenAiResponseErrorObject, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseStatus, OpenAiResponseUsage,
    OpenAiResponsesCreateRequest, OpenAiResponsesInput, OpenAiResponsesInputItem,
    OpenAiResponsesMessageRole,
};
#[cfg(feature = "openai-compat-beta")]
pub use responses_workflow::{
    OpenAiResponseProjection, OpenAiResponseReadRequest, OpenAiResponseWaitRequest,
    OpenAiResponsesProjectionReader, OpenAiResponsesWorkflow,
};
#[cfg(feature = "openai-compat-beta")]
pub use router::{OpenAiCompatRouterState, openai_compat_router, openai_compat_router_with_state};
#[cfg(feature = "openai-compat-beta")]
pub use streaming::{
    OpenAiChatProjectionStreamRequest, OpenAiCompatProjectionStreamer,
    OpenAiResponseProjectionStreamRequest,
};
