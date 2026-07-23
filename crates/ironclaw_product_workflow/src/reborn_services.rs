//! WebUI-facing Reborn service facade.
//!
//! This module is the stable high-level API beta WebUI route handlers use
//! instead of reaching into turn coordination, thread stores, runtime lanes, DB
//! stores, dispatchers, or capability hosts directly.

// arch-exempt: large_file, holds the ProductSurface facade awaiting the JIT domain-port split, plan #5985

use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex as StdMutex, Weak},
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use futures::future::try_join_all;
use ironclaw_attachments::InboundAttachment;
use ironclaw_auth::{
    AuthFlowStatus, AuthProductScope, AuthProviderId, CredentialAccountId,
    CredentialAccountProjection, CredentialAccountStatus, CredentialAccountUpdateBinding,
    ProviderScope,
};
use ironclaw_common::{AutomationName, AutomationNameError};
use ironclaw_host_api::{
    ActivityId, AgentId, CapabilityId, EffectKind, ExtensionId, FailureKind, GrantConstraints,
    InvocationId, Outcome, OutcomeRefs, PermissionMode, Principal, ProjectId, Resolution,
    ResourceScope, ResultPreviewMeta, ResultProgress, ResultRef, SafeSummary, SecretHandle,
    TenantId, TerminateHint, ThreadId, ToolVerdict, UserId,
};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductWorkflowRejectionKind, ProjectionStream,
    ProjectionSubscriptionRequest,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessageReplay, AttachmentRef, EnsureThreadRequest,
    MessageContent, MessageStatus, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, ThreadHistory, ThreadHistoryRequest,
    ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, GateRef, GetRunStateRequest, IdempotencyKey, ResumeTurnPrecondition,
    ResumeTurnRequest, RetryTurnRequest, SanitizedCancelReason, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnScope, TurnStatus,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, mpsc};
use url::Url;
use uuid::Uuid;

use crate::{
    ApprovalInteractionDecision, ApprovalInteractionService, AuthInteractionDecision,
    AuthInteractionRejectionKind, AuthInteractionService, LifecycleProductFacade,
    ListPendingApprovalsRequest, ProductWorkflowError, ResolveApprovalInteractionRequest,
    ResolveApprovalInteractionResponse, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse, UnsupportedLifecycleProductFacade, WebUiAuthenticatedCaller,
    WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiGateResolution, WebUiInboundCommand,
    WebUiInboundValidationCode, WebUiInboundValidationError, WebUiListAutomationsRequest,
    WebUiListThreadsRequest, WebUiRenameAutomationRequest, WebUiResolveGateRequest,
    WebUiRetryRunRequest, WebUiSendMessageRequest,
    approval_interaction::RejectingApprovalInteractionService,
    auth_interaction::RejectingAuthInteractionService,
    binding_ref::{
        DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_reply_target_binding_ref,
        bounded_source_binding_ref,
    },
    is_approval_gate_ref, is_auth_gate_ref, thread_metadata_is_automation_trigger,
};

mod admin_configuration;
mod admin_users;
mod error;
mod extension_credentials;
mod extension_onboarding;
mod extension_setup_credentials;
mod extensions;
mod fs_browse;
mod lifecycle_setup;
mod llm_config;
mod log_views;
mod operator_command_views;
mod operator_config_views;
mod outbound_delivery_capability_surface;
mod outbound_preferences;
mod outbound_views;
mod product_operations;
mod project_fs;
mod projects;
mod run_artifact;
mod trace_credits;
mod types;
mod views;

pub use admin_configuration::{
    ADMIN_CONFIGURATION_REPLACE_CAPABILITY, ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID,
    ADMIN_CONFIGURATION_VIEW, RebornAdminConfigurationField, RebornAdminConfigurationGroup,
    RebornAdminConfigurationListResponse, RebornAdminConfigurationUse,
};
use admin_users::{
    ADMIN_USER_LIST_DEFAULT_LIMIT, ADMIN_USER_LIST_MAX_LIMIT, RejectingAdminUserService,
};
pub use admin_users::{
    AdminCreateUserFields, AdminCreatedUser, AdminUserError, AdminUserRecord, AdminUserRole,
    AdminUserSecretMeta, AdminUserService, AdminUserStatus, RebornAdminCreateUserRequest,
    RebornAdminDeleteSecretProductRequest, RebornAdminPutSecretProductRequest,
    RebornAdminPutSecretRequest, RebornAdminSecretDeletedResponse, RebornAdminSecretResponse,
    RebornAdminSetRoleProductRequest, RebornAdminSetRoleRequest,
    RebornAdminSetStatusProductRequest, RebornAdminSetStatusRequest,
    RebornAdminUpdateUserProductRequest, RebornAdminUpdateUserRequest,
    RebornAdminUserCreatedResponse, RebornAdminUserDeletedResponse, RebornAdminUserListQuery,
    RebornAdminUserListResponse, RebornAdminUserRequest, RebornAdminUserResponse,
    RebornAdminUserSecretsListResponse,
};
pub use error::{RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind};
pub use trace_credits::{
    RebornAccountLoginLinkResponse, RebornAccountTrace, RebornAccountTracesResponse,
    RebornTraceCreditsResponse, RebornTraceHoldAuthorizeResponse, TRACE_ACCOUNT_TRACES_VIEW,
    TRACE_CREDITS_VIEW,
};

pub use extensions::{EXTENSION_REGISTRY_VIEW, EXTENSIONS_VIEW};
pub use fs_browse::{
    FilesystemBrowseReader, FsMount, RebornFsListRequest, RebornFsListResponse, RebornFsMountInfo,
    RebornFsMountsRequest, RebornFsMountsResponse, RebornFsReadRequest, RebornFsStatRequest,
    RebornFsStatResponse,
};
use ironclaw_approvals::{
    AUTO_APPROVE_DEFAULT_ENABLED, AutoApproveSettingKey, AutoApproveSettingStore,
    PersistentApprovalAction, PersistentApprovalPolicyError, PersistentApprovalPolicyInput,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore, ToolPermissionOverride,
    ToolPermissionOverrideInput, ToolPermissionOverrideKey, ToolPermissionOverrideStore,
    ToolPermissionState, permission_mode_allows_persistent_approval,
};
pub use lifecycle_setup::EXTENSION_SETUP_VIEW;
pub use llm_config::{
    ActiveModelReader, CodexLoginStart, LLM_CONFIG_VIEW, LlmActiveSelection, LlmConfigService,
    LlmConfigServiceError, LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest, LlmProbeResult,
    LlmProviderView, NearAiAuthProvider, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, SetActiveLlmRequest,
    UpsertLlmProviderRequest,
};
pub use log_views::{LOGS_VIEW, OPERATOR_LOGS_VIEW};
pub use operator_command_views::{
    OPERATOR_DIAGNOSTICS_VIEW, OPERATOR_SETUP_VIEW, OPERATOR_STATUS_VIEW,
};
pub use operator_config_views::{
    OPERATOR_CONFIG_KEY_VIEW, OPERATOR_CONFIG_LIST_VIEW, OPERATOR_CONFIG_VALIDATE_VIEW,
};
pub use outbound_delivery_capability_surface::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION,
    OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
    OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION, OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
    OutboundDeliveryCapabilityInputError, OutboundDeliveryTargetSetInput,
    OutboundDeliveryTargetsListInput, list_outbound_delivery_targets_for_model,
    outbound_delivery_synthetic_provider, outbound_delivery_target_set_input_schema,
    outbound_delivery_target_set_operator_tool_info, outbound_delivery_targets_list_input_schema,
    parse_outbound_delivery_target_set_input, parse_outbound_delivery_targets_list_input,
    set_outbound_delivery_target_for_model,
};
pub use outbound_preferences::RebornOutboundPreferencesFacade;
pub use outbound_views::{OUTBOUND_DELIVERY_TARGETS_VIEW, OUTBOUND_PREFERENCES_VIEW};
pub use project_fs::{
    ProjectFilesystemReader, ProjectFsEntry, ProjectFsEntryKind, ProjectFsError, ProjectFsFile,
    ProjectFsStat, RebornProjectFsListRequest, RebornProjectFsListResponse,
    RebornProjectFsReadRequest, RebornProjectFsStatRequest, RebornProjectFsStatResponse,
};
pub use projects::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornProjectInfo, RebornProjectMemberInfo,
    RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole, RebornProjectState,
    RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
};
pub use run_artifact::{
    RUN_ARTIFACT_SCHEMA, RUN_ARTIFACT_VIEW, RebornRunArtifact, RebornRunArtifactRequest,
    RunArtifactLogs, RunArtifactMessage, RunArtifactRedaction, RunArtifactToolCall,
};
pub use types::{
    RebornAccountBindingSource, RebornAttachmentBytes, RebornAttachmentRequest, RebornAuthAccount,
    RebornAutomationActiveHold, RebornAutomationHoldReason, RebornAutomationInfo,
    RebornAutomationMutationResponse, RebornAutomationRecentRunInfo,
    RebornAutomationRecentRunStatus, RebornAutomationRequest, RebornAutomationRunStatus,
    RebornAutomationSource, RebornAutomationState, RebornCancelRunResponse,
    RebornChannelConnectAction, RebornChannelConnectStrategy, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionCredentialSetup, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionOnboardingPayload, RebornExtensionOnboardingState, RebornExtensionRegistryEntry,
    RebornExtensionRegistryResponse, RebornExtensionSetupField, RebornExtensionSetupSecret,
    RebornExtensionSurface, RebornGetRunStateRequest, RebornGetRunStateResponse,
    RebornGlobalAutoApproveRequest, RebornGlobalAutoApproveResponse, RebornListAutomationsResponse,
    RebornListThreadsResponse, RebornLogEntry, RebornLogLevel, RebornLogQueryRequest,
    RebornLogQueryResponse, RebornOperatorArea, RebornOperatorCommandPlaneResponse,
    RebornOperatorConfigDiagnostic, RebornOperatorConfigDiagnosticSeverity,
    RebornOperatorConfigEntry, RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetProductRequest, RebornOperatorConfigSetRequest,
    RebornOperatorConfigValidateRequest, RebornOperatorConfigValidateResponse,
    RebornOperatorLogsQuery, RebornOperatorServiceLifecycleAction,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupRequest, RebornOperatorSetupResponse,
    RebornOperatorSetupStatus, RebornOperatorSetupStep, RebornOperatorSetupStepStatus,
    RebornOperatorStatusCheck, RebornOperatorStatusResponse, RebornOperatorStatusSeverity,
    RebornOperatorStatusState, RebornOperatorSurfaceStatus, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetChannel,
    RebornOutboundDeliveryTargetDescription, RebornOutboundDeliveryTargetDisplayName,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
    RebornRenameAutomationProductRequest, RebornResolveGateResponse, RebornResumeGateResponse,
    RebornRetryRunResponse, RebornServiceLifecycleAction, RebornServiceLifecycleRequest,
    RebornServiceLifecycleResponse, RebornServiceLifecycleState,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillInfo, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornSkillSourceKind, RebornSkillTrustLevel,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornStreamEventsSubscription,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    RebornTraceHoldAuthorizeProductRequest, RebornVendorAuthAccounts,
};
pub use views::{
    ProductOperation, ProductOperationId, ProductOperationRequest, ProductOperationResponse,
    ProductView, RebornViewDescriptor, RebornViewPage, RebornViewProvider, RebornViewQuery,
    UnavailableRebornViewProvider,
};

type SkillActivationRecorder =
    dyn Fn(&TurnScope, &AcceptedMessageRef, &str) -> Result<(), RebornServicesError> + Send + Sync;
type SkillActivationClearer =
    dyn Fn(&TurnScope, &AcceptedMessageRef) -> Result<(), RebornServicesError> + Send + Sync;

const AUTO_APPROVE_CONFIG_KEY: &str = "agent.auto_approve_tools";
const TOOL_CONFIG_PREFIX: &str = "tool.";
pub const OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID: &str =
    "builtin.operator_config_set_auto_approve";
pub const OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID);
pub const OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID: &str =
    "builtin.operator_config_set_tool_permission";
pub const OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID);
pub const OPERATOR_SETUP_RUN_CAPABILITY_ID: &str = "builtin.operator_setup_run";
pub const OPERATOR_SETUP_RUN_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(OPERATOR_SETUP_RUN_CAPABILITY_ID);
pub const OUTBOUND_PREFERENCES_SET_CAPABILITY_ID: &str = "builtin.outbound_preferences_set";
pub const OUTBOUND_PREFERENCES_SET_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID);
pub const LLM_PROVIDER_UPSERT_CAPABILITY_ID: &str = "builtin.llm_provider_upsert";
pub const LLM_PROVIDER_UPSERT_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(LLM_PROVIDER_UPSERT_CAPABILITY_ID);
pub const LLM_PROVIDER_DELETE_CAPABILITY_ID: &str = "builtin.llm_provider_delete";
pub const LLM_PROVIDER_DELETE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(LLM_PROVIDER_DELETE_CAPABILITY_ID);
pub const LLM_ACTIVE_SET_CAPABILITY_ID: &str = "builtin.llm_active_set";
pub const LLM_ACTIVE_SET_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(LLM_ACTIVE_SET_CAPABILITY_ID);
pub const EXTENSION_INSTALL_CAPABILITY_ID: &str = "builtin.extension_install";
pub const EXTENSION_INSTALL_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(EXTENSION_INSTALL_CAPABILITY_ID);
pub const EXTENSION_IMPORT_CAPABILITY_ID: &str = "builtin.extension_import";
pub const EXTENSION_IMPORT_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(EXTENSION_IMPORT_CAPABILITY_ID);
pub const EXTENSION_ACTIVATE_CAPABILITY_ID: &str = "builtin.extension_activate";
pub const EXTENSION_ACTIVATE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(EXTENSION_ACTIVATE_CAPABILITY_ID);
pub const EXTENSION_REMOVE_CAPABILITY_ID: &str = "builtin.extension_remove";
pub const EXTENSION_REMOVE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(EXTENSION_REMOVE_CAPABILITY_ID);
pub const EXTENSION_SETUP_SUBMIT_CAPABILITY_ID: &str = "builtin.extension_setup_submit";
pub const EXTENSION_SETUP_SUBMIT_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(EXTENSION_SETUP_SUBMIT_CAPABILITY_ID);
pub const PROJECT_UPDATE_CAPABILITY_ID: &str = "builtin.project_update";
pub const PROJECT_UPDATE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(PROJECT_UPDATE_CAPABILITY_ID);
pub const PROJECT_DELETE_CAPABILITY_ID: &str = "builtin.project_delete";
pub const PROJECT_DELETE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(PROJECT_DELETE_CAPABILITY_ID);
pub const THREAD_DELETE_CAPABILITY_ID: &str = "builtin.thread_delete";
pub const THREAD_DELETE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(THREAD_DELETE_CAPABILITY_ID);
pub const ADMIN_USER_UPDATE_CAPABILITY_ID: &str = "builtin.admin_user_update";
pub const ADMIN_USER_UPDATE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_USER_UPDATE_CAPABILITY_ID);
pub const ADMIN_USER_SET_STATUS_CAPABILITY_ID: &str = "builtin.admin_user_set_status";
pub const ADMIN_USER_SET_STATUS_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_USER_SET_STATUS_CAPABILITY_ID);
pub const ADMIN_USER_SET_ROLE_CAPABILITY_ID: &str = "builtin.admin_user_set_role";
pub const ADMIN_USER_SET_ROLE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_USER_SET_ROLE_CAPABILITY_ID);
pub const ADMIN_USER_DELETE_CAPABILITY_ID: &str = "builtin.admin_user_delete";
pub const ADMIN_USER_DELETE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_USER_DELETE_CAPABILITY_ID);
pub const ADMIN_USER_PUT_SECRET_CAPABILITY_ID: &str = "builtin.admin_user_put_secret";
pub const ADMIN_USER_PUT_SECRET_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_USER_PUT_SECRET_CAPABILITY_ID);
pub const ADMIN_USER_DELETE_SECRET_CAPABILITY_ID: &str = "builtin.admin_user_delete_secret";
pub const ADMIN_USER_DELETE_SECRET_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_USER_DELETE_SECRET_CAPABILITY_ID);
pub const PROJECT_MEMBER_ADD_CAPABILITY_ID: &str = "builtin.project_member_add";
pub const PROJECT_MEMBER_ADD_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(PROJECT_MEMBER_ADD_CAPABILITY_ID);
pub const PROJECT_MEMBER_UPDATE_CAPABILITY_ID: &str = "builtin.project_member_update";
pub const PROJECT_MEMBER_UPDATE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(PROJECT_MEMBER_UPDATE_CAPABILITY_ID);
pub const PROJECT_MEMBER_REMOVE_CAPABILITY_ID: &str = "builtin.project_member_remove";
pub const PROJECT_MEMBER_REMOVE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(PROJECT_MEMBER_REMOVE_CAPABILITY_ID);
pub const AUTOMATION_PAUSE_CAPABILITY_ID: &str = "builtin.automation_pause";
pub const AUTOMATION_PAUSE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(AUTOMATION_PAUSE_CAPABILITY_ID);
pub const AUTOMATION_RESUME_CAPABILITY_ID: &str = "builtin.automation_resume";
pub const AUTOMATION_RESUME_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(AUTOMATION_RESUME_CAPABILITY_ID);
pub const AUTOMATION_RENAME_CAPABILITY_ID: &str = "builtin.automation_rename";
pub const AUTOMATION_RENAME_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(AUTOMATION_RENAME_CAPABILITY_ID);
pub const AUTOMATION_DELETE_CAPABILITY_ID: &str = "builtin.automation_delete";
pub const AUTOMATION_DELETE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(AUTOMATION_DELETE_CAPABILITY_ID);
pub const CREATE_THREAD_OPERATION: ProductOperation<
    WebUiCreateThreadRequest,
    RebornCreateThreadResponse,
> = ProductOperation::new(ProductOperationId::CreateThread);
pub const SUBMIT_TURN_OPERATION: ProductOperation<
    WebUiSendMessageRequest,
    RebornSubmitTurnResponse,
> = ProductOperation::new(ProductOperationId::SubmitTurn);
pub const CANCEL_RUN_OPERATION: ProductOperation<WebUiCancelRunRequest, RebornCancelRunResponse> =
    ProductOperation::new(ProductOperationId::CancelRun);
pub const RESOLVE_GATE_OPERATION: ProductOperation<
    WebUiResolveGateRequest,
    RebornResolveGateResponse,
> = ProductOperation::new(ProductOperationId::ResolveGate);
pub const RETRY_RUN_OPERATION: ProductOperation<WebUiRetryRunRequest, RebornRetryRunResponse> =
    ProductOperation::new(ProductOperationId::RetryRun);
pub const PROJECT_CREATE_OPERATION: ProductOperation<
    RebornCreateProjectRequest,
    RebornProjectResponse,
> = ProductOperation::new(ProductOperationId::ProjectCreate);
pub const PROJECT_FS_READ_OPERATION: ProductOperation<RebornProjectFsReadRequest, ProjectFsFile> =
    ProductOperation::new(ProductOperationId::ProjectFsRead);
pub const FS_READ_OPERATION: ProductOperation<RebornFsReadRequest, ProjectFsFile> =
    ProductOperation::new(ProductOperationId::FsRead);
pub const ATTACHMENT_READ_OPERATION: ProductOperation<
    RebornAttachmentRequest,
    RebornAttachmentBytes,
> = ProductOperation::new(ProductOperationId::AttachmentRead);
pub const TRACE_ACCOUNT_LOGIN_LINK_OPERATION: ProductOperation<
    serde_json::Value,
    RebornAccountLoginLinkResponse,
> = ProductOperation::new(ProductOperationId::TraceAccountLoginLink);
pub const TRACE_HOLD_AUTHORIZE_OPERATION: ProductOperation<
    RebornTraceHoldAuthorizeProductRequest,
    RebornTraceHoldAuthorizeResponse,
> = ProductOperation::new(ProductOperationId::TraceHoldAuthorize);
pub const OPERATOR_CONFIG_SET_KEY_OPERATION: ProductOperation<
    RebornOperatorConfigSetProductRequest,
    RebornOperatorConfigGetResponse,
> = ProductOperation::new(ProductOperationId::OperatorConfigSetKey);
pub const OPERATOR_SERVICE_LIFECYCLE_OPERATION: ProductOperation<
    RebornOperatorServiceLifecycleRequest,
    RebornOperatorCommandPlaneResponse,
> = ProductOperation::new(ProductOperationId::OperatorServiceLifecycle);
pub const LLM_TEST_CONNECTION_OPERATION: ProductOperation<serde_json::Value, LlmProbeResult> =
    ProductOperation::new(ProductOperationId::LlmTestConnection);
pub const LLM_LIST_MODELS_OPERATION: ProductOperation<serde_json::Value, LlmModelsResult> =
    ProductOperation::new(ProductOperationId::LlmListModels);
pub const LLM_NEARAI_LOGIN_OPERATION: ProductOperation<serde_json::Value, NearAiLoginStart> =
    ProductOperation::new(ProductOperationId::LlmNearAiLogin);
pub const LLM_NEARAI_WALLET_LOGIN_OPERATION: ProductOperation<
    serde_json::Value,
    NearAiWalletLoginResult,
> = ProductOperation::new(ProductOperationId::LlmNearAiWalletLogin);
pub const LLM_CODEX_LOGIN_OPERATION: ProductOperation<serde_json::Value, CodexLoginStart> =
    ProductOperation::new(ProductOperationId::LlmCodexLogin);
pub const ADMIN_USER_CREATE_OPERATION: ProductOperation<
    RebornAdminCreateUserRequest,
    RebornAdminUserCreatedResponse,
> = ProductOperation::new(ProductOperationId::AdminUserCreate);
pub const ADMIN_USER_DELETE_SECRET_OPERATION: ProductOperation<
    RebornAdminDeleteSecretProductRequest,
    RebornAdminSecretDeletedResponse,
> = ProductOperation::new(ProductOperationId::AdminUserDeleteSecret);
pub const AUTOMATION_PAUSE_OPERATION: ProductOperation<
    RebornAutomationRequest,
    RebornAutomationMutationResponse,
> = ProductOperation::new(ProductOperationId::AutomationPause);
pub const AUTOMATION_RESUME_OPERATION: ProductOperation<
    RebornAutomationRequest,
    RebornAutomationMutationResponse,
> = ProductOperation::new(ProductOperationId::AutomationResume);
pub const AUTOMATION_RENAME_OPERATION: ProductOperation<
    RebornRenameAutomationProductRequest,
    RebornAutomationMutationResponse,
> = ProductOperation::new(ProductOperationId::AutomationRename);
pub const AUTOMATION_DELETE_OPERATION: ProductOperation<
    RebornAutomationRequest,
    RebornAutomationMutationResponse,
> = ProductOperation::new(ProductOperationId::AutomationDelete);
pub const THREADS_VIEW: ProductView<WebUiListThreadsRequest, RebornListThreadsResponse> =
    ProductView::paginated("threads");
pub const TIMELINE_VIEW: ProductView<RebornTimelineRequest, RebornTimelineResponse> =
    ProductView::paginated("timeline");
pub const GLOBAL_AUTO_APPROVE_VIEW: ProductView<
    RebornGlobalAutoApproveRequest,
    RebornGlobalAutoApproveResponse,
> = ProductView::unpaginated("global_auto_approve");
pub const AUTOMATIONS_VIEW: ProductView<
    WebUiListAutomationsRequest,
    RebornListAutomationsResponse,
> = ProductView::unpaginated("automations");
pub const PROJECT_FS_LIST_VIEW: ProductView<
    RebornProjectFsListRequest,
    RebornProjectFsListResponse,
> = ProductView::unpaginated("project_fs_list");
pub const PROJECT_FS_STAT_VIEW: ProductView<
    RebornProjectFsStatRequest,
    RebornProjectFsStatResponse,
> = ProductView::unpaginated("project_fs_stat");
pub const FS_MOUNTS_VIEW: ProductView<RebornFsMountsRequest, RebornFsMountsResponse> =
    ProductView::unpaginated("fs_mounts");
pub const FS_LIST_VIEW: ProductView<RebornFsListRequest, RebornFsListResponse> =
    ProductView::unpaginated("fs_list");
pub const FS_STAT_VIEW: ProductView<RebornFsStatRequest, RebornFsStatResponse> =
    ProductView::unpaginated("fs_stat");
pub const PROJECTS_VIEW: ProductView<RebornListProjectsRequest, RebornListProjectsResponse> =
    ProductView::unpaginated("projects");
pub const PROJECT_VIEW: ProductView<RebornGetProjectRequest, RebornProjectResponse> =
    ProductView::unpaginated("project");
pub const PROJECT_MEMBERS_VIEW: ProductView<RebornListMembersRequest, RebornListMembersResponse> =
    ProductView::unpaginated("project_members");
pub const ADMIN_USERS_VIEW: ProductView<RebornAdminUserListQuery, RebornAdminUserListResponse> =
    ProductView::paginated("admin_users");
pub const ADMIN_USER_VIEW: ProductView<RebornAdminUserRequest, RebornAdminUserResponse> =
    ProductView::unpaginated("admin_user");
pub const ADMIN_USER_SECRETS_VIEW: ProductView<
    RebornAdminUserRequest,
    RebornAdminUserSecretsListResponse,
> = ProductView::unpaginated("admin_user_secrets");
pub const SKILL_INSTALL_CAPABILITY_ID: &str = "builtin.skill_install";
pub const SKILL_INSTALL_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(SKILL_INSTALL_CAPABILITY_ID);
pub const SKILL_UPDATE_CAPABILITY_ID: &str = "builtin.skill_update";
pub const SKILL_UPDATE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(SKILL_UPDATE_CAPABILITY_ID);
pub const SKILL_REMOVE_CAPABILITY_ID: &str = "builtin.skill_remove";
pub const SKILL_REMOVE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(SKILL_REMOVE_CAPABILITY_ID);
pub const SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID: &str = "builtin.skill_auto_activate_set";
pub const SKILL_AUTO_ACTIVATE_SET_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID);
pub const SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID: &str =
    "builtin.skill_auto_activate_learned_set";
pub const SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID);
pub const SKILLS_VIEW: ProductView<serde_json::Value, RebornSkillListResponse> =
    ProductView::unpaginated("skills");
pub const SKILL_SEARCH_VIEW: ProductView<serde_json::Value, RebornSkillSearchResponse> =
    ProductView::unpaginated("skill_search");
pub const SKILL_CONTENT_VIEW: ProductView<serde_json::Value, RebornSkillContentResponse> =
    ProductView::unpaginated("skill_content");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornOperatorToolInfo {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub description: Arc<str>,
    pub default_permission: PermissionMode,
    pub effects: Arc<[EffectKind]>,
}

#[async_trait]
pub trait RebornOperatorToolCatalog: Send + Sync {
    /// Tools visible to `caller` in the operator/settings surface (#5459 P1).
    ///
    /// The settings/tools routes are authenticated-caller routes (not
    /// operator-gated), so a member reads this catalog. It MUST therefore be
    /// filtered by installation owner exactly like the model capability
    /// surface: tenant-shared tools for everyone, user-private tools only for
    /// their owner. An unfiltered catalog would disclose another user's
    /// private install (its capability id, description, effects) — the leak
    /// this parameter closes.
    async fn list_operator_tools(&self, caller: &UserId) -> Vec<RebornOperatorToolInfo>;
}

#[derive(Clone)]
struct RebornOperatorApprovalConfig {
    overrides: Arc<dyn ToolPermissionOverrideStore>,
    auto_approve: Arc<dyn AutoApproveSettingStore>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
    tool_catalog: Arc<dyn RebornOperatorToolCatalog>,
}
type ThreadOperationLocks = StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>;

const OPERATOR_LOGS_DEFAULT_LIMIT: u32 = 100;
const OPERATOR_LOGS_MAX_LIMIT: u32 = 500;
const OPERATOR_LOGS_CURSOR_MAX_BYTES: usize = 512;
const OPERATOR_LOGS_TARGET_MAX_BYTES: usize = 256;
const OPERATOR_LOGS_CONTEXT_MAX_BYTES: usize = 256;
const OPERATOR_LOG_CONTEXT_TRUNCATED_SUFFIX: &str = " ... [truncated]";

const NOTICE_BLOCKED_APPROVAL: &str = "An approval gate is open on this thread — resolve it (approve or deny) before continuing, then resend your message.";
const NOTICE_BLOCKED_AUTH: &str = "An authentication gate is open on this thread — complete authentication before continuing, then resend your message.";
const NOTICE_BUSY_GENERIC: &str = "Ironclaw is still working on a previous message — resend yours once the current task finishes.";

fn rejected_busy_notice(status: TurnStatus) -> String {
    match status {
        TurnStatus::BlockedApproval => NOTICE_BLOCKED_APPROVAL.to_string(),
        TurnStatus::BlockedAuth => NOTICE_BLOCKED_AUTH.to_string(),
        _ => NOTICE_BUSY_GENERIC.to_string(),
    }
}

/// The caller's durable auth-account signal for one channel extension's vendor
/// — the raw inputs the extensions-list facade feeds to
/// [`ironclaw_auth::project_auth_account_state`] so an account renders its real
/// §6.3 state (`expired` / `refresh-failed` / `authenticating`) plus a typed
/// last error, instead of the connected/disconnected collapse the
/// [`ChannelConnectionFacade::caller_channel_connections`] bool alone permits.
///
/// Both inputs are optional. A facade that only knows the caller holds a live
/// grant leaves both `None` and the projection falls back to the connection
/// bool (a live grant backfills to `connected`, MIG-1); a facade that reads the
/// durable credential-account status supplies `account_status` (and, mid-flow,
/// `active_flow_status`) so the wire surfaces the real state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelAuthAccountState {
    /// The caller's durable credential-account status for the extension's
    /// vendor, when the facade can read it.
    pub account_status: Option<CredentialAccountStatus>,
    /// A live (non-terminal) auth flow for the extension's vendor, when one is
    /// in progress — projects to `authenticating`.
    pub active_flow_status: Option<AuthFlowStatus>,
}

/// Per-user channel connection state. Returns, for the calling user, which
/// channel extensions they have personally connected (for example, Slack OAuth).
/// Keyed by channel package id (e.g. `"slack"`) -> `true` when connected.
/// Only channels that have a per-user connection concept appear in the map;
/// absence means "no per-user connection concept for this channel".
#[async_trait]
pub trait ChannelConnectionFacade: Send + Sync {
    async fn caller_channel_connections(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<std::collections::HashMap<String, bool>, RebornServicesError>;

    /// The caller's durable auth-account signal per channel extension, keyed by
    /// channel package id — richer than the connected/disconnected bool
    /// [`Self::caller_channel_connections`] returns. Lets the extensions wire
    /// project the shared §6.3 auth-account state (`expired` / `refresh-failed`)
    /// and its typed last error for each vendor account.
    ///
    /// Default: empty. A facade that does not yet read durable credential-account
    /// status reports none and the wire falls back to the connection bool; the
    /// production channel-connection facade overrides this to project each
    /// caller's account status.
    async fn caller_channel_account_states(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<std::collections::HashMap<String, ChannelAuthAccountState>, RebornServicesError>
    {
        Ok(std::collections::HashMap::new())
    }

    async fn disconnect_channel_for_caller(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _channel: &str,
    ) -> Result<(), RebornServicesError> {
        Err(RebornServicesError::service_unavailable(false))
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticChannelConnectionFacade;

#[async_trait]
impl ChannelConnectionFacade for StaticChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<std::collections::HashMap<String, bool>, RebornServicesError> {
        Ok(std::collections::HashMap::new())
    }
}

/// Presence-only projection of one manifest-declared channel-config field.
/// Secret fields report `provided` only; stored values are never echoed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornChannelConfigField {
    /// The manifest-declared field handle (the submit key).
    pub name: String,
    /// Operator-facing label from the manifest.
    pub label: String,
    pub secret: bool,
    pub provided: bool,
}

/// The generic channel-config configure port: per-extension operator config
/// declared by the extension manifest's channel-config fields. Host
/// composition implements it over the durable installation store and the
/// scoped secret store; the setup facade routes submitted values through it
/// and derives config completeness from the field status.
#[async_trait]
pub trait ChannelConfigFacade: Send + Sync {
    /// Per-field presence for the extension's declared channel config.
    /// Empty when the extension declares none (or is not installed yet).
    async fn field_status(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<RebornChannelConfigField>, RebornServicesError>;

    /// Validate submitted `(handle, value)` pairs against the installed
    /// manifest's declared fields and persist them (non-secret values
    /// durably per installation, secret values into the scoped secret
    /// store). Saving while the extension is active re-runs its activation
    /// with the new values.
    async fn save_values(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), RebornServicesError>;
}

#[async_trait]
pub trait OperatorStatusService: Send + Sync {
    async fn status(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorStatusResponse, RebornServicesError>;
}

#[derive(Debug, Clone)]
pub struct StaticOperatorStatusService {
    response: RebornOperatorStatusResponse,
}

impl StaticOperatorStatusService {
    pub fn new(response: RebornOperatorStatusResponse) -> Self {
        Self { response }
    }
}

#[async_trait]
impl OperatorStatusService for StaticOperatorStatusService {
    async fn status(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorStatusResponse, RebornServicesError> {
        Ok(self.response.clone())
    }
}

#[derive(Debug, Default)]
pub struct UnsupportedOperatorStatusService;

#[async_trait]
impl OperatorStatusService for UnsupportedOperatorStatusService {
    async fn status(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorStatusResponse, RebornServicesError> {
        Err(operator_surface_unavailable())
    }
}

#[async_trait]
pub trait OperatorLogsService: Send + Sync {
    async fn query_logs(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornLogQueryRequest,
    ) -> Result<RebornLogQueryResponse, RebornServicesError>;
}

#[derive(Debug, Default)]
pub struct UnsupportedOperatorLogsService;

#[async_trait]
impl OperatorLogsService for UnsupportedOperatorLogsService {
    async fn query_logs(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornLogQueryRequest,
    ) -> Result<RebornLogQueryResponse, RebornServicesError> {
        Err(operator_surface_unavailable())
    }
}

#[async_trait]
pub trait OperatorServiceLifecycleService: Send + Sync {
    async fn control_service(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornServiceLifecycleRequest,
    ) -> Result<RebornServiceLifecycleResponse, RebornServicesError>;
}

#[derive(Debug, Default)]
pub struct UnsupportedOperatorServiceLifecycleService;

#[async_trait]
impl OperatorServiceLifecycleService for UnsupportedOperatorServiceLifecycleService {
    async fn control_service(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornServiceLifecycleRequest,
    ) -> Result<RebornServiceLifecycleResponse, RebornServicesError> {
        Ok(RebornServiceLifecycleResponse {
            action: request.action,
            state: RebornServiceLifecycleState::Unsupported,
            message: "local service lifecycle management is not wired for this runtime".to_string(),
            remediation: Some(
                "use the host process manager directly until a platform lifecycle backend is configured"
                    .to_string(),
            ),
        })
    }
}

#[async_trait]
pub trait SkillsProductFacade: Send + Sync {
    async fn list_skills(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        let _ = caller;
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn search_skills(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        let _ = (caller, query);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn read_skill_content(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        let _ = (caller, name);
        Err(RebornServicesError::service_unavailable(false))
    }
}

#[derive(Debug, Default)]
pub struct UnsupportedSkillsProductFacade;

impl UnsupportedSkillsProductFacade {
    pub fn new_static() -> Self {
        Self
    }
}

#[async_trait]
impl SkillsProductFacade for UnsupportedSkillsProductFacade {}

#[async_trait]
pub trait OutboundPreferencesProductFacade: Send + Sync {
    /// Return the authenticated caller's scoped outbound preferences.
    ///
    /// Real implementations must scope stored preferences by the caller's
    /// tenant/user identity. The Phase 1 unsupported implementation returns an
    /// empty projection so read callers can treat "not configured yet" as a
    /// stable state while mutation and target inventory remain fail-closed.
    async fn get_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError>;

    /// Persist the caller's scoped outbound delivery preferences.
    ///
    /// Implementations must scope writes by the caller's tenant/user identity.
    /// `RebornServices` installs `UnsupportedOutboundPreferencesProductFacade`
    /// by default, which keeps Phase 1 mutation attempts fail-closed with a
    /// non-retryable service-unavailable response until a real facade is wired.
    async fn set_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError>;

    /// List delivery targets available to the authenticated caller.
    ///
    /// Implementations must scope target inventory by the caller's tenant/user
    /// identity. `RebornServices` installs
    /// `UnsupportedOutboundPreferencesProductFacade` by default, which keeps
    /// Phase 1 target discovery fail-closed with a non-retryable
    /// service-unavailable response until a real facade is wired.
    async fn list_outbound_delivery_targets(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError>;
}

#[derive(Debug)]
pub struct UnsupportedOutboundPreferencesProductFacade;

impl UnsupportedOutboundPreferencesProductFacade {
    pub fn new_static() -> Self {
        Self
    }
}

#[async_trait]
impl OutboundPreferencesProductFacade for UnsupportedOutboundPreferencesProductFacade {
    async fn get_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Ok(RebornOutboundPreferencesResponse::default())
    }

    async fn set_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Err(outbound_preferences_unavailable())
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        Err(outbound_preferences_unavailable())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionCredentialStatusRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub setup: crate::LifecycleExtensionCredentialSetup,
    pub provider_scopes: Vec<ProviderScope>,
    pub requester_extension: ExtensionId,
}

#[derive(Debug)]
pub struct ExtensionCredentialSubmitRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: String,
    pub requester_extension: ExtensionId,
    pub existing_account: Option<CredentialAccountUpdateBinding>,
    pub secret: SecretString,
}

#[async_trait]
pub trait ExtensionCredentialSetupService: Send + Sync {
    async fn credential_status(
        &self,
        request: ExtensionCredentialStatusRequest,
    ) -> Result<Option<CredentialAccountProjection>, RebornServicesError>;

    async fn submit_manual_token(
        &self,
        request: ExtensionCredentialSubmitRequest,
    ) -> Result<CredentialAccountId, RebornServicesError>;
}

/// Product caller scope for actions that must run against a concrete agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAgentBoundCaller {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
}

impl ProductAgentBoundCaller {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationListRequest {
    pub limit: usize,
    pub run_limit: usize,
    /// When `true`, include completed (fire-once) automations alongside the
    /// active ones. When `false` (the default), only active automations are
    /// returned. Facades apply `limit` after this filter, so a full page of
    /// active automations is returned regardless of how many completed ones
    /// exist.
    pub include_completed: bool,
}

/// Stored scope of a trigger-fired thread, returned by
/// `AutomationProductFacade::resolve_run_thread_scope`.
///
/// Trigger threads are written by `record_trigger_prompt` with:
///  - `agent_id` = trigger record's `agent_id` (or default agent)
///  - `project_id` = trigger record's `project_id`
///  - `owner_user_id` = `Some(creator_user_id)` (the actor that fired it)
///
/// These three fields let the caller reconstruct the true `TurnScope` / `ThreadScope`
/// needed to locate the thread in storage without guessing.
#[derive(Debug, Clone)]
pub struct TriggerRunThreadScope {
    /// `agent_id` stored on the trigger record.
    pub agent_id: Option<AgentId>,
    /// `project_id` stored on the trigger record.
    pub project_id: Option<ProjectId>,
    /// `creator_user_id` stored on the trigger record, which equals
    /// `owner_user_id` in the stored thread scope.
    pub creator_user_id: UserId,
}

#[derive(Debug, Clone)]
struct AutomationNotificationTitle(String);

impl AutomationNotificationTitle {
    const MAX_CHARS: usize = 120;

    fn from_name(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        let sanitized = trimmed
            .chars()
            .filter(|character| !character.is_control())
            .take(Self::MAX_CHARS)
            .collect::<String>();
        let sanitized = sanitized.trim();
        if sanitized.is_empty() {
            None
        } else {
            Some(Self(sanitized.to_string()))
        }
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
struct AutomationApprovalThreadCandidate {
    thread_id: ThreadId,
    title: Option<AutomationNotificationTitle>,
}

#[async_trait]
pub trait AutomationProductFacade: Send + Sync {
    async fn list_automations(
        &self,
        caller: ProductAgentBoundCaller,
        request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError>;

    async fn pause_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn resume_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn rename_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
        _name: AutomationName,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn delete_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    /// Whether the background trigger poller (scheduler) is running.
    ///
    /// Surfaced to the browser so the panel can warn that listed automations
    /// will not fire while scheduling is off. Defaults to `true` so a facade
    /// that does not know its scheduler state never produces a false "off"
    /// notice; the production facade overrides this with the real value.
    fn scheduler_enabled(&self) -> bool {
        true
    }

    /// Looks up the stored trigger-thread scope for a given `thread_id`.
    ///
    /// Scans the caller-scoped triggers for one whose run history contains
    /// `thread_id`, then returns the scope fields from that trigger record.
    /// The lookup is caller-scoped via `list_scoped_triggers`, so authorization
    /// is embedded: if the trigger exists for this caller and contains the run,
    /// the caller is permitted to access it.
    ///
    /// Returns `Ok(None)` when no caller-scoped trigger has a run with this
    /// `thread_id`. Backend lookup failures should return a stable
    /// `RebornServicesError` so outages do not masquerade as authorization
    /// misses.
    ///
    /// Implementors that do not support trigger-thread access must provide an
    /// explicit `Ok(None)` body with a short comment noting the unsupported
    /// state. No default body is provided here so a future production facade
    /// cannot silently forget to implement this method and degrade
    /// timeline/SSE/gate/cancel/run-state to 404.
    async fn resolve_run_thread_scope(
        &self,
        caller: ProductAgentBoundCaller,
        thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError>;
}

#[derive(Debug)]
pub struct UnsupportedAutomationProductFacade;

impl UnsupportedAutomationProductFacade {
    pub fn new_static() -> Self {
        Self
    }
}

#[async_trait]
impl AutomationProductFacade for UnsupportedAutomationProductFacade {
    async fn list_automations(
        &self,
        _caller: ProductAgentBoundCaller,
        _request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn pause_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn resume_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn rename_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
        _name: AutomationName,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn delete_automation(
        &self,
        _caller: ProductAgentBoundCaller,
        _automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        Err(automation_unavailable())
    }

    async fn resolve_run_thread_scope(
        &self,
        _caller: ProductAgentBoundCaller,
        _thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError> {
        // Trigger-thread access is unsupported when no automation facade is wired.
        Ok(None)
    }
}

#[derive(Clone, Copy)]
enum GateResolutionRoute {
    Approval,
    Auth,
    Generic,
}

impl GateResolutionRoute {
    fn from_run_state(
        status: TurnStatus,
        parked_gate_ref: Option<&GateRef>,
        requested_gate_ref: &GateRef,
        resolution: &WebUiGateResolution,
    ) -> Result<Self, RebornServicesError> {
        match status {
            TurnStatus::BlockedApproval => {
                validate_current_gate_ref(
                    parked_gate_ref,
                    requested_gate_ref,
                    RebornServicesErrorKind::BlockedApproval,
                )?;
                Ok(Self::Approval)
            }
            TurnStatus::BlockedAuth => {
                validate_current_gate_ref(
                    parked_gate_ref,
                    requested_gate_ref,
                    RebornServicesErrorKind::BlockedAuthentication,
                )?;
                Ok(Self::Auth)
            }
            status if status.is_terminal() => Err(RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::Conflict,
                409,
                false,
            )),
            _ => Ok(Self::from_gate_shape(requested_gate_ref, resolution)),
        }
    }

    fn from_gate_shape(gate_ref: &GateRef, resolution: &WebUiGateResolution) -> Self {
        match (
            is_approval_gate_ref(gate_ref.as_str()),
            is_auth_gate_ref(gate_ref.as_str()),
            matches!(resolution, WebUiGateResolution::CredentialProvided { .. }),
        ) {
            (true, _, _) => Self::Approval,
            (_, true, _) | (_, _, true) => Self::Auth,
            _ => Self::Generic,
        }
    }
}

fn operator_setup_validation_error(field: &str) -> RebornServicesError {
    WebUiInboundValidationError {
        field: field.to_string(),
        code: WebUiInboundValidationCode::InvalidValue,
    }
    .into()
}

/// Stable WebUI-facing facade surface for beta Reborn routes.
fn operator_setup_diagnostic(
    key: &str,
    severity: RebornOperatorConfigDiagnosticSeverity,
    reason_code: &str,
    message: &str,
    remediation: &str,
) -> RebornOperatorConfigDiagnostic {
    RebornOperatorConfigDiagnostic {
        key: key.to_string(),
        severity,
        reason_code: reason_code.to_string(),
        message: message.to_string(),
        owning_area: RebornOperatorArea::Setup,
        remediation: remediation.to_string(),
    }
}

const OPERATOR_SETUP_PROFILE_ID_MAX_BYTES: usize = 128;
const OPERATOR_SETUP_WEBUI_TOKEN_MIN_BYTES: usize = 32;
const OPERATOR_SETUP_WEBUI_TOKEN_MAX_BYTES: usize = 4096;
const OPERATOR_SETUP_REDACTED_SECRET_SENTINEL: &str = "••••••••";

fn validate_operator_setup_profile_id(
    profile_id: Option<&str>,
) -> Result<Option<String>, RebornServicesError> {
    let Some(profile_id) = profile_id else {
        return Ok(None);
    };
    let trimmed = profile_id.trim();
    if trimmed.is_empty() || trimmed.len() > OPERATOR_SETUP_PROFILE_ID_MAX_BYTES {
        return Err(operator_setup_validation_error("profile_id"));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_operator_setup_webui_access_token(
    webui_access_token: Option<&SecretString>,
) -> Result<bool, RebornServicesError> {
    let Some(token) = webui_access_token else {
        return Ok(false);
    };
    let token = token.expose_secret().trim();
    if token == OPERATOR_SETUP_REDACTED_SECRET_SENTINEL {
        return Ok(false);
    }
    if token.len() < OPERATOR_SETUP_WEBUI_TOKEN_MIN_BYTES
        || token.len() > OPERATOR_SETUP_WEBUI_TOKEN_MAX_BYTES
    {
        return Err(operator_setup_validation_error("webui_access_token"));
    }
    Ok(true)
}

fn reject_unwired_operator_setup_host_mutation(
    profile_id: Option<String>,
    webui_access_token_updated: bool,
) -> Result<(), RebornServicesError> {
    if profile_id.is_some() || webui_access_token_updated {
        return Err(RebornServicesError::service_unavailable(false));
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct OperatorSetupHostState {
    profile_id: Option<String>,
    webui_access_token_updated: bool,
}

fn setup_response_from_llm_snapshot(
    snapshot: LlmConfigSnapshot,
    diagnostics: Vec<RebornOperatorConfigDiagnostic>,
    host_state: OperatorSetupHostState,
) -> RebornOperatorSetupResponse {
    let active_provider_id = snapshot
        .active
        .as_ref()
        .map(|active| active.provider_id.clone());
    let active_model = snapshot
        .active
        .as_ref()
        .and_then(|active| active.model.clone());
    let provider_complete = active_provider_id.is_some();
    let model_complete = active_model.is_some();
    let profile_message = host_state.profile_id.as_deref().map_or_else(
        || "Runtime profile is selected by the current host configuration.".to_string(),
        |profile_id| format!("Runtime profile `{profile_id}` was accepted by the setup API."),
    );
    let webui_access_message = if host_state.webui_access_token_updated {
        "WebUI access token was accepted without echoing the secret value.".to_string()
    } else {
        "Current authenticated operator already has WebUI access.".to_string()
    };

    let status = if provider_complete && model_complete {
        RebornOperatorSetupStatus::Complete
    } else {
        RebornOperatorSetupStatus::Incomplete
    };

    RebornOperatorSetupResponse {
        area: RebornOperatorArea::Setup,
        status,
        message: if provider_complete {
            "Provider setup is available through the operator setup API.".to_string()
        } else {
            "Provider setup is incomplete.".to_string()
        },
        active_provider_id,
        active_model,
        steps: vec![
            RebornOperatorSetupStep {
                name: "provider".to_string(),
                status: if provider_complete {
                    RebornOperatorSetupStepStatus::Complete
                } else {
                    RebornOperatorSetupStepStatus::Required
                },
                message: if provider_complete {
                    "An active provider is configured.".to_string()
                } else {
                    "Select a provider before first use.".to_string()
                },
            },
            RebornOperatorSetupStep {
                name: "model".to_string(),
                status: if model_complete {
                    RebornOperatorSetupStepStatus::Complete
                } else {
                    RebornOperatorSetupStepStatus::Required
                },
                message: if model_complete {
                    "An active model is configured.".to_string()
                } else {
                    "Select a model for the active provider.".to_string()
                },
            },
            RebornOperatorSetupStep {
                name: "profile".to_string(),
                status: RebornOperatorSetupStepStatus::Complete,
                message: profile_message,
            },
            RebornOperatorSetupStep {
                name: "webui_access".to_string(),
                status: RebornOperatorSetupStepStatus::Complete,
                message: webui_access_message,
            },
        ],
        diagnostics,
    }
}

fn caller_resource_scope(caller: &WebUiAuthenticatedCaller) -> ResourceScope {
    ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn operator_config_not_wired_response() -> RebornOperatorConfigListResponse {
    RebornOperatorConfigListResponse {
        entries: Vec::new(),
        precedence: Vec::new(),
        diagnostics: vec![operator_config_surface_not_wired_diagnostic()],
    }
}

fn operator_config_unknown_key_error(field: &'static str) -> RebornServicesError {
    RebornServicesError::validation(WebUiInboundValidationError::new(
        field,
        WebUiInboundValidationCode::UnknownKey,
    ))
}

fn operator_config_invalid_value(field: &'static str) -> RebornServicesError {
    RebornServicesError::validation(WebUiInboundValidationError::new(
        field,
        WebUiInboundValidationCode::InvalidValue,
    ))
}

// `internal_from` logs the backend cause while keeping the facade payload
// sanitized, so operator diagnostics survive without leaking over the wire.
fn operator_config_store_error(error: impl std::fmt::Display) -> RebornServicesError {
    RebornServicesError::internal_from(error)
}

fn operator_config_capability_forbidden() -> RebornServicesError {
    RebornServicesError::from_status(RebornServicesErrorCode::Forbidden, 403, false)
}

fn product_view_forbidden() -> RebornServicesError {
    RebornServicesError::from_status(RebornServicesErrorCode::Forbidden, 403, false)
}

fn product_view_requires_operator_webui_config(view_id: &str) -> bool {
    matches!(
        view_id,
        id if id == ADMIN_CONFIGURATION_VIEW.id
            || id == OPERATOR_LOGS_VIEW.id
            || id == LLM_CONFIG_VIEW.id
            || id == OPERATOR_SETUP_VIEW.id
            || id == OPERATOR_DIAGNOSTICS_VIEW.id
            || id == OPERATOR_STATUS_VIEW.id
    )
}

fn authorize_product_view(
    caller: &WebUiAuthenticatedCaller,
    view_id: &str,
) -> Result<(), RebornServicesError> {
    if product_view_requires_operator_webui_config(view_id) && !caller.operator_webui_config {
        return Err(product_view_forbidden());
    }
    Ok(())
}

fn operator_config_auto_approve_activity_id(
    caller: &WebUiAuthenticatedCaller,
    enabled: bool,
) -> ActivityId {
    let mut seed = Vec::new();
    for segment in [
        "product-surface-operator-config-auto-approve",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        if enabled { "enabled" } else { "disabled" },
    ] {
        seed.extend_from_slice(&(segment.len() as u64).to_be_bytes());
        seed.extend_from_slice(segment.as_bytes());
    }
    ActivityId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_OID, &seed))
}

fn operator_config_mutation_succeeded(resolution: Resolution) -> Result<(), RebornServicesError> {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => Ok(()),
        Resolution::Done(outcome) => match outcome.verdict.error_kind() {
            Some(FailureKind::InvalidInput) => Err(operator_config_invalid_value("value")),
            Some(FailureKind::Authorization | FailureKind::PolicyDenied) => {
                Err(operator_config_capability_forbidden())
            }
            Some(FailureKind::Backend | FailureKind::Transient | FailureKind::Unavailable) => {
                Err(RebornServicesError::service_unavailable(true))
            }
            _ => Err(RebornServicesError::internal_from(
                "operator config capability returned a non-success result",
            )),
        },
        Resolution::Denied(_) => Err(operator_config_capability_forbidden()),
        Resolution::Blocked(_) | Resolution::Suspended(_) => {
            Err(RebornServicesError::service_unavailable(true))
        }
    }
}

async fn auto_approve_config_entry(
    config: &RebornOperatorApprovalConfig,
    scope: &ResourceScope,
) -> Result<RebornOperatorConfigEntry, RebornServicesError> {
    let operator_scope = operator_tool_permission_scope(scope);
    let key = AutoApproveSettingKey::from_resource_scope(&operator_scope);
    let record = config
        .auto_approve
        .get(&key)
        .await
        .map_err(operator_config_store_error)?;
    let enabled = record
        .as_ref()
        .map_or(AUTO_APPROVE_DEFAULT_ENABLED, |record| record.enabled);
    Ok(RebornOperatorConfigEntry {
        key: AUTO_APPROVE_CONFIG_KEY.to_string(),
        value: serde_json::json!(enabled),
        source: if record.is_some() {
            "override".to_string()
        } else {
            "default".to_string()
        },
        redacted: false,
        mutable: true,
    })
}

async fn find_operator_tool(
    config: &RebornOperatorApprovalConfig,
    raw_capability_id: &str,
    caller: &UserId,
) -> Result<RebornOperatorToolInfo, RebornServicesError> {
    // Look up within the CALLER-filtered catalog so a foreign user-private
    // tool reads as an unknown key (same masking as list), never disclosing
    // that it exists or letting a member set a permission on it (#5459 P1).
    config
        .tool_catalog
        .list_operator_tools(caller)
        .await
        .into_iter()
        .find(|tool| tool.capability_id.as_str() == raw_capability_id)
        .ok_or_else(|| operator_config_unknown_key_error("key"))
}

async fn tool_config_entry(
    config: &RebornOperatorApprovalConfig,
    scope: &ResourceScope,
    tool: &RebornOperatorToolInfo,
) -> Result<RebornOperatorConfigEntry, RebornServicesError> {
    let context =
        operator_tool_permission_context(config, scope, std::slice::from_ref(tool)).await?;
    tool_config_entry_with_context(&context, tool).await
}

async fn tool_config_entry_with_context(
    context: &OperatorToolPermissionContext,
    tool: &RebornOperatorToolInfo,
) -> Result<RebornOperatorConfigEntry, RebornServicesError> {
    let (state, source) = effective_tool_permission(context, tool).await?;
    let default_state = default_tool_permission_state(tool.default_permission);
    let locked = tool_permission_locked(tool);
    let value = serde_json::json!({
        "name": tool.capability_id.as_str(),
        "description": tool.description.as_ref(),
        "state": tool_permission_state_wire(state),
        "default_state": tool_permission_state_wire(if locked && hard_floor_tool(tool) {
            ToolPermissionState::AskEachTime
        } else {
            default_state
        }),
        "locked": locked,
        "effective_source": source,
    });
    Ok(RebornOperatorConfigEntry {
        key: format!("{TOOL_CONFIG_PREFIX}{}", tool.capability_id),
        value,
        source: source.to_string(),
        redacted: false,
        mutable: !locked,
    })
}

struct OperatorToolPermissionContext {
    global_auto_approve: bool,
    overrides: HashMap<CapabilityId, ToolPermissionOverride>,
    persistent_active: HashSet<CapabilityId>,
}

async fn operator_tool_permission_context(
    config: &RebornOperatorApprovalConfig,
    scope: &ResourceScope,
    tools: &[RebornOperatorToolInfo],
) -> Result<OperatorToolPermissionContext, RebornServicesError> {
    let operator_scope = operator_tool_permission_scope(scope);
    let global_auto_approve = config
        .auto_approve
        .is_enabled(&operator_scope)
        .await
        .map_err(operator_config_store_error)?;
    let override_records = try_join_all(
        tools
            .iter()
            .filter(|tool| !tool_permission_locked(tool))
            .map(|tool| {
                let key =
                    ToolPermissionOverrideKey::new(&operator_scope, tool.capability_id.clone());
                async move {
                    config
                        .overrides
                        .get(&key)
                        .await
                        .map(|record| (tool.capability_id.clone(), record))
                        .map_err(operator_config_store_error)
                }
            }),
    )
    .await?;
    let overrides = override_records
        .into_iter()
        .filter_map(|(capability_id, record)| record.map(|record| (capability_id, record.state)))
        .collect::<HashMap<_, _>>();
    let persistent_records = try_join_all(
        tools
            .iter()
            .filter(|tool| {
                !tool_permission_locked(tool) && !overrides.contains_key(&tool.capability_id)
            })
            .map(|tool| {
                let operator_scope = operator_scope.clone();
                async move {
                    persistent_user_policy_active(config, &operator_scope, tool)
                        .await
                        .map(|active| (tool.capability_id.clone(), active))
                }
            }),
    )
    .await?;
    let persistent_active = persistent_records
        .into_iter()
        .filter_map(|(capability_id, active)| active.then_some(capability_id))
        .collect();
    Ok(OperatorToolPermissionContext {
        global_auto_approve,
        overrides,
        persistent_active,
    })
}

async fn effective_tool_permission(
    context: &OperatorToolPermissionContext,
    tool: &RebornOperatorToolInfo,
) -> Result<(ToolPermissionState, &'static str), RebornServicesError> {
    if tool.default_permission == PermissionMode::Deny {
        return Ok((ToolPermissionState::Disabled, "default"));
    }
    if hard_floor_tool(tool) {
        return Ok((ToolPermissionState::AskEachTime, "locked"));
    }

    if let Some(state) = context.overrides.get(&tool.capability_id) {
        return Ok((state.as_state(), "override"));
    }

    if context.persistent_active.contains(&tool.capability_id) {
        return Ok((ToolPermissionState::AlwaysAllow, "override"));
    }

    if permission_mode_allows_persistent_approval(tool.default_permission) {
        if context.global_auto_approve {
            return Ok((ToolPermissionState::AlwaysAllow, "global"));
        }
        return Ok((ToolPermissionState::AskEachTime, "global"));
    }

    Ok((
        default_tool_permission_state(tool.default_permission),
        "default",
    ))
}

async fn persistent_user_policy_active(
    config: &RebornOperatorApprovalConfig,
    operator_scope: &ResourceScope,
    tool: &RebornOperatorToolInfo,
) -> Result<bool, RebornServicesError> {
    let key = persistent_user_policy_key(operator_scope, tool);
    Ok(config
        .persistent_policies
        .lookup(&key)
        .await
        .map_err(operator_config_store_error)?
        .and_then(|policy| policy.active_grant())
        .is_some())
}

fn persistent_user_policy_key(
    scope: &ResourceScope,
    tool: &RebornOperatorToolInfo,
) -> PersistentApprovalPolicyKey {
    let operator_scope = operator_tool_permission_scope(scope);
    PersistentApprovalPolicyKey::new(
        &operator_scope,
        PersistentApprovalAction::Dispatch,
        tool.capability_id.clone(),
        Principal::Extension(tool.provider.clone()),
    )
}

fn operator_tool_permission_scope(scope: &ResourceScope) -> ResourceScope {
    scope.tenant_user_settings_scope()
}

fn tool_permission_locked(tool: &RebornOperatorToolInfo) -> bool {
    tool.default_permission == PermissionMode::Deny || hard_floor_tool(tool)
}

fn hard_floor_tool(tool: &RebornOperatorToolInfo) -> bool {
    tool.effects.iter().any(|effect| {
        matches!(
            effect,
            EffectKind::Financial | EffectKind::ModifyApproval | EffectKind::ModifyBudget
        )
    })
}

fn default_tool_permission_state(permission: PermissionMode) -> ToolPermissionState {
    match permission {
        PermissionMode::Allow | PermissionMode::Ask => ToolPermissionState::AskEachTime,
        PermissionMode::Deny => ToolPermissionState::Disabled,
    }
}

fn tool_permission_state_wire(state: ToolPermissionState) -> &'static str {
    match state {
        ToolPermissionState::AlwaysAllow => "always_allow",
        ToolPermissionState::AskEachTime => "ask_each_time",
        ToolPermissionState::Disabled => "disabled",
    }
}

/// Wire enum for the WebUI settings/tools permission request body.
///
/// Request-side vocabulary on the ProductSurface contract surface: the
/// three resolved [`ToolPermissionState`] values plus `default`, which clears
/// the stored per-capability override. The serialized strings must stay
/// byte-identical to what [`parse_tool_permission_state`] accepts and
/// [`tool_permission_state_wire`] emits — the
/// `settings_tool_permission_state_wire_strings_stay_linked` test pins that
/// link so the request enum cannot drift from the storage vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsToolPermissionState {
    Default,
    AlwaysAllow,
    AskEachTime,
    Disabled,
}

enum ToolPermissionUpdate {
    Default,
    State(ToolPermissionState),
}

fn parse_tool_permission_state(
    value: &serde_json::Value,
) -> Result<ToolPermissionUpdate, RebornServicesError> {
    let raw = value
        .as_str()
        .or_else(|| value.get("state").and_then(serde_json::Value::as_str))
        .ok_or_else(|| operator_config_invalid_value("state"))?;
    match raw {
        "default" => Ok(ToolPermissionUpdate::Default),
        "always_allow" => Ok(ToolPermissionUpdate::State(
            ToolPermissionState::AlwaysAllow,
        )),
        // Backward-compatible read alias from earlier Tools UI payloads. The
        // service always writes the canonical `ask_each_time` wire value.
        "ask_each_time" | "ask" => Ok(ToolPermissionUpdate::State(
            ToolPermissionState::AskEachTime,
        )),
        "disabled" => Ok(ToolPermissionUpdate::State(ToolPermissionState::Disabled)),
        _ => Err(operator_config_invalid_value("state")),
    }
}

async fn apply_tool_permission_state(
    config: &RebornOperatorApprovalConfig,
    scope: &ResourceScope,
    actor: &TurnActor,
    tool: &RebornOperatorToolInfo,
    update: ToolPermissionUpdate,
) -> Result<(), RebornServicesError> {
    match update {
        ToolPermissionUpdate::Default => {
            let operator_scope = operator_tool_permission_scope(scope);
            match config
                .persistent_policies
                .revoke(&persistent_user_policy_key(&operator_scope, tool))
                .await
            {
                Ok(_) | Err(PersistentApprovalPolicyError::UnknownPolicy) => {}
                Err(error) => return Err(operator_config_store_error(error)),
            }
            config
                .overrides
                .clear(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    tool.capability_id.clone(),
                ))
                .await
                .map_err(operator_config_store_error)?;
        }
        ToolPermissionUpdate::State(ToolPermissionState::AlwaysAllow) => {
            let operator_scope = operator_tool_permission_scope(scope);
            config
                .persistent_policies
                .allow(PersistentApprovalPolicyInput {
                    scope: operator_scope.clone(),
                    action: PersistentApprovalAction::Dispatch,
                    capability_id: tool.capability_id.clone(),
                    grantee: Principal::Extension(tool.provider.clone()),
                    approved_by: Principal::User(actor.user_id.clone()),
                    constraints: GrantConstraints {
                        allowed_effects: tool.effects.as_ref().to_vec(),
                        mounts: Default::default(),
                        network: Default::default(),
                        secrets: Vec::new(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                    source_approval_request_id: None,
                })
                .await
                .map_err(operator_config_store_error)?;
            config
                .overrides
                .clear(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    tool.capability_id.clone(),
                ))
                .await
                .map_err(operator_config_store_error)?;
        }
        ToolPermissionUpdate::State(state @ ToolPermissionState::AskEachTime)
        | ToolPermissionUpdate::State(state @ ToolPermissionState::Disabled) => {
            let operator_scope = operator_tool_permission_scope(scope);
            let override_state = match state {
                ToolPermissionState::AskEachTime => ToolPermissionOverride::AskEachTime,
                ToolPermissionState::Disabled => ToolPermissionOverride::Disabled,
                ToolPermissionState::AlwaysAllow => unreachable!(),
            };
            match config
                .persistent_policies
                .revoke(&persistent_user_policy_key(&operator_scope, tool))
                .await
            {
                Ok(_) | Err(PersistentApprovalPolicyError::UnknownPolicy) => {}
                Err(error) => return Err(operator_config_store_error(error)),
            }
            config
                .overrides
                .set(ToolPermissionOverrideInput {
                    scope: operator_scope.clone(),
                    capability_id: tool.capability_id.clone(),
                    state: override_state,
                    updated_by: Principal::User(actor.user_id.clone()),
                })
                .await
                .map_err(operator_config_store_error)?;
        }
    }
    Ok(())
}

const LLM_BASE_URL_MAX_BYTES: usize = 2048;

/// Validate an operator-supplied LLM `base_url` before it is persisted or
/// probed.
///
/// Mirrors the `AllowPrivateNetwork` posture used at the model-discovery egress
/// point (`ironclaw_llm`'s `check_models_url`) and the binary's
/// `validate_operator_base_url`: a self-hosted provider on a loopback or private
/// address (Ollama, vLLM) is the primary local use case and must be allowed.
/// Only the never-legitimate classes — cloud metadata / link-local, multicast,
/// and the unspecified address — are rejected here. DNS-name hosts are resolved,
/// re-validated, and pinned by the egress guard; this syntactic check only
/// screens literal IPs.
fn validate_llm_base_url(base_url: Option<&str>) -> Result<(), RebornServicesError> {
    let Some(raw) = base_url else {
        return Ok(());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > LLM_BASE_URL_MAX_BYTES {
        return Err(operator_setup_validation_error("base_url"));
    }
    let parsed = Url::parse(trimmed).map_err(|error| {
        tracing::debug!(%error, "failed to parse operator setup base URL");
        operator_setup_validation_error("base_url")
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(operator_setup_validation_error("base_url"));
    }
    let Some(host) = parsed.host_str() else {
        return Err(operator_setup_validation_error("base_url"));
    };
    // `localhost` and loopback/private literals are intentionally allowed —
    // pointing the operator's provider at a self-hosted endpoint is the main
    // reason this field exists. Only literal IPs in the always-blocked classes
    // are rejected.
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = normalized_host.parse::<IpAddr>()
        && forbidden_llm_base_url_ip(ip)
    {
        return Err(operator_setup_validation_error("base_url"));
    }
    Ok(())
}

fn forbidden_llm_base_url_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => forbidden_llm_base_url_ipv4(ip),
        IpAddr::V6(ip) => forbidden_llm_base_url_ipv6(ip),
    }
}

/// Always-blocked IPv4 classes: the unspecified address, multicast, and
/// link-local (which includes the cloud-metadata endpoint 169.254.169.254).
/// Loopback and private ranges are allowed so self-hosted providers work.
fn forbidden_llm_base_url_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_unspecified() || ip.is_multicast() || ip.is_link_local()
}

/// Always-blocked IPv6 classes: unspecified, multicast, and link-local.
/// Loopback (`::1`) and unique-local are allowed so self-hosted providers work.
/// Embedded-IPv4 forms (`::ffff:a.b.c.d` and `::a.b.c.d`) are unwrapped so an
/// IPv4-compatible metadata address can't slip through as a "plain" v6 host.
fn forbidden_llm_base_url_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_multicast() || ip.is_unicast_link_local() {
        return true;
    }
    if let Some(v4) = ip.to_ipv4() {
        return forbidden_llm_base_url_ipv4(v4);
    }
    false
}

fn operator_config_surface_not_wired_diagnostic() -> RebornOperatorConfigDiagnostic {
    RebornOperatorConfigDiagnostic {
        key: "*".to_string(),
        severity: RebornOperatorConfigDiagnosticSeverity::Error,
        reason_code: "operator_config_service_not_wired".to_string(),
        message: "Operator config diagnostics are available, but the effective config service is not wired yet.".to_string(),
        owning_area: RebornOperatorArea::Config,
        remediation: "Use bootstrap config, environment variables, or existing CLI setup until the operator config service is enabled.".to_string(),
    }
}

fn operator_config_validation_diagnostics(
    keys: Vec<String>,
) -> Vec<RebornOperatorConfigDiagnostic> {
    let keys = if keys.is_empty() {
        vec!["*".to_string()]
    } else {
        keys
    };

    keys.into_iter()
        .map(operator_config_key_diagnostic)
        .collect()
}

fn operator_config_key_diagnostic(key: String) -> RebornOperatorConfigDiagnostic {
    let normalized = key.to_ascii_lowercase();
    let is_secret = ["api_key", "credential", "password", "secret", "token"]
        .iter()
        .any(|marker| normalized.contains(marker));

    let (reason_code, message, remediation) = if key == "*" {
        (
            "operator_config_service_not_wired",
            "Operator config validation is available, but the effective config service is not wired yet.",
            "Use bootstrap config, environment variables, or existing CLI setup until the operator config service is enabled.",
        )
    } else if is_secret {
        (
            "operator_config_secret_not_wired",
            "Secret-backed operator config is not writable through the operator API yet.",
            "Store secrets through the configured secret provider or bootstrap environment until the operator secrets flow is enabled.",
        )
    } else if normalized.starts_with("deprecated.") || normalized.starts_with("legacy.") {
        (
            "operator_config_deprecated",
            "This operator config key is deprecated and is not applied by the Reborn runtime.",
            "Move the setting to the current config key before relying on operator-managed startup.",
        )
    } else if normalized.starts_with("bootstrap.") {
        (
            "operator_config_immutable",
            "Bootstrap config is immutable from the browser operator API.",
            "Change this setting in bootstrap config and restart the host process.",
        )
    } else if matches!(
        normalized.as_str(),
        "provider.default" | "model.default" | "profile.default"
    ) {
        (
            "operator_config_not_wired",
            "This parsed operator config key is not wired into runtime behavior yet.",
            "Keep using the existing setup path for this setting until effective config persistence is enabled.",
        )
    } else {
        (
            "operator_config_unknown_key",
            "This operator config key is not recognized by the current Reborn runtime.",
            "Remove the key or rename it to a documented operator config key.",
        )
    };

    RebornOperatorConfigDiagnostic {
        key,
        severity: RebornOperatorConfigDiagnosticSeverity::Error,
        reason_code: reason_code.to_string(),
        message: message.to_string(),
        owning_area: RebornOperatorArea::Config,
        remediation: remediation.to_string(),
    }
}

fn operator_doctor_status_diagnostic(
    check: &RebornOperatorStatusCheck,
) -> Option<RebornOperatorConfigDiagnostic> {
    if check.status == RebornOperatorStatusState::Ready {
        return None;
    }

    let severity = match check.severity {
        RebornOperatorStatusSeverity::Info => RebornOperatorConfigDiagnosticSeverity::Info,
        RebornOperatorStatusSeverity::Warning => RebornOperatorConfigDiagnosticSeverity::Warning,
        RebornOperatorStatusSeverity::Critical => RebornOperatorConfigDiagnosticSeverity::Error,
    };
    let state = match check.status {
        RebornOperatorStatusState::Ready => "ready",
        RebornOperatorStatusState::Degraded => "degraded",
        RebornOperatorStatusState::Blocked => "blocked",
        RebornOperatorStatusState::Unsupported => "unsupported",
        RebornOperatorStatusState::NotConfigured => "not_configured",
    };
    let reason_code = operator_doctor_status_reason_code(&check.id, state);
    let remediation = check
        .remediation
        .as_deref()
        .unwrap_or("inspect the corresponding operator status check");
    Some(RebornOperatorConfigDiagnostic {
        key: operator_doctor_status_text(&check.id),
        severity,
        reason_code,
        message: operator_doctor_status_text(&check.summary),
        owning_area: RebornOperatorArea::Status,
        remediation: operator_doctor_status_text(remediation),
    })
}

fn operator_doctor_status_response(
    mut status: RebornOperatorStatusResponse,
) -> RebornOperatorStatusResponse {
    status.checks = status
        .checks
        .into_iter()
        .map(operator_doctor_status_check)
        .collect();
    status
}

fn operator_doctor_status_check(mut check: RebornOperatorStatusCheck) -> RebornOperatorStatusCheck {
    check.id = operator_doctor_status_text(&check.id);
    check.summary = operator_doctor_status_text(&check.summary);
    check.remediation = check
        .remediation
        .as_deref()
        .map(operator_doctor_status_text);
    check
}

fn operator_doctor_status_reason_code(check_id: &str, state: &str) -> String {
    if is_operator_doctor_reason_code_component(check_id)
        && !operator_doctor_status_text_needs_redaction(check_id)
    {
        format!("operator_doctor_{check_id}_{state}")
    } else {
        format!("operator_doctor_status_{state}")
    }
}

fn is_operator_doctor_reason_code_component(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && value.len() <= 64
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn operator_doctor_status_text(value: &str) -> String {
    if operator_doctor_status_text_needs_redaction(value) {
        "[redacted operator status detail]".to_string()
    } else {
        value.to_string()
    }
}

fn operator_doctor_status_text_needs_redaction(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("sk-")
        || lower.contains("/home/")
        || lower.contains("/workspace/")
        || lower.contains("\\users\\")
        || lower.contains("/users/")
        || lower.contains(".ssh")
        || lower.contains(".env")
        || lower.contains("api_key")
        || lower.contains("password")
        || lower.contains("credential")
}

fn operator_doctor_setup_unavailable_diagnostic(
    reason_code: &str,
    message: &str,
) -> RebornOperatorConfigDiagnostic {
    operator_setup_diagnostic(
        "setup",
        RebornOperatorConfigDiagnosticSeverity::Error,
        reason_code,
        message,
        "Complete provider/model setup through the operator setup API or bootstrap configuration.",
    )
}

fn operator_doctor_status_unavailable_diagnostic() -> RebornOperatorConfigDiagnostic {
    RebornOperatorConfigDiagnostic {
        key: "status".to_string(),
        severity: RebornOperatorConfigDiagnosticSeverity::Error,
        reason_code: "operator_doctor_status_unavailable".to_string(),
        message: "Operator status checks are unavailable.".to_string(),
        owning_area: RebornOperatorArea::Status,
        remediation: "wire the operator status service before relying on doctor diagnostics"
            .to_string(),
    }
}

fn operator_diagnostics_surface_status(
    diagnostics: &[RebornOperatorConfigDiagnostic],
) -> RebornOperatorSurfaceStatus {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == RebornOperatorConfigDiagnosticSeverity::Error)
    {
        RebornOperatorSurfaceStatus::Unavailable
    } else {
        RebornOperatorSurfaceStatus::Available
    }
}

#[async_trait]
pub trait ProductSurface: Send + Sync {
    /// Invoke one descriptor-declared product capability through the generic
    /// mutation conduit targeted by architecture simplification §5.2.
    ///
    /// `caller` is trusted ingress input. `capability` and `input` are
    /// designators only; a wired implementation must resolve and authorize
    /// them rather than treating either as authority. `activity_id` is the
    /// stable idempotency identity for this mutation and must be preserved
    /// across retries.
    /// The unwired default fails closed without performing any side effect.
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: ProductCapabilityInput,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        let _ = (caller, capability, input, activity_id);
        Err(RebornServicesError::service_unavailable(false))
    }

    /// Query one descriptor-declared, read-only product view. This is the
    /// generic read conduit; new views register an id rather than growing this
    /// facade with per-feature methods.
    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornViewQuery,
    ) -> Result<RebornViewPage, RebornServicesError> {
        let _ = (caller, query);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        let _ = (caller, request);
        Err(RebornServicesError::service_unavailable(false))
    }

    fn supports_stream_events_subscription(&self) -> bool {
        false
    }

    async fn subscribe_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsSubscription, RebornServicesError> {
        Err(RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ReplayUnavailable,
            503,
            true,
        ))
    }

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        let _ = (caller, request);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn execute_command(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: ProductOperationRequest,
    ) -> Result<ProductOperationResponse, RebornServicesError> {
        let _ = (caller, request);
        Err(RebornServicesError::service_unavailable(false))
    }
}

impl<Params, Output> ProductOperation<Params, Output>
where
    Params: Serialize,
    Output: DeserializeOwned,
{
    pub async fn execute_on<S>(
        &self,
        surface: &S,
        caller: WebUiAuthenticatedCaller,
        input: Params,
    ) -> Result<Output, RebornServicesError>
    where
        S: ProductSurface + ?Sized,
    {
        surface
            .execute_command(caller, self.request(input)?)
            .await?
            .into_json()
    }
}

impl<Params> ProductOperation<Params, ProjectFsFile>
where
    Params: Serialize,
{
    pub async fn execute_file_on<S>(
        &self,
        surface: &S,
        caller: WebUiAuthenticatedCaller,
        input: Params,
    ) -> Result<ProjectFsFile, RebornServicesError>
    where
        S: ProductSurface + ?Sized,
    {
        surface
            .execute_command(caller, self.request(input)?)
            .await?
            .into_project_file()
    }
}

impl<Params> ProductOperation<Params, RebornAttachmentBytes>
where
    Params: Serialize,
{
    pub async fn execute_attachment_on<S>(
        &self,
        surface: &S,
        caller: WebUiAuthenticatedCaller,
        input: Params,
    ) -> Result<RebornAttachmentBytes, RebornServicesError>
    where
        S: ProductSurface + ?Sized,
    {
        surface
            .execute_command(caller, self.request(input)?)
            .await?
            .into_attachment()
    }
}

/// Input carried by the generic ProductSurface operation conduit.
///
/// Most capabilities are ordinary JSON payloads. Secret-bearing ProductSurface
/// requests use typed variants so they do not cross the WebUI/product boundary
/// as raw, debug-printable JSON values.
pub enum ProductCapabilityInput {
    Json(serde_json::Value),
    LlmProviderUpsert(UpsertLlmProviderRequest),
}

impl ProductCapabilityInput {
    pub fn json(input: serde_json::Value) -> Self {
        Self::Json(input)
    }

    pub fn llm_provider_upsert(request: UpsertLlmProviderRequest) -> Self {
        Self::LlmProviderUpsert(request)
    }

    pub fn into_json(self) -> Result<serde_json::Value, RebornServicesError> {
        match self {
            Self::Json(input) => Ok(input),
            Self::LlmProviderUpsert(_) => Err(RebornServicesError::internal_from(
                "secret-bearing product capability input cannot be delegated as JSON",
            )),
        }
    }
}

/// ProductSurface operation descriptor.
///
/// Capability declarations stay as one stable id plus origin/policy metadata
/// elsewhere. Product-workflow-owned ids are backed by the
/// [`product_operations`] registry; runtime-backed ids delegate to the wired
/// first-party capability invoker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductCapabilityDescriptor {
    pub id: &'static str,
}

impl ProductCapabilityDescriptor {
    pub const fn product_operation(id: &'static str) -> Self {
        Self { id }
    }

    pub const fn api_only(id: &'static str) -> Self {
        Self::product_operation(id)
    }

    pub fn capability_id(&self) -> Result<CapabilityId, RebornServicesError> {
        CapabilityId::new(self.id).map_err(RebornServicesError::internal_from)
    }

    pub async fn invoke_on<T, S>(
        &self,
        surface: &S,
        caller: WebUiAuthenticatedCaller,
        input: T,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError>
    where
        T: Serialize,
        S: ProductSurface + ?Sized,
    {
        let input = serde_json::to_value(input).map_err(RebornServicesError::internal_from)?;
        surface
            .invoke(
                caller,
                self.capability_id()?,
                ProductCapabilityInput::json(input),
                activity_id,
            )
            .await
    }
}

/// Lands inbound attachment bytes into durable, agent-accessible storage and
/// returns the transcript references to persist on the user message.
///
/// Injected by host composition, which owns the project-scoped filesystem
/// authority. `message_id` is a stable per-message id (the idempotency key)
/// used only to disambiguate the storage path; the implementation writes
/// through the same `MountView` the agent's file tools resolve through, so
/// landed bytes are readable by `file_read`/`list_dir` in later turns.
#[async_trait]
pub trait InboundAttachmentLander: Send + Sync {
    async fn land(
        &self,
        thread_scope: &ThreadScope,
        message_id: &str,
        attachments: Vec<InboundAttachment>,
    ) -> Result<Vec<AttachmentRef>, RebornServicesError>;
}

/// Reads a landed attachment's bytes back for the WebUI bytes endpoint. The
/// read counterpart of [`InboundAttachmentLander`]: host composition implements
/// it over the same project-scoped workspace filesystem the lander wrote
/// through, so `storage_key` is re-scoped through that mount authority and never
/// treated as a host path.
#[async_trait]
pub trait InboundAttachmentReader: Send + Sync {
    async fn read(
        &self,
        thread_scope: &ThreadScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, RebornServicesError>;
}

/// Product-side command membrane for the generic [`ProductSurface::invoke`]
/// conduit.
///
/// The concrete execution adapter lives in composition: this crate owns the
/// product contract and remains independent of runtime implementation crates.
/// The facade is generic over this boundary so the production capability hot
/// path does not add another `Arc<dyn ...>` seam solely for test substitution.
#[async_trait]
pub trait ProductCapabilityInvoker: Send + Sync {
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError>;
}

/// Fail-closed default for compositions that have not attached the product
/// capability membrane.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableProductCapabilityInvoker;

#[async_trait]
impl ProductCapabilityInvoker for UnavailableProductCapabilityInvoker {
    async fn invoke(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _capability: CapabilityId,
        _input: serde_json::Value,
        _activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        Err(RebornServicesError::service_unavailable(false))
    }
}

/// Default facade implementation composed at the WebUI boundary.
#[derive(Clone)]
pub struct RebornServices<
    I = UnavailableProductCapabilityInvoker,
    V = UnavailableRebornViewProvider,
> {
    product_capability_invoker: I,
    view_provider: V,
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    inbound_attachments: Option<Arc<dyn InboundAttachmentLander>>,
    project_filesystem: Option<Arc<dyn ProjectFilesystemReader>>,
    filesystem_browser: Option<Arc<dyn FilesystemBrowseReader>>,
    project_service: Option<Arc<dyn ProjectService>>,
    inbound_attachment_reader: Option<Arc<dyn InboundAttachmentReader>>,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    lifecycle_facade: Arc<dyn LifecycleProductFacade>,
    automation_facade: Arc<dyn AutomationProductFacade>,
    skills_facade: Arc<dyn SkillsProductFacade>,
    channel_connection_facade: Arc<dyn ChannelConnectionFacade>,
    channel_config_facade: Option<Arc<dyn ChannelConfigFacade>>,
    outbound_preferences_facade: Arc<dyn OutboundPreferencesProductFacade>,
    operator_status: Arc<dyn OperatorStatusService>,
    operator_logs: Arc<dyn OperatorLogsService>,
    operator_service_lifecycle: Arc<dyn OperatorServiceLifecycleService>,
    approval_interactions: Arc<dyn ApprovalInteractionService>,
    auth_interactions: Arc<dyn AuthInteractionService>,
    admin_users: Arc<dyn AdminUserService>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    skill_activation_recorder: Option<Arc<SkillActivationRecorder>>,
    skill_activation_clearer: Option<Arc<SkillActivationClearer>>,
    llm_config: Option<Arc<dyn LlmConfigService>>,
    // arch-exempt: optional_arc, genuinely optional — the active-model reader is wired only when the runtime has an LLM reload handle; runtimes built without one, and tests, run without it (mirrors the sibling optional llm_config field), plan #5985
    active_model_reader: Option<Arc<dyn ActiveModelReader>>,
    operator_approval_config: Option<RebornOperatorApprovalConfig>,
    thread_operation_locks: Arc<ThreadOperationLocks>,
}

impl RebornServices<UnavailableProductCapabilityInvoker, UnavailableRebornViewProvider> {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self::new_with_product_ports(
            thread_service,
            turn_coordinator,
            UnavailableProductCapabilityInvoker,
            UnavailableRebornViewProvider,
        )
    }
}

impl<I> RebornServices<I, UnavailableRebornViewProvider>
where
    I: ProductCapabilityInvoker + Clone + 'static,
{
    pub fn new_with_product_capability_invoker(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        product_capability_invoker: I,
    ) -> Self {
        Self::new_with_product_ports(
            thread_service,
            turn_coordinator,
            product_capability_invoker,
            UnavailableRebornViewProvider,
        )
    }
}

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub fn new_with_product_ports(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        product_capability_invoker: I,
        view_provider: V,
    ) -> Self {
        Self {
            product_capability_invoker,
            view_provider,
            thread_service,
            turn_coordinator,
            inbound_attachments: None,
            project_filesystem: None,
            filesystem_browser: None,
            project_service: None,
            inbound_attachment_reader: None,
            event_stream: None,
            lifecycle_facade: Arc::new(UnsupportedLifecycleProductFacade::new_static(
                "reborn_lifecycle_facade_unwired",
            )),
            automation_facade: Arc::new(UnsupportedAutomationProductFacade::new_static()),
            skills_facade: Arc::new(UnsupportedSkillsProductFacade::new_static()),
            channel_connection_facade: Arc::new(StaticChannelConnectionFacade),
            channel_config_facade: None,
            outbound_preferences_facade: Arc::new(
                UnsupportedOutboundPreferencesProductFacade::new_static(),
            ),
            operator_status: Arc::new(UnsupportedOperatorStatusService),
            operator_logs: Arc::new(UnsupportedOperatorLogsService),
            operator_service_lifecycle: Arc::new(UnsupportedOperatorServiceLifecycleService),
            approval_interactions: Arc::new(RejectingApprovalInteractionService),
            auth_interactions: Arc::new(RejectingAuthInteractionService),
            admin_users: Arc::new(RejectingAdminUserService),
            extension_credentials: None,
            skill_activation_recorder: None,
            skill_activation_clearer: None,
            llm_config: None,
            active_model_reader: None,
            operator_approval_config: None,
            thread_operation_locks: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub fn with_event_stream(mut self, event_stream: Arc<dyn ProjectionStream>) -> Self {
        self.event_stream = Some(event_stream);
        self
    }

    /// Wire the port that lands inbound attachment bytes into project storage.
    /// Without it, a send-message carrying attachments is rejected rather than
    /// silently dropping the files.
    pub fn with_inbound_attachments(
        mut self,
        inbound_attachments: Arc<dyn InboundAttachmentLander>,
    ) -> Self {
        self.inbound_attachments = Some(inbound_attachments);
        self
    }

    /// Wire the read-only project-filesystem port backing directory listing and
    /// file download. Without it, the `list_project_dir` / `stat_project_path` /
    /// `read_project_file` methods report the service unavailable.
    pub fn with_project_filesystem_reader(
        mut self,
        project_filesystem: Arc<dyn ProjectFilesystemReader>,
    ) -> Self {
        self.project_filesystem = Some(project_filesystem);
        self
    }

    /// Wire the read-only multi-mount browse port backing the standalone
    /// filesystem viewer (memory / workspace files / skills). Without it,
    /// `list_fs_mounts` reports no mounts and the `browse_fs_dir` /
    /// `stat_fs_path` / `read_fs_file` methods report the service unavailable.
    pub fn with_filesystem_browser(
        mut self,
        filesystem_browser: Arc<dyn FilesystemBrowseReader>,
    ) -> Self {
        self.filesystem_browser = Some(filesystem_browser);
        self
    }

    /// Wire the project management + membership (ACL) port. Without it, the
    /// `list_projects` / `create_project` / … methods report the service
    /// unavailable.
    pub fn with_project_service(mut self, project_service: Arc<dyn ProjectService>) -> Self {
        self.project_service = Some(project_service);
        self
    }

    /// Wire the port that reads landed attachment bytes back for the WebUI bytes
    /// endpoint. Without it, `read_attachment` reports the bytes unavailable
    /// (the timeline still renders the attachment card from its ref).
    pub fn with_inbound_attachment_reader(
        mut self,
        reader: Arc<dyn InboundAttachmentReader>,
    ) -> Self {
        self.inbound_attachment_reader = Some(reader);
        self
    }

    pub fn with_llm_config_service(mut self, llm_config: Arc<dyn LlmConfigService>) -> Self {
        self.llm_config = Some(llm_config);
        self
    }

    /// Wire the read-only port exposing the runtime's live active/default model
    /// id. Without it, `get_run_state` cannot price a default-model run (one
    /// submitted without an explicit `model`, so it carries no
    /// `resolved_model_route`): such a run reports token `usage` but no `cost`.
    pub fn with_active_model_reader(
        mut self,
        active_model_reader: Arc<dyn ActiveModelReader>,
    ) -> Self {
        self.active_model_reader = Some(active_model_reader);
        self
    }

    pub fn with_operator_approval_config(
        mut self,
        overrides: Arc<dyn ToolPermissionOverrideStore>,
        auto_approve: Arc<dyn AutoApproveSettingStore>,
        persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
        tool_catalog: Arc<dyn RebornOperatorToolCatalog>,
    ) -> Self {
        self.operator_approval_config = Some(RebornOperatorApprovalConfig {
            overrides,
            auto_approve,
            persistent_policies,
            tool_catalog,
        });
        self
    }

    pub fn with_lifecycle_product_facade(
        mut self,
        lifecycle_facade: Arc<dyn LifecycleProductFacade>,
    ) -> Self {
        self.lifecycle_facade = lifecycle_facade;
        self
    }

    pub fn with_automation_product_facade(
        mut self,
        automation_facade: Arc<dyn AutomationProductFacade>,
    ) -> Self {
        self.automation_facade = automation_facade;
        self
    }

    pub fn with_skills_product_facade(
        mut self,
        skills_facade: Arc<dyn SkillsProductFacade>,
    ) -> Self {
        self.skills_facade = skills_facade;
        self
    }

    pub fn with_channel_connection_facade(
        mut self,
        channel_connection_facade: Arc<dyn ChannelConnectionFacade>,
    ) -> Self {
        self.channel_connection_facade = channel_connection_facade;
        self
    }

    pub fn with_outbound_preferences_facade(
        mut self,
        outbound_preferences_facade: Arc<dyn OutboundPreferencesProductFacade>,
    ) -> Self {
        self.outbound_preferences_facade = outbound_preferences_facade;
        self
    }

    async fn invoke_json_capability<T>(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: ProductCapabilityDescriptor,
        input: T,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError>
    where
        T: Serialize,
    {
        let input = serde_json::to_value(input).map_err(RebornServicesError::internal_from)?;
        self.product_capability_invoker
            .invoke(caller, capability.capability_id()?, input, activity_id)
            .await
    }

    fn api_capability_success(
        &self,
        activity_id: ActivityId,
        summary: &'static str,
    ) -> Result<Resolution, RebornServicesError> {
        Ok(Resolution::Done(Outcome {
            refs: OutcomeRefs {
                result: ResultRef::from_uuid(activity_id.as_uuid()),
                byte_len: 0,
                preview: None,
                preview_meta: ResultPreviewMeta::default(),
                origin: None,
                output_digest: None,
            },
            verdict: ToolVerdict::Success,
            summary: SafeSummary::new(summary).map_err(RebornServicesError::internal_from)?,
            progress: ResultProgress::MadeProgress,
            terminate_hint: TerminateHint::Continue,
        }))
    }

    async fn invoke_operator_setup_run(
        &self,
        caller: WebUiAuthenticatedCaller,
        input: serde_json::Value,
    ) -> Result<(), RebornServicesError> {
        let request: RebornOperatorSetupRequest =
            serde_json::from_value(input).map_err(|error| {
                tracing::debug!(?error, "failed to decode operator setup input");
                operator_setup_validation_error("input")
            })?;
        self.apply_operator_setup_request(caller.clone(), request)
            .await?;
        self.build_operator_setup_view(caller).await?;
        Ok(())
    }

    async fn apply_operator_setup_request(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorSetupRequest,
    ) -> Result<(), RebornServicesError> {
        if self.llm_config.is_none() {
            return Err(llm_config::llm_config_unavailable());
        }

        if request.model.is_some() && request.provider_id.is_none() {
            return Err(operator_setup_validation_error("model"));
        }
        if request.provider_id.is_none()
            && (request.adapter.is_some()
                || request.base_url.is_some()
                || request.api_key.is_some())
        {
            return Err(operator_setup_validation_error("provider_id"));
        }
        if request.base_url.is_some() && request.adapter.is_none() {
            return Err(operator_setup_validation_error("base_url"));
        }
        if request.api_key.is_some() && request.adapter.is_none() {
            return Err(operator_setup_validation_error("api_key"));
        }
        validate_llm_base_url(request.base_url.as_deref())?;
        let profile_id = validate_operator_setup_profile_id(request.profile_id.as_deref())?;
        let webui_access_token_updated =
            validate_operator_setup_webui_access_token(request.webui_access_token.as_ref())?;
        reject_unwired_operator_setup_host_mutation(profile_id, webui_access_token_updated)?;

        match (request.provider_id, request.adapter) {
            (Some(provider_id), Some(adapter)) => {
                let model = request.model;
                self.invoke_llm_provider_upsert(
                    caller.clone(),
                    UpsertLlmProviderRequest {
                        id: provider_id,
                        name: None,
                        adapter,
                        base_url: request.base_url,
                        default_model: model.clone(),
                        api_key: request.api_key,
                        set_active: true,
                        model,
                    },
                )
                .await?;
            }
            (Some(provider_id), None) => {
                self.invoke_llm_active_set(
                    caller,
                    serde_json::json!({
                        "provider_id": provider_id,
                        "model": request.model,
                    }),
                )
                .await?;
            }
            (None, _) => {}
        }

        Ok(())
    }

    async fn build_automations_view(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiListAutomationsRequest,
    ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
        let Some(caller) = product_agent_bound_caller_from_webui(caller) else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        let limit = clamp_automation_list_limit(request.limit);
        let run_limit = clamp_automation_run_limit(request.run_limit);
        let scheduler_enabled = self.automation_facade.scheduler_enabled();
        let automations = self
            .automation_facade
            .list_automations(
                caller,
                AutomationListRequest {
                    limit,
                    run_limit,
                    include_completed: request.include_completed,
                },
            )
            .await?;
        Ok(RebornListAutomationsResponse {
            automations,
            scheduler_enabled,
        })
    }

    async fn build_threads_view(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        // Reuse the same scope-construction shape the other v2 facade
        // methods use: fail-closed when the caller has no agent
        // binding, owner-scope to the caller's user_id so the listing
        // is per-caller.
        let Some(agent_id) = caller.agent_id.clone() else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        let scope = ThreadScope {
            tenant_id: caller.tenant_id.clone(),
            agent_id,
            project_id: caller.project_id.clone(),
            owner_user_id: Some(caller.user_id.clone()),
            mission_id: None,
        };
        self.list_visible_threads_for_scope(scope, request, caller)
            .await
    }

    /// Wire the generic channel-config configure port. Without it, the
    /// setup facade renders no channel-config fields and rejects
    /// channel-config submissions as unavailable.
    pub fn with_channel_config_facade(
        mut self,
        channel_config_facade: Arc<dyn ChannelConfigFacade>,
    ) -> Self {
        self.channel_config_facade = Some(channel_config_facade);
        self
    }

    pub fn with_operator_status_service(
        mut self,
        operator_status: Arc<dyn OperatorStatusService>,
    ) -> Self {
        self.operator_status = operator_status;
        self
    }

    pub fn with_operator_logs_service(
        mut self,
        operator_logs: Arc<dyn OperatorLogsService>,
    ) -> Self {
        self.operator_logs = operator_logs;
        self
    }

    pub fn with_operator_service_lifecycle_service(
        mut self,
        operator_service_lifecycle: Arc<dyn OperatorServiceLifecycleService>,
    ) -> Self {
        self.operator_service_lifecycle = operator_service_lifecycle;
        self
    }

    pub fn with_approval_interactions(
        mut self,
        approval_interactions: Arc<dyn ApprovalInteractionService>,
    ) -> Self {
        self.approval_interactions = approval_interactions;
        self
    }

    pub fn with_auth_interactions(
        mut self,
        auth_interactions: Arc<dyn AuthInteractionService>,
    ) -> Self {
        self.auth_interactions = auth_interactions;
        self
    }

    pub fn with_extension_credentials(
        mut self,
        extension_credentials: Arc<dyn ExtensionCredentialSetupService>,
    ) -> Self {
        self.extension_credentials = Some(extension_credentials);
        self
    }

    /// Wire the admin user-management port (user CRUD + per-user secret
    /// provisioning). Without it, every admin facade method reports the service
    /// unavailable via the fail-closed [`RejectingAdminUserService`] default.
    pub fn with_admin_user_service(mut self, admin_users: Arc<dyn AdminUserService>) -> Self {
        self.admin_users = admin_users;
        self
    }

    pub fn with_skill_activation_recorder<F>(mut self, recorder: F) -> Self
    where
        F: Fn(&TurnScope, &AcceptedMessageRef, &str) -> Result<(), RebornServicesError>
            + Send
            + Sync
            + 'static,
    {
        self.skill_activation_recorder = Some(Arc::new(recorder));
        self
    }

    pub fn with_skill_activation_hooks<R, C>(mut self, recorder: R, clearer: C) -> Self
    where
        R: Fn(&TurnScope, &AcceptedMessageRef, &str) -> Result<(), RebornServicesError>
            + Send
            + Sync
            + 'static,
        C: Fn(&TurnScope, &AcceptedMessageRef) -> Result<(), RebornServicesError>
            + Send
            + Sync
            + 'static,
    {
        self.skill_activation_recorder = Some(Arc::new(recorder));
        self.skill_activation_clearer = Some(Arc::new(clearer));
        self
    }

    fn record_skill_activation_message(
        &self,
        scope: &TurnScope,
        accepted_message_ref: &AcceptedMessageRef,
        content: &str,
    ) -> Result<(), RebornServicesError> {
        if let Some(recorder) = &self.skill_activation_recorder {
            recorder(scope, accepted_message_ref, content)?;
        }
        Ok(())
    }

    fn clear_skill_activation_message(
        &self,
        scope: &TurnScope,
        accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<(), RebornServicesError> {
        if let Some(clearer) = &self.skill_activation_clearer {
            clearer(scope, accepted_message_ref)?;
        }
        Ok(())
    }

    /// Authorize the caller for admin operations. An env-bearer operator is an
    /// implicit owner; otherwise the caller's persisted role must be admin or
    /// owner. The role is read from the directory on EVERY call (never cached),
    /// so a demoted admin loses access immediately — see
    /// `product_workflow/CLAUDE.md` ("No caching. Caching the authz result is
    /// explicitly forbidden").
    async fn authorize_admin(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<(), RebornServicesError> {
        if caller.operator_webui_config {
            return Ok(());
        }
        let record = self
            .admin_users
            .get_user(&caller.tenant_id, &caller.user_id)
            .await
            .map_err(map_admin_user_error)?;
        match record {
            // Admin/owner role AND an active account. A suspended admin keeps
            // the role field but must not act: status gates authorization, so
            // suspending an admin immediately revokes their admin API access
            // (same "read on every call, never cache" contract as role).
            Some(user) if user.role.is_admin() && user.status == AdminUserStatus::Active => Ok(()),
            // "No record", "not admin", and "suspended admin" are all a 403: the
            // caller is authenticated but not authorized. Never leak which.
            _ => Err(RebornServicesError::from_status(
                RebornServicesErrorCode::Forbidden,
                403,
                false,
            )),
        }
    }

    /// Fetch the target user, mapping absence to a sanitized 404.
    async fn require_admin_target(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<AdminUserRecord, RebornServicesError> {
        self.admin_users
            .get_user(tenant, user_id)
            .await
            .map_err(map_admin_user_error)?
            .ok_or_else(|| {
                RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
            })
    }

    /// Reject a mutation that would strand the tenant without an admin.
    /// `target` is the user's CURRENT record; `still_admin_after` is whether the
    /// user remains an active admin once the mutation lands. Re-reads the
    /// active-admin count immediately before the decision as a TOCTOU guard
    /// (mirrors the `blocked_gate_state` re-read pattern).
    async fn ensure_not_last_admin(
        &self,
        tenant: &TenantId,
        target: &AdminUserRecord,
        still_admin_after: bool,
    ) -> Result<(), RebornServicesError> {
        // Only a mutation that drops a currently-active admin below the line can
        // strand the tenant. If the target is not now an active admin, or stays
        // one, there is nothing to protect.
        if still_admin_after || target.status != AdminUserStatus::Active || !target.role.is_admin()
        {
            return Ok(());
        }
        let active_admins = self
            .admin_users
            .count_active_admins(tenant)
            .await
            .map_err(map_admin_user_error)?;
        if active_admins <= 1 {
            return Err(last_admin_error());
        }
        Ok(())
    }
}

/// Map the coarse admin-port error into the sanitized WebUI wire taxonomy.
fn map_admin_user_error(error: AdminUserError) -> RebornServicesError {
    match error {
        AdminUserError::NotFound => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        // Client-supplied value is malformed (e.g. a bad secret handle) — a 400,
        // never a 500: the input is at fault, not the backend.
        AdminUserError::InvalidInput => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        // Transient backend failure — the browser may retry.
        AdminUserError::Unavailable => RebornServicesError::service_unavailable(true),
        AdminUserError::Internal => RebornServicesError::internal(),
    }
}

/// Stable last-admin-protection error: a 409 conflict carrying a `last_admin`
/// marker so the UI can render a specific message and tests can pin it.
fn last_admin_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Conflict,
        kind: RebornServicesErrorKind::Conflict,
        status_code: 409,
        retryable: false,
        field: Some("last_admin".to_string()),
        validation_code: None,
    }
}

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    async fn execute_product_surface_operation(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: ProductOperationRequest,
    ) -> Result<ProductOperationResponse, RebornServicesError> {
        let operation_id = ProductOperationId::parse(request.operation_id.as_str())
            .ok_or_else(|| RebornServicesError::service_unavailable(false))?;
        match operation_id {
            ProductOperationId::CreateThread => ProductOperationResponse::json(
                self.create_thread(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::SubmitTurn => ProductOperationResponse::json(
                self.submit_turn(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::CancelRun => ProductOperationResponse::json(
                self.cancel_run(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::ResolveGate => ProductOperationResponse::json(
                self.resolve_gate(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::RetryRun => ProductOperationResponse::json(
                self.retry_run(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::ProjectCreate => ProductOperationResponse::json(
                self.create_project(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::ProjectFsRead => Ok(ProductOperationResponse::project_file(
                self.read_project_file(caller, product_command_input(request.input)?)
                    .await?,
            )),
            ProductOperationId::FsRead => Ok(ProductOperationResponse::project_file(
                self.read_fs_file(caller, product_command_input(request.input)?)
                    .await?,
            )),
            ProductOperationId::AttachmentRead => Ok(ProductOperationResponse::attachment(
                self.read_attachment(caller, product_command_input(request.input)?)
                    .await?,
            )),
            ProductOperationId::TraceAccountLoginLink => {
                views::parse_empty_view_params(request.input)?;
                ProductOperationResponse::json(self.trace_account_login_link(caller).await?)
            }
            ProductOperationId::TraceHoldAuthorize => {
                let request: RebornTraceHoldAuthorizeProductRequest =
                    product_command_input(request.input)?;
                ProductOperationResponse::json(
                    self.authorize_trace_hold(caller, request.submission_id)
                        .await?,
                )
            }
            ProductOperationId::OperatorConfigSetKey => {
                let request: RebornOperatorConfigSetProductRequest =
                    product_command_input(request.input)?;
                ProductOperationResponse::json(
                    self.set_operator_config_key(
                        caller,
                        request.key,
                        RebornOperatorConfigSetRequest {
                            value: request.value,
                        },
                    )
                    .await?,
                )
            }
            ProductOperationId::OperatorServiceLifecycle => ProductOperationResponse::json(
                self.run_operator_service_lifecycle(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::LlmTestConnection => ProductOperationResponse::json(
                self.test_llm_connection(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::LlmListModels => ProductOperationResponse::json(
                self.list_llm_models(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::LlmNearAiLogin => ProductOperationResponse::json(
                self.start_nearai_login(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::LlmNearAiWalletLogin => ProductOperationResponse::json(
                self.complete_nearai_wallet_login(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::LlmCodexLogin => {
                views::parse_empty_view_params(request.input)?;
                ProductOperationResponse::json(self.start_codex_login(caller).await?)
            }
            ProductOperationId::AdminUserCreate => ProductOperationResponse::json(
                self.create_admin_user(caller, product_command_input(request.input)?)
                    .await?,
            ),
            ProductOperationId::AdminUserDeleteSecret => {
                let request: RebornAdminDeleteSecretProductRequest =
                    product_command_input(request.input)?;
                let handle = product_secret_handle(request.handle)?;
                ProductOperationResponse::json(
                    self.delete_admin_user_secret(caller, request.user_id, handle)
                        .await?,
                )
            }
            ProductOperationId::AutomationPause => {
                let request: RebornAutomationRequest = product_command_input(request.input)?;
                ProductOperationResponse::json(
                    self.pause_automation(caller, request.automation_id).await?,
                )
            }
            ProductOperationId::AutomationResume => {
                let request: RebornAutomationRequest = product_command_input(request.input)?;
                ProductOperationResponse::json(
                    self.resume_automation(caller, request.automation_id)
                        .await?,
                )
            }
            ProductOperationId::AutomationRename => {
                let request: RebornRenameAutomationProductRequest =
                    product_command_input(request.input)?;
                ProductOperationResponse::json(
                    self.rename_automation(
                        caller,
                        request.automation_id,
                        WebUiRenameAutomationRequest { name: request.name },
                    )
                    .await?,
                )
            }
            ProductOperationId::AutomationDelete => {
                let request: RebornAutomationRequest = product_command_input(request.input)?;
                ProductOperationResponse::json(
                    self.delete_automation(caller, request.automation_id)
                        .await?,
                )
            }
        }
    }

    /// Mint a one-time Trace Commons browser login link for the authenticated
    /// caller. The returned URL is a one-time account-access credential and must
    /// never be logged or exposed on a model-visible surface.
    async fn trace_account_login_link(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornAccountLoginLinkResponse, RebornServicesError> {
        let actor = caller.actor();
        trace_credits::account_login_link_for_user(&caller.tenant_id, &actor.user_id)
            .await
            .map_err(RebornServicesError::internal_from)
    }

    /// Authorize the caller's held manual-review trace for submission.
    async fn authorize_trace_hold(
        &self,
        caller: WebUiAuthenticatedCaller,
        submission_id: String,
    ) -> Result<RebornTraceHoldAuthorizeResponse, RebornServicesError> {
        let actor = caller.actor();
        let submission = uuid::Uuid::parse_str(submission_id.trim()).map_err(|_| {
            RebornServicesError::validation(WebUiInboundValidationError::new(
                "submission_id",
                WebUiInboundValidationCode::InvalidId,
            ))
        })?;
        let scope = ironclaw_reborn_traces::contribution::trace_scope_key(
            caller.tenant_id.as_str(),
            actor.user_id.as_str(),
        );
        let authorized =
            trace_credits::authorize_trace_hold_for_user(&scope, submission).map_err(|error| {
                tracing::debug!(%error, "failed to authorize Trace Commons held trace");
                RebornServicesError::internal_invariant()
            })?;
        Ok(RebornTraceHoldAuthorizeResponse { authorized })
    }

    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: ProductCapabilityInput,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        if let Some(operation) = product_operations::ProductOperationHandler::parse(&capability) {
            let summary = operation.success_summary();
            operation.invoke(self, caller, input).await?;
            return self.api_capability_success(activity_id, summary);
        }
        self.product_capability_invoker
            .invoke(caller, capability, input.into_json()?, activity_id)
            .await
    }

    pub async fn list_admin_users(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornAdminUserListQuery,
    ) -> Result<RebornAdminUserListResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        // Bound the page: clamp the caller's `limit` and parse the opaque
        // cursor into a `UserId`. A malformed cursor is caller input at fault
        // (it should only ever be a value we minted), so it is a 400.
        let limit = query
            .limit
            .map(|value| value as usize)
            .unwrap_or(ADMIN_USER_LIST_DEFAULT_LIMIT)
            .clamp(1, ADMIN_USER_LIST_MAX_LIMIT);
        let after = match query.cursor.as_deref() {
            Some(raw) => Some(UserId::new(raw).map_err(|_| {
                RebornServicesError::from_status(
                    RebornServicesErrorCode::InvalidRequest,
                    400,
                    false,
                )
            })?),
            None => None,
        };
        let users = self
            .admin_users
            .list_users(&caller.tenant_id, query.status, after.as_ref(), limit)
            .await
            .map_err(map_admin_user_error)?;
        // A full page means there may be more rows past it; hand back the last
        // id as the next cursor. A short page is the end of the tenant's users.
        let next_cursor = (users.len() == limit)
            .then(|| users.last().map(|user| user.user_id.as_str().to_string()))
            .flatten();
        Ok(RebornAdminUserListResponse { users, next_cursor })
    }

    pub async fn get_admin_user(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
    ) -> Result<RebornAdminUserResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        let user = self
            .require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        Ok(RebornAdminUserResponse { user })
    }

    pub async fn create_admin_user(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornAdminCreateUserRequest,
    ) -> Result<RebornAdminUserCreatedResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        let created = self
            .admin_users
            .create_user(
                &caller.tenant_id,
                &caller.user_id,
                AdminCreateUserFields {
                    email: request.email,
                    display_name: request.display_name,
                    role: request.role,
                },
            )
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminUserCreatedResponse {
            user: created.record,
            // Exposed exactly once, here. The DTO carries it in no other path.
            api_token: created.api_token.expose_secret().to_string(),
        })
    }

    pub async fn update_admin_user(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
        request: RebornAdminUpdateUserRequest,
    ) -> Result<RebornAdminUserResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        // Surface a 404 before attempting the mutation.
        self.require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        let user = self
            .admin_users
            .update_profile(
                &caller.tenant_id,
                &user_id,
                request.display_name,
                request.metadata,
            )
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminUserResponse { user })
    }

    pub async fn set_admin_user_status(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
        request: RebornAdminSetStatusRequest,
    ) -> Result<RebornAdminUserResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        // Serialize with concurrent role/status/delete on this tenant so the
        // last-admin count read below reflects any in-flight demotion.
        let _admin_guard = self.lock_admin_mutation(&caller.tenant_id).await;
        let target = self
            .require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        // Activating keeps/raises an admin; suspending drops one.
        let still_admin_after = matches!(request.status, AdminUserStatus::Active);
        self.ensure_not_last_admin(&caller.tenant_id, &target, still_admin_after)
            .await?;
        let user = self
            .admin_users
            .set_status(&caller.tenant_id, &user_id, request.status)
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminUserResponse { user })
    }

    pub async fn set_admin_user_role(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
        request: RebornAdminSetRoleRequest,
    ) -> Result<RebornAdminUserResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        // Serialize with concurrent role/status/delete on this tenant so the
        // last-admin count read below reflects any in-flight demotion.
        let _admin_guard = self.lock_admin_mutation(&caller.tenant_id).await;
        let target = self
            .require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        let still_admin_after = request.role.is_admin();
        self.ensure_not_last_admin(&caller.tenant_id, &target, still_admin_after)
            .await?;
        let user = self
            .admin_users
            .set_role(&caller.tenant_id, &user_id, request.role)
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminUserResponse { user })
    }

    pub async fn delete_admin_user(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
    ) -> Result<RebornAdminUserDeletedResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        // Serialize with concurrent role/status/delete on this tenant so the
        // last-admin count read below reflects any in-flight demotion.
        let _admin_guard = self.lock_admin_mutation(&caller.tenant_id).await;
        let target = self
            .require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        // Deletion always removes the user, so it can never leave them an admin.
        self.ensure_not_last_admin(&caller.tenant_id, &target, false)
            .await?;
        self.admin_users
            .delete_user(&caller.tenant_id, &user_id)
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminUserDeletedResponse {
            user_id,
            deleted: true,
        })
    }

    pub async fn list_admin_user_secrets(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
    ) -> Result<RebornAdminUserSecretsListResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        self.require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        let secrets = self
            .admin_users
            .list_secrets(&caller.tenant_id, &user_id)
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminUserSecretsListResponse { secrets })
    }

    pub async fn put_admin_user_secret(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
        handle: SecretHandle,
        request: RebornAdminPutSecretRequest,
    ) -> Result<RebornAdminSecretResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        self.require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        let secret = self
            .admin_users
            .put_secret(
                &caller.tenant_id,
                &user_id,
                handle,
                SecretString::from(request.value),
            )
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminSecretResponse { secret })
    }

    pub async fn delete_admin_user_secret(
        &self,
        caller: WebUiAuthenticatedCaller,
        user_id: UserId,
        handle: SecretHandle,
    ) -> Result<RebornAdminSecretDeletedResponse, RebornServicesError> {
        self.authorize_admin(&caller).await?;
        self.require_admin_target(&caller.tenant_id, &user_id)
            .await?;
        // Echo the parsed, canonical handle back on the wire as a plain string.
        let handle_str = handle.as_str().to_string();
        let deleted = self
            .admin_users
            .delete_secret(&caller.tenant_id, &user_id, handle)
            .await
            .map_err(map_admin_user_error)?;
        Ok(RebornAdminSecretDeletedResponse {
            handle: handle_str,
            deleted,
        })
    }

    pub async fn global_auto_approve_enabled(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<bool, RebornServicesError> {
        let Some(config) = &self.operator_approval_config else {
            return Ok(false);
        };
        let scope = caller_resource_scope(&caller);
        let operator_scope = operator_tool_permission_scope(&scope);
        config
            .auto_approve
            .is_enabled(&operator_scope)
            .await
            .map_err(|error| {
                tracing::debug!(
                    tenant_id = %caller.tenant_id,
                    user_id = %caller.user_id,
                    error = %error,
                    "failed to read global auto-approve setting"
                );
                operator_config_store_error(error)
            })
    }

    pub async fn run_operator_setup(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorSetupRequest,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        self.apply_operator_setup_request(caller.clone(), request)
            .await?;
        self.build_operator_setup_view(caller).await
    }

    pub async fn set_operator_config_key(
        &self,
        caller: WebUiAuthenticatedCaller,
        key: String,
        request: RebornOperatorConfigSetRequest,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        let Some(config) = &self.operator_approval_config else {
            let _ = (caller, key, request);
            return Err(RebornServicesError::service_unavailable(false));
        };
        let scope = caller_resource_scope(&caller);
        if key == AUTO_APPROVE_CONFIG_KEY {
            let enabled = request
                .value
                .as_bool()
                .ok_or_else(|| operator_config_invalid_value("value"))?;
            let resolution = self
                .invoke_json_capability(
                    caller.clone(),
                    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY,
                    serde_json::json!({ "enabled": enabled }),
                    operator_config_auto_approve_activity_id(&caller, enabled),
                )
                .await?;
            operator_config_mutation_succeeded(resolution)?;
            return self
                .build_operator_config_key_view(caller, serde_json::json!({ "key": key }))
                .await;
        }

        let actor = caller.actor();
        let entry = if let Some(capability_id) = key.strip_prefix(TOOL_CONFIG_PREFIX) {
            let tool = find_operator_tool(config, capability_id, &scope.user_id).await?;
            if tool_permission_locked(&tool) {
                return Err(operator_config_invalid_value("state"));
            }
            let state = parse_tool_permission_state(&request.value)?;
            apply_tool_permission_state(config, &scope, &actor, &tool, state).await?;
            tool_config_entry(config, &scope, &tool).await?
        } else {
            return Err(operator_config_unknown_key_error("key"));
        };
        Ok(RebornOperatorConfigGetResponse { entry })
    }

    /// `requested_thread_id` makes the caller's choice authoritative.
    /// Without it, `client_action_id` deterministically derives the thread id
    /// so a retry of the same create maps back to the same thread.
    ///
    /// When the caller supplies an explicit `requested_thread_id`, an
    /// `ensure_thread` collision with a thread owned by another user is
    /// remapped to `NotFound` rather than the underlying `409 Conflict`.
    /// Otherwise the 400/409 distinction would be an existence oracle:
    /// callers sharing the same (tenant, agent, project) scope could probe
    /// for thread ids they did not create.
    pub async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        // A browser may propose a project for the new thread; authorize the
        // caller's access to it (never trust the body alone) and adopt it as the
        // thread's scope for this request only. Without a proposed project the
        // caller's default scope is used unchanged.
        let caller = self
            .authorize_create_thread_project(caller, request.project_id.clone())
            .await?;
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::CreateThread {
            caller,
            client_action_id,
            requested_thread_id,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };
        let caller_supplied_id = requested_thread_id.is_some();
        let thread_id =
            requested_thread_id.unwrap_or_else(|| generated_thread_id(&caller, &client_action_id));
        let scope = caller.turn_scope(thread_id.clone());
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(caller.user_id.clone()))?;
        let thread = self
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope,
                thread_id: Some(thread_id),
                created_by_actor_id: caller.user_id.as_str().to_string(),
                title: None,
                metadata_json: Some(create_thread_metadata_json(&client_action_id)?),
            })
            .await
            .map_err(|error| {
                if caller_supplied_id {
                    map_ownership_probe_error(error)
                } else {
                    // Deterministic generated ids derive from caller scope so
                    // a cross-user collision implies a UUIDv5 hash collision,
                    // which is not an oracle the caller can usefully probe.
                    // Preserve the underlying mapping for diagnosability.
                    map_thread_error(error)
                }
            })?;
        Ok(RebornCreateThreadResponse { thread })
    }

    pub async fn submit_turn(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        // Decode + budget inline attachment bytes before the request is
        // consumed into the (bytes-free, serializable) command.
        let attachments = request.decode_attachments()?;
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::SendMessage {
            scope,
            actor,
            client_action_id,
            content,
            requested_model,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };

        let (scope, thread_scope) = self.resolve_webui_thread_metadata(scope, &actor).await?;
        let _thread_operation_guard = self.lock_thread_operation(&scope).await;
        let source_binding_id = webui_source_binding_id(&scope, &actor);
        let external_event_id = client_action_id.as_str().to_string();

        let handoff = if let Some((replay, replay_source_binding_id)) = replay_webui_send_message(
            &*self.thread_service,
            &thread_scope,
            &scope,
            &actor,
            &external_event_id,
        )
        .await?
        {
            if replay.thread_id != scope.thread_id {
                return Err(RebornServicesError::from_status_kind(
                    RebornServicesErrorCode::Conflict,
                    RebornServicesErrorKind::Duplicate,
                    409,
                    false,
                ));
            }
            match replay.status {
                MessageStatus::Submitted => {
                    let run_id = parse_replay_run_id(replay.turn_run_id)?;
                    let state = self
                        .turn_coordinator
                        .get_run_state(GetRunStateRequest {
                            scope: scope.clone(),
                            run_id,
                        })
                        .await
                        .map_err(map_turn_error)?;
                    return Ok(RebornSubmitTurnResponse::AlreadySubmitted {
                        thread_id: replay.thread_id,
                        accepted_message_ref: accepted_message_ref(replay.message_id.to_string())?,
                        run_id,
                        status: state.status,
                        event_cursor: state.event_cursor,
                    });
                }
                MessageStatus::RejectedBusy => {
                    // Idempotent re-rejection: the original busy rejection was
                    // lost before it reached the client.  The blocking run may
                    // already be finished, so we cannot recover its run-id or
                    // cursor.  Return a RejectedBusy with None run metadata so
                    // the client knows to resend rather than treating this as
                    // a new submission.  Fabricating a run-id or status here
                    // would give the client a reference it cannot query.
                    return Ok(RebornSubmitTurnResponse::RejectedBusy {
                        thread_id: replay.thread_id,
                        accepted_message_ref: accepted_message_ref(replay.message_id.to_string())?,
                        active_run_id: None,
                        status: None,
                        event_cursor: None,
                        notice: NOTICE_BUSY_GENERIC.to_string(),
                    });
                }
                MessageStatus::Accepted | MessageStatus::DeferredBusy => AcceptedWebUiMessage {
                    thread_id: replay.thread_id,
                    message_id: replay.message_id,
                    actor_id: actor.user_id.as_str().to_string(),
                    source_binding_id: replay
                        .source_binding_id
                        .unwrap_or_else(|| replay_source_binding_id.clone()),
                    reply_target_binding_id: replay
                        .reply_target_binding_id
                        .unwrap_or(replay_source_binding_id),
                },
                _ => {
                    return Err(RebornServicesError::from_status(
                        RebornServicesErrorCode::Conflict,
                        409,
                        false,
                    ));
                }
            }
        } else {
            // Land attachment bytes (if any) into project storage before the
            // message is accepted, recording each as a transcript reference.
            // The stable per-message external_event_id is the path's message
            // segment, so a same-day retry re-lands at the same path; the lander
            // also partitions by UTC day, so a retry that crosses midnight UTC
            // lands under the new day's directory (the earlier bytes are left
            // addressable but unreferenced). Idempotency is enforced at message
            // acceptance, not by the storage path.
            let message_content = if attachments.is_empty() {
                MessageContent::text(content.clone())
            } else {
                let lander = self
                    .inbound_attachments
                    .as_ref()
                    .ok_or_else(|| RebornServicesError::service_unavailable(false))?;
                let refs = lander
                    .land(&thread_scope, &external_event_id, attachments)
                    .await?;
                MessageContent::with_attachments(content.clone(), refs)
            };
            let accepted = self
                .thread_service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: thread_scope.clone(),
                    thread_id: scope.thread_id.clone(),
                    actor_id: actor.user_id.as_str().to_string(),
                    source_binding_id: Some(source_binding_id.clone()),
                    reply_target_binding_id: Some(source_binding_id.clone()),
                    external_event_id: Some(external_event_id),
                    content: message_content,
                })
                .await
                .map_err(map_thread_error)?;
            AcceptedWebUiMessage {
                thread_id: accepted.thread_id,
                message_id: accepted.message_id,
                actor_id: actor.user_id.as_str().to_string(),
                source_binding_id: source_binding_id.clone(),
                reply_target_binding_id: source_binding_id.clone(),
            }
        };

        let accepted_message_ref = accepted_message_ref(handoff.message_id.to_string())?;
        let source_binding_ref =
            webui_source_binding_ref_from_raw("webui-src", &handoff.source_binding_id)?;
        let reply_target_binding_ref = webui_reply_target_binding_ref_from_raw(
            "webui-reply",
            &handoff.reply_target_binding_id,
        )?;
        let product_context = ironclaw_product_context::resolve_web_ui(scope.product_owner(&actor));
        let submit = SubmitTurnRequest {
            requested_model,
            scope: scope.clone(),
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            idempotency_key: client_action_id.clone(),
            received_at: Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: Some(product_context),
        };

        self.record_skill_activation_message(&scope, &accepted_message_ref, &content)?;
        match self.turn_coordinator.submit_turn(submit).await {
            Ok(SubmitTurnResponse::Accepted {
                turn_id,
                run_id,
                status,
                resolved_run_profile_id,
                resolved_run_profile_version,
                event_cursor,
                ..
            }) => {
                tracing::debug!(
                    thread_id = ?scope.thread_id,
                    run_id = %run_id,
                    "webui submit_turn accepted: turn run enqueued"
                );
                mark_message_submitted_or_replay(
                    &*self.thread_service,
                    &thread_scope,
                    &handoff,
                    &client_action_id,
                    turn_id.to_string(),
                    run_id.to_string(),
                )
                .await?;

                Ok(RebornSubmitTurnResponse::Submitted {
                    thread_id: handoff.thread_id,
                    accepted_message_ref,
                    turn_id: turn_id.to_string(),
                    run_id,
                    status,
                    resolved_run_profile_id: resolved_run_profile_id.as_str().to_string(),
                    resolved_run_profile_version: resolved_run_profile_version.as_u64(),
                    event_cursor,
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                tracing::debug!(
                    thread_id = ?scope.thread_id,
                    active_run_id = ?busy.active_run_id,
                    "webui submit_turn deferred: thread busy with an active run"
                );
                self.clear_skill_activation_message(&scope, &accepted_message_ref)?;
                mark_message_rejected_busy_or_replay(
                    &*self.thread_service,
                    &thread_scope,
                    &handoff,
                    &client_action_id,
                )
                .await?;
                let notice = rejected_busy_notice(busy.status);
                Ok(RebornSubmitTurnResponse::RejectedBusy {
                    thread_id: handoff.thread_id,
                    accepted_message_ref,
                    active_run_id: Some(busy.active_run_id),
                    status: Some(busy.status),
                    event_cursor: Some(busy.event_cursor),
                    notice,
                })
            }
            Err(error) => {
                tracing::debug!(
                    thread_id = ?scope.thread_id,
                    error = ?error,
                    "webui submit_turn rejected by coordinator; no run enqueued"
                );
                self.clear_skill_activation_message(&scope, &accepted_message_ref)?;
                Err(map_turn_error(error))
            }
        }
    }

    pub async fn delete_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let scope = caller.turn_scope(thread_id.clone());
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(caller.user_id.clone()))?;
        let _thread_operation_guard = self.lock_thread_operation(&scope).await;
        self.reject_delete_with_active_run(&scope, &thread_scope, &thread_id)
            .await?;
        self.thread_service
            .delete_thread(&thread_scope, &thread_id)
            .await
            .map_err(map_ownership_probe_error)?;
        Ok(RebornDeleteThreadResponse {
            thread_id,
            deleted: true,
        })
    }

    pub async fn get_timeline(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let limit = clamp_timeline_limit(request.limit);
        let cursor = parse_timeline_cursor(request.cursor.as_deref())?;
        let scope = caller.turn_scope(thread_id);
        let (_thread_scope, history) = self
            .resolve_thread_history_for_caller(caller, &scope)
            .await?;

        let (messages, next_cursor) = paginate_timeline_messages(history.messages, limit, cursor);
        let summary_artifacts = cap_summary_artifacts(history.summary_artifacts);

        Ok(RebornTimelineResponse {
            thread: history.thread,
            messages,
            summary_artifacts,
            next_cursor,
        })
    }

    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornViewQuery,
    ) -> Result<RebornViewPage, RebornServicesError> {
        authorize_product_view(&caller, &query.view_id)?;
        if self.view_provider.descriptor().id == query.view_id {
            return self
                .view_provider
                .query(caller, query.params, query.cursor)
                .await;
        }
        match query.view_id.as_str() {
            id if id == LOGS_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.build_logs_view(caller, request, query.cursor).await?;
                let next_cursor = response.next_cursor.clone();
                views::view_page_with_cursor(response, next_cursor)
            }
            id if id == OPERATOR_LOGS_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self
                    .build_operator_logs_view(caller, request, query.cursor)
                    .await?;
                let next_cursor = response
                    .logs
                    .as_ref()
                    .and_then(|logs| logs.next_cursor.clone());
                views::view_page_with_cursor(response, next_cursor)
            }
            id if id == LLM_CONFIG_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_llm_config_view(caller).await?;
                views::view_page(response)
            }
            id if id == THREADS_VIEW.id => {
                let mut request: WebUiListThreadsRequest = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                request.cursor = query.cursor.or(request.cursor);
                let response = self.build_threads_view(caller, request).await?;
                let next_cursor = response.next_cursor.clone();
                views::view_page_with_cursor(response, next_cursor)
            }
            id if id == AUTOMATIONS_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.build_automations_view(caller, request).await?;
                views::view_page(response)
            }
            id if id == OUTBOUND_PREFERENCES_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_outbound_preferences_view(caller).await?;
                views::view_page(response)
            }
            id if id == OUTBOUND_DELIVERY_TARGETS_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_outbound_delivery_targets_view(caller).await?;
                views::view_page(response)
            }
            id if id == TRACE_CREDITS_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_trace_credits_view(caller).await?;
                views::view_page(response)
            }
            id if id == TRACE_ACCOUNT_TRACES_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_trace_account_traces_view(caller).await?;
                views::view_page(response)
            }
            id if id == RUN_ARTIFACT_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let artifact = self.build_run_artifact(caller, request).await?;
                views::view_page(artifact)
            }
            id if id == GLOBAL_AUTO_APPROVE_VIEW.id => {
                let _: RebornGlobalAutoApproveRequest = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let enabled = self.global_auto_approve_enabled(caller).await?;
                views::view_page(RebornGlobalAutoApproveResponse { enabled })
            }
            id if id == TIMELINE_VIEW.id => {
                let mut request: RebornTimelineRequest = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                request.cursor = query.cursor.or(request.cursor);
                let response = self.get_timeline(caller, request).await?;
                views::view_page(response)
            }
            id if id == PROJECT_FS_LIST_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.list_project_dir(caller, request).await?;
                views::view_page(response)
            }
            id if id == PROJECT_FS_STAT_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.stat_project_path(caller, request).await?;
                views::view_page(response)
            }
            id if id == FS_MOUNTS_VIEW.id => {
                let _: RebornFsMountsRequest = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.list_fs_mounts(caller).await?;
                views::view_page(response)
            }
            id if id == FS_LIST_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.browse_fs_dir(caller, request).await?;
                views::view_page(response)
            }
            id if id == FS_STAT_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.stat_fs_path(caller, request).await?;
                views::view_page(response)
            }
            id if id == PROJECTS_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.list_projects(caller, request).await?;
                views::view_page(response)
            }
            id if id == PROJECT_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.get_project(caller, request).await?;
                views::view_page(response)
            }
            id if id == PROJECT_MEMBERS_VIEW.id => {
                let request = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.list_project_members(caller, request).await?;
                views::view_page(response)
            }
            id if id == ADMIN_USERS_VIEW.id => {
                let mut request: RebornAdminUserListQuery = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                request.cursor = query.cursor.or(request.cursor);
                let response = self.list_admin_users(caller, request).await?;
                views::view_page(response)
            }
            id if id == ADMIN_USER_VIEW.id => {
                let request: RebornAdminUserRequest = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self.get_admin_user(caller, request.user_id).await?;
                views::view_page(response)
            }
            id if id == ADMIN_USER_SECRETS_VIEW.id => {
                let request: RebornAdminUserRequest = serde_json::from_value(query.params)
                    .map_err(RebornServicesError::internal_from)?;
                let response = self
                    .list_admin_user_secrets(caller, request.user_id)
                    .await?;
                views::view_page(response)
            }
            id if id == OPERATOR_CONFIG_LIST_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_operator_config_list_view(caller).await?;
                views::view_page(response)
            }
            id if id == OPERATOR_CONFIG_KEY_VIEW.id => {
                let response = self
                    .build_operator_config_key_view(caller, query.params)
                    .await?;
                views::view_page(response)
            }
            id if id == OPERATOR_CONFIG_VALIDATE_VIEW.id => {
                let response = self.build_operator_config_validate_view(query.params)?;
                views::view_page(response)
            }
            id if id == OPERATOR_SETUP_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_operator_setup_view(caller).await?;
                views::view_page(response)
            }
            id if id == EXTENSIONS_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = extensions::list_extensions(
                    Arc::clone(&self.lifecycle_facade),
                    self.extension_credentials.clone(),
                    Arc::clone(&self.channel_connection_facade),
                    caller,
                )
                .await?;
                views::view_page(response)
            }
            id if id == EXTENSION_REGISTRY_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response =
                    extensions::list_extension_registry(self.lifecycle_facade.as_ref(), caller)
                        .await?;
                views::view_page(response)
            }
            id if id == EXTENSION_SETUP_VIEW.id => {
                let response = lifecycle_setup::setup_extension_view(
                    self.lifecycle_facade.as_ref(),
                    self.extension_credentials.as_deref(),
                    self.channel_config_facade.as_deref(),
                    caller,
                    query.params,
                )
                .await?;
                views::view_page(response)
            }
            id if id == SKILLS_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.skills_facade.list_skills(caller).await?;
                views::view_page(response)
            }
            id if id == SKILL_SEARCH_VIEW.id => {
                let search_query = views::required_string_view_param(query.params, "query")?;
                let response = self
                    .skills_facade
                    .search_skills(caller, search_query)
                    .await?;
                views::view_page(response)
            }
            id if id == SKILL_CONTENT_VIEW.id => {
                let name = views::required_string_view_param(query.params, "name")?;
                let response = self.skills_facade.read_skill_content(caller, name).await?;
                views::view_page(response)
            }
            id if id == OPERATOR_DIAGNOSTICS_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_operator_diagnostics_view(caller).await?;
                views::view_page(response)
            }
            id if id == OPERATOR_STATUS_VIEW.id => {
                views::parse_empty_view_params(query.params)?;
                let response = self.build_operator_status_view(caller).await?;
                views::view_page(response)
            }
            _ => Err(RebornServicesError::not_found()),
        }
    }

    async fn list_project_dir(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornProjectFsListRequest,
    ) -> Result<RebornProjectFsListResponse, RebornServicesError> {
        let reader = self.require_project_filesystem()?;
        let thread_scope = self
            .authorize_project_fs_access(caller, request.thread_id)
            .await?;
        // dispatch-exempt: read-only, already-authorized workspace listing through
        // the facade's own port — not an in-turn mutating tool call, so it does
        // not route through ToolDispatcher.
        let entries = reader
            .list_dir(&thread_scope, &request.path)
            .await
            .map_err(map_project_fs_error)?;
        Ok(RebornProjectFsListResponse { entries })
    }

    async fn stat_project_path(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornProjectFsStatRequest,
    ) -> Result<RebornProjectFsStatResponse, RebornServicesError> {
        let reader = self.require_project_filesystem()?;
        let thread_scope = self
            .authorize_project_fs_access(caller, request.thread_id)
            .await?;
        // dispatch-exempt: read-only, already-authorized workspace stat through
        // the facade's own port — not an in-turn mutating tool call.
        let stat = reader
            .stat(&thread_scope, &request.path)
            .await
            .map_err(map_project_fs_error)?;
        Ok(RebornProjectFsStatResponse { stat })
    }

    pub async fn read_project_file(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornProjectFsReadRequest,
    ) -> Result<ProjectFsFile, RebornServicesError> {
        let reader = self.require_project_filesystem()?;
        let thread_scope = self
            .authorize_project_fs_access(caller, request.thread_id)
            .await?;
        // dispatch-exempt: read-only, already-authorized workspace file download
        // through the facade's own port — not an in-turn mutating tool call.
        reader
            .read_file(&thread_scope, &request.path)
            .await
            .map_err(map_project_fs_error)
    }

    async fn list_fs_mounts(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornFsMountsResponse, RebornServicesError> {
        // No wired browser is not an error: the UI renders an empty viewer.
        let mounts = self
            .filesystem_browser
            .as_ref()
            .map(|browser| {
                browser
                    .available_mounts()
                    .into_iter()
                    .map(|mount| RebornFsMountInfo {
                        mount,
                        label: mount.label().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(RebornFsMountsResponse { mounts })
    }

    pub async fn browse_fs_dir(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornFsListRequest,
    ) -> Result<RebornFsListResponse, RebornServicesError> {
        let browser = self.require_filesystem_browser(request.mount)?;
        let scope = self
            .authorize_browse_scope(caller, request.project_id)
            .await?;
        // dispatch-exempt: read-only, caller-scoped internal-filesystem listing
        // through the facade's own port — not an in-turn mutating tool call.
        let entries = browser
            .list_dir(&scope, request.mount, &request.path)
            .await
            .map_err(map_project_fs_error)?;
        Ok(RebornFsListResponse {
            mount: request.mount,
            path: request.path,
            entries,
        })
    }

    async fn stat_fs_path(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornFsStatRequest,
    ) -> Result<RebornFsStatResponse, RebornServicesError> {
        let browser = self.require_filesystem_browser(request.mount)?;
        let scope = self
            .authorize_browse_scope(caller, request.project_id)
            .await?;
        // dispatch-exempt: read-only, caller-scoped internal-filesystem stat.
        let stat = browser
            .stat(&scope, request.mount, &request.path)
            .await
            .map_err(map_project_fs_error)?;
        Ok(RebornFsStatResponse { stat })
    }

    pub async fn read_fs_file(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornFsReadRequest,
    ) -> Result<ProjectFsFile, RebornServicesError> {
        let browser = self.require_filesystem_browser(request.mount)?;
        let scope = self
            .authorize_browse_scope(caller, request.project_id)
            .await?;
        // dispatch-exempt: read-only, caller-scoped internal-filesystem download.
        browser
            .read_file(&scope, request.mount, &request.path)
            .await
            .map_err(map_project_fs_error)
    }

    async fn list_projects(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .list_projects(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    pub async fn create_project(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .create_project(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn get_project(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .get_project(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn update_project(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .update_project(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn delete_project(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornDeleteProjectRequest,
    ) -> Result<(), RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .delete_project(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn list_project_members(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .list_members(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn add_project_member(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .add_member(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn update_project_member_role(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .update_member_role(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    async fn remove_project_member(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornRemoveMemberRequest,
    ) -> Result<(), RebornServicesError> {
        let service = self.require_project_service()?;
        service
            .remove_member(project_caller(&caller), request)
            .await
            .map_err(map_project_service_error)
    }

    pub async fn read_attachment(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornAttachmentRequest,
    ) -> Result<RebornAttachmentBytes, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let message_id = ThreadMessageId::parse(&request.message_id).map_err(|_| {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        })?;
        let scope = caller.turn_scope(thread_id);

        // Resolve the thread the same way the timeline does (including the
        // automation-trigger fallback) and read the bytes back through the
        // scope the history actually lives under — for a trigger-fired thread
        // that is the creator's scope, not the caller's session scope, so the
        // reader addresses the right project mount.
        //
        // This loads the whole thread history to find one ref, so it is
        // O(messages) per fetch. Acceptable for now: the cost equals the
        // timeline load already incurred when the thread is open, and the
        // browser caches each attachment (private max-age plus the resolved
        // data/blob URL), so it is one fetch per attachment per session. A
        // single-message fast path would need a new scope-validated "load one
        // message *record* by id" service method — `load_context_messages`
        // projects to `ContextMessage`, which carries only image refs (no
        // filename, no non-image kinds), so it can't resolve an arbitrary
        // attachment. Left as a follow-up rather than widening the thread
        // service contract here.
        let (thread_scope, history) = self
            .resolve_thread_history_for_caller(caller, &scope)
            .await?;

        // The (message, attachment-id) pair is required: an attachment id is
        // only unique within its message. Resolve the ref server-side so the
        // browser never supplies the storage path and the Content-Type is
        // authoritative.
        let attachment = history
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .and_then(|message| {
                message
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == request.attachment_id)
            })
            .ok_or_else(|| {
                RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
            })?;

        let storage_key = attachment.storage_key.as_deref().ok_or_else(|| {
            // An attachment that never landed has no bytes to serve.
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        })?;

        // The ref landed (it has a storage_key) but no read port is wired: that
        // is a composition fault, not an absent file. Surface a retryable 503
        // rather than a 404 that would make real bytes look gone. (In the
        // shipped composition the reader and lander are wired together, so this
        // only trips a misconfigured custom host.)
        let Some(reader) = self.inbound_attachment_reader.as_ref() else {
            // Not retryable: a missing port won't appear on a retry, it needs
            // composition wiring.
            return Err(RebornServicesError::service_unavailable(false));
        };

        let bytes = reader.read(&thread_scope, storage_key).await?;
        Ok(RebornAttachmentBytes {
            mime_type: attachment.mime_type.clone(),
            filename: attachment.filename.clone(),
            bytes,
        })
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let actor = caller.actor();
        // Ownership probe: the SSE handler calls stream_events once per poll,
        // so the cheap read_thread probe is used rather than loading the full
        // transcript. Without it a caller sharing (tenant, agent, project)
        // could read another user's projection feed by guessing thread_id.
        // The automation fallback allows the owner of an automation to stream
        // events for a trigger-fired thread (which is stored under the trigger
        // creator). The returned scope may contain an explicit owner for
        // trigger threads.
        //
        // Authorization is revalidated on every poll — no caching — so a
        // caller that loses automation visibility between polls cannot keep
        // draining the trigger-owned stream.
        let access = self
            .resolve_thread_access_for_caller(caller.clone(), caller.turn_scope(thread_id), &actor)
            .await?;
        let Some(event_stream) = &self.event_stream else {
            return Err(RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ReplayUnavailable,
                503,
                false,
            ));
        };
        // Projection identity must be the thread owner, not necessarily the
        // caller. Turn events and the runtime event stream are keyed under the
        // identity of the actor that submitted the run (the trigger creator for
        // trigger threads; the session user for normal threads). The caller
        // already proved visibility via automation ownership above; using the
        // caller's id here would filter to the wrong stream/events.
        //
        // For normal session threads `explicit_owner_user_id()` is `None` and
        // we fall back to the caller's id — behaviour is unchanged.
        let events = event_stream
            .drain(ProjectionSubscriptionRequest {
                actor: access.run_actor,
                scope: access.scope,
                after_cursor: request.after_cursor,
            })
            .await
            .map_err(map_projection_error)?;
        Ok(RebornStreamEventsResponse { events })
    }

    fn supports_stream_events_subscription(&self) -> bool {
        self.event_stream
            .as_ref()
            .is_some_and(|stream| stream.supports_subscription())
    }

    async fn subscribe_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsSubscription, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let actor = caller.actor();
        let access = self
            .resolve_thread_access_for_caller(
                caller.clone(),
                caller.turn_scope(thread_id.clone()),
                &actor,
            )
            .await?;
        let Some(event_stream) = &self.event_stream else {
            return Err(RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ReplayUnavailable,
                503,
                false,
            ));
        };
        let mut subscription = event_stream
            .subscribe(ProjectionSubscriptionRequest {
                actor: access.run_actor,
                scope: access.scope,
                after_cursor: request.after_cursor,
            })
            .await
            .map_err(map_projection_error)?;

        let service = self.clone();
        let caller_for_revalidation = caller.clone();
        let actor_for_revalidation = actor;
        let (sender, receiver) = mpsc::channel(128);
        tokio::spawn(async move {
            loop {
                let next = tokio::select! {
                    _ = sender.closed() => return,
                    next = subscription.next() => next,
                };
                let Some(next) = next else {
                    return;
                };
                let envelope = match next {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        let _ = sender.send(Err(map_projection_error(error))).await;
                        return;
                    }
                };
                if sender.is_closed() {
                    return;
                }
                let revalidate = service
                    .resolve_thread_access_for_caller(
                        caller_for_revalidation.clone(),
                        caller_for_revalidation.turn_scope(thread_id.clone()),
                        &actor_for_revalidation,
                    )
                    .await;
                if let Err(error) = revalidate {
                    let _ = sender.send(Err(error)).await;
                    return;
                }
                if sender.is_closed() {
                    return;
                }
                if sender.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }
        });

        Ok(RebornStreamEventsSubscription::new(receiver))
    }

    pub async fn cancel_run(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        let caller_for_fallback = caller.clone();
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::CancelRun { mut request } = command else {
            return Err(RebornServicesError::internal_invariant());
        };
        // Ownership probe with automation-trigger fallback. If the thread is a
        // trigger-fired thread belonging to the caller's automation, the probe
        // succeeds and returns the trigger-owned scope/actor so the cancel
        // arrives at the actual run, not the browser caller's session scope.
        let access = self
            .resolve_thread_access_for_caller(
                caller_for_fallback,
                request.scope.clone(),
                &request.actor,
            )
            .await?;
        request.scope = access.scope;
        request.actor = access.run_actor;
        let response = self
            .turn_coordinator
            .cancel_run(request)
            .await
            .map_err(map_turn_error)?;
        Ok(response.into())
    }

    pub async fn resolve_gate(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        let caller_for_fallback = caller.clone();
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::ResolveGate {
            scope,
            actor,
            run_id,
            gate_ref,
            client_action_id,
            resolution,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };

        // Ownership probe with automation-trigger fallback. Trigger threads
        // return the trigger-owned scope and run actor; gate routing and resume
        // paths must use that run actor while authorization remains tied to the
        // WebUI caller's automation visibility.
        let access = self
            .resolve_thread_access_for_caller(caller_for_fallback, scope, &actor)
            .await?;
        match self
            .gate_resolution_route(
                &access.scope,
                &access.run_actor,
                run_id,
                &gate_ref,
                &resolution,
            )
            .await?
        {
            GateResolutionRoute::Approval => {
                self.resolve_approval_gate(
                    access.scope,
                    access.run_actor,
                    run_id,
                    gate_ref,
                    client_action_id,
                    resolution,
                )
                .await
            }
            GateResolutionRoute::Auth => {
                self.resolve_auth_gate(
                    access.scope,
                    access.run_actor,
                    run_id,
                    gate_ref,
                    client_action_id,
                    resolution,
                )
                .await
            }
            GateResolutionRoute::Generic => {
                self.resolve_generic_gate(
                    access.scope,
                    access.run_actor,
                    run_id,
                    gate_ref,
                    client_action_id,
                    resolution,
                )
                .await
            }
        }
    }

    pub async fn retry_run(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiRetryRunRequest,
    ) -> Result<RebornRetryRunResponse, RebornServicesError> {
        let caller_for_fallback = caller.clone();
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::RetryRun {
            scope,
            actor,
            run_id,
            client_action_id,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };

        let access = self
            .resolve_thread_access_for_caller(caller_for_fallback, scope, &actor)
            .await?;
        // Serialize retry admission with thread deletion. `delete_thread` holds
        // this same per-thread lock across its active-run probe + delete; taking
        // it here closes the window where a concurrent delete passes its probe
        // (the failed run is terminal) and then deletes the thread while
        // `retry_turn` enqueues a replacement run against it.
        let _thread_operation_guard = self.lock_thread_operation(&access.scope).await;
        let binding_id = webui_retry_binding_id(&access.scope, run_id, &client_action_id);
        let response = self
            .turn_coordinator
            .retry_turn(RetryTurnRequest {
                scope: access.scope,
                actor: access.run_actor,
                run_id,
                source_binding_ref: webui_source_binding_ref_from_raw(
                    "webui-retry-src",
                    &binding_id,
                )?,
                reply_target_binding_ref: webui_reply_target_binding_ref_from_raw(
                    "webui-retry-reply",
                    &binding_id,
                )?,
                idempotency_key: client_action_id,
            })
            .await
            .map_err(map_turn_error)?;
        Ok(response.into())
    }

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let run_id = parse_run_id_field("run_id", request.run_id)?;
        let scope = caller.turn_scope(thread_id);
        let actor = caller.actor();
        // Ownership probe with automation-trigger fallback. Without this gate
        // any caller sharing (tenant, agent, project) could read another user's
        // run state by guessing thread_id and run_id. The fallback also allows
        // the owner of an automation to poll run state on a trigger-fired thread.
        let access = self
            .resolve_thread_access_for_caller(caller, scope, &actor)
            .await?;
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: access.scope,
                run_id,
            })
            .await
            .map_err(map_turn_error)?;
        // Price a default-model run (no `resolved_model_route`) against the
        // runtime's live active model. Cheap synchronous read; `None` when no
        // reader is wired or no concrete model is configured, in which case the
        // run reports usage without cost (unchanged behavior).
        let active_model = self
            .active_model_reader
            .as_ref()
            .and_then(|reader| reader.active_model_id());
        Ok(RebornGetRunStateResponse::from_run_state(
            state,
            active_model.as_deref(),
        ))
    }

    async fn pause_automation(
        &self,
        caller: WebUiAuthenticatedCaller,
        automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        let Some(caller) = product_agent_bound_caller_from_webui(caller) else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        self.automation_facade
            .pause_automation(caller, automation_id)
            .await
    }

    async fn resume_automation(
        &self,
        caller: WebUiAuthenticatedCaller,
        automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        let Some(caller) = product_agent_bound_caller_from_webui(caller) else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        self.automation_facade
            .resume_automation(caller, automation_id)
            .await
    }

    async fn rename_automation(
        &self,
        caller: WebUiAuthenticatedCaller,
        automation_id: String,
        request: WebUiRenameAutomationRequest,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        let Some(caller) = product_agent_bound_caller_from_webui(caller) else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        let name = parse_automation_name(request)?;
        self.automation_facade
            .rename_automation(caller, automation_id, name)
            .await
    }

    async fn delete_automation(
        &self,
        caller: WebUiAuthenticatedCaller,
        automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        let Some(caller) = product_agent_bound_caller_from_webui(caller) else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        self.automation_facade
            .delete_automation(caller, automation_id)
            .await
    }

    pub async fn run_operator_service_lifecycle(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorServiceLifecycleRequest,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let request = RebornServiceLifecycleRequest {
            action: match request.action {
                RebornOperatorServiceLifecycleAction::Install => {
                    RebornServiceLifecycleAction::Install
                }
                RebornOperatorServiceLifecycleAction::Start => RebornServiceLifecycleAction::Start,
                RebornOperatorServiceLifecycleAction::Stop => RebornServiceLifecycleAction::Stop,
                RebornOperatorServiceLifecycleAction::Status => {
                    RebornServiceLifecycleAction::Status
                }
            },
        };
        let service_lifecycle = self
            .operator_service_lifecycle
            .control_service(caller, request)
            .await?;
        let status = match service_lifecycle.state {
            RebornServiceLifecycleState::Installed
            | RebornServiceLifecycleState::Running
            | RebornServiceLifecycleState::Stopped
            | RebornServiceLifecycleState::Unknown => RebornOperatorSurfaceStatus::Available,
            RebornServiceLifecycleState::Unsupported | RebornServiceLifecycleState::Failed => {
                RebornOperatorSurfaceStatus::Unavailable
            }
        };
        Ok(RebornOperatorCommandPlaneResponse {
            area: RebornOperatorArea::ServiceLifecycle,
            status,
            message: service_lifecycle.message.clone(),
            operator_status: None,
            logs: None,
            service_lifecycle: Some(service_lifecycle),
            diagnostics: Vec::new(),
        })
    }

    pub async fn test_llm_connection(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        validate_llm_base_url(request.base_url.as_deref())?;
        service
            .test_connection(caller, request)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    pub async fn list_llm_models(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        validate_llm_base_url(request.base_url.as_deref())?;
        service
            .list_models(caller, request)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    pub async fn start_nearai_login(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: NearAiLoginRequest,
    ) -> Result<NearAiLoginStart, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        service
            .start_nearai_login(caller, request)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    pub async fn start_codex_login(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<CodexLoginStart, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        service
            .start_codex_login(caller)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    pub async fn complete_nearai_wallet_login(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: NearAiWalletLoginRequest,
    ) -> Result<NearAiWalletLoginResult, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        service
            .complete_nearai_wallet_login(caller, request)
            .await
            .map_err(llm_config::map_llm_config_error)
    }
}

#[async_trait]
impl<I, V> ProductSurface for RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: ProductCapabilityInput,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        RebornServices::invoke(self, caller, capability, input, activity_id).await
    }

    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornViewQuery,
    ) -> Result<RebornViewPage, RebornServicesError> {
        RebornServices::query(self, caller, query).await
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        RebornServices::stream_events(self, caller, request).await
    }

    fn supports_stream_events_subscription(&self) -> bool {
        RebornServices::supports_stream_events_subscription(self)
    }

    async fn subscribe_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsSubscription, RebornServicesError> {
        RebornServices::subscribe_events(self, caller, request).await
    }

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        RebornServices::get_run_state(self, caller, request).await
    }

    async fn execute_command(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: ProductOperationRequest,
    ) -> Result<ProductOperationResponse, RebornServicesError> {
        self.execute_product_surface_operation(caller, request)
            .await
    }
}

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker,
    V: RebornViewProvider,
{
    async fn list_visible_threads_for_scope(
        &self,
        scope: ThreadScope,
        request: WebUiListThreadsRequest,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        let visible_limit = clamp_thread_list_limit(request.limit);
        let needs_approval = request.needs_approval;
        if needs_approval {
            return tokio::time::timeout(
                NOTIFICATION_APPROVAL_QUERY_TIMEOUT,
                self.list_automation_threads_needing_approval(
                    caller,
                    visible_limit,
                    request.candidate_thread_id,
                ),
            )
            .await
            .map_err(|_| notification_approval_timeout_error())?;
        }
        let fetch_limit = visible_limit
            .max(THREAD_LIST_FILTER_MIN_FETCH_SIZE)
            .min(THREAD_LIST_MAX_PAGE_SIZE as usize);
        let mut cursor = request.cursor;
        let mut visible_threads = Vec::with_capacity(visible_limit);
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while visible_threads.len() < visible_limit {
            if pages_fetched >= THREAD_LIST_FILTER_MAX_PAGES {
                tracing::warn!(
                    cursor = ?cursor,
                    pages_fetched,
                    max_pages = THREAD_LIST_FILTER_MAX_PAGES,
                    visible_threads = visible_threads.len(),
                    visible_limit,
                    "thread listing filter page budget exhausted while skipping automation threads"
                );
                next_cursor = None;
                break;
            }
            pages_fetched += 1;
            let response = self
                .thread_service
                .list_threads_for_scope(ironclaw_threads::ListThreadsForScopeRequest {
                    scope: scope.clone(),
                    limit: Some(fetch_limit as u32),
                    cursor: cursor.clone(),
                })
                .await
                .map_err(map_thread_error)?;
            for thread in response.threads {
                if is_automation_trigger_thread(&thread) {
                    continue;
                }
                visible_threads.push(thread);
            }
            next_cursor = response.next_cursor;
            let Some(next) = next_cursor.clone() else {
                break;
            };
            if cursor.as_deref() == Some(next.as_str()) {
                tracing::warn!(
                    cursor = %next,
                    "thread listing cursor did not advance while filtering automation threads"
                );
                next_cursor = None;
                break;
            }
            cursor = Some(next);
        }

        if visible_threads.len() > visible_limit {
            next_cursor = visible_threads
                .get(visible_limit.saturating_sub(1))
                .map(|thread| thread.thread_id.as_str().to_string());
            visible_threads.truncate(visible_limit);
        }

        Ok(RebornListThreadsResponse {
            threads: visible_threads,
            next_cursor,
        })
    }

    async fn list_automation_threads_needing_approval(
        &self,
        caller: WebUiAuthenticatedCaller,
        visible_limit: usize,
        candidate_thread_id: Option<String>,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        let Some(bound_caller) = product_agent_bound_caller_from_webui(caller.clone()) else {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        let automations = self
            .automation_facade
            .list_automations(
                bound_caller.clone(),
                AutomationListRequest {
                    limit: NOTIFICATION_APPROVAL_AUTOMATION_LIMIT,
                    run_limit: NOTIFICATION_APPROVAL_RUN_LIMIT,
                    include_completed: true,
                },
            )
            .await?;

        let mut candidate_seen = HashSet::new();
        let mut candidates = Vec::with_capacity(NOTIFICATION_APPROVAL_CANDIDATE_LIMIT);
        for automation in &automations {
            let title = AutomationNotificationTitle::from_name(&automation.name);
            for run in &automation.recent_runs {
                if let Some(thread_id) = &run.thread_id {
                    if candidate_seen.insert(thread_id.clone()) {
                        candidates.push(AutomationApprovalThreadCandidate {
                            thread_id: thread_id.clone(),
                            title: title.clone(),
                        });
                    }
                    if candidates.len() >= NOTIFICATION_APPROVAL_CANDIDATE_LIMIT {
                        break;
                    }
                }
            }
            if candidates.len() >= NOTIFICATION_APPROVAL_CANDIDATE_LIMIT {
                break;
            }
        }

        let mut seen = HashSet::new();
        let mut threads = Vec::with_capacity(visible_limit);
        if let Some(candidate_thread_id) = candidate_thread_id {
            let thread_id = parse_thread_id_field("candidate_thread_id", candidate_thread_id)?;
            if seen.insert(thread_id.clone()) {
                let listed_candidate = candidates
                    .iter()
                    .find(|candidate| candidate.thread_id == thread_id)
                    .cloned();
                let record = if let Some(candidate) = listed_candidate {
                    self.automation_run_thread_record(
                        &caller,
                        &bound_caller,
                        candidate.thread_id,
                        candidate.title,
                    )
                    .await?
                } else {
                    self.automation_run_thread_record(&caller, &bound_caller, thread_id, None)
                        .await?
                };
                if let Some(record) = record {
                    threads.push(record);
                }
            }
        }
        for candidate in candidates {
            if threads.len() >= visible_limit {
                break;
            }
            if !seen.insert(candidate.thread_id.clone()) {
                continue;
            }
            let Some(record) = self
                .automation_run_thread_record(
                    &caller,
                    &bound_caller,
                    candidate.thread_id,
                    candidate.title,
                )
                .await?
            else {
                continue;
            };
            threads.push(record);
        }

        Ok(RebornListThreadsResponse {
            threads,
            next_cursor: None,
        })
    }

    async fn automation_run_thread_record(
        &self,
        caller: &WebUiAuthenticatedCaller,
        bound_caller: &ProductAgentBoundCaller,
        thread_id: ThreadId,
        automation_title: Option<AutomationNotificationTitle>,
    ) -> Result<Option<SessionThreadRecord>, RebornServicesError> {
        let Some(trigger_scope) = self
            .automation_facade
            .resolve_run_thread_scope(bound_caller.clone(), &thread_id)
            .await?
        else {
            return Ok(None);
        };
        self.automation_run_thread_record_for_scope(
            caller,
            bound_caller,
            thread_id,
            trigger_scope,
            automation_title,
        )
        .await
    }

    async fn automation_run_thread_record_for_scope(
        &self,
        caller: &WebUiAuthenticatedCaller,
        bound_caller: &ProductAgentBoundCaller,
        thread_id: ThreadId,
        trigger_scope: TriggerRunThreadScope,
        title: Option<AutomationNotificationTitle>,
    ) -> Result<Option<SessionThreadRecord>, RebornServicesError> {
        let true_agent_id = trigger_scope
            .agent_id
            .clone()
            .or_else(|| Some(bound_caller.agent_id.clone()));
        let creator_user_id = trigger_scope.creator_user_id.clone();

        let approval_turn_scope = TurnScope::new(
            caller.tenant_id.clone(),
            true_agent_id.clone(),
            trigger_scope.project_id.clone(),
            thread_id.clone(),
        );
        let run_actor = TurnActor::new(creator_user_id.clone());
        if !self
            .thread_scope_has_pending_approval(&approval_turn_scope, &run_actor)
            .await?
        {
            return Ok(None);
        }

        let mut record = None;
        for owner_user_id in [Some(creator_user_id.clone()), None] {
            let thread_turn_scope = TurnScope::new_with_owner(
                caller.tenant_id.clone(),
                true_agent_id.clone(),
                trigger_scope.project_id.clone(),
                thread_id.clone(),
                owner_user_id,
            );
            let thread_scope = thread_scope_from_turn_scope(
                &thread_turn_scope,
                thread_turn_scope.explicit_owner_user_id().cloned(),
            )?;
            match self
                .thread_service
                .read_thread(ThreadHistoryRequest {
                    scope: thread_scope,
                    thread_id: thread_turn_scope.thread_id.clone(),
                })
                .await
            {
                Ok(found) => {
                    record = Some(found);
                    break;
                }
                Err(
                    SessionThreadError::UnknownThread { .. }
                    | SessionThreadError::ThreadScopeMismatch { .. },
                ) => {}
                Err(error) => return Err(map_ownership_probe_error(error)),
            }
        }
        let Some(mut record) = record else {
            return Ok(None);
        };
        if record
            .title
            .as_ref()
            .is_none_or(|title| title.trim().is_empty())
            && let Some(title) = title.as_ref()
        {
            record.title = Some(title.as_str().to_string());
        }
        Ok(Some(record))
    }

    async fn thread_scope_has_pending_approval(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
    ) -> Result<bool, RebornServicesError> {
        let pending = self
            .approval_interactions
            .list_pending(ListPendingApprovalsRequest {
                scope: scope.clone(),
                actor: actor.clone(),
            })
            .await
            .map_err(|error| map_adapter_error(error.into()))?;
        Ok(!pending.approvals.is_empty())
    }

    fn thread_operation_lock(&self, scope: &TurnScope) -> Arc<AsyncMutex<()>> {
        let key = thread_operation_key(scope);
        let mut locks = match self.thread_operation_locks.lock() {
            Ok(locks) => locks,
            Err(poisoned) => poisoned.into_inner(),
        };
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    async fn lock_thread_operation(&self, scope: &TurnScope) -> OwnedMutexGuard<()> {
        self.thread_operation_lock(scope).lock_owned().await
    }

    /// Per-tenant lock serializing admin mutations that affect the active-admin
    /// count (role/status/delete). `ensure_not_last_admin` re-reads the count
    /// then mutates; without serialization two concurrent demotions each see
    /// "2 admins", both pass, and both land — stranding the tenant with zero
    /// admins (a TOCTOU race). Holding this across the check+mutation makes the
    /// count read authoritative. Reuses the same weak-ref keyed registry as
    /// `thread_operation_lock`, namespaced so the keyspaces cannot collide.
    ///
    /// Scope of the guarantee: this lock lives in the current `RebornServices`
    /// instance, so it serializes every admin mutation within one process. The
    /// standalone `ironclaw-reborn serve` binary is single-process, so last-
    /// admin protection is airtight there. It does NOT span multiple runtime
    /// instances sharing one identity filesystem (a not-yet-supported multi-
    /// replica deployment): two processes each hold their own lock and could
    /// both read `active_admins > 1` before demoting different admins. Closing
    /// that requires a durable per-tenant lease (a CAS-guarded lock record in
    /// the identity store) shared by all instances — deferred until a multi-
    /// replica deployment mode exists, since a hand-rolled filesystem lease adds
    /// crash-recovery/stale-takeover risk that outweighs the bounded race it
    /// would replace in the single-process product shipping today.
    async fn lock_admin_mutation(&self, tenant: &TenantId) -> OwnedMutexGuard<()> {
        let key = format!("admin-mutation:{}", tenant.as_str());
        let lock = {
            let mut locks = match self.thread_operation_locks.lock() {
                Ok(locks) => locks,
                Err(poisoned) => poisoned.into_inner(),
            };
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    async fn reject_delete_with_active_run(
        &self,
        scope: &TurnScope,
        thread_scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), RebornServicesError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(map_timeline_probe_error)?;
        let mut seen = HashSet::new();
        for run_id in history
            .messages
            .iter()
            .filter_map(|message| message.turn_run_id.as_deref())
            .map(parse_persisted_turn_run_id)
        {
            let run_id = run_id?;
            if !seen.insert(run_id) {
                continue;
            }
            match self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
            {
                Ok(state) if state.status.keeps_active_lock() => {
                    return Err(delete_thread_busy());
                }
                Ok(_) | Err(TurnError::ScopeNotFound) => {}
                Err(error) => return Err(map_turn_error(error)),
            }
        }
        Ok(())
    }
}

fn automation_unavailable() -> RebornServicesError {
    RebornServicesError::service_unavailable(true)
}

fn is_automation_trigger_thread(thread: &SessionThreadRecord) -> bool {
    let Some(metadata) = thread.metadata_json.as_deref() else {
        return false;
    };
    match thread_metadata_is_automation_trigger(metadata) {
        Ok(is_automation_trigger) => is_automation_trigger,
        Err(error) => {
            tracing::debug!(
                error = %error,
                thread_id = %thread.thread_id,
                "failed to parse thread metadata_json for automation filter"
            );
            false
        }
    }
}

fn outbound_preferences_unavailable() -> RebornServicesError {
    RebornServicesError::service_unavailable(false)
}

fn operator_surface_unavailable() -> RebornServicesError {
    RebornServicesError::service_unavailable(false)
}

struct AcceptedWebUiMessage {
    thread_id: ThreadId,
    message_id: ThreadMessageId,
    actor_id: String,
    source_binding_id: String,
    reply_target_binding_id: String,
}

async fn mark_message_submitted_or_replay(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
    turn_id: String,
    run_id: String,
) -> Result<(), RebornServicesError> {
    match thread_service
        .mark_message_submitted(
            thread_scope,
            &handoff.thread_id,
            handoff.message_id,
            turn_id,
            run_id.clone(),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            reconcile_terminal_duplicate(
                thread_service,
                thread_scope,
                handoff,
                client_action_id,
                |replay| {
                    replay.status == MessageStatus::Submitted && replay.turn_run_id == Some(run_id)
                },
                error,
            )
            .await
        }
    }
}

async fn mark_message_rejected_busy_or_replay(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
) -> Result<(), RebornServicesError> {
    match thread_service
        .mark_message_rejected_busy(thread_scope, &handoff.thread_id, handoff.message_id)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            // Only RejectedBusy is the terminal settled state here.
            // DeferredBusy is non-terminal legacy — a later replay may
            // resubmit it, so claiming it settled would violate the
            // no-resubmit guarantee. Let a DeferredBusy replay fall
            // through to the `_` arm so the original mark failure
            // surfaces honestly instead of being masked as settled.
            reconcile_terminal_duplicate(
                thread_service,
                thread_scope,
                handoff,
                client_action_id,
                |replay| matches!(replay.status, MessageStatus::RejectedBusy),
                error,
            )
            .await
        }
    }
}

async fn reconcile_terminal_duplicate(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
    matches_replay: impl FnOnce(&AcceptedInboundMessageReplay) -> bool,
    original_error: SessionThreadError,
) -> Result<(), RebornServicesError> {
    let replay = thread_service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: thread_scope.clone(),
            actor_id: handoff.actor_id.clone(),
            source_binding_id: handoff.source_binding_id.clone(),
            external_event_id: client_action_id.as_str().to_string(),
        })
        .await
        .map_err(map_thread_error)?;
    match replay {
        Some(replay)
            if replay.thread_id == handoff.thread_id
                && replay.message_id == handoff.message_id
                && matches_replay(&replay) =>
        {
            Ok(())
        }
        _ => Err(map_thread_error(original_error)),
    }
}

async fn replay_webui_send_message(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    scope: &TurnScope,
    actor: &TurnActor,
    external_event_id: &str,
) -> Result<Option<(AcceptedInboundMessageReplay, String)>, RebornServicesError> {
    let source_binding_id = webui_source_binding_id(scope, actor);
    if let Some(replay) = replay_accepted_message(
        thread_service,
        thread_scope,
        actor,
        &source_binding_id,
        external_event_id,
    )
    .await?
    {
        return Ok(Some((replay, source_binding_id)));
    }

    let legacy_source_binding_id = legacy_webui_source_binding_id(scope, actor);
    replay_accepted_message(
        thread_service,
        thread_scope,
        actor,
        &legacy_source_binding_id,
        external_event_id,
    )
    .await
    .map(|replay| replay.map(|replay| (replay, legacy_source_binding_id)))
}

async fn replay_accepted_message(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    actor: &TurnActor,
    source_binding_id: &str,
    external_event_id: &str,
) -> Result<Option<AcceptedInboundMessageReplay>, RebornServicesError> {
    thread_service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: thread_scope.clone(),
            actor_id: actor.user_id.as_str().to_string(),
            source_binding_id: source_binding_id.to_string(),
            external_event_id: external_event_id.to_string(),
        })
        .await
        .map_err(map_thread_error)
}

struct ResolvedThreadAccess {
    scope: TurnScope,
    run_actor: TurnActor,
}

// Owner-bound thread resolution shared by the WebUI-facing methods that
// only need to prove a browser thread id belongs to the authenticated actor.
// The actor is pinned as `owner_user_id` so a caller sharing (tenant, agent,
// project) cannot act on a thread it does not own; `map_ownership_probe_error`
// collapses both UnknownThread and ThreadScopeMismatch into NotFound so the
// response cannot be used as an existence oracle.
//
// Automation-trigger threads are an exception: they are stored by
// `record_trigger_prompt` (trigger_poller_trusted_submit.rs) with
// `owner_user_id = Some(creator_user_id)` — the actor that fired the trigger
// — not the WebUI caller's user_id. The user-scoped probe therefore misses
// them. `resolve_thread_access_for_caller` handles that case via the shared
// automation fallback path; all interaction endpoints (stream, cancel, gate
// resolve, run-state) route through it so the reconstructed `TurnScope` (with
// `owner_user_id = Some(creator_user_id)`) is returned to callers that need
// to act on a trigger run.
//
// Authorization is revalidated on every call — no caching of the authz result
// — so a caller that loses automation visibility between polls cannot keep
// accessing the trigger-owned thread.
//
// Scope reconstruction field-by-field match against `record_trigger_prompt`
// (trigger_poller_trusted_submit.rs:285-291):
//   tenant_id    : resolution.turn_scope.tenant_id == caller's tenant_id (same installation)
//   agent_id     : resolution.turn_scope.agent_id OR default_agent_id
//                → trigger_scope.agent_id OR bound_caller.agent_id  (same fallback shape)
//   project_id   : resolution.turn_scope.project_id == trigger_scope.project_id
//   owner_user_id: Some(resolution.actor.user_id)
//                == Some(trigger_scope.creator_user_id)
//                == Some(fire.creator_user_id) [post-#4754: new first-fire bindings
//                   persist creator; legacy (pre-#4754) bindings remain owner-None
//                   and will not match — accepted breakage; recreate trigger to fix].
impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    /// Shared authorization check for automation-trigger threads.
    ///
    /// Checks whether `scope.thread_id` belongs to one of the authenticated
    /// caller's automation triggers and, if so, returns a `TurnScope` with the
    /// TRUE stored scope (agent_id, project_id, and owner_user_id = creator_user_id).
    ///
    /// Requires #4754 ("Part A"): `record_trigger_prompt` stores threads with
    /// `owner_user_id = Some(fire.creator_user_id)` only for new first-fire
    /// bindings created after #4754 landed. Pre-#4754 (legacy) runs were stored
    /// with `owner_user_id = None`; their gate/cancel/run-state will NOT match
    /// the reconstructed scope — this is accepted breakage; recreating the
    /// trigger creates a fresh owner-bearing binding.
    ///
    /// Delegates to `AutomationProductFacade::resolve_run_thread_scope` which
    /// is caller-scoped: authorization is embedded in the repository lookup.
    /// If the trigger exists for this caller and contains the run, the returned
    /// scope lets all downstream storage lookups (timeline, gate, cancel, SSE)
    /// find the thread as stored rather than under the caller's session scope.
    ///
    /// Authorization is revalidated on every call (no caching) so a caller
    /// that loses automation visibility cannot keep acting on the thread.
    ///
    /// Returns `original_not_found_error` when:
    ///  - The caller has no bound agent.
    ///  - `resolve_run_thread_scope` returns `None` (thread not in caller's triggers).
    ///
    /// This is the authorization half of the trigger-thread fallback. Callers
    /// that need the full transcript call `try_automation_trigger_timeline_fallback`.
    async fn check_automation_trigger_access(
        &self,
        caller: WebUiAuthenticatedCaller,
        scope: &TurnScope,
        original_not_found_error: RebornServicesError,
    ) -> Result<ResolvedThreadAccess, RebornServicesError> {
        let Some(bound_caller) = product_agent_bound_caller_from_webui(caller) else {
            return Err(original_not_found_error);
        };
        let thread_id = &scope.thread_id;
        let Some(trigger_scope) = self
            .automation_facade
            .resolve_run_thread_scope(bound_caller.clone(), thread_id)
            .await?
        else {
            return Err(original_not_found_error);
        };
        // Use the trigger's stored agent_id; fall back to the caller's agent_id
        // when the trigger record had no explicit agent.
        let true_agent_id = trigger_scope
            .agent_id
            .or_else(|| Some(bound_caller.agent_id.clone()));
        let run_actor = TurnActor::new(trigger_scope.creator_user_id.clone());
        Ok(ResolvedThreadAccess {
            scope: TurnScope::new_with_owner(
                scope.tenant_id.clone(),
                true_agent_id,
                trigger_scope.project_id,
                thread_id.clone(),
                Some(trigger_scope.creator_user_id),
            ),
            run_actor,
        })
    }

    /// Fallback timeline fetch for automation-trigger threads.
    ///
    /// Automation-trigger threads are created under the trigger creator's
    /// scope, not the caller's session scope. The normal user-scoped
    /// `list_thread_history` therefore always misses them. This fallback is
    /// only reached when the user-scoped lookup returned `UnknownThread` or
    /// `ThreadScopeMismatch`.
    ///
    /// Authorization: the thread_id must appear in at least one `recent_run`
    /// for an automation returned by `list_automations` for this caller. That
    /// is the same authorization check the Automations list endpoint applies,
    /// so no new trust boundary is introduced. Authorization is revalidated on
    /// every call — no caching.
    ///
    /// On authorization success, the history is loaded with the trigger-owned
    /// scope. On authorization failure (thread not in any of the caller's
    /// automation runs), the `original_not_found_error` is returned so the
    /// response is indistinguishable from a genuinely absent thread.
    /// Resolve a caller-visible thread's history together with the thread scope
    /// it actually lives under.
    ///
    /// The primary path is the caller's own session scope. On a 404-class miss
    /// it applies the automation-trigger fallback: trigger-fired threads are
    /// stored under the creator's scope, not the WebUI caller's session scope,
    /// so the user-scoped lookup always misses them. If the thread belongs to
    /// one of the caller's automations (`list_automations` applies the same
    /// authorization), the history is re-fetched under the trigger-owned scope.
    /// Both `UnknownThread` and `ThreadScopeMismatch` are eligible for the
    /// fallback; backend/serialization errors propagate as-is.
    ///
    /// Returning the resolved scope — not just the history — lets callers that
    /// must do further scope-bound work (e.g. reading attachment bytes through
    /// the project mount) address the correct scope instead of re-deriving the
    /// caller's session scope, which would be wrong for a trigger thread.
    async fn resolve_thread_history_for_caller(
        &self,
        caller: WebUiAuthenticatedCaller,
        scope: &TurnScope,
    ) -> Result<(ThreadScope, ThreadHistory), RebornServicesError> {
        let thread_scope =
            thread_scope_from_turn_scope(scope, Some(caller.actor().user_id.clone()))?;
        match self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: scope.thread_id.clone(),
            })
            .await
        {
            Ok(history) => Ok((thread_scope, history)),
            Err(
                SessionThreadError::UnknownThread { .. }
                | SessionThreadError::ThreadScopeMismatch { .. },
            ) => {
                let original_error =
                    RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false);
                self.try_automation_trigger_timeline_fallback(caller, scope, original_error)
                    .await
            }
            Err(err) => Err(map_timeline_probe_error(err)),
        }
    }

    async fn try_automation_trigger_timeline_fallback(
        &self,
        caller: WebUiAuthenticatedCaller,
        scope: &TurnScope,
        original_not_found_error: RebornServicesError,
    ) -> Result<(ThreadScope, ThreadHistory), RebornServicesError> {
        let access = self
            .check_automation_trigger_access(caller, scope, original_not_found_error)
            .await?;
        // Authorized: re-fetch the history using the TRUE stored scope
        // (owner_user_id = creator_user_id, not the caller's session user) and
        // return that scope so byte reads address the trigger creator's mount.
        let true_thread_scope = thread_scope_from_turn_scope(
            &access.scope,
            access.scope.explicit_owner_user_id().cloned(),
        )?;
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: true_thread_scope.clone(),
                thread_id: access.scope.thread_id.clone(),
            })
            .await
            .map_err(map_timeline_probe_error)?;
        Ok((true_thread_scope, history))
    }

    /// Ownership probe for interaction endpoints (stream, cancel, gate resolve,
    /// run-state).
    ///
    /// Tries the primary user-scoped `read_thread` probe. On a 404-class miss
    /// (UnknownThread / ThreadScopeMismatch), falls back to the automation
    /// trigger authorization check. If the thread belongs to one of the
    /// caller's automations, returns the trigger-owned `TurnScope` and run
    /// actor so downstream turn operations address the submitted run. Non-owner
    /// callers and genuinely absent threads both receive the same canonical
    /// NotFound response.
    ///
    /// Authorization is revalidated on every call — no caching of the authz
    /// result — so a caller that loses automation visibility cannot keep
    /// acting on the thread after their access is revoked.
    async fn resolve_thread_access_for_caller(
        &self,
        caller: WebUiAuthenticatedCaller,
        scope: TurnScope,
        actor: &TurnActor,
    ) -> Result<ResolvedThreadAccess, RebornServicesError> {
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(actor.user_id.clone()))?;
        match self
            .thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: scope.thread_id.clone(),
            })
            .await
        {
            Ok(_) => Ok(ResolvedThreadAccess {
                scope,
                run_actor: actor.clone(),
            }),
            Err(
                SessionThreadError::UnknownThread { .. }
                | SessionThreadError::ThreadScopeMismatch { .. },
            ) => {
                let original_error =
                    RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false);
                let access = self
                    .check_automation_trigger_access(caller, &scope, original_error)
                    .await?;
                Ok(ResolvedThreadAccess {
                    scope: access.scope,
                    run_actor: access.run_actor,
                })
            }
            Err(err) => Err(map_ownership_probe_error(err)),
        }
    }

    fn require_project_filesystem(
        &self,
    ) -> Result<&Arc<dyn ProjectFilesystemReader>, RebornServicesError> {
        self.project_filesystem
            .as_ref()
            .ok_or_else(|| RebornServicesError::service_unavailable(false))
    }

    /// Resolve the wired browse reader and verify it serves the requested
    /// mount. An unwired reader is a 503 (composition fault, retryable-false);
    /// a known-but-unserved mount is a 404 so probing an unavailable mount
    /// cannot distinguish "wrong path" from "not wired".
    fn require_filesystem_browser(
        &self,
        mount: FsMount,
    ) -> Result<&Arc<dyn FilesystemBrowseReader>, RebornServicesError> {
        let browser = self
            .filesystem_browser
            .as_ref()
            .ok_or_else(|| RebornServicesError::service_unavailable(false))?;
        if !browser.available_mounts().contains(&mount) {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::NotFound,
                404,
                false,
            ));
        }
        Ok(browser)
    }

    fn require_project_service(&self) -> Result<&Arc<dyn ProjectService>, RebornServicesError> {
        self.project_service
            .as_ref()
            .ok_or_else(|| RebornServicesError::service_unavailable(false))
    }

    /// Authorize a browser-proposed project for a new thread and, on success,
    /// adopt it as the caller's scope for that thread only.
    ///
    /// The project must never be trusted from the request body alone: the
    /// proposed id is authorized through the same access-controlled
    /// [`get_project`](ProductSurface::get_project) read the project detail
    /// route uses (`Ok` only when the caller can access the project, otherwise a
    /// not-found/denied error). Without a proposed project the caller's default
    /// scope is returned unchanged.
    async fn authorize_create_thread_project(
        &self,
        caller: WebUiAuthenticatedCaller,
        requested_project_id: Option<String>,
    ) -> Result<WebUiAuthenticatedCaller, RebornServicesError> {
        let Some(raw) = requested_project_id else {
            return Ok(caller);
        };
        let project_id = ProjectId::new(raw).map_err(|error| {
            // Carry the cause to the server log before mapping to the
            // sanitized validation error (.claude/rules/error-handling.md —
            // never `map_err(|_| …)` on a parse/validation failure).
            tracing::debug!(?error, "create_thread received an invalid project_id");
            RebornServicesError::validation(WebUiInboundValidationError::new(
                "project_id",
                WebUiInboundValidationCode::InvalidId,
            ))
        })?;
        self.authorize_project_caller(caller, project_id).await
    }

    /// Authorize a project selector through the project service and adopt it
    /// only after the access probe succeeds.
    async fn authorize_project_caller(
        &self,
        mut caller: WebUiAuthenticatedCaller,
        project_id: ProjectId,
    ) -> Result<WebUiAuthenticatedCaller, RebornServicesError> {
        self.get_project(
            caller.clone(),
            RebornGetProjectRequest {
                project_id: project_id.as_str().to_string(),
            },
        )
        .await?;
        caller.project_id = Some(project_id);
        Ok(caller)
    }

    /// Resolve the one authorized scope used by all standalone browse reads.
    /// An omitted selector preserves the caller's existing project scope.
    async fn authorize_browse_scope(
        &self,
        caller: WebUiAuthenticatedCaller,
        project_id: Option<ProjectId>,
    ) -> Result<ResourceScope, RebornServicesError> {
        let caller = match project_id {
            Some(project_id) => self.authorize_project_caller(caller, project_id).await?,
            None => caller,
        };
        Ok(caller_browse_scope(&caller))
    }

    /// Verify the caller may access the thread and return the project-scoped
    /// [`ThreadScope`] its workspace files resolve under. Reuses the same
    /// ownership + automation-trigger fallback probe as event streaming, so a
    /// caller sharing (tenant, agent, project) cannot read another user's
    /// project files by guessing a thread id.
    async fn authorize_project_fs_access(
        &self,
        caller: WebUiAuthenticatedCaller,
        thread_id: String,
    ) -> Result<ThreadScope, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", thread_id)?;
        let actor = caller.actor();
        let access = self
            .resolve_thread_access_for_caller(caller.clone(), caller.turn_scope(thread_id), &actor)
            .await?;
        thread_scope_from_turn_scope(&access.scope, Some(access.run_actor.user_id.clone()))
    }

    /// Ownership probe for `submit_turn` and `delete_thread` — these only
    /// operate on session-owned threads (not trigger threads), so the probe
    /// is user-scoped with no automation fallback.
    async fn resolve_webui_thread_metadata(
        &self,
        scope: TurnScope,
        actor: &TurnActor,
    ) -> Result<(TurnScope, ThreadScope), RebornServicesError> {
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(actor.user_id.clone()))?;
        // `read_thread` is the metadata-only probe; production backends
        // override it to skip the message/summary load entirely. The
        // ownership semantics (UnknownThread / ThreadScopeMismatch
        // collapse to NotFound) must match `list_thread_history`'s
        // path, which `map_ownership_probe_error` guarantees.
        self.thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: scope.thread_id.clone(),
            })
            .await
            .map_err(map_ownership_probe_error)?;
        Ok((scope, thread_scope))
    }

    async fn resolve_approval_gate(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: GateRef,
        client_action_id: IdempotencyKey,
        resolution: WebUiGateResolution,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        let decision = match resolution {
            WebUiGateResolution::Approved { always } => {
                if always {
                    ApprovalInteractionDecision::AlwaysAllow
                } else {
                    ApprovalInteractionDecision::ApproveOnce
                }
            }
            WebUiGateResolution::Declined => ApprovalInteractionDecision::Deny,
            WebUiGateResolution::CredentialProvided { .. } => {
                return Err(blocked_authentication_unavailable());
            }
        };
        let response = self
            .approval_interactions
            .resolve(ResolveApprovalInteractionRequest {
                scope,
                actor,
                run_id_hint: Some(run_id),
                gate_ref,
                decision,
                idempotency_key: client_action_id,
            })
            .await
            .map_err(|error| map_adapter_error(error.into()))?;
        match response {
            ResolveApprovalInteractionResponse::Approved(response)
            | ResolveApprovalInteractionResponse::Resumed(response) => {
                Ok(RebornResolveGateResponse::Resumed(response.into()))
            }
        }
    }

    async fn gate_resolution_route(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        run_id: TurnRunId,
        gate_ref: &GateRef,
        resolution: &WebUiGateResolution,
    ) -> Result<GateResolutionRoute, RebornServicesError> {
        let state = match self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
        {
            Ok(state) => state,
            Err(error) if error.category() == ironclaw_turns::TurnErrorCategory::ScopeNotFound => {
                return Ok(GateResolutionRoute::from_gate_shape(gate_ref, resolution));
            }
            Err(error) => return Err(map_turn_error(error)),
        };
        if state.actor.as_ref() != Some(actor) {
            return Err(participant_denied());
        }
        // This read only selects the WebUI route. The typed auth/approval
        // services intentionally re-read run-state through `blocked_gate_state`
        // before mutating auth/approval records or resuming/cancelling a run,
        // so stale facade classification cannot authorize a side effect.
        GateResolutionRoute::from_run_state(
            state.status,
            state.gate_ref.as_ref(),
            gate_ref,
            resolution,
        )
    }

    async fn resolve_auth_gate(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: GateRef,
        client_action_id: IdempotencyKey,
        resolution: WebUiGateResolution,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        let decision = match resolution {
            WebUiGateResolution::CredentialProvided { credential_ref } => {
                AuthInteractionDecision::CredentialProvided {
                    credential_ref: parse_credential_account_id(&credential_ref)
                        .map_err(map_auth_interaction_error)?,
                }
            }
            WebUiGateResolution::Declined => AuthInteractionDecision::Deny,
            WebUiGateResolution::Approved { .. } => {
                return Err(blocked_authentication_unavailable());
            }
        };
        let response = self
            .auth_interactions
            .resolve(ResolveAuthInteractionRequest {
                scope,
                actor,
                run_id_hint: Some(run_id),
                gate_ref,
                decision,
                idempotency_key: client_action_id,
            })
            .await
            .map_err(map_auth_interaction_error)?;
        match response {
            ResolveAuthInteractionResponse::Resumed(response) => {
                Ok(RebornResolveGateResponse::Resumed(response.into()))
            }
            ResolveAuthInteractionResponse::Canceled(response) => {
                Ok(RebornResolveGateResponse::Cancelled(response.into()))
            }
        }
    }

    async fn resolve_generic_gate(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: GateRef,
        client_action_id: IdempotencyKey,
        resolution: WebUiGateResolution,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        match resolution {
            WebUiGateResolution::Approved { always } => {
                reject_generic_auth_gate_resolution(self.turn_coordinator.as_ref(), &scope, run_id)
                    .await?;
                // Generic fallback has only one-shot `resume_turn`; persistent
                // approval must go through the typed approval interaction path.
                if always {
                    return Err(persistent_approval_unavailable());
                }
                let binding_id = webui_gate_binding_id(&scope, &gate_ref_string(&gate_ref));
                let response = self
                    .turn_coordinator
                    .resume_turn(ResumeTurnRequest {
                        scope,
                        actor,
                        run_id,
                        gate_resolution_ref: gate_ref,
                        precondition: ResumeTurnPrecondition::AnyBlockedGate,
                        source_binding_ref: webui_source_binding_ref_from_raw(
                            "webui-gate-src",
                            &binding_id,
                        )?,
                        reply_target_binding_ref: webui_reply_target_binding_ref_from_raw(
                            "webui-gate-reply",
                            &binding_id,
                        )?,
                        idempotency_key: client_action_id,
                        resume_disposition: None,
                    })
                    .await
                    .map_err(map_turn_error)?;
                Ok(RebornResolveGateResponse::Resumed(response.into()))
            }
            WebUiGateResolution::CredentialProvided { .. } => {
                Err(blocked_authentication_unavailable())
            }
            WebUiGateResolution::Declined => {
                assert_generic_run_parked_on_gate(
                    self.turn_coordinator.as_ref(),
                    &scope,
                    run_id,
                    &gate_ref,
                )
                .await?;
                // `cancel_run` is not gate-aware, so without this check a
                // denied/cancelled resolution for a stale or attacker-supplied
                // gate_ref would terminate any non-terminal run sharing run_id.
                let response = self
                    .turn_coordinator
                    .cancel_run(ironclaw_turns::CancelRunRequest {
                        scope,
                        actor,
                        run_id,
                        reason: SanitizedCancelReason::UserRequested,
                        idempotency_key: client_action_id,
                    })
                    .await
                    .map_err(map_turn_error)?;
                Ok(RebornResolveGateResponse::Cancelled(response.into()))
            }
        }
    }
}

/// Ownership probes must collapse "thread does not exist" and "thread exists
/// but is owned by another caller" into NotFound so that a caller sharing the
/// (tenant, agent, project) scope cannot tell whether the supplied `thread_id`
/// matches a real thread under a different owner. The current backends return
/// `UnknownThread` for both cases on `list_thread_history`, but the contract
/// also permits `ThreadScopeMismatch`; remap it explicitly so a future backend
/// change cannot silently reintroduce an existence-leak.
fn map_ownership_probe_error(error: SessionThreadError) -> RebornServicesError {
    match &error {
        SessionThreadError::ThreadScopeMismatch { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        _ => map_thread_error(error),
    }
}

/// Derive the read-only browse scope from the authenticated caller.
///
/// The standalone filesystem viewer is not thread-bound, so the scope comes
/// straight from the trusted caller identity (tenant/user/agent/project) — never
/// from the request body. A fresh `invocation_id` is minted per call; the
/// scoped filesystem namespaces storage by tenant/user/agent/project, so this
/// addresses the same mount the agent's own tools wrote through.
fn caller_browse_scope(caller: &WebUiAuthenticatedCaller) -> ResourceScope {
    ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Map a project-filesystem read error to the sanitized facade error taxonomy.
/// No host paths or backend strings cross this boundary — only coarse
/// transport/status shape.
fn map_project_fs_error(error: ProjectFsError) -> RebornServicesError {
    match error {
        ProjectFsError::NotFound => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        ProjectFsError::NotAFile | ProjectFsError::NotADirectory | ProjectFsError::InvalidPath => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        ProjectFsError::Denied => participant_denied(),
        ProjectFsError::TooLarge { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 413, false)
        }
        ProjectFsError::Unavailable => RebornServicesError::service_unavailable(true),
        ProjectFsError::Internal => RebornServicesError::internal(),
    }
}

fn project_caller(caller: &WebUiAuthenticatedCaller) -> ProjectCaller {
    ProjectCaller {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
    }
}

fn product_capability_input_error(field: &'static str) -> RebornServicesError {
    RebornServicesError::validation(WebUiInboundValidationError::new(
        field,
        WebUiInboundValidationCode::InvalidValue,
    ))
}

fn product_command_input<T>(input: serde_json::Value) -> Result<T, RebornServicesError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(input).map_err(|error| {
        tracing::debug!(?error, "failed to decode product command input");
        product_capability_input_error("input")
    })
}

fn product_secret_handle(handle: String) -> Result<SecretHandle, RebornServicesError> {
    SecretHandle::new(handle).map_err(|error| {
        tracing::debug!(%error, "admin user secret handle validation failed");
        product_capability_input_error("handle")
    })
}

fn map_project_service_error(error: ProjectServiceError) -> RebornServicesError {
    match error {
        ProjectServiceError::NotFound => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        ProjectServiceError::Denied => participant_denied(),
        ProjectServiceError::InvalidInput { field } => {
            let mut error = RebornServicesError::from_status(
                RebornServicesErrorCode::InvalidRequest,
                400,
                false,
            );
            error.field = Some(field);
            error
        }
        ProjectServiceError::Conflict => {
            RebornServicesError::from_status(RebornServicesErrorCode::Conflict, 409, false)
        }
        ProjectServiceError::Unavailable => RebornServicesError::service_unavailable(true),
        ProjectServiceError::Internal => RebornServicesError::internal(),
    }
}

fn validate_current_gate_ref(
    parked_gate_ref: Option<&GateRef>,
    requested_gate_ref: &GateRef,
    kind: RebornServicesErrorKind,
) -> Result<(), RebornServicesError> {
    match parked_gate_ref {
        Some(parked) if parked == requested_gate_ref => Ok(()),
        _ => Err(RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Conflict,
            kind,
            409,
            false,
        )),
    }
}

fn participant_denied() -> RebornServicesError {
    RebornServicesError::from_status_kind(
        RebornServicesErrorCode::Forbidden,
        RebornServicesErrorKind::ParticipantDenied,
        403,
        false,
    )
}

/// Reject denied/cancelled generic gate resolutions whose `gate_ref` does not
/// match the gate the run is actually parked on. `cancel_run` is not gate-aware,
/// so without this guard a stale or attacker-supplied `gate_ref` would cancel
/// any non-terminal run sharing the same `run_id`.
async fn assert_generic_run_parked_on_gate(
    turn_coordinator: &dyn TurnCoordinator,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected_gate_ref: &GateRef,
) -> Result<(), RebornServicesError> {
    let state = turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .map_err(map_turn_error)?;
    if state.status == TurnStatus::BlockedAuth {
        return Err(blocked_authentication_unavailable());
    }
    if state.status == TurnStatus::BlockedApproval {
        return Err(blocked_approval_unavailable());
    }
    match state.gate_ref.as_ref() {
        Some(parked) if parked == expected_gate_ref => Ok(()),
        _ => Err(RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::BlockedApproval,
            409,
            false,
        )),
    }
}

/// Generic WebUI gate handling is intentionally not allowed to resume or
/// cancel auth-blocked runs. Auth gates must pass through
/// AuthInteractionService so completed-flow/credential validation and
/// BlockedAuthGate preconditions are enforced.
async fn reject_generic_auth_gate_resolution(
    turn_coordinator: &dyn TurnCoordinator,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> Result<(), RebornServicesError> {
    let state = turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .map_err(map_turn_error)?;
    if state.status == TurnStatus::BlockedAuth {
        return Err(blocked_authentication_unavailable());
    }
    if state.status == TurnStatus::BlockedApproval {
        return Err(blocked_approval_unavailable());
    }
    Ok(())
}

fn parse_credential_account_id(value: &str) -> Result<CredentialAccountId, ProductWorkflowError> {
    uuid::Uuid::parse_str(value)
        .map(CredentialAccountId::from_uuid)
        .map_err(|_| ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidCredentialRef,
        })
}

fn thread_scope_from_turn_scope(
    scope: &TurnScope,
    owner_user_id: Option<ironclaw_host_api::UserId>,
) -> Result<ThreadScope, RebornServicesError> {
    let Some(agent_id) = scope.agent_id.clone() else {
        return Err(RebornServicesError::from_status(
            RebornServicesErrorCode::InvalidRequest,
            400,
            false,
        ));
    };
    Ok(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

fn parse_thread_id_field(
    field: &'static str,
    value: String,
) -> Result<ThreadId, RebornServicesError> {
    ThreadId::new(value).map_err(|_| {
        RebornServicesError::validation(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::InvalidId,
        ))
    })
}

fn parse_run_id_field(
    field: &'static str,
    value: String,
) -> Result<TurnRunId, RebornServicesError> {
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| {
            RebornServicesError::validation(WebUiInboundValidationError::new(
                field,
                WebUiInboundValidationCode::InvalidId,
            ))
        })
}

fn parse_persisted_turn_run_id(value: &str) -> Result<TurnRunId, RebornServicesError> {
    TurnRunId::parse(value).map_err(|_| RebornServicesError::internal_invariant())
}

fn accepted_message_ref(message_id: String) -> Result<AcceptedMessageRef, RebornServicesError> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(|_| {
        RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
    })
}

fn parse_replay_run_id(value: Option<String>) -> Result<TurnRunId, RebornServicesError> {
    let Some(value) = value else {
        return Err(RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::ReplayUnavailable,
            409,
            false,
        ));
    };
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::ReplayUnavailable,
                409,
                false,
            )
        })
}

fn webui_source_binding_ref_from_raw(
    prefix: &str,
    raw: &str,
) -> Result<ironclaw_turns::SourceBindingRef, RebornServicesError> {
    bounded_source_binding_ref(prefix, raw, DEFAULT_BINDING_REF_RAW_MAX_BYTES).map_err(|_| {
        RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
    })
}

fn webui_reply_target_binding_ref_from_raw(
    prefix: &str,
    raw: &str,
) -> Result<ironclaw_turns::ReplyTargetBindingRef, RebornServicesError> {
    bounded_reply_target_binding_ref(prefix, raw, DEFAULT_BINDING_REF_RAW_MAX_BYTES).map_err(|_| {
        RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
    })
}

fn webui_source_binding_id(scope: &TurnScope, actor: &TurnActor) -> String {
    // WebUI retries are scoped to the authenticated caller context, not the
    // thread id. When the caller is not project-bound, we encode that
    // explicitly rather than collapsing onto an empty string.
    format!(
        "{}{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment(
            "project_scope",
            if scope.project_id.is_some() {
                "bound"
            } else {
                "none"
            }
        ),
        scope
            .project_id
            .as_ref()
            .map(|project_id| segment("project", project_id.as_str()))
            .unwrap_or_default(),
        segment("actor", actor.user_id.as_str())
    )
}

fn legacy_webui_source_binding_id(scope: &TurnScope, actor: &TurnActor) -> String {
    format!(
        "{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment("thread", scope.thread_id.as_str()),
        segment("actor", actor.user_id.as_str())
    )
}

fn thread_operation_key(scope: &TurnScope) -> String {
    format!(
        "{}{}{}{}{}",
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment(
            "project",
            scope
                .project_id
                .as_ref()
                .map(ProjectId::as_str)
                .unwrap_or("")
        ),
        segment("thread", scope.thread_id.as_str()),
        segment(
            "owner",
            scope
                .explicit_owner_user_id()
                .map(UserId::as_str)
                .unwrap_or("")
        )
    )
}

/// Default page size for [`ProductSurface::get_timeline`] when the
/// caller does not supply one. Sized to cover a typical chat history
/// without forcing a multi-megabyte JSON response on first load.
pub(crate) const TIMELINE_DEFAULT_PAGE_SIZE: u32 = 100;

/// Hard ceiling on the number of messages a single timeline response can
/// carry. Callers asking for more get the cap. Without this, a large
/// thread would let the per-route rate limit be the only thing bounding
/// per-request response size, which was the original Medium review
/// issue.
pub(crate) const TIMELINE_MAX_PAGE_SIZE: u32 = 200;

/// Default number of automation rows returned when the browser does not
/// request a smaller page.
pub const AUTOMATION_LIST_DEFAULT_PAGE_SIZE: u32 = 50;

/// Hard ceiling for the beta automation management list response. This keeps
/// the user-facing endpoint bounded until the trigger capability exposes an
/// opaque cursor contract.
pub const AUTOMATION_LIST_MAX_PAGE_SIZE: u32 = 100;

/// Default number of recent runs returned per automation row.
pub const AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE: u32 = 25;

/// Hard ceiling for recent runs embedded in each automation row.
pub const AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE: u32 = 100;

/// Hard ceiling on summary artifacts returned per response. Summary
/// artifacts are typically much smaller than the message transcript so
/// this cap is generous; it exists to bound the worst case where a
/// thread accumulates an unusual number of summaries.
const TIMELINE_MAX_SUMMARY_ARTIFACTS: usize = 200;

const THREAD_LIST_DEFAULT_PAGE_SIZE: u32 = 50;
const THREAD_LIST_MAX_PAGE_SIZE: u32 = 200;
const THREAD_LIST_FILTER_MIN_FETCH_SIZE: usize = 50;
const THREAD_LIST_FILTER_MAX_PAGES: usize = 20;
const NOTIFICATION_APPROVAL_AUTOMATION_LIMIT: usize = 20;
const NOTIFICATION_APPROVAL_RUN_LIMIT: usize = 20;
const NOTIFICATION_APPROVAL_CANDIDATE_LIMIT: usize = 20;
const NOTIFICATION_APPROVAL_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

fn clamp_timeline_limit(requested: Option<u32>) -> usize {
    let raw = requested.unwrap_or(TIMELINE_DEFAULT_PAGE_SIZE);
    let clamped = raw.clamp(1, TIMELINE_MAX_PAGE_SIZE);
    clamped as usize
}

fn clamp_thread_list_limit(requested: Option<u32>) -> usize {
    let raw = requested.unwrap_or(THREAD_LIST_DEFAULT_PAGE_SIZE);
    let clamped = raw.clamp(1, THREAD_LIST_MAX_PAGE_SIZE);
    clamped as usize
}

fn clamp_automation_list_limit(requested: Option<u32>) -> usize {
    let raw = requested.unwrap_or(AUTOMATION_LIST_DEFAULT_PAGE_SIZE);
    let clamped = raw.clamp(1, AUTOMATION_LIST_MAX_PAGE_SIZE);
    clamped as usize
}

fn clamp_automation_run_limit(requested: Option<u32>) -> usize {
    let raw = requested.unwrap_or(AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE);
    // 0 is intentional: callers suppress embedded run history by passing run_limit=0.
    let clamped = raw.min(AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE);
    clamped as usize
}

fn parse_automation_name(
    request: WebUiRenameAutomationRequest,
) -> Result<AutomationName, RebornServicesError> {
    let Some(raw_name) = request.name else {
        return Err(RebornServicesError::validation(
            WebUiInboundValidationError::new("name", WebUiInboundValidationCode::MissingField),
        ));
    };
    AutomationName::new(raw_name).map_err(automation_name_validation_error)
}

impl From<AutomationNameError> for WebUiInboundValidationCode {
    fn from(error: AutomationNameError) -> Self {
        match error {
            AutomationNameError::Empty => WebUiInboundValidationCode::Blank,
            AutomationNameError::TooLong => WebUiInboundValidationCode::TooLong,
        }
    }
}

fn automation_name_validation_error(error: AutomationNameError) -> RebornServicesError {
    RebornServicesError::validation(WebUiInboundValidationError::new("name", error.into()))
}

fn notification_approval_timeout_error() -> RebornServicesError {
    RebornServicesError::service_unavailable(true)
}

/// Wire shape of the opaque timeline cursor. The browser does not need
/// to interpret this; it just echoes the previous response's
/// `next_cursor` back as the next request's `cursor`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TimelineCursor {
    /// Only return messages whose `sequence` is strictly less than this
    /// value. Naming is deliberate: `before_*` makes the directional
    /// semantics (page backward through history) obvious at call sites.
    before_message_sequence: u64,
}

fn parse_timeline_cursor(raw: Option<&str>) -> Result<Option<TimelineCursor>, RebornServicesError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let cursor: TimelineCursor = serde_json::from_str(raw).map_err(|_| {
        RebornServicesError::validation(WebUiInboundValidationError::new(
            "cursor",
            WebUiInboundValidationCode::InvalidValue,
        ))
    })?;
    Ok(Some(cursor))
}

fn serialize_timeline_cursor(cursor: &TimelineCursor) -> Option<String> {
    // Serialization of a tiny tagged struct is total in practice, but
    // returning Option keeps the call site honest without an unwrap.
    serde_json::to_string(cursor).ok()
}

/// Slice the message transcript to the most recent `limit` messages
/// strictly older than `cursor.before_message_sequence` (or the most
/// recent `limit` overall when no cursor is supplied), returning the
/// page plus the cursor the caller should pass back to load the page
/// preceding this one. `None` for `next_cursor` means there is nothing
/// older — the caller has reached the start of the thread.
///
/// Messages are sorted by `sequence` ascending before slicing so the
/// returned page is in monotonic order regardless of the input order
/// the underlying store happens to produce.
fn paginate_timeline_messages(
    mut messages: Vec<ironclaw_threads::ThreadMessageRecord>,
    limit: usize,
    cursor: Option<TimelineCursor>,
) -> (Vec<ironclaw_threads::ThreadMessageRecord>, Option<String>) {
    messages.sort_by_key(|message| message.sequence);
    if let Some(cursor) = cursor.as_ref() {
        messages.retain(|message| message.sequence < cursor.before_message_sequence);
    }
    let total = messages.len();
    let start = total.saturating_sub(limit);
    let next_cursor = if start > 0 {
        // The next page is older than the oldest message in *this* page.
        // We take the sequence of the page's first (oldest) message and
        // use it as `before_message_sequence` for the follow-up: that
        // request returns messages with sequence < this one, i.e. the
        // page strictly preceding the current one.
        messages.get(start).and_then(|message| {
            serialize_timeline_cursor(&TimelineCursor {
                before_message_sequence: message.sequence,
            })
        })
    } else {
        None
    };
    let page: Vec<_> = messages.into_iter().skip(start).collect();
    (page, next_cursor)
}

fn cap_summary_artifacts(
    mut artifacts: Vec<ironclaw_threads::SummaryArtifact>,
) -> Vec<ironclaw_threads::SummaryArtifact> {
    if artifacts.len() > TIMELINE_MAX_SUMMARY_ARTIFACTS {
        artifacts.truncate(TIMELINE_MAX_SUMMARY_ARTIFACTS);
    }
    artifacts
}

fn webui_gate_binding_id(scope: &TurnScope, gate_ref: &str) -> String {
    format!(
        "{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment("thread", scope.thread_id.as_str()),
        segment("gate", gate_ref)
    )
}

fn webui_retry_binding_id(
    scope: &TurnScope,
    run_id: TurnRunId,
    client_action_id: &IdempotencyKey,
) -> String {
    format!(
        "{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment("thread", scope.thread_id.as_str()),
        segment("failed_run", run_id.as_uuid().to_string().as_str()),
        segment("action", client_action_id.as_str())
    )
}

fn gate_ref_string(gate_ref: &ironclaw_turns::GateRef) -> String {
    gate_ref.as_str().to_string()
}

fn persistent_approval_unavailable() -> RebornServicesError {
    RebornServicesError::from_status_kind(
        RebornServicesErrorCode::Unavailable,
        RebornServicesErrorKind::BlockedApproval,
        503,
        false,
    )
}

fn blocked_approval_unavailable() -> RebornServicesError {
    persistent_approval_unavailable()
}

fn blocked_authentication_unavailable() -> RebornServicesError {
    RebornServicesError::from_status_kind(
        RebornServicesErrorCode::Unavailable,
        RebornServicesErrorKind::BlockedAuthentication,
        503,
        false,
    )
}

fn segment(name: &str, value: &str) -> String {
    format!("{name}:{}:{value};", value.len())
}

fn map_timeline_probe_error(error: SessionThreadError) -> RebornServicesError {
    match error {
        SessionThreadError::Serialization(_)
        | SessionThreadError::Deserialization(_)
        | SessionThreadError::InvalidMessageTimestamp { .. }
        | SessionThreadError::Backend(_) => RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::TimelineUnavailable,
            503,
            true,
        ),
        _ => map_ownership_probe_error(error),
    }
}

fn map_thread_error(error: SessionThreadError) -> RebornServicesError {
    match error {
        SessionThreadError::UnknownThread { .. } | SessionThreadError::UnknownMessage { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        SessionThreadError::IdempotentReplayThreadMismatch { .. } => {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::Duplicate,
                409,
                false,
            )
        }
        SessionThreadError::ThreadScopeMismatch { .. }
        | SessionThreadError::IdempotentReplayActorMismatch { .. }
        | SessionThreadError::InvalidMessageTransition { .. }
        | SessionThreadError::MessageNotDraft { .. }
        | SessionThreadError::InvalidSummaryRange { .. }
        | SessionThreadError::OverlappingSummaryRange { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::Conflict, 409, false)
        }
        SessionThreadError::InvalidAttachment(_) => RebornServicesError::from_status_kind(
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
            400,
            false,
        ),
        SessionThreadError::GeneratedThreadId(_)
        | SessionThreadError::Serialization(_)
        | SessionThreadError::Deserialization(_)
        | SessionThreadError::InvalidMessageTimestamp { .. }
        | SessionThreadError::Backend(_) => RebornServicesError::service_unavailable(true),
    }
}

fn delete_thread_busy() -> RebornServicesError {
    RebornServicesError::from_status_kind(
        RebornServicesErrorCode::Conflict,
        RebornServicesErrorKind::Busy,
        409,
        false,
    )
}

fn map_turn_error(error: TurnError) -> RebornServicesError {
    if matches!(error, TurnError::RunNotRetryable { .. }) {
        return RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
            409,
            false,
        );
    }
    let (code, kind, status_code, retryable) = match error.category() {
        ironclaw_turns::TurnErrorCategory::ThreadBusy => (
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Busy,
            409,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::Conflict => (
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
            409,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::AdmissionRejected => (
            RebornServicesErrorCode::RateLimited,
            RebornServicesErrorKind::Busy,
            429,
            true,
        ),
        ironclaw_turns::TurnErrorCategory::CapacityExceeded => (
            RebornServicesErrorCode::RateLimited,
            RebornServicesErrorKind::Busy,
            429,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::ScopeNotFound => (
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorKind::NotFound,
            404,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::Unauthorized => (
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorKind::ParticipantDenied,
            403,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::InvalidRequest => (
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
            400,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::Unavailable => (
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            true,
        ),
    };
    RebornServicesError::from_status_kind(code, kind, status_code, retryable)
}

fn map_adapter_error(error: ProductAdapterError) -> RebornServicesError {
    match error {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => RebornServicesError::from_status_kind(
            code_for_status(status_code),
            kind_for_workflow_rejection(kind),
            status_code,
            retryable,
        ),
        ProductAdapterError::WorkflowTransient { .. }
        | ProductAdapterError::EgressTransient { .. } => {
            RebornServicesError::service_unavailable(true)
        }
        ProductAdapterError::Authentication(_) => {
            RebornServicesError::from_status(RebornServicesErrorCode::Unauthenticated, 401, false)
        }
        ProductAdapterError::MalformedInboundPayload { .. }
        | ProductAdapterError::InvalidIdentifier { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        ProductAdapterError::EgressDenied { .. }
        | ProductAdapterError::EgressUndeclaredHost { .. } => {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::BlockedResource,
                403,
                false,
            )
        }
        ProductAdapterError::Internal { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
        }
    }
}

fn map_auth_interaction_error(error: ProductWorkflowError) -> RebornServicesError {
    match error {
        ProductWorkflowError::AuthInteractionRejected { kind } => {
            RebornServicesError::from_status_kind(
                code_for_status(kind.status_code()),
                RebornServicesErrorKind::BlockedAuthentication,
                kind.status_code(),
                kind.retryable(),
            )
        }
        error => map_adapter_error(error.into()),
    }
}

fn map_projection_error(error: ProductAdapterError) -> RebornServicesError {
    match error {
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code,
            retryable,
            ..
        } => RebornServicesError::from_status_kind(
            code_for_status(status_code),
            RebornServicesErrorKind::ReplayUnavailable,
            status_code,
            retryable,
        ),
        ProductAdapterError::WorkflowTransient { .. }
        | ProductAdapterError::EgressTransient { .. } => RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ReplayUnavailable,
            503,
            true,
        ),
        _ => map_adapter_error(error),
    }
}

fn code_for_status(status_code: u16) -> RebornServicesErrorCode {
    match status_code {
        400 => RebornServicesErrorCode::InvalidRequest,
        401 => RebornServicesErrorCode::Unauthenticated,
        403 => RebornServicesErrorCode::Forbidden,
        404 => RebornServicesErrorCode::NotFound,
        409 => RebornServicesErrorCode::Conflict,
        429 => RebornServicesErrorCode::RateLimited,
        503 => RebornServicesErrorCode::Unavailable,
        _ => RebornServicesErrorCode::Internal,
    }
}

fn kind_for_workflow_rejection(kind: ProductWorkflowRejectionKind) -> RebornServicesErrorKind {
    match kind {
        ProductWorkflowRejectionKind::ThreadBusy
        | ProductWorkflowRejectionKind::AdmissionRejected => RebornServicesErrorKind::Busy,
        ProductWorkflowRejectionKind::ScopeNotFound => RebornServicesErrorKind::NotFound,
        ProductWorkflowRejectionKind::Unauthorized => RebornServicesErrorKind::ParticipantDenied,
        ProductWorkflowRejectionKind::InvalidRequest => RebornServicesErrorKind::Validation,
        ProductWorkflowRejectionKind::Unavailable => RebornServicesErrorKind::ServiceUnavailable,
        ProductWorkflowRejectionKind::Conflict | ProductWorkflowRejectionKind::Ambiguous => {
            RebornServicesErrorKind::Conflict
        }
    }
}

fn create_thread_metadata_json(
    client_action_id: &ironclaw_turns::IdempotencyKey,
) -> Result<String, RebornServicesError> {
    serde_json::to_string(&serde_json::json!({
        "client_action_id": client_action_id.as_str(),
    }))
    .map_err(|_| RebornServicesError::internal_invariant())
}

fn validate_log_query_modes(tail: bool, follow: bool) -> Result<(), RebornServicesError> {
    if tail && follow {
        return Err(RebornServicesError::validation(
            WebUiInboundValidationError::new("follow", WebUiInboundValidationCode::InvalidValue),
        ));
    }
    Ok(())
}

fn bounded_operator_logs_query(query: RebornOperatorLogsQuery) -> RebornLogQueryRequest {
    bounded_log_query(RebornLogQueryRequest {
        limit: query.limit,
        cursor: query.cursor,
        level: query.level,
        target: query.target,
        thread_id: query.thread_id,
        run_id: query.run_id,
        turn_id: query.turn_id,
        tool_call_id: query.tool_call_id,
        tool_name: query.tool_name,
        source: query.source,
        tail: query.tail,
        follow: query.follow,
    })
}

fn bounded_log_query(query: RebornLogQueryRequest) -> RebornLogQueryRequest {
    RebornLogQueryRequest {
        limit: query
            .limit
            .map(|limit| limit.clamp(1, OPERATOR_LOGS_MAX_LIMIT))
            .or(Some(OPERATOR_LOGS_DEFAULT_LIMIT)),
        cursor: bounded_operator_logs_string(query.cursor, OPERATOR_LOGS_CURSOR_MAX_BYTES),
        level: query.level,
        target: bounded_operator_logs_string(query.target, OPERATOR_LOGS_TARGET_MAX_BYTES),
        thread_id: bounded_operator_logs_context_string(query.thread_id),
        run_id: bounded_operator_logs_context_string(query.run_id),
        turn_id: bounded_operator_logs_context_string(query.turn_id),
        tool_call_id: bounded_operator_logs_context_string(query.tool_call_id),
        tool_name: bounded_operator_logs_context_string(query.tool_name),
        source: bounded_operator_logs_context_string(query.source),
        tail: query.tail,
        follow: query.follow,
    }
}

fn bounded_operator_logs_string(value: Option<String>, max_bytes: usize) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.len() <= max_bytes {
            Some(trimmed.to_string())
        } else {
            Some(truncate_utf8_to_bytes(trimmed, max_bytes))
        }
    })
}

fn bounded_operator_logs_context_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(normalize_operator_log_context_value(trimmed))
        }
    })
}

pub fn normalize_operator_log_context_value(value: &str) -> String {
    truncate_utf8_with_suffix(value, OPERATOR_LOGS_CONTEXT_MAX_BYTES)
}

fn truncate_utf8_to_bytes(value: &str, max_bytes: usize) -> String {
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn truncate_utf8_with_suffix(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    if max_bytes <= OPERATOR_LOG_CONTEXT_TRUNCATED_SUFFIX.len() {
        return OPERATOR_LOG_CONTEXT_TRUNCATED_SUFFIX[..max_bytes].to_string();
    }

    let mut end = max_bytes - OPERATOR_LOG_CONTEXT_TRUNCATED_SUFFIX.len();
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    let mut truncated = String::with_capacity(max_bytes);
    truncated.push_str(&value[..end]);
    truncated.push_str(OPERATOR_LOG_CONTEXT_TRUNCATED_SUFFIX);
    truncated
}

fn product_agent_bound_caller_from_webui(
    caller: WebUiAuthenticatedCaller,
) -> Option<ProductAgentBoundCaller> {
    let agent_id = caller.agent_id?;
    Some(ProductAgentBoundCaller::new(
        caller.tenant_id,
        caller.user_id,
        agent_id,
        caller.project_id,
    ))
}

fn generated_thread_id(
    caller: &WebUiAuthenticatedCaller,
    client_action_id: &ironclaw_turns::IdempotencyKey,
) -> ThreadId {
    let seed = format!(
        "{}{}{}{}{}{}",
        segment("surface", "webui-create-thread"),
        segment("tenant", caller.tenant_id.as_str()),
        segment("user", caller.user_id.as_str()),
        segment(
            "agent",
            caller.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment(
            "project",
            caller
                .project_id
                .as_ref()
                .map(ironclaw_host_api::ProjectId::as_str)
                .unwrap_or("")
        ),
        segment("action", client_action_id.as_str())
    );
    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes());
    // UUID text contains no path separators/control characters and is accepted by ThreadId.
    match ThreadId::new(id.to_string()) {
        Ok(thread_id) => thread_id,
        Err(error) => {
            debug_assert!(false, "generated UUID thread id should be valid: {error}");
            // Fallback remains valid under ThreadId validation rules.
            ThreadId::new("generated-thread-fallback").unwrap_or_else(|_| unreachable!())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WebUI settings/tools request enum must use the exact wire strings
    /// the operator-config storage parser accepts and the entry writer emits.
    /// This pins the type link so the request vocabulary cannot drift from
    /// the approvals-owned resolved-state vocabulary (audit 2026-07, 6a).
    #[test]
    fn settings_tool_permission_state_wire_strings_stay_linked() {
        let cases = [
            (SettingsToolPermissionState::Default, "default", None),
            (
                SettingsToolPermissionState::AlwaysAllow,
                "always_allow",
                Some(ToolPermissionState::AlwaysAllow),
            ),
            (
                SettingsToolPermissionState::AskEachTime,
                "ask_each_time",
                Some(ToolPermissionState::AskEachTime),
            ),
            (
                SettingsToolPermissionState::Disabled,
                "disabled",
                Some(ToolPermissionState::Disabled),
            ),
        ];
        for (state, wire, resolved) in cases {
            let serialized = serde_json::to_value(state).unwrap();
            assert_eq!(serialized, serde_json::Value::String(wire.to_string()));
            assert_eq!(
                serde_json::from_value::<SettingsToolPermissionState>(serialized).unwrap(),
                state
            );
            // Round-trips through the storage parser the facade applies on set.
            let update =
                parse_tool_permission_state(&serde_json::json!({ "state": wire })).unwrap();
            match (update, resolved) {
                (ToolPermissionUpdate::Default, None) => {}
                (ToolPermissionUpdate::State(parsed), Some(expected)) => {
                    assert_eq!(parsed, expected);
                    // The resolved states serialize to the same strings the
                    // config entry writer emits.
                    assert_eq!(tool_permission_state_wire(expected), wire);
                }
                _ => panic!("wire string {wire} no longer parses to the expected update"),
            }
        }
    }

    /// Every `ProjectServiceError` variant projects to a sanitized facade error
    /// with the expected coarse code/status, and `InvalidInput`'s field name is
    /// carried through (it is a controlled constant, never backend text).
    #[test]
    fn project_service_error_maps_to_sanitized_facade_error() {
        let not_found = map_project_service_error(ProjectServiceError::NotFound);
        assert_eq!(not_found.code, RebornServicesErrorCode::NotFound);
        assert_eq!(not_found.status_code, 404);

        let denied = map_project_service_error(ProjectServiceError::Denied);
        assert_eq!(denied.kind, RebornServicesErrorKind::ParticipantDenied);
        assert_eq!(denied.status_code, 403);

        let invalid = map_project_service_error(ProjectServiceError::InvalidInput {
            field: "project_id".to_string(),
        });
        assert_eq!(invalid.code, RebornServicesErrorCode::InvalidRequest);
        assert_eq!(invalid.status_code, 400);
        assert_eq!(invalid.field.as_deref(), Some("project_id"));

        let conflict = map_project_service_error(ProjectServiceError::Conflict);
        assert_eq!(conflict.code, RebornServicesErrorCode::Conflict);
        assert_eq!(conflict.status_code, 409);

        let unavailable = map_project_service_error(ProjectServiceError::Unavailable);
        assert_eq!(unavailable.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(unavailable.status_code, 503);
        assert!(unavailable.retryable, "unavailable is retryable");

        let internal = map_project_service_error(ProjectServiceError::Internal);
        assert_eq!(internal.code, RebornServicesErrorCode::Internal);
        assert_eq!(internal.status_code, 500);
    }

    /// `require_project_service` returns `service_unavailable(false)` when no
    /// project service is wired (see the helper in this file). This locks the
    /// full shape of that sentinel — a clean, non-retryable 503 — so an unwired
    /// runtime returns a stable error rather than a panic or a 500.
    #[test]
    fn unwired_project_service_sentinel_is_503() {
        let unavailable = RebornServicesError::service_unavailable(false);
        assert_eq!(unavailable.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(unavailable.status_code, 503);
        assert!(
            !unavailable.retryable,
            "false-arg sentinel is non-retryable"
        );
    }
}
