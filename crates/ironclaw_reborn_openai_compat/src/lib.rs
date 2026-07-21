#![forbid(unsafe_code)]

//! Reborn-native OpenAI-compatible API contract surface.
//!
//! The crate owns DTOs, route descriptors, and a sanitized error envelope for
//! the OpenAI-compatible Chat Completions and Responses surfaces. It exposes
//! axum route fragments for host composition without routing through the v1
//! gateway. By default the router is
//! fail-closed; host composition can inject ProductWorkflow-backed Chat,
//! Responses, and projection-streaming services for the wired Reborn slices.

mod ack_helpers;
mod chat;
mod chat_workflow;
mod content_parts;
mod cost;
mod descriptors;
mod error;
mod external_tools;
mod handlers;
mod identity;
mod model_validation;
mod models;
mod models_catalog;
mod projection_helpers;
mod refs;
// Durable filesystem-backed ref store. Gated behind `storage` so the
// contract-only surface stays free of the `ironclaw_filesystem` dependency.
mod refs_storage;
mod responses;
mod responses_workflow;
mod router;
mod streaming;

pub use chat::{
    OpenAiChatChoice, OpenAiChatCompletionChunk, OpenAiChatCompletionRequest,
    OpenAiChatCompletionResponse, OpenAiChatDelta, OpenAiChatFinishReason, OpenAiChatFunction,
    OpenAiChatMessage, OpenAiChatMessageRole, OpenAiChatStreamChoice, OpenAiChatTool,
    OpenAiChatToolCall, OpenAiChatToolCallDelta, OpenAiChatToolCallFunction,
    OpenAiChatToolCallFunctionDelta, OpenAiChatToolKind, OpenAiPromptTokensDetails, OpenAiUsage,
};
pub use chat_workflow::{
    OPENAI_COMPAT_CONVERSATION_PREFIX, OpenAiChatCompletionProjection,
    OpenAiChatCompletionProjectionReader, OpenAiChatCompletionProjectionRequest,
    OpenAiChatCompletionsWorkflow, OpenAiChatModelOnlyTools, OpenAiCompatAuthenticatedCaller,
    OpenAiCompatInboundAttachmentSubmit,
};
pub use cost::OpenAiCompatCost;
pub use descriptors::{
    OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS, OPENAI_COMPAT_PATTERN_MODELS_API_LIST,
    OPENAI_COMPAT_PATTERN_MODELS_LIST, OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE,
    OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM, OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE, OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM,
    OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL, OPENAI_COMPAT_ROUTE_CHAT_COMPLETIONS,
    OPENAI_COMPAT_ROUTE_MODELS_API_LIST, OPENAI_COMPAT_ROUTE_MODELS_LIST,
    OPENAI_COMPAT_ROUTE_RESPONSES_API_CANCEL, OPENAI_COMPAT_ROUTE_RESPONSES_API_CREATE,
    OPENAI_COMPAT_ROUTE_RESPONSES_API_RETRIEVE, OPENAI_COMPAT_ROUTE_RESPONSES_V1_CANCEL,
    OPENAI_COMPAT_ROUTE_RESPONSES_V1_CREATE, OPENAI_COMPAT_ROUTE_RESPONSES_V1_RETRIEVE,
    openai_compat_routes,
};
pub use error::{
    OpenAiCompatError, OpenAiCompatErrorCode, OpenAiCompatErrorKind, OpenAiCompatErrorResponse,
    OpenAiCompatErrorType, OpenAiCompatHttpError,
};
pub use external_tools::{
    OpenAiCompatExternalToolResume, OpenAiCompatExternalToolResumeRequest,
    OpenAiCompatExternalToolSpec, OpenAiCompatExternalToolStore,
};
pub use handlers::{
    chat_completions, models_list, responses_api_cancel, responses_api_create,
    responses_api_retrieve, responses_v1_cancel, responses_v1_create, responses_v1_retrieve,
};
pub use identity::{
    OPENAI_COMPAT_ACTOR_KIND, OPENAI_COMPAT_ADAPTER_ID, OPENAI_COMPAT_INSTALLATION_ID,
};
pub use models::{OpenAiModelListResponse, OpenAiModelObject};
pub use models_catalog::{OpenAiCompatModelCatalog, OpenAiCompatModelEntry};
pub use refs::{
    OpenAiChatCompletionId, OpenAiCompatActorScope, OpenAiCompatBindInternalRefs,
    OpenAiCompatIdempotencyConflict, OpenAiCompatIdempotencyKey, OpenAiCompatInternalRefs,
    OpenAiCompatMarkExternalToolResumeCompleted, OpenAiCompatProductActionRef,
    OpenAiCompatProjectionRef, OpenAiCompatPublicId, OpenAiCompatRecordAcceptedAck,
    OpenAiCompatRefError, OpenAiCompatRefLookup, OpenAiCompatRefOperation,
    OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore,
    OpenAiCompatRequestFingerprint, OpenAiCompatResourceBinding, OpenAiCompatResourceKind,
    OpenAiCompatResourceMapping, OpenAiCompatRouteSurface, OpenAiCompatTurnRunRef,
    OpenAiResponseId, unix_timestamp_now,
};
pub use refs_storage::FilesystemOpenAiCompatRefStore;
pub use refs_storage::RebornLibSqlOpenAiCompatRefStore;
pub use refs_storage::RebornPostgresOpenAiCompatRefStore;
pub use responses::{
    OpenAiResponseErrorObject, OpenAiResponseInputTokensDetails, OpenAiResponseObject,
    OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus, OpenAiResponseStatus,
    OpenAiResponseUsage, OpenAiResponsesCreateRequest, OpenAiResponsesInput,
    OpenAiResponsesInputItem, OpenAiResponsesMessageRole,
};
pub use responses_workflow::{
    OpenAiResponseProjection, OpenAiResponseReadRequest, OpenAiResponseWaitRequest,
    OpenAiResponsesProjectionReader, OpenAiResponsesWorkflow,
};
pub use router::{OpenAiCompatRouterState, openai_compat_router, openai_compat_router_with_state};
pub use streaming::{
    OpenAiChatProjectionStreamRequest, OpenAiCompatProjectionStreamer,
    OpenAiResponseProjectionStreamRequest,
};
