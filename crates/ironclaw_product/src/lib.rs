//! Product-facing workflow service for IronClaw Reborn.
//!
//! `ironclaw_product` sits between product adapters and host-layer
//! Reborn services. It owns the product action orchestration so that adapters
//! (Web, API, CLI, Telegram, etc.) do not each reimplement binding resolution,
//! message staging, idempotency, busy/deferred handling, gate routing, mission
//! routing, and redacted acknowledgements.
//!
//! ## Key types
//!
//! - [`DefaultProductSurface`] — top-level orchestrator that implements
//!   [`ProductSurface`].
//! - [`InboundTurnService`] / [`DefaultInboundTurnService`] — the narrower
//!   user-message path that coordinates binding + turn submission.
//! - [`ConversationBindingService`] — resolves external adapter refs to
//!   canonical Reborn identifiers.
//! - [`ProductConversationBindingService`] — bridges product adapter bindings to
//!   `ironclaw_conversations` using trusted installation configuration for
//!   tenant/default scope selection.
//! - [`IdempotencyLedger`] — durable action deduplication port.
//! - [`InMemoryIdempotencyLedger`] — standalone/test ledger with in-flight lease
//!   recovery semantics.
//! - [`ProductInboundAction`] — durable ledger record for inbound actions.

#![forbid(unsafe_code)]

mod action;
pub mod adapter_registry;
mod approval_interaction;
mod approval_prompt;
mod auth_continuation;
mod auth_interaction;
mod auth_prompt;
mod automation_product_service;
mod automation_thread_metadata;
mod binding;
mod binding_ref;
mod command_admission;
mod command_dispatch;
mod commands;
mod communication_context;
mod conversation_binding;
mod error;
mod extension_account_setup;
#[cfg(any(test, feature = "test-support"))]
mod fakes;
// Durable filesystem-backed idempotency ledger. The filesystem contract is a
// base dependency; concrete libSQL/Postgres implementations remain gated.
mod delivery_coordinator;
mod filesystem_ledger;
mod gate_state;
mod in_memory_ledger;
mod inbound_turn;
mod ledger;
mod lifecycle;
mod outbound_delivery;
mod policy;
mod product_auth_prompt;
mod product_surface_inbound;
mod project_create_capability;
mod project_service;
pub mod projection;
mod reborn_services;
mod run_delivery;
mod scoped_fs;
mod workflow;

pub use product_auth_prompt::{blocked_auth_flow_canceller, product_auth_challenge_provider};
pub use project_create_capability::{PROJECT_CREATE_CAPABILITY_ID, project_create_capability};
pub use project_service::RebornProjectService;

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
pub use approval_prompt::{
    ApprovalPromptLookup, ApprovalPromptLookupError, approval_prompt_context_view,
    approval_prompt_lookup,
};
/// Concrete turn-gate resume dispatcher used by the Reborn composition crate to
/// bridge product-auth continuations into the workflow-owned turn boundary.
pub use auth_continuation::{
    ProductAuthContinuationDispatcher, ProductAuthTurnGateResumeDispatcher,
    lifecycle_auth_continuation_dispatcher,
};
pub use auth_interaction::{
    AuthCredentialAccountChoiceView, AuthGateRecord, AuthInteractionChallengeView,
    AuthInteractionDecision, AuthInteractionReadModel, AuthInteractionRejectionKind,
    AuthInteractionScope, AuthInteractionService, AuthInteractionStatus,
    DefaultAuthInteractionService, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, PendingAuthInteractionView, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse, is_auth_gate_ref,
};
pub use auth_prompt::{
    AuthChallengeProvider, AuthChallengeView, BlockedAuthFlowCanceller, BlockedAuthPromptRequest,
    PairingAuthChallengeView, auth_prompt_view_for_blocked_auth,
};
pub use automation_product_service::RebornAutomationProductService;
pub use automation_thread_metadata::{
    AUTOMATION_TRIGGER_THREAD_SOURCE_TAG, automation_trigger_thread_metadata_json,
    thread_metadata_is_automation_trigger,
};
pub use binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding, route_kind_for_inbound_payload,
};
pub use command_admission::{CommandActorRoleResolver, DirectConversationCommandAdmission};
pub use command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService, ProductCommandContext,
    RejectingProductCommandAdmissionService,
};
pub use commands::{
    CommandAudience, CommandResultField, CommandResultView, PRODUCT_LIFECYCLE_COMMAND_OPERATION_ID,
    PRODUCT_MODEL_COMMAND_OPERATION_ID, PRODUCT_STATUS_COMMAND_OPERATION_ID, ProductCommand,
    ProductCommandDescriptor, ProductLifecycleCommandInput, ProductModelCommand,
    ProductModelCommandInput, ProductStatusCommandInput, UnknownProductCommandName,
    declared_command_help_text, product_command_descriptors, render_command_result_text,
    required_audience, validate_declared_product_command,
};
pub use communication_context::RuntimeCommunicationContextProvider;
pub use conversation_binding::{
    ProductActorBindingPolicy, ProductActorUserResolutionRequest, ProductActorUserResolver,
    ProductConversationBindingService, ProductConversationRouteKey,
    ProductConversationSubjectRouteResolutionRequest, ProductConversationSubjectRouteResolver,
    ProductInstallationKey, ProductInstallationScope, ResolvedProductActorUser,
    StaticProductActorUserResolver, StaticProductInstallationResolver,
};
pub use error::{
    AuthContinuationRejectionKind, ProductSurfaceFailure, lifecycle_product_surface_error,
};
pub use extension_account_setup::{
    AccountConnectionStatusError, AccountConnectionStatusSource, ChannelConnectionNoticePolicy,
    ExtensionAccountSetupDescriptor, ExtensionAccountSetupError, ExtensionAccountSetupRegistry,
};
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeBeforeInboundPolicy, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, NoProjectFilesystem, rejecting_product_surface_error,
};
pub use scoped_fs::{
    ProjectScopedAttachmentLander,
    ProjectScopedAttachmentReader,
    ProjectScopedFilesystemReader,
    // Shared scoped-path helpers: the mount-browse reader in composition
    // derives the same MIME/size/error semantics from them.
    file_name_of,
    guard_readable_file,
    map_filesystem_error,
    map_kind,
    mime_for_path,
};

pub use filesystem_ledger::RebornFilesystemIdempotencyLedger;
pub use filesystem_ledger::RebornLibSqlIdempotencyLedger;
pub use filesystem_ledger::RebornPostgresIdempotencyLedger;
pub use in_memory_ledger::InMemoryIdempotencyLedger;
pub use inbound_turn::{
    DefaultInboundTurnService, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
};
pub use ironclaw_common::{AutomationName, AutomationNameError, MAX_AUTOMATION_NAME_BYTES};
pub use ironclaw_extension_contracts::auth_prompt::{
    AuthPromptChallengeKind, AuthPromptContextView, AuthPromptView, ConnectionPromptContext,
};
pub use ironclaw_extension_contracts::channel_adapter::{
    ChannelAttachmentRef, ChannelContext, ChannelError, DeliveryReport, ImmediateResponse,
    InboundBatchFragment, InboundOutcome, MAX_IMMEDIATE_RESPONSE_BYTES,
    MAX_INBOUND_BATCH_REF_BYTES, MAX_INBOUND_BATCH_SETTLE_MILLIS, MAX_REPLY_CONTEXT_BYTES,
    OutboundEnvelope, OutboundPart, OutboundTarget, PartDeliveryOutcome, ProductTriggerReason,
    TargetCandidate, TargetQuery, VerifiedInbound,
};
pub use ironclaw_extension_contracts::egress::{
    DeclaredEgressHost, DeclaredEgressTarget, DeliveryAttemptId, DeliveryStatus,
    EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath, EgressRequest, EgressResponse,
};
pub use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
    ProductAttachmentKind,
};
#[cfg(any(test, feature = "test-support"))]
pub use ironclaw_extension_contracts::test_support::fakes::{
    FakeOutboundDeliverySink, FakeProtocolHttpEgress, RecordedEgressCall,
};
pub use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, AuthRequirement, ProductAdapterCapabilities, ProductAdapterError,
    ProductAdapterId, ProductCapabilityFlag, ProductSurfaceKind, ProductSurfaceRejectionKind,
    ProtocolAuthEvidence, ProtocolAuthFailure, ProtocolHttpEgressError, REDACTED_PLACEHOLDER,
    RedactedDebug, RedactedString, VerifiedAuthClaim,
};
#[cfg(feature = "host-auth-mint")]
pub use ironclaw_host_api::product_adapter::{
    mark_bearer_token_verified, mark_bearer_token_verified_for_tenant,
    mark_request_signature_verified, mark_request_signature_verified_for_tenant,
    mark_session_verified, mark_session_verified_for_tenant, mark_shared_secret_header_verified,
    mark_shared_secret_header_verified_for_tenant,
};
pub use ironclaw_product_contracts::inbound::{
    ApprovalDecision, ApprovalResolutionPayload, AuthResolutionPayload, AuthResolutionResult,
    ChannelInboundClassification, InboundCommandPayload, InboundRetryDisposition,
    LinkedThreadActionPayload, ParsedProductInbound, ProductCommandResultPayload,
    ProductControlActionPayload, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionDisposition, ProductRejectionKind,
    ProductSlashCommandParseError, ProductSourceChannel, ProjectionReadPayload,
    ProjectionSubscriptionPayload, ScopedApprovalResolutionPayload, TrustedInboundContext,
    UserMessagePayload, classify_channel_inbound_text, parse_product_slash_command,
};
pub use ironclaw_product_contracts::interaction_commands::{
    parse_interaction_resolution_text, strip_wrapping_inline_code,
};
pub use ironclaw_product_contracts::outbound::{
    ApprovalPromptActionView, ApprovalPromptContextView, ApprovalPromptDestinationView,
    ApprovalPromptDetailView, ApprovalPromptScopeView, CAPABILITY_DISPLAY_KIND_MAX_BYTES,
    CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES, CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES,
    CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityActivityViewInput, CapabilityDisplayPreviewView, CapabilityDisplayPreviewViewInput,
    FinalReplyView, GatePromptView, PROJECTION_SKILL_ACTIVATION_MAX_ITEMS,
    PROJECTION_SKILL_FEEDBACK_MAX_BYTES, PROJECTION_SKILL_NAME_MAX_BYTES,
    PROJECTION_TEXT_MAX_BYTES, ProductGateKind, ProductOutboundEnvelope, ProductOutboundPayload,
    ProductOutboundTarget, ProductProjectionItem, ProductProjectionState, ProductRenderOutcome,
    ProductSynchronousResponse, ProductWorkSummaryPhase, ProgressKind, ProgressUpdateView,
    ProjectionCursor,
};
pub use ironclaw_product_contracts::projection::{
    ProductProjectionReadInput, ProductProjectionSubject, ProductProjectionSubscribeInput,
    ProjectionReadRequest, ProjectionStreamSubscription, ProjectionSubscriptionRequest,
};
#[cfg(any(test, feature = "test-support"))]
pub use ironclaw_product_contracts::test_support::fakes::FakeProjectionStream;

pub mod auth {
    pub use ironclaw_host_api::product_adapter::auth::{
        AuthRequirement, ProtocolAuthEvidence, VerifiedAuthClaim,
    };
    pub use ironclaw_host_api::product_adapter_error::ProtocolAuthFailure;

    #[cfg(feature = "host-auth-mint")]
    pub use ironclaw_host_api::product_adapter::auth::{
        mark_bearer_token_verified, mark_bearer_token_verified_for_tenant,
        mark_request_signature_verified, mark_request_signature_verified_for_tenant,
        mark_session_verified, mark_session_verified_for_tenant,
        mark_shared_secret_header_verified, mark_shared_secret_header_verified_for_tenant,
    };
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    // The channel-adapter conformance suite (§11.2.10) moved with
    // `ChannelAdapter`; `fakes` split by the port each fake implements.
    pub use ironclaw_extension_contracts::test_support::conformance;
    pub use ironclaw_extension_contracts::test_support::conformance::*;
    pub use ironclaw_extension_contracts::test_support::fakes::*;
    pub use ironclaw_product_contracts::surface::{
        RecordedProductSurfaceInvoke, RecordedProductSurfaceQuery, RecordedProductSurfaceStream,
        RecordingProductSurface,
    };
    pub use ironclaw_product_contracts::test_support::fakes::*;
}
pub use ledger::{IdempotencyDecision, IdempotencyLedger};
pub use lifecycle::{
    ChannelConnectionRequirement, LifecycleBlockerRef, LifecycleChannelDirections,
    LifecycleCommandKind, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleInstallScope,
    LifecycleInstalledExtensionSummary, LifecyclePackageId, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductContext, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleProductService, LifecycleProductSurfaceContext,
    LifecycleReadinessBlocker, LifecycleSearchExtensionSummary, LifecycleSkillSource,
    LifecycleSkillSummary, UnsupportedLifecycleProductService, project_public_lifecycle_states,
    public_lifecycle_response_json,
};
// Product hosts use this outbound orchestration seam to wire outbound policy
// decisions to adapter rendering without reaching into module internals.
pub use delivery_coordinator::{
    ChannelDeliveryResolver, CoordinatedDeliveryError, CoordinatedDeliveryOutcome,
    CoordinatedDeliveryRequest, DeliveryCoordinator, DeliveryIntent, DeliveryReplyContextSource,
    DeliveryRetryPolicy, NoReplyContext, NoticeDeliveryRequest, ResolvedChannelDelivery,
};
pub use outbound_delivery::{ProductOutboundTargetResolver, VerifiedProductOutboundTargetMetadata};
// The generic run-delivery components (§5.4): channel hosts wire these over
// the coordinator; vendor residue enters only through the ports.
pub use policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
pub use run_delivery::{
    ApprovalPromptContextSource, BlockedAuthPromptSource, DeliveredChannelMessage,
    RunDeliveryError, RunDeliveryObserver, RunDeliveryServices, RunDeliverySettings,
    TriggeredRunDeliveryDriver, TriggeredRunDeliveryRequest, triggered_run_delivery_settings,
};
// Adapter, projection, and event DTOs are re-exported from
// `ironclaw_host_api::product_adapter` above so product terminals consume a
// single product service.
pub use reborn_services::{
    ADMIN_CONFIGURATION_REPLACE_CAPABILITY, ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID,
    ADMIN_CONFIGURATION_VIEW, ADMIN_USER_CREATE_COMMAND, ADMIN_USER_DELETE_CAPABILITY,
    ADMIN_USER_DELETE_CAPABILITY_ID, ADMIN_USER_DELETE_SECRET_CAPABILITY,
    ADMIN_USER_DELETE_SECRET_CAPABILITY_ID, ADMIN_USER_DELETE_SECRET_COMMAND,
    ADMIN_USER_PUT_SECRET_CAPABILITY, ADMIN_USER_PUT_SECRET_CAPABILITY_ID, ADMIN_USER_SECRETS_VIEW,
    ADMIN_USER_SET_ROLE_CAPABILITY, ADMIN_USER_SET_ROLE_CAPABILITY_ID,
    ADMIN_USER_SET_STATUS_CAPABILITY, ADMIN_USER_SET_STATUS_CAPABILITY_ID,
    ADMIN_USER_UPDATE_CAPABILITY, ADMIN_USER_UPDATE_CAPABILITY_ID, ADMIN_USER_VIEW,
    ADMIN_USERS_VIEW, ATTACHMENT_READ_COMMAND, AUTOMATION_DELETE_CAPABILITY,
    AUTOMATION_DELETE_CAPABILITY_ID, AUTOMATION_DELETE_COMMAND, AUTOMATION_LIST_DEFAULT_PAGE_SIZE,
    AUTOMATION_LIST_MAX_PAGE_SIZE, AUTOMATION_PAUSE_CAPABILITY, AUTOMATION_PAUSE_CAPABILITY_ID,
    AUTOMATION_PAUSE_COMMAND, AUTOMATION_RENAME_CAPABILITY, AUTOMATION_RENAME_CAPABILITY_ID,
    AUTOMATION_RENAME_COMMAND, AUTOMATION_RESUME_CAPABILITY, AUTOMATION_RESUME_CAPABILITY_ID,
    AUTOMATION_RESUME_COMMAND, AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE,
    AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE, AUTOMATIONS_VIEW, ActiveModelReader,
    AdminCreateUserFields, AdminCreatedUser, AdminUserError, AdminUserRecord, AdminUserRole,
    AdminUserSecretMeta, AdminUserService, AdminUserStatus, AttachmentCleanupReport,
    AutomationListRequest, AutomationProductService, CANCEL_RUN_COMMAND, CREATE_THREAD_COMMAND,
    ChannelAuthAccountState, ChannelConfigProductService, ChannelConnectionService,
    ChannelInboundSurfaceAdmission, ChannelInboundSurfaceOutcome,
    ChannelInboundSurfaceRejectedAdmission, ChannelInboundSurfaceRequest, CodexLoginStart,
    EXTENSION_ACTIVATE_CAPABILITY, EXTENSION_ACTIVATE_CAPABILITY_ID, EXTENSION_IMPORT_CAPABILITY,
    EXTENSION_IMPORT_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY, EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_REGISTRY_VIEW, EXTENSION_REMOVE_CAPABILITY, EXTENSION_REMOVE_CAPABILITY_ID,
    EXTENSION_SETUP_SUBMIT_CAPABILITY, EXTENSION_SETUP_SUBMIT_CAPABILITY_ID, EXTENSION_SETUP_VIEW,
    EXTENSIONS_VIEW, EmptyProductCommandInput, ExtensionCredentialSetupService,
    ExtensionCredentialStatusRequest, ExtensionCredentialSubmitRequest, FS_LIST_VIEW,
    FS_MOUNTS_VIEW, FS_READ_COMMAND, FS_STAT_VIEW, FilesystemBrowseReader, FsMount,
    GLOBAL_AUTO_APPROVE_VIEW, InboundAttachmentLander, InboundAttachmentReader,
    LLM_ACTIVE_SET_CAPABILITY, LLM_ACTIVE_SET_CAPABILITY_ID, LLM_CODEX_LOGIN_COMMAND,
    LLM_CONFIG_VIEW, LLM_LIST_MODELS_COMMAND, LLM_NEARAI_LOGIN_COMMAND,
    LLM_NEARAI_WALLET_LOGIN_COMMAND, LLM_PROVIDER_DELETE_CAPABILITY,
    LLM_PROVIDER_DELETE_CAPABILITY_ID, LLM_PROVIDER_UPSERT_CAPABILITY,
    LLM_PROVIDER_UPSERT_CAPABILITY_ID, LLM_TEST_CONNECTION_COMMAND, LOGS_VIEW, LlmActiveSelection,
    LlmConfigService, LlmConfigServiceError, LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest,
    LlmProbeResult, LlmProviderView, NearAiAuthProvider, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, OPERATOR_CONFIG_KEY_VIEW,
    OPERATOR_CONFIG_LIST_VIEW, OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY,
    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID, OPERATOR_CONFIG_SET_KEY_COMMAND,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID, OPERATOR_CONFIG_VALIDATE_VIEW,
    OPERATOR_DIAGNOSTICS_VIEW, OPERATOR_LOGS_VIEW, OPERATOR_SERVICE_LIFECYCLE_COMMAND,
    OPERATOR_SETUP_RUN_CAPABILITY, OPERATOR_SETUP_RUN_CAPABILITY_ID, OPERATOR_SETUP_VIEW,
    OPERATOR_STATUS_VIEW, OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
    OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION, OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME,
    OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION,
    OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME, OUTBOUND_DELIVERY_TARGETS_VIEW,
    OUTBOUND_PREFERENCES_SET_CAPABILITY, OUTBOUND_PREFERENCES_SET_CAPABILITY_ID,
    OUTBOUND_PREFERENCES_VIEW, OperatorLogsService, OperatorServiceLifecycleService,
    OperatorStatusService, OutboundDeliveryCapabilityInputError, OutboundDeliveryTargetSetInput,
    OutboundDeliveryTargetsListInput, OutboundPreferencesProductService,
    PRODUCT_COMMAND_EXECUTE_COMMAND, PRODUCT_COMMAND_EXECUTE_COMMAND_ID,
    PRODUCT_COMMAND_LIST_COMMAND, PRODUCT_COMMAND_LIST_COMMAND_ID, PROJECT_CREATE_COMMAND,
    PROJECT_DELETE_CAPABILITY, PROJECT_DELETE_CAPABILITY_ID, PROJECT_FS_LIST_VIEW,
    PROJECT_FS_READ_COMMAND, PROJECT_FS_STAT_VIEW, PROJECT_MEMBER_ADD_CAPABILITY,
    PROJECT_MEMBER_ADD_CAPABILITY_ID, PROJECT_MEMBER_REMOVE_CAPABILITY,
    PROJECT_MEMBER_REMOVE_CAPABILITY_ID, PROJECT_MEMBER_UPDATE_CAPABILITY,
    PROJECT_MEMBER_UPDATE_CAPABILITY_ID, PROJECT_MEMBERS_VIEW, PROJECT_UPDATE_CAPABILITY,
    PROJECT_UPDATE_CAPABILITY_ID, PROJECT_VIEW, PROJECTS_VIEW, ProductAgentBoundCaller,
    ProductCapabilityDescriptor, ProductCapabilityInvoker, ProductSurfaceCommandDescriptor,
    ProductView, ProjectCaller, ProjectFilesystemReader, ProjectFsEntry, ProjectFsEntryKind,
    ProjectFsError, ProjectFsFile, ProjectFsStat, ProjectService, ProjectServiceError,
    RESOLVE_GATE_COMMAND, RETRY_RUN_COMMAND, RUN_ARTIFACT_SCHEMA, RUN_ARTIFACT_VIEW,
    RebornAccountBindingSource, RebornAccountLoginLinkResponse, RebornAccountTrace,
    RebornAccountTracesResponse, RebornAddMemberRequest, RebornAdminConfigurationField,
    RebornAdminConfigurationGroup, RebornAdminConfigurationListResponse,
    RebornAdminConfigurationUse, RebornAdminCreateUserRequest,
    RebornAdminDeleteSecretProductRequest, RebornAdminPutSecretProductRequest,
    RebornAdminPutSecretRequest, RebornAdminSecretDeletedResponse, RebornAdminSecretResponse,
    RebornAdminSetRoleProductRequest, RebornAdminSetRoleRequest,
    RebornAdminSetStatusProductRequest, RebornAdminSetStatusRequest,
    RebornAdminUpdateUserProductRequest, RebornAdminUpdateUserRequest,
    RebornAdminUserCreatedResponse, RebornAdminUserDeletedResponse, RebornAdminUserListQuery,
    RebornAdminUserListResponse, RebornAdminUserRequest, RebornAdminUserResponse,
    RebornAdminUserSecretsListResponse, RebornAttachmentBytes, RebornAttachmentRequest,
    RebornAuthAccount, RebornAutomationActiveHold, RebornAutomationHoldReason,
    RebornAutomationInfo, RebornAutomationMutationResponse, RebornAutomationRecentRunInfo,
    RebornAutomationRecentRunStatus, RebornAutomationRequest, RebornAutomationRunStatus,
    RebornAutomationSource, RebornAutomationState, RebornCancelRunResponse,
    RebornChannelConfigField, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornCommandRejection, RebornCreateProjectRequest, RebornCreateThreadResponse,
    RebornDeleteProjectRequest, RebornDeleteThreadRequest, RebornDeleteThreadResponse,
    RebornExecuteProductCommandRequest, RebornExecuteProductCommandResponse,
    RebornExtensionActionResponse, RebornExtensionCredentialSetup, RebornExtensionInfo,
    RebornExtensionListResponse, RebornExtensionOnboardingPayload, RebornExtensionOnboardingState,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornExtensionSetupField,
    RebornExtensionSetupSecret, RebornExtensionSurface, RebornFsListRequest, RebornFsListResponse,
    RebornFsMountInfo, RebornFsMountsRequest, RebornFsMountsResponse, RebornFsReadRequest,
    RebornFsStatRequest, RebornFsStatResponse, RebornGetProjectRequest, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornGlobalAutoApproveRequest, RebornGlobalAutoApproveResponse,
    RebornListAutomationsResponse, RebornListMembersRequest, RebornListMembersResponse,
    RebornListProjectsRequest, RebornListProjectsResponse, RebornListThreadsResponse,
    RebornLogEntry, RebornLogLevel, RebornLogQueryRequest, RebornLogQueryResponse,
    RebornOperatorArea, RebornOperatorCommandPlaneResponse, RebornOperatorConfigDiagnostic,
    RebornOperatorConfigDiagnosticSeverity, RebornOperatorConfigEntry,
    RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetProductRequest, RebornOperatorConfigSetRequest,
    RebornOperatorConfigValidateRequest, RebornOperatorConfigValidateResponse,
    RebornOperatorLogsQuery, RebornOperatorServiceLifecycleAction,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupRequest, RebornOperatorSetupResponse,
    RebornOperatorSetupStatus, RebornOperatorSetupStep, RebornOperatorSetupStepStatus,
    RebornOperatorStatusCheck, RebornOperatorStatusResponse, RebornOperatorStatusSeverity,
    RebornOperatorStatusState, RebornOperatorSurfaceStatus, RebornOperatorToolCatalog,
    RebornOperatorToolInfo, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetChannel,
    RebornOutboundDeliveryTargetDescription, RebornOutboundDeliveryTargetDisplayName,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
    RebornOutboundPreferencesService, RebornProductCommandInfo, RebornProductCommandListResponse,
    RebornProjectFsListRequest, RebornProjectFsListResponse, RebornProjectFsReadRequest,
    RebornProjectFsStatRequest, RebornProjectFsStatResponse, RebornProjectInfo,
    RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole,
    RebornProjectState, RebornRemoveMemberRequest, RebornRenameAutomationProductRequest,
    RebornResolveGateResponse, RebornResumeGateResponse, RebornRetryRunResponse, RebornRunArtifact,
    RebornRunArtifactRequest, RebornServiceLifecycleAction, RebornServiceLifecycleRequest,
    RebornServiceLifecycleResponse, RebornServiceLifecycleState, RebornServices,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillInfo, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornSkillSourceKind, RebornSkillTrustLevel,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornThreadArtifact, RebornThreadArtifactRequest, RebornTimelineRequest,
    RebornTimelineResponse, RebornTraceCreditsResponse, RebornTraceHoldAuthorizeProductRequest,
    RebornTraceHoldAuthorizeResponse, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    RebornVendorAuthAccounts, RebornViewDescriptor, RebornViewPage, RebornViewProvider,
    RebornViewQuery, RunArtifactLogs, RunArtifactMessage, RunArtifactRedaction,
    RunArtifactToolCall, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY,
    SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID, SKILL_AUTO_ACTIVATE_SET_CAPABILITY,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_CONTENT_VIEW, SKILL_INSTALL_CAPABILITY,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY, SKILL_REMOVE_CAPABILITY_ID,
    SKILL_SEARCH_VIEW, SKILL_UPDATE_CAPABILITY, SKILL_UPDATE_CAPABILITY_ID, SKILLS_VIEW,
    SUBMIT_TURN_COMMAND, SetActiveLlmRequest, SettingsToolPermissionState, SkillsProductService,
    StaticOperatorStatusService, THREAD_ARTIFACT_MAX_MESSAGES, THREAD_ARTIFACT_SCHEMA,
    THREAD_ARTIFACT_VIEW, THREAD_DELETE_CAPABILITY, THREAD_DELETE_CAPABILITY_ID, THREADS_VIEW,
    TIMELINE_VIEW, TRACE_ACCOUNT_LOGIN_LINK_COMMAND, TRACE_ACCOUNT_TRACES_VIEW, TRACE_CREDITS_VIEW,
    TRACE_HOLD_AUTHORIZE_COMMAND, TriggerRunThreadScope, UnavailableRebornViewProvider,
    UnsupportedAutomationProductService, UnsupportedOperatorLogsService,
    UnsupportedOperatorServiceLifecycleService, UnsupportedOperatorStatusService,
    UnsupportedOutboundPreferencesProductService, UpsertLlmProviderRequest,
    list_outbound_delivery_targets_for_model, normalize_operator_log_context_value,
    outbound_delivery_synthetic_provider, outbound_delivery_target_set_input_schema,
    outbound_delivery_target_set_operator_tool_info, outbound_delivery_targets_list_input_schema,
    parse_outbound_delivery_target_set_input, parse_outbound_delivery_targets_list_input,
    set_outbound_delivery_target_for_model,
};

pub use product_surface_inbound::{
    ProductAttachmentCapabilities, ProductCancelReason, ProductCancelRunRequest,
    ProductCreateThreadRequest, ProductGateResolution, ProductInboundAttachment,
    ProductInboundCommand, ProductListAutomationsRequest, ProductListThreadsRequest,
    ProductRenameAutomationRequest, ProductResolveGateRequest, ProductRetryRunRequest,
    ProductSetupExtensionRequest, ProductSubmitTurnRequest, product_attachment_capabilities,
};
pub use workflow::DefaultProductSurface;
