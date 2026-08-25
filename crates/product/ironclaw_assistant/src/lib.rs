//! Product-facing workflow service for IronClaw Reborn.
//!
//! `ironclaw_assistant` sits between product adapters and host-layer
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
//! - [`ProductBindingResolver`] — resolves external adapter refs to
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
mod admin_user_directory;
mod approval_interaction;
mod approval_prompt;
mod auth_continuation;
mod auth_interaction;
mod automation_product_service;
mod automation_thread_metadata;
mod binding_ref;
mod blocked_auth_resume;
mod channel_workflow;
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
pub mod inspector_store;
mod ledger;
mod lifecycle;
mod model_channel_delivery;
mod notification_channel_resolution;
mod outbound_delivery;
mod policy;
mod process_gate_turn_view;
mod product_surface_inbound;
mod project_create_capability;
pub mod projection;
mod reborn_services;
mod run_delivery;
mod run_outcome_observer;
mod scoped_fs;
mod steering;
mod suggestions;
mod suggestions_observer;
mod suggestions_store;
mod unbound_turn;
mod workflow;

pub use project_create_capability::{PROJECT_CREATE_CAPABILITY_ID, project_create_capability};

pub use action::{ActionDispatchKind, ActionPhase, ProductInboundAction};
pub use admin_user_directory::{
    AdminSecretProvisioner, RebornAdminUserDirectory, RejectingAdminApiTokenMinter,
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
// `AuthChallengeProvider`, `AuthChallengeView`, `BlockedAuthFlowCanceller`,
// `PairingAuthChallengeView` and `auth_prompt_view_for_blocked_auth` moved to
// `ironclaw_auth::product_prompt` (WS2.5): every type in their signatures is
// auth's own vocabulary, and the extension host implements the challenge port.
// No re-export here — consumers import from the owner
// (`.claude/rules/type-placement.md`).
pub use automation_product_service::RebornAutomationProductService;
pub use automation_thread_metadata::{
    AUTOMATION_TRIGGER_THREAD_SOURCE_TAG, automation_trigger_thread_metadata_json,
    thread_metadata_is_automation_trigger,
};
pub use blocked_auth_resume::BlockedAuthResumeFanout;
pub use channel_workflow::{
    ChannelWorkflowDeliveryServices, ChannelWorkflowIdentity, RebornChannelWorkflowFactory,
    RebornChannelWorkflowServices, build_session_inbound_ledger, channel_conversation_services,
};
pub use run_outcome_observer::RunOutcomeProcessCommitObserver;
// The conversation-binding family moved to
// `ironclaw_product_contracts::binding` (§12.11 D-A): the channel host's
// workflow factory hands a live binding service back to a caller that sits
// below product, so the port had to be declared at the boundary. Its value
// DTOs (`ResolvedBinding`, `ResolveBindingRequest`, …) are reached there too
// as of the WS5 facade dissolution — this crate no longer offers a second
// import path to anything it does not declare.
pub use command_admission::DirectConversationCommandAdmission;
pub use command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService,
    RejectingProductCommandAdmissionService,
};
pub use commands::{
    CommandAudience, CommandResultField, CommandResultView, PRODUCT_LIFECYCLE_COMMAND_OPERATION_ID,
    PRODUCT_MODEL_COMMAND_OPERATION_ID, PRODUCT_NEW_COMMAND_OPERATION_ID,
    PRODUCT_STATUS_COMMAND_OPERATION_ID, PRODUCT_STOP_COMMAND_OPERATION_ID, ProductCommand,
    ProductCommandDescriptor, ProductLifecycleCommandInput, ProductModelCommand,
    ProductModelCommandInput, ProductNewCommandInput, ProductNewCommandOutput,
    ProductStatusCommandInput, ProductStopCommandInput, ProductStopInvocation,
    UnknownProductCommandName, declared_command_help_text, product_command_descriptors,
    render_command_result_text, required_audience, validate_declared_product_command,
};
pub use communication_context::RuntimeCommunicationContextProvider;
pub use process_gate_turn_view::{current_turn_gate_runs, first_turn_run_for_gate};
// `ProductConversationRouteKey`, `SharedConversationAdmissionRequest`, and
// `SharedConversationAdmission` are deliberately absent: they live in
// `ironclaw_product_contracts::shared_admission` (WS2.2 lineage), and that
// crate grants no second import path (`reborn_product_contract_location_scan.rs`).
// `ProductActorUserResolutionRequest`, `ProductActorUserResolver` and
// `ResolvedProductActorUser` left for the same reason and under the same rule:
// `ironclaw_product_contracts::actor_identity` (WS2.5).
pub use conversation_binding::{
    ProductActorBindingPolicy, ProductConversationBindingService, ProductInstallationKey,
    ProductInstallationScope, SessionLaneRejectingBindingResolver, StaticProductActorUserResolver,
    StaticProductInstallationResolver,
};
pub use error::{
    AuthContinuationRejectionKind, ProductSurfaceFailure, lifecycle_product_surface_error,
};
pub use extension_account_setup::ExtensionAccountSetupRegistry;
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeBeforeInboundPolicy, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, NoProjectFilesystem, rejecting_product_surface_error,
};
pub use scoped_fs::{
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
pub use in_memory_ledger::InMemoryIdempotencyLedger;
pub use inbound_turn::{
    DefaultInboundTurnService, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
    SessionSkillActivationClearer, SessionSkillActivationPorts, SessionSkillActivationRecorder,
};
// **No foreign re-export facade.** This crate re-exports only what it
// *declares*. The 144-symbol block that used to sit here — the channel-adapter,
// egress, external-ref, auth-prompt, inbound/outbound/projection and
// `product_adapter` vocabulary — was the "~120-symbol `host_api::product_adapter`
// re-export facade" PROPOSAL §6.9.1 asks this row to dissolve, and it is gone:
// consumers import each name from the crate that owns it
// (`ironclaw_product_contracts`, `ironclaw_extension_contracts`,
// `ironclaw_host_api::product_adapter`). A second import path is the defect
// §11.2.4's location scans exist to prevent, and
// `reborn_product_contract_location_scan.rs` now polices the *value* DTOs here
// too, not only the ports.
//
// WS1.5 had already deleted this crate's two re-export paths to the
// protocol-auth mint family for the same reason, one seam earlier. Product is
// not a minter: bearer/session evidence comes from
// `ironclaw_host_api::product_adapter::auth` (host transport only) and
// channel/webhook evidence from `ironclaw_extension_contracts::verified_inbound`
// (generic ingress verifier only), both witness-gated. Re-exporting them here is
// what gave `ironclaw_extension_host` and `ironclaw_webui` a product-shaped path
// to a security seam neither should reach through product;
// `reborn_sealed_evidence_mint_ratchet` pins that it stays deleted.
pub use ledger::{IdempotencyDecision, IdempotencyLedger};
pub use lifecycle::{
    ChannelConnectionRequirement, LifecycleBlockerRef, LifecycleChannelDirections,
    LifecycleCommandKind, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleInstallScope,
    LifecycleInstalledExtensionSummary, LifecyclePackageId, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleReadinessBlocker, LifecycleSearchExtensionSummary, LifecycleSkillSource,
    LifecycleSkillSummary, UnsupportedLifecycleProductService, project_public_lifecycle_states,
    public_lifecycle_response_json,
};
// Product hosts use this outbound orchestration seam to wire outbound policy
// decisions to adapter rendering without reaching into module internals.
pub use delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryCoordinator, DeliveryIntent, DeliveryRetryPolicy, NoDeliveryRegistrations,
    NoReplyContext, NoticeDeliveryRequest,
};
pub use outbound_delivery::{ProductOutboundTargetResolver, VerifiedProductOutboundTargetMetadata};
// The generic run-delivery components (§5.4): channel hosts wire these over
// the coordinator; vendor residue enters only through the ports.
pub use model_channel_delivery::{
    CodecChannelTargetResolver, CoordinatedModelChannelDelivery, DeferredModelChannelDelivery,
    ModelChannelDeliveryDeps,
};
pub use policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
pub use run_delivery::notifications::{
    ChannelNotification, ChannelNotificationContext, NotificationChannelTarget,
    NotificationDeliveryFailure, ResolvedUserNotificationTargets, notify, notify_user,
    resolve_user_notification_targets,
};
pub use run_delivery::{
    DeliveredChannelMessage, RunDeliveryError, RunDeliveryObserver, RunDeliveryServices,
    RunDeliverySettings, TriggeredRunDeliveryDriver, triggered_run_delivery_settings,
};
// `TriggeredRunDeliveryRequest` is deliberately absent: it moved to
// `ironclaw_outbound` with the `TriggeredRunDelivery` port it crosses
// (§12.11 D-A), so the generic post-submit hook below product can name it.
// Adapter, projection, and event DTOs are re-exported from
// `ironclaw_host_api::product_adapter` above so product terminals consume a
// single product service.
pub use reborn_services::run_artifact::timings::{
    RunArtifactIterationTiming, RunArtifactTimingTotals, RunArtifactTimings, RunArtifactToolTiming,
};
pub use reborn_services::{
    ADMIN_CONFIGURATION_REPLACE_CAPABILITY, ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID,
    ADMIN_CONFIGURATION_VIEW, ADMIN_THREAD_SCRAPE_ARTIFACT_VIEW,
    ADMIN_THREAD_SCRAPE_RUN_ARTIFACT_VIEW, ADMIN_THREAD_SCRAPE_THREADS_VIEW,
    ADMIN_USER_CREATE_COMMAND, ADMIN_USER_DELETE_CAPABILITY, ADMIN_USER_DELETE_CAPABILITY_ID,
    ADMIN_USER_DELETE_SECRET_CAPABILITY, ADMIN_USER_DELETE_SECRET_CAPABILITY_ID,
    ADMIN_USER_DELETE_SECRET_COMMAND, ADMIN_USER_PUT_SECRET_CAPABILITY,
    ADMIN_USER_PUT_SECRET_CAPABILITY_ID, ADMIN_USER_SECRETS_VIEW, ADMIN_USER_SET_ROLE_CAPABILITY,
    ADMIN_USER_SET_ROLE_CAPABILITY_ID, ADMIN_USER_SET_STATUS_CAPABILITY,
    ADMIN_USER_SET_STATUS_CAPABILITY_ID, ADMIN_USER_UPDATE_CAPABILITY,
    ADMIN_USER_UPDATE_CAPABILITY_ID, ADMIN_USER_VIEW, ADMIN_USERS_VIEW, ATTACHMENT_READ_COMMAND,
    AUTOMATION_DELETE_CAPABILITY, AUTOMATION_DELETE_CAPABILITY_ID, AUTOMATION_DELETE_COMMAND,
    AUTOMATION_LIST_DEFAULT_PAGE_SIZE, AUTOMATION_LIST_MAX_PAGE_SIZE, AUTOMATION_PAUSE_CAPABILITY,
    AUTOMATION_PAUSE_CAPABILITY_ID, AUTOMATION_PAUSE_COMMAND, AUTOMATION_RENAME_CAPABILITY,
    AUTOMATION_RENAME_CAPABILITY_ID, AUTOMATION_RENAME_COMMAND, AUTOMATION_RESUME_CAPABILITY,
    AUTOMATION_RESUME_CAPABILITY_ID, AUTOMATION_RESUME_COMMAND, AUTOMATION_RUN_CAPABILITY,
    AUTOMATION_RUN_CAPABILITY_ID, AUTOMATION_RUN_COMMAND, AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE,
    AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE, AUTOMATIONS_VIEW, AutomationListRequest,
    AutomationProductService, CANCEL_RUN_COMMAND, CREATE_THREAD_COMMAND,
    ChannelInboundSurfaceAdmission, ChannelInboundSurfaceOutcome,
    ChannelInboundSurfaceRejectedAdmission, ChannelInboundSurfaceRequest,
    ChannelNotificationSetupService, DeliveryClientBootstrap, DeliveryClientBootstrapError,
    EXTENSION_ACTIVATE_CAPABILITY, EXTENSION_ACTIVATE_CAPABILITY_ID, EXTENSION_IMPORT_CAPABILITY,
    EXTENSION_IMPORT_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY, EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY, EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID,
    EXTENSION_REGISTRY_VIEW, EXTENSION_REMOVE_CAPABILITY, EXTENSION_REMOVE_CAPABILITY_ID,
    EXTENSION_SETUP_SUBMIT_CAPABILITY, EXTENSION_SETUP_SUBMIT_CAPABILITY_ID, EXTENSION_SETUP_VIEW,
    EXTENSIONS_VIEW, EmptyProductCommandInput, ExtensionCredentialSetupService,
    ExtensionCredentialStatusRequest, ExtensionCredentialSubmitRequest, FS_LIST_VIEW,
    FS_MOUNTS_VIEW, FS_READ_COMMAND, FS_STAT_VIEW, FilesystemBrowseReader, FsMount,
    GLOBAL_AUTO_APPROVE_VIEW, LLM_ACTIVE_SET_CAPABILITY, LLM_ACTIVE_SET_CAPABILITY_ID,
    LLM_CODEX_LOGIN_COMMAND, LLM_CONFIG_VIEW, LLM_LIST_MODELS_COMMAND, LLM_NEARAI_LOGIN_COMMAND,
    LLM_NEARAI_WALLET_LOGIN_COMMAND, LLM_PROVIDER_DELETE_CAPABILITY,
    LLM_PROVIDER_DELETE_CAPABILITY_ID, LLM_PROVIDER_UPSERT_CAPABILITY,
    LLM_PROVIDER_UPSERT_CAPABILITY_ID, LLM_TEST_CONNECTION_COMMAND, LOGS_VIEW,
    NOTIFICATION_CHANNELS_SET_COMMAND, NOTIFICATION_CHANNELS_SET_COMMAND_ID,
    NOTIFICATION_CHANNELS_SET_MAX_ITEMS, NOTIFICATION_CHANNELS_VIEW, NoDeliveryClientBootstrap,
    NotificationChannelsSetInput, OPERATOR_CONFIG_KEY_VIEW, OPERATOR_CONFIG_LIST_VIEW,
    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY, OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
    OPERATOR_CONFIG_SET_KEY_COMMAND, OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID, OPERATOR_CONFIG_VALIDATE_VIEW,
    OPERATOR_DIAGNOSTICS_VIEW, OPERATOR_LOGS_VIEW, OPERATOR_SERVICE_LIFECYCLE_COMMAND,
    OPERATOR_SETUP_RUN_CAPABILITY, OPERATOR_SETUP_RUN_CAPABILITY_ID, OPERATOR_SETUP_VIEW,
    OPERATOR_STATUS_VIEW, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
    OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION, OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
    OUTBOUND_DELIVERY_TARGETS_VIEW, OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID,
    OUTBOUND_NOTIFICATION_CHANNELS_SET_DESCRIPTION,
    OUTBOUND_NOTIFICATION_CHANNELS_SET_PROVIDER_TOOL_NAME, OutboundDeliveryCapabilityInputError,
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
    ProjectFsError, ProjectFsFile, ProjectFsStat, RESOLVE_GATE_COMMAND, RETRY_RUN_COMMAND,
    RUN_ARTIFACT_SCHEMA, RUN_ARTIFACT_VIEW, RebornAccountBindingSource,
    RebornAccountLoginLinkResponse, RebornAccountTrace, RebornAccountTracesResponse,
    RebornAddMemberRequest, RebornAdminConfigurationField, RebornAdminConfigurationGroup,
    RebornAdminConfigurationListResponse, RebornAdminConfigurationUse,
    RebornAdminCreateUserRequest, RebornAdminDeleteSecretProductRequest,
    RebornAdminPutSecretProductRequest, RebornAdminPutSecretRequest,
    RebornAdminSecretDeletedResponse, RebornAdminSecretResponse, RebornAdminSetRoleProductRequest,
    RebornAdminSetRoleRequest, RebornAdminSetStatusProductRequest, RebornAdminSetStatusRequest,
    RebornAdminThreadScrapeArtifactRequest, RebornAdminThreadScrapeListRequest,
    RebornAdminThreadScrapeRunArtifactRequest, RebornAdminUpdateUserProductRequest,
    RebornAdminUpdateUserRequest, RebornAdminUserCreatedResponse, RebornAdminUserDeletedResponse,
    RebornAdminUserListQuery, RebornAdminUserListResponse, RebornAdminUserRequest,
    RebornAdminUserResponse, RebornAdminUserSecretsListResponse, RebornAttachmentBytes,
    RebornAttachmentRequest, RebornAuthAccount, RebornAutomationActiveHold,
    RebornAutomationHoldReason, RebornAutomationInfo, RebornAutomationMutationResponse,
    RebornAutomationRecentRunInfo, RebornAutomationRecentRunStatus, RebornAutomationRequest,
    RebornAutomationRunMutationResult, RebornAutomationRunMutationStatus,
    RebornAutomationRunStatus, RebornAutomationSource, RebornAutomationState,
    RebornCancelRunResponse, RebornChannelConnectAction, RebornChannelConnectStrategy,
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
    RebornNotificationChannel, RebornNotificationChannelsResponse, RebornOperatorArea,
    RebornOperatorCommandPlaneResponse, RebornOperatorConfigDiagnostic,
    RebornOperatorConfigDiagnosticSeverity, RebornOperatorConfigEntry,
    RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetProductRequest, RebornOperatorConfigSetRequest,
    RebornOperatorConfigValidateRequest, RebornOperatorConfigValidateResponse,
    RebornOperatorLogsQuery, RebornOperatorServiceLifecycleAction,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupRequest, RebornOperatorSetupResponse,
    RebornOperatorSetupStatus, RebornOperatorSetupStep, RebornOperatorSetupStepStatus,
    RebornOperatorSurfaceStatus, RebornOutboundDeliveryTargetCapabilities,
    RebornOutboundDeliveryTargetChannel, RebornOutboundDeliveryTargetDescription,
    RebornOutboundDeliveryTargetDisplayName, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornOutboundPreferencesService, RebornProductCommandEffect, RebornProductCommandInfo,
    RebornProductCommandListResponse, RebornProjectFsListRequest, RebornProjectFsListResponse,
    RebornProjectFsReadRequest, RebornProjectFsStatRequest, RebornProjectFsStatResponse,
    RebornProjectInfo, RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse,
    RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest,
    RebornRenameAutomationProductRequest, RebornResolveGateResponse, RebornResumeGateResponse,
    RebornRetryRunResponse, RebornRunArtifact, RebornRunArtifactRequest, RebornServices,
    RebornSetNotificationChannelsRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillInfo, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornSkillSourceKind, RebornSkillTrustLevel,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornThreadArtifact, RebornThreadArtifactRequest, RebornTimelineRequest,
    RebornTimelineResponse, RebornTraceCreditsResponse, RebornTraceHoldAuthorizeProductRequest,
    RebornTraceHoldAuthorizeResponse, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    RebornVendorAuthAccounts, RegistrationChannelNotificationSetupService, RunArtifactLogs,
    RunArtifactMessage, RunArtifactRedaction, RunArtifactRunTimings, RunArtifactToolCall,
    SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY, SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_CONTENT_VIEW,
    SKILL_INSTALL_CAPABILITY, SKILL_INSTALL_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY,
    SKILL_REMOVE_CAPABILITY_ID, SKILL_SEARCH_VIEW, SKILL_UPDATE_CAPABILITY,
    SKILL_UPDATE_CAPABILITY_ID, SKILLS_VIEW, SUBMIT_TURN_COMMAND, SettingsToolPermissionState,
    SkillsProductService, StaticOperatorStatusService, THREAD_ARTIFACT_MAX_MESSAGES,
    THREAD_ARTIFACT_SCHEMA, THREAD_ARTIFACT_VIEW, THREAD_DELETE_CAPABILITY,
    THREAD_DELETE_CAPABILITY_ID, THREADS_VIEW, TIMELINE_VIEW, TRACE_ACCOUNT_LOGIN_LINK_COMMAND,
    TRACE_ACCOUNT_TRACES_VIEW, TRACE_CREDITS_VIEW, TRACE_HOLD_AUTHORIZE_COMMAND,
    TriggerRunThreadScope, UnavailableRebornViewProvider, UnsupportedAutomationProductService,
    UnsupportedChannelNotificationSetupService, UnsupportedOperatorLogsService,
    UnsupportedOperatorServiceLifecycleService, UnsupportedOperatorStatusService,
    UnsupportedOutboundPreferencesProductService, list_outbound_delivery_targets_for_model,
    notification_channels_set_input_schema, notification_channels_set_operator_tool_info,
    outbound_delivery_synthetic_provider, outbound_delivery_targets_list_input_schema,
    parse_notification_channels_set_input, parse_outbound_delivery_targets_list_input,
    set_notification_channels_for_model,
};
pub use suggestions_observer::SuggestionsProcessCommitObserver;
pub use suggestions_store::{FilesystemSuggestionsStore, SuggestionsStore};
pub use unbound_turn::{
    UnboundTurnError, UnboundTurnOutcome, UnboundTurnService, UnboundTurnSubmission,
};

pub use product_surface_inbound::{
    DecodeInboundAttachments, IntoProductInboundCommand, ProductInboundCommand,
};
pub use workflow::DefaultProductSurface;
