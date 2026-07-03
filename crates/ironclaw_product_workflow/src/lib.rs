//! Product-facing workflow facade for IronClaw Reborn.
//!
//! `ironclaw_product_workflow` sits between product adapters and host-layer
//! Reborn services. It owns the product action orchestration so that adapters
//! (Web, API, CLI, Telegram, etc.) do not each reimplement binding resolution,
//! message staging, idempotency, busy/deferred handling, gate routing, mission
//! routing, and redacted acknowledgements.
//!
//! ## Key types
//!
//! - [`DefaultProductWorkflow`] — top-level orchestrator that implements
//!   [`ironclaw_product_adapters::ProductWorkflow`].
//! - [`InboundTurnService`] / [`DefaultInboundTurnService`] — the narrower
//!   user-message path that coordinates binding + turn submission.
//! - [`ConversationBindingService`] — resolves external adapter refs to
//!   canonical Reborn identifiers.
//! - [`ProductConversationBindingService`] — bridges product adapter bindings to
//!   `ironclaw_conversations` using trusted installation configuration for
//!   tenant/default scope selection.
//! - [`IdempotencyLedger`] — durable action deduplication port.
//! - [`InMemoryIdempotencyLedger`] — local-dev/test ledger with in-flight lease
//!   recovery semantics.
//! - [`ProductInboundAction`] — durable ledger record for inbound actions.

#![forbid(unsafe_code)]

mod action;
mod approval_interaction;
mod auth_continuation;
mod auth_interaction;
mod automation_thread_metadata;
mod binding;
mod binding_ref;
mod channel_connection_resume;
mod command_dispatch;
mod commands;
mod conversation_binding;
mod error;
#[cfg(any(test, feature = "test-support"))]
mod fakes;
// Durable filesystem-backed idempotency ledger (folded in from the former
// `ironclaw_product_workflow_storage` crate). Gated behind `storage` so the
// facade surface stays free of the `ironclaw_filesystem` dependency unless a
// consumer opts into a durable backend.
#[cfg(feature = "storage")]
mod filesystem_ledger;
mod gate_state;
mod in_memory_ledger;
mod inbound_turn;
mod ledger;
mod lifecycle;
mod outbound_delivery;
mod policy;
mod reborn_services;
mod webui_inbound;
mod workflow;

pub use action::{
    ActionDispatchKind, ActionFingerprintKey, ActionPhase, AuthRequestRef, LinkedThreadActionId,
    ProductActionId, ProductCommandName, ProductInboundAction, SourceBindingKey,
};
pub use approval_interaction::{
    ApprovalBlockedTurnRun, ApprovalGateRecord, ApprovalInteractionActionView,
    ApprovalInteractionDecision, ApprovalInteractionReadModel, ApprovalInteractionRejectionKind,
    ApprovalInteractionScope, ApprovalInteractionService, ApprovalLeaseTermsProvider,
    ApprovalResolutionPort, ApprovalResolverPort, ApprovalTurnRunLocator,
    DefaultApprovalInteractionService, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    PendingApprovalInteractionView, PersistentApprovalGranteeResolver,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
    RunStateApprovalInteractionReadModel, approval_gate_ref, approval_request_id_from_gate_ref,
    is_approval_gate_ref,
};
/// Concrete turn-gate resume dispatcher used by the Reborn composition crate to
/// bridge product-auth continuations into the workflow-owned turn boundary.
pub use auth_continuation::ProductAuthTurnGateResumeDispatcher;
pub use auth_interaction::{
    AuthCredentialAccountChoiceView, AuthGateRecord, AuthInteractionChallengeView,
    AuthInteractionDecision, AuthInteractionReadModel, AuthInteractionRejectionKind,
    AuthInteractionScope, AuthInteractionService, AuthInteractionStatus,
    DefaultAuthInteractionService, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, PendingAuthInteractionView, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse, is_auth_gate_ref,
};
pub use automation_thread_metadata::{
    AUTOMATION_TRIGGER_THREAD_SOURCE_TAG, automation_trigger_thread_metadata_json,
    thread_metadata_is_automation_trigger,
};
pub use binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding, route_kind_for_inbound_payload,
};
pub use channel_connection_resume::{
    ChannelConnectionResumeReadModel, ChannelConnectionResumeScope, ChannelConnectionResumeService,
    ChannelPairingBlockedRun, DefaultChannelConnectionResumeService,
    ResumeChannelConnectionRequest, ResumeChannelConnectionResponse,
};
pub use command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService, ProductCommandContext,
    ProductCommandService, RejectingProductCommandAdmissionService, RejectingProductCommandService,
};
pub use commands::{
    LifecycleProductCommandService, ProductCommand, ProductCommandDescriptor, ProductModelCommand,
    product_command_descriptors,
};
pub use conversation_binding::{
    ProductActorBindingPolicy, ProductActorUserResolutionRequest, ProductActorUserResolver,
    ProductConversationBindingService, ProductConversationRouteKey,
    ProductConversationSubjectRouteResolutionRequest, ProductConversationSubjectRouteResolver,
    ProductInstallationKey, ProductInstallationScope, StaticProductActorUserResolver,
    StaticProductInstallationResolver,
};
pub use error::{AuthContinuationRejectionKind, ProductWorkflowError};
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeBeforeInboundPolicy, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, rejecting_reborn_services_error,
};
#[cfg(feature = "storage")]
pub use filesystem_ledger::RebornFilesystemIdempotencyLedger;
#[cfg(feature = "libsql")]
pub use filesystem_ledger::RebornLibSqlIdempotencyLedger;
#[cfg(feature = "postgres")]
pub use filesystem_ledger::RebornPostgresIdempotencyLedger;
pub use in_memory_ledger::InMemoryIdempotencyLedger;
pub use inbound_turn::{
    DefaultInboundTurnService, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
};
pub use ledger::{IdempotencyDecision, IdempotencyLedger};
pub use lifecycle::{
    ChannelConnectionRequirement, LifecycleBlockerRef, LifecycleCommandKind,
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
    LifecycleExtensionSummary, LifecycleExtensionSurfaceKind, LifecycleInstalledExtensionSummary,
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleProductSurfaceContext,
    LifecycleReadinessBlocker, LifecycleSearchExtensionSummary, LifecycleSkillSource,
    LifecycleSkillSummary, UnsupportedLifecycleProductFacade,
};
// Product hosts use this outbound orchestration seam to wire outbound policy
// decisions to adapter rendering without reaching into module internals.
pub use outbound_delivery::{
    ProductOutboundDeliveryError, ProductOutboundDeliveryOutcome, ProductOutboundDeliveryRequest,
    ProductOutboundStatusUpdateFailure, ProductOutboundTargetResolver,
    VerifiedProductOutboundTargetMetadata, prepare_and_render_product_outbound,
};
pub use policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
// Projection/event types that route handlers need to thread through SSE
// (parse the resume cursor, render browser-safe event payloads). Re-exported
// so `ironclaw_webui_v2` consumes them via the facade crate and does not need
// a direct dependency on `ironclaw_product_adapters` — the single-facade
// boundary is enforced by `ironclaw_architecture`.
pub use ironclaw_product_adapters::{
    AuthPromptView, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityDisplayPreviewView, FinalReplyView, GatePromptView, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductProjectionItem, ProductProjectionState, ProductWorkSummaryPhase,
    ProgressKind, ProgressUpdateView, ProjectionCursor,
};
pub use reborn_services::{
    AUTOMATION_LIST_DEFAULT_PAGE_SIZE, AUTOMATION_LIST_MAX_PAGE_SIZE,
    AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE, AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE,
    AutomationListRequest, AutomationProductFacade, ChannelConnectionFacade, CodexLoginStart,
    ConnectableChannelsProductFacade, ExtensionCredentialSetupService,
    ExtensionCredentialStatusRequest, ExtensionCredentialSubmitRequest, FilesystemBrowseReader,
    FsMount, InboundAttachmentLander, InboundAttachmentReader, LlmActiveSelection,
    LlmConfigService, LlmConfigServiceError, LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest,
    LlmProbeResult, LlmProviderView, NearAiAuthProvider, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, OperatorLogsService,
    OperatorServiceLifecycleService, OperatorStatusService, OutboundPreferencesProductFacade,
    ProductAgentBoundCaller, ProjectCaller, ProjectFilesystemReader, ProjectFsEntry,
    ProjectFsEntryKind, ProjectFsError, ProjectFsFile, ProjectFsStat, ProjectService,
    ProjectServiceError, RebornAddMemberRequest, RebornAttachmentBytes, RebornAttachmentRequest,
    RebornAutomationInfo, RebornAutomationMutationResponse, RebornAutomationRecentRunInfo,
    RebornAutomationRecentRunStatus, RebornAutomationRunStatus, RebornAutomationSource,
    RebornAutomationState, RebornCancelRunResponse, RebornChannelConnectAction,
    RebornChannelConnectStrategy, RebornConnectableChannelInfo,
    RebornConnectableChannelListResponse, RebornCreateProjectRequest, RebornCreateThreadResponse,
    RebornDeleteProjectRequest, RebornDeleteThreadRequest, RebornDeleteThreadResponse,
    RebornExtensionActionResponse, RebornExtensionCredentialSetup, RebornExtensionInfo,
    RebornExtensionListResponse, RebornExtensionOnboardingPayload, RebornExtensionOnboardingState,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornExtensionSetupField,
    RebornExtensionSetupSecret, RebornFsListRequest, RebornFsListResponse, RebornFsMountInfo,
    RebornFsMountsResponse, RebornFsReadRequest, RebornFsStatRequest, RebornFsStatResponse,
    RebornGetProjectRequest, RebornGetRunStateRequest, RebornGetRunStateResponse,
    RebornListAutomationsResponse, RebornListMembersRequest, RebornListMembersResponse,
    RebornListProjectsRequest, RebornListProjectsResponse, RebornListThreadsResponse,
    RebornLogEntry, RebornLogLevel, RebornLogQueryRequest, RebornLogQueryResponse,
    RebornOperatorArea, RebornOperatorCommandPlaneResponse, RebornOperatorConfigDiagnostic,
    RebornOperatorConfigDiagnosticSeverity, RebornOperatorConfigEntry,
    RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetRequest, RebornOperatorConfigValidateRequest,
    RebornOperatorConfigValidateResponse, RebornOperatorLogsQuery,
    RebornOperatorServiceLifecycleAction, RebornOperatorServiceLifecycleRequest,
    RebornOperatorSetupRequest, RebornOperatorSetupResponse, RebornOperatorSetupStatus,
    RebornOperatorSetupStep, RebornOperatorSetupStepStatus, RebornOperatorStatusCheck,
    RebornOperatorStatusResponse, RebornOperatorStatusSeverity, RebornOperatorStatusState,
    RebornOperatorSurfaceStatus, RebornOperatorToolCatalog, RebornOperatorToolInfo,
    RebornOutboundDeliveryModality, RebornOutboundDeliveryTargetCapabilities,
    RebornOutboundDeliveryTargetChannel, RebornOutboundDeliveryTargetDescription,
    RebornOutboundDeliveryTargetDisplayName, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornOutboundPreferencesResponse, RebornProjectFsListRequest, RebornProjectFsListResponse,
    RebornProjectFsReadRequest, RebornProjectFsStatRequest, RebornProjectFsStatResponse,
    RebornProjectInfo, RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse,
    RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest, RebornResolveGateResponse,
    RebornResumeGateResponse, RebornServiceLifecycleAction, RebornServiceLifecycleRequest,
    RebornServiceLifecycleResponse, RebornServiceLifecycleState, RebornServices, RebornServicesApi,
    RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillInfo, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornSkillSourceKind, RebornSkillTrustLevel,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse, RebornTraceCreditsResponse,
    RebornTraceHoldAuthorizeResponse, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    SetActiveLlmRequest, SkillsProductFacade, StaticConnectableChannelsProductFacade,
    StaticOperatorStatusService, TriggerRunThreadScope, UnsupportedAutomationProductFacade,
    UnsupportedOperatorLogsService, UnsupportedOperatorServiceLifecycleService,
    UnsupportedOperatorStatusService, UnsupportedOutboundPreferencesProductFacade,
    UpsertLlmProviderRequest, normalize_operator_log_context_value,
};

pub use webui_inbound::{
    WebUiAttachmentCapabilities, WebUiAuthenticatedCaller, WebUiCancelReason,
    WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiGateResolution, WebUiInboundAttachment,
    WebUiInboundCommand, WebUiInboundValidationCode, WebUiInboundValidationError,
    WebUiListAutomationsRequest, WebUiListThreadsRequest, WebUiResolveGateRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest, webui_attachment_capabilities,
};
pub use workflow::DefaultProductWorkflow;
