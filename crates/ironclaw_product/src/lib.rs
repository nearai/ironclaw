//! Product-facing workflow facade for IronClaw Reborn.
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
//! - [`InMemoryIdempotencyLedger`] — local-dev/test ledger with in-flight lease
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
mod automation_thread_metadata;
mod binding;
mod binding_ref;
mod channel_pairing;
mod channel_workflow_state;
mod command_dispatch;
mod commands;
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
mod lifecycle_auth_continuation;
mod outbound_delivery;
mod policy;
mod product_surface_inbound;
mod reborn_services;
mod run_delivery;
mod trigger_final_reply_target;
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
pub use approval_prompt::{
    ApprovalPromptLookup, ApprovalPromptLookupError, approval_prompt_context_view,
    approval_prompt_lookup,
};
/// Concrete turn-gate resume dispatcher used by the Reborn composition crate to
/// bridge product-auth continuations into the workflow-owned turn boundary.
pub use auth_continuation::{
    ProductAuthContinuationDispatcher, ProductAuthTurnGateResumeDispatcher,
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
pub use automation_thread_metadata::{
    AUTOMATION_TRIGGER_THREAD_SOURCE_TAG, automation_trigger_thread_metadata_json,
    thread_metadata_is_automation_trigger,
};
pub use binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolveStoredProductReplyTargetRequest, ResolvedBinding, ResolvedStoredProductReplyTarget,
    StoredProductReplyTargetAccess, route_kind_for_inbound_payload,
};
pub use channel_pairing::{
    ChannelPairingConsumeOutcome, ChannelPairingDirectTargetStore, ChannelPairingError,
    ChannelPairingIdentityBindOutcome, ChannelPairingIdentityStore,
    ChannelPairingInstallationSource, ChannelPairingInterception, ChannelPairingInterceptor,
    ChannelPairingRegistry, ChannelPairingService, ChannelPairingServiceDependencies,
    ChannelPairingStatus, ChannelPairingTemplateValues, FilesystemChannelPairingStore,
};
pub use channel_workflow_state::{
    ChannelDisconnectActions, ChannelWorkflowState, ChannelWorkflowStateError,
    ChannelWorkflowStateService, disconnect_channel_in_order,
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
    ProductInstallationKey, ProductInstallationScope, ResolvedProductActorUser,
    StaticProductActorUserResolver, StaticProductInstallationResolver,
};
pub use error::{AuthContinuationRejectionKind, ProductWorkflowError};
pub use extension_account_setup::{
    AccountConnectionStatusError, AccountConnectionStatusSource, CHANNEL_PAIRING_CODE_ALPHABET,
    CHANNEL_PAIRING_CODE_LEN, ChannelConnectionNoticePolicy, ChannelPairingCode,
    ChannelPairingCodeError, ChannelPairingIssue, ExtensionAccountSetupDescriptor,
    ExtensionAccountSetupError, ExtensionAccountSetupRegistry,
};
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeBeforeInboundPolicy, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, rejecting_product_surface_error,
};
pub use filesystem_ledger::RebornFilesystemIdempotencyLedger;
pub use filesystem_ledger::RebornLibSqlIdempotencyLedger;
pub use filesystem_ledger::RebornPostgresIdempotencyLedger;
pub use in_memory_ledger::InMemoryIdempotencyLedger;
pub use inbound_turn::{
    DefaultInboundTurnService, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
};
pub use ironclaw_common::{AutomationName, AutomationNameError, MAX_AUTOMATION_NAME_BYTES};
pub use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ApprovalDecision, ApprovalPromptActionView, ApprovalPromptContextView,
    ApprovalPromptDestinationView, ApprovalPromptDetailView, ApprovalPromptScopeView,
    ApprovalResolutionPayload, AttachmentRef, AuthPromptChallengeKind, AuthPromptContextView,
    AuthPromptView, AuthRequirement, AuthResolutionPayload, AuthResolutionResult,
    CAPABILITY_DISPLAY_KIND_MAX_BYTES, CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES,
    CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES, CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
    CapabilityActivityStatusView, CapabilityActivityView, CapabilityActivityViewInput,
    CapabilityDisplayPreviewView, CapabilityDisplayPreviewViewInput, ChannelAdapter,
    ChannelContext, ChannelError, ChannelInboundClassification, ConnectionPromptContext,
    DeclaredEgressHost, DeclaredEgressTarget, DeliveryAttemptId, DeliveryReport, DeliveryStatus,
    EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath, EgressRequest, EgressResponse,
    ExternalActorRef, ExternalConversationRef, ExternalEventId, FinalReplyView, GatePromptView,
    ImmediateResponse, InboundCommandPayload, InboundOutcome, InboundRetryDisposition,
    LinkedThreadActionPayload, MAX_IMMEDIATE_RESPONSE_BYTES, MAX_REPLY_CONTEXT_BYTES,
    NormalizedInboundMessage, OutboundDeliverySink, OutboundEnvelope, OutboundPart, OutboundTarget,
    PROJECTION_SKILL_ACTIVATION_MAX_ITEMS, PROJECTION_SKILL_FEEDBACK_MAX_BYTES,
    PROJECTION_SKILL_NAME_MAX_BYTES, PROJECTION_TEXT_MAX_BYTES, PairingPromptView,
    ParsedProductInbound, PartDeliveryOutcome, PreferenceTargetCodec,
    PreferenceTargetEncodeRequest, ProductAdapterCapabilities, ProductAdapterError,
    ProductAdapterId, ProductAttachmentDescriptor, ProductAttachmentKind, ProductCapabilityFlag,
    ProductCommandResultPayload, ProductControlActionPayload, ProductGateKind, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductOutboundEnvelope, ProductOutboundPayload,
    ProductOutboundTarget, ProductProjectionItem, ProductProjectionReadInput,
    ProductProjectionState, ProductProjectionSubject, ProductProjectionSubscribeInput,
    ProductRejection, ProductRejectionDisposition, ProductRejectionKind, ProductRenderOutcome,
    ProductSlashCommandParseError, ProductSourceChannel, ProductSurfaceKind,
    ProductSynchronousResponse, ProductTriggerReason, ProductWorkSummaryPhase,
    ProductWorkflowRejectionKind, ProgressKind, ProgressUpdateView, ProjectionCursor,
    ProjectionReadPayload, ProjectionReadRequest, ProjectionStream, ProjectionStreamSubscription,
    ProjectionSubscriptionPayload, ProjectionSubscriptionRequest, ProtocolAuthEvidence,
    ProtocolAuthFailure, ProtocolHttpEgress, ProtocolHttpEgressError, REDACTED_PLACEHOLDER,
    RedactedDebug, RedactedString, ScopedApprovalResolutionPayload, TargetCandidate, TargetQuery,
    TrustedInboundContext, UserMessagePayload, VerifiedAuthClaim, VerifiedInbound,
    parse_interaction_resolution_text, parse_product_slash_command, render_channel_auth_prompt,
    strip_wrapping_inline_code,
};
#[cfg(feature = "host-auth-mint")]
pub use ironclaw_host_api::product_adapter::{
    mark_bearer_token_verified, mark_bearer_token_verified_for_tenant,
    mark_request_signature_verified, mark_request_signature_verified_for_tenant,
    mark_session_verified, mark_session_verified_for_tenant, mark_shared_secret_header_verified,
    mark_shared_secret_header_verified_for_tenant,
};
#[cfg(any(test, feature = "test-support"))]
pub use ironclaw_turns::product_adapter::{
    FakeOutboundDeliverySink, FakeProjectionStream, FakeProtocolHttpEgress, RecordedEgressCall,
};

pub mod auth {
    pub use ironclaw_host_api::ProtocolAuthFailure;
    pub use ironclaw_host_api::product_adapter::auth::{
        AuthRequirement, ProtocolAuthEvidence, VerifiedAuthClaim,
    };

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
    pub use ironclaw_host_api::product_adapter::test_support::*;
    pub use ironclaw_host_api::{
        RecordedProductSurfaceInvoke, RecordedProductSurfaceQuery, RecordedProductSurfaceStream,
        RecordingProductSurface,
    };
}
pub use ironclaw_turns::IdempotencyKey;
pub use ledger::{IdempotencyDecision, IdempotencyLedger};
pub use lifecycle::{
    ChannelConnectionRequirement, LifecycleBlockerRef, LifecycleChannelDirections,
    LifecycleCommandKind, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleInstallScope,
    LifecycleInstalledExtensionSummary, LifecyclePackageId, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleProductSurfaceContext,
    LifecyclePublicState, LifecycleReadinessBlocker, LifecycleSearchExtensionSummary,
    LifecycleSkillSource, LifecycleSkillSummary, UnsupportedLifecycleProductFacade,
};
pub use lifecycle_auth_continuation::lifecycle_auth_continuation_dispatcher;
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
pub use ironclaw_outbound::WEB_APP_OUTBOUND_DELIVERY_TARGET_ID;
pub use policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
pub use run_delivery::{
    ApprovalPromptContextSource, BlockedAuthPromptSource, CurrentDeliveryTarget,
    CurrentDeliveryTargetResolver, DeliveredChannelMessage, RunDeliveryError,
    RunDeliveryEventHandler, RunDeliveryEventRouter, RunDeliveryObserver, RunDeliveryServices,
    TriggeredRunDeliveryChannel, TriggeredRunDeliveryDriver, TriggeredRunDeliveryRequest,
    TriggeredRunDeliveryRouter, TriggeredRunExternalDeliveryTarget,
};
pub use trigger_final_reply_target::{RunFinalReplyRoutingService, TriggerFinalReplyTargetService};
// Adapter, projection, and event DTOs are re-exported from
// `ironclaw_host_api::product_adapter` above so product terminals consume a
// single product facade.
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
    AdminUserSecretMeta, AdminUserService, AdminUserStatus, AutomationListRequest,
    AutomationProductFacade, CANCEL_RUN_COMMAND, CREATE_THREAD_COMMAND, ChannelAuthAccountState,
    ChannelConnectionFacade, ChannelInboundSurfaceAdmission, ChannelInboundSurfaceOutcome,
    ChannelInboundSurfaceRejectedAdmission, ChannelInboundSurfaceRequest, CodexLoginStart,
    EXTENSION_IMPORT_CAPABILITY, EXTENSION_IMPORT_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY,
    EXTENSION_INSTALL_CAPABILITY_ID, EXTENSION_REGISTRY_VIEW, EXTENSION_REMOVE_CAPABILITY,
    EXTENSION_REMOVE_CAPABILITY_ID, EXTENSION_SETUP_SUBMIT_CAPABILITY,
    EXTENSION_SETUP_SUBMIT_CAPABILITY_ID, EXTENSION_SETUP_VIEW, EXTENSIONS_VIEW,
    EmptyProductCommandInput, ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, FS_LIST_VIEW, FS_MOUNTS_VIEW, FS_READ_COMMAND, FS_STAT_VIEW,
    FilesystemBrowseReader, FsMount, GLOBAL_AUTO_APPROVE_VIEW, InboundAttachmentLander,
    InboundAttachmentReader, LLM_ACTIVE_SET_CAPABILITY, LLM_ACTIVE_SET_CAPABILITY_ID,
    LLM_CODEX_LOGIN_COMMAND, LLM_CONFIG_VIEW, LLM_LIST_MODELS_COMMAND, LLM_NEARAI_LOGIN_COMMAND,
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
    OutboundDeliveryTargetsListInput, OutboundPreferencesProductFacade, PROJECT_CREATE_COMMAND,
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
    RebornChannelConnectAction, RebornChannelConnectStrategy, RebornCreateProjectRequest,
    RebornCreateThreadResponse, RebornDeleteProjectRequest, RebornDeleteThreadRequest,
    RebornDeleteThreadResponse, RebornExtensionActionResponse, RebornExtensionCredentialSetup,
    RebornExtensionInfo, RebornExtensionListResponse, RebornExtensionOnboardingPayload,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornExtensionSetupSecret,
    RebornExtensionSurface, RebornFsListRequest, RebornFsListResponse, RebornFsMountInfo,
    RebornFsMountsRequest, RebornFsMountsResponse, RebornFsReadRequest, RebornFsStatRequest,
    RebornFsStatResponse, RebornGetProjectRequest, RebornGetRunStateRequest,
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
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesFacade,
    RebornOutboundPreferencesResponse, RebornProjectFsListRequest, RebornProjectFsListResponse,
    RebornProjectFsReadRequest, RebornProjectFsStatRequest, RebornProjectFsStatResponse,
    RebornProjectInfo, RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse,
    RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest,
    RebornRenameAutomationProductRequest, RebornResolveGateResponse, RebornResumeGateResponse,
    RebornRetryRunResponse, RebornRunArtifact, RebornRunArtifactRequest,
    RebornServiceLifecycleAction, RebornServiceLifecycleRequest, RebornServiceLifecycleResponse,
    RebornServiceLifecycleState, RebornServices, RebornSetOutboundPreferencesRequest,
    RebornSetupExtensionResponse, RebornSkillActionResponse, RebornSkillContentResponse,
    RebornSkillInfo, RebornSkillListResponse, RebornSkillSearchResponse, RebornSkillSourceKind,
    RebornSkillTrustLevel, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    RebornTraceCreditsResponse, RebornTraceHoldAuthorizeProductRequest,
    RebornTraceHoldAuthorizeResponse, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    RebornVendorAuthAccounts, RebornViewDescriptor, RebornViewPage, RebornViewProvider,
    RebornViewQuery, RunArtifactLogs, RunArtifactMessage, RunArtifactRedaction,
    RunArtifactToolCall, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY,
    SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID, SKILL_AUTO_ACTIVATE_SET_CAPABILITY,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_CONTENT_VIEW, SKILL_INSTALL_CAPABILITY,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY, SKILL_REMOVE_CAPABILITY_ID,
    SKILL_SEARCH_VIEW, SKILL_UPDATE_CAPABILITY, SKILL_UPDATE_CAPABILITY_ID, SKILLS_VIEW,
    SUBMIT_TURN_COMMAND, SetActiveLlmRequest, SettingsToolPermissionState, SkillsProductFacade,
    StaticOperatorStatusService, THREAD_DELETE_CAPABILITY, THREAD_DELETE_CAPABILITY_ID,
    THREADS_VIEW, TIMELINE_VIEW, TRACE_ACCOUNT_LOGIN_LINK_COMMAND, TRACE_ACCOUNT_TRACES_VIEW,
    TRACE_CREDITS_VIEW, TRACE_HOLD_AUTHORIZE_COMMAND, TriggerRunThreadScope,
    UnavailableRebornViewProvider, UnsupportedAutomationProductFacade,
    UnsupportedOperatorLogsService, UnsupportedOperatorServiceLifecycleService,
    UnsupportedOperatorStatusService, UnsupportedOutboundPreferencesProductFacade,
    UpsertLlmProviderRequest, install_extension_on_surface,
    list_outbound_delivery_targets_for_model, normalize_operator_log_context_value,
    outbound_delivery_synthetic_provider, outbound_delivery_target_set_input_schema,
    outbound_delivery_target_set_operator_tool_info, outbound_delivery_targets_list_input_schema,
    parse_outbound_delivery_target_set_input, parse_outbound_delivery_targets_list_input,
    set_outbound_delivery_target_for_model, web_app_outbound_delivery_target_option,
};

pub use product_surface_inbound::{
    ProductAttachmentCapabilities, ProductCancelReason, ProductCancelRunRequest,
    ProductCreateThreadRequest, ProductGateResolution, ProductInboundAttachment,
    ProductInboundCommand, ProductListAutomationsRequest, ProductListThreadsRequest,
    ProductRenameAutomationRequest, ProductResolveGateRequest, ProductRetryRunRequest,
    ProductSetupExtensionRequest, ProductSubmitTurnRequest, parse_client_action_id,
    product_attachment_capabilities,
};
pub use workflow::DefaultProductSurface;
