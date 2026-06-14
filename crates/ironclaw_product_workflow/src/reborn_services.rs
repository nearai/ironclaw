//! WebUI-facing Reborn service facade.
//!
//! This module is the stable high-level API beta WebUI route handlers use
//! instead of reaching into turn coordination, thread stores, runtime lanes, DB
//! stores, dispatchers, or capability hosts directly.

use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex as StdMutex, Weak},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_attachments::InboundAttachment;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, CredentialAccountId, CredentialAccountProjection,
    CredentialAccountUpdateBinding, ProviderScope,
};
use ironclaw_host_api::{AgentId, ExtensionId, ProjectId, TenantId, ThreadId, UserId};
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
    ResumeTurnRequest, SanitizedCancelReason, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCoordinator, TurnError, TurnRunId, TurnScope, TurnStatus,
};
use secrecy::SecretString;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use url::Url;
use uuid::Uuid;

use crate::{
    ApprovalInteractionDecision, ApprovalInteractionService, AuthInteractionDecision,
    AuthInteractionRejectionKind, AuthInteractionService, LifecyclePackageRef,
    LifecycleProductFacade, ProductWorkflowError, ResolveApprovalInteractionRequest,
    ResolveApprovalInteractionResponse, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse, UnsupportedLifecycleProductFacade, WebUiAuthenticatedCaller,
    WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiGateResolution, WebUiInboundCommand,
    WebUiInboundValidationCode, WebUiInboundValidationError, WebUiListAutomationsRequest,
    WebUiListThreadsRequest, WebUiResolveGateRequest, WebUiSendMessageRequest,
    WebUiSetupExtensionRequest,
    approval_interaction::RejectingApprovalInteractionService,
    auth_interaction::RejectingAuthInteractionService,
    binding_ref::{
        DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_reply_target_binding_ref,
        bounded_source_binding_ref,
    },
    is_approval_gate_ref, is_auth_gate_ref, thread_metadata_is_automation_trigger,
};

mod error;
mod extension_credentials;
mod extension_onboarding;
mod extension_setup_credentials;
mod extensions;
mod lifecycle_setup;
mod llm_config;
mod types;

pub use error::{RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind};
pub use llm_config::{
    CodexLoginStart, LlmActiveSelection, LlmConfigService, LlmConfigServiceError,
    LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest, LlmProbeResult, LlmProviderView,
    NearAiAuthProvider, NearAiLoginRequest, NearAiLoginStart, NearAiWalletLoginRequest,
    NearAiWalletLoginResult, SetActiveLlmRequest, UpsertLlmProviderRequest,
};
pub use types::{
    RebornAutomationInfo, RebornAutomationRecentRunInfo, RebornAutomationRecentRunStatus,
    RebornAutomationRunStatus, RebornAutomationSource, RebornAutomationState,
    RebornCancelRunResponse, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, RebornConnectableChannelListResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionCredentialSetup, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionOnboardingPayload, RebornExtensionOnboardingState, RebornExtensionRegistryEntry,
    RebornExtensionRegistryResponse, RebornExtensionSetupField, RebornExtensionSetupSecret,
    RebornGetRunStateRequest, RebornGetRunStateResponse, RebornListAutomationsResponse,
    RebornListThreadsResponse, RebornLogEntry, RebornLogLevel, RebornLogQueryRequest,
    RebornLogQueryResponse, RebornOperatorArea, RebornOperatorCommandPlaneResponse,
    RebornOperatorConfigDiagnostic, RebornOperatorConfigDiagnosticSeverity,
    RebornOperatorConfigEntry, RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetRequest, RebornOperatorConfigValidateRequest,
    RebornOperatorConfigValidateResponse, RebornOperatorLogsQuery,
    RebornOperatorServiceLifecycleAction, RebornOperatorServiceLifecycleRequest,
    RebornOperatorSetupRequest, RebornOperatorSetupResponse, RebornOperatorSetupStatus,
    RebornOperatorSetupStep, RebornOperatorSetupStepStatus, RebornOperatorStatusCheck,
    RebornOperatorStatusResponse, RebornOperatorStatusSeverity, RebornOperatorStatusState,
    RebornOperatorSurfaceStatus, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetChannel,
    RebornOutboundDeliveryTargetDescription, RebornOutboundDeliveryTargetDisplayName,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
    RebornResolveGateResponse, RebornResumeGateResponse, RebornServiceLifecycleAction,
    RebornServiceLifecycleRequest, RebornServiceLifecycleResponse, RebornServiceLifecycleState,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillInfo, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornSkillSourceKind, RebornSkillTrustLevel,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse,
};

type SkillActivationRecorder =
    dyn Fn(&TurnScope, &AcceptedMessageRef, &str) -> Result<(), RebornServicesError> + Send + Sync;
type SkillActivationClearer =
    dyn Fn(&TurnScope, &AcceptedMessageRef) -> Result<(), RebornServicesError> + Send + Sync;
type ThreadOperationLocks = StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>;

const OPERATOR_LOGS_DEFAULT_LIMIT: u32 = 100;
const OPERATOR_LOGS_MAX_LIMIT: u32 = 500;
const OPERATOR_LOGS_CURSOR_MAX_BYTES: usize = 512;
const OPERATOR_LOGS_TARGET_MAX_BYTES: usize = 256;
const OPERATOR_LOGS_CONTEXT_MAX_BYTES: usize = 256;
const OPERATOR_LOG_CONTEXT_TRUNCATED_SUFFIX: &str = " ... [truncated]";

#[async_trait]
pub trait ConnectableChannelsProductFacade: Send + Sync {
    async fn list_connectable_channels(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError>;
}

#[derive(Debug, Clone, Default)]
pub struct StaticConnectableChannelsProductFacade {
    channels: Arc<[RebornConnectableChannelInfo]>,
}

impl StaticConnectableChannelsProductFacade {
    pub fn new(channels: impl Into<Vec<RebornConnectableChannelInfo>>) -> Self {
        Self {
            channels: Arc::from(channels.into()),
        }
    }
}

#[async_trait]
impl ConnectableChannelsProductFacade for StaticConnectableChannelsProductFacade {
    async fn list_connectable_channels(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        Ok(RebornConnectableChannelListResponse {
            channels: self.channels.iter().cloned().collect(),
        })
    }
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

    async fn install_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
        content: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        let _ = (caller, name, content);
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

    async fn update_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
        content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        let _ = (caller, name, content);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn remove_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
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

#[async_trait]
pub trait AutomationProductFacade: Send + Sync {
    async fn list_automations(
        &self,
        caller: ProductAgentBoundCaller,
        request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError>;

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

fn operator_setup_info_diagnostic(
    key: &str,
    reason_code: &str,
    message: &str,
    remediation: &str,
) -> RebornOperatorConfigDiagnostic {
    operator_setup_diagnostic(
        key,
        RebornOperatorConfigDiagnosticSeverity::Info,
        reason_code,
        message,
        remediation,
    )
}

fn operator_setup_validation_error(field: &str) -> RebornServicesError {
    WebUiInboundValidationError {
        field: field.to_string(),
        code: WebUiInboundValidationCode::InvalidValue,
    }
    .into()
}

fn setup_response_from_llm_snapshot(
    snapshot: LlmConfigSnapshot,
    mut diagnostics: Vec<RebornOperatorConfigDiagnostic>,
) -> RebornOperatorSetupResponse {
    diagnostics.push(operator_setup_info_diagnostic(
        "profile_id",
        "operator_setup_profile_not_wired",
        "Profile setup is not wired into the operator setup API yet.",
        "Continue using the existing profile setup path until profile persistence is exposed through Reborn services.",
    ));
    diagnostics.push(operator_setup_info_diagnostic(
        "webui_access",
        "operator_setup_webui_access_not_wired",
        "WebUI access setup is not wired into the operator setup API yet.",
        "Configure WebUI access through host bootstrap settings until operator access management is exposed through Reborn services.",
    ));

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
                status: RebornOperatorSetupStepStatus::Unsupported,
                message: "Profile setup is not wired into this API yet.".to_string(),
            },
            RebornOperatorSetupStep {
                name: "webui_access".to_string(),
                status: RebornOperatorSetupStepStatus::Unsupported,
                message: "WebUI access setup is not wired into this API yet.".to_string(),
            },
        ],
        diagnostics,
    }
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
    let parsed = Url::parse(trimmed).map_err(|_| operator_setup_validation_error("base_url"))?;
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

fn operator_config_diagnostic_command_plane_response(
    area: RebornOperatorArea,
) -> RebornOperatorCommandPlaneResponse {
    RebornOperatorCommandPlaneResponse {
        area,
        status: RebornOperatorSurfaceStatus::Unavailable,
        message: "Operator config has unsupported or not-yet-wired settings.".to_string(),
        operator_status: None,
        logs: None,
        service_lifecycle: None,
        diagnostics: vec![operator_config_surface_not_wired_diagnostic()],
    }
}

#[async_trait]
pub trait RebornServicesApi: Send + Sync {
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError>;

    async fn submit_turn(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError>;

    async fn delete_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError>;

    async fn get_timeline(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError>;

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError>;

    async fn cancel_run(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError>;

    async fn resolve_gate(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError>;

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError>;

    /// List the caller-scoped threads. Pagination is opaque: callers
    /// echo back the `next_cursor` from a prior response to retrieve
    /// the next page; the cursor encoding is implementation-defined.
    ///
    /// Returns an empty list + `next_cursor: None` when no threads
    /// exist for the caller's scope.
    async fn list_threads(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError>;

    async fn list_automations(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiListAutomationsRequest,
    ) -> Result<RebornListAutomationsResponse, RebornServicesError>;

    async fn list_connectable_channels(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        Ok(RebornConnectableChannelListResponse {
            channels: Vec::new(),
        })
    }

    /// Return the authenticated caller's scoped outbound preferences.
    ///
    /// Implementations must scope stored preferences by the caller's
    /// tenant/user identity. Unsupported behavior belongs in
    /// `UnsupportedOutboundPreferencesProductFacade`, not in trait defaults.
    async fn get_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError>;

    /// Persist the authenticated caller's outbound delivery preference.
    ///
    /// Implementations must scope mutations by the caller's tenant/user
    /// identity and fail closed when no writable outbound-preferences facade is
    /// wired.
    async fn set_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError>;

    /// List delivery targets available to the authenticated caller.
    ///
    /// Implementations must scope target inventory by the caller's tenant/user
    /// identity and fail closed when no outbound target inventory facade is
    /// wired.
    async fn list_outbound_delivery_targets(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError>;

    async fn list_extensions(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionListResponse, RebornServicesError>;

    async fn list_skills(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError>;

    async fn search_skills(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError>;

    async fn install_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
        content: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError>;

    async fn read_skill_content(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError>;

    async fn update_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
        content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError>;

    async fn remove_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError>;

    async fn list_extension_registry(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionRegistryResponse, RebornServicesError>;

    async fn install_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError>;

    async fn activate_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError>;

    async fn remove_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError>;

    /// Run a step in a v2-native extension onboarding flow. Today the
    /// facade returns
    /// [`RebornSetupExtensionStatus::NotImplemented`](types::RebornSetupExtensionStatus::NotImplemented)
    /// because the underlying extension lifecycle is still v1-only.
    /// The route exists so the WebUI v2 entrypoint inventory is
    /// complete and so future onboarding port work has a fixed surface
    /// to fill in.
    ///
    /// `package_ref` is the validated lifecycle package identity from
    /// the route path or request body. The browser can still render
    /// display names from registry metadata, but lifecycle side effects
    /// use package refs end to end.
    async fn setup_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
        request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError>;

    /// LLM provider configuration: merged catalog + active selection.
    ///
    /// The six LLM-config methods default to "service unavailable" so facade
    /// impls (and test fakes) that don't wire an [`LlmConfigService`] inherit a
    /// safe surface; the default `RebornServices` overrides them all.
    async fn get_llm_config(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let _ = caller;
        Err(llm_config::llm_config_unavailable())
    }

    /// Add or update a custom LLM provider (and optionally its key / active state).
    async fn upsert_llm_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let _ = (caller, request);
        Err(llm_config::llm_config_unavailable())
    }

    /// Remove a custom LLM provider and any stored key for it.
    async fn delete_llm_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let _ = (caller, provider_id);
        Err(llm_config::llm_config_unavailable())
    }

    /// Select the active LLM provider + model.
    async fn set_active_llm(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let _ = (caller, request);
        Err(llm_config::llm_config_unavailable())
    }

    /// Probe an LLM provider's credentials/endpoint without persisting.
    async fn test_llm_connection(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, RebornServicesError> {
        let _ = (caller, request);
        Err(llm_config::llm_config_unavailable())
    }

    /// List the models an LLM provider exposes, without persisting.
    async fn list_llm_models(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, RebornServicesError> {
        let _ = (caller, request);
        Err(llm_config::llm_config_unavailable())
    }

    /// Begin a NEAR AI browser login; returns the authorization URL to open.
    async fn start_nearai_login(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: NearAiLoginRequest,
    ) -> Result<NearAiLoginStart, RebornServicesError> {
        let _ = (caller, request);
        Err(llm_config::llm_config_unavailable())
    }

    /// Complete a NEAR AI wallet (NEP-413) login from a browser-signed message.
    async fn complete_nearai_wallet_login(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: NearAiWalletLoginRequest,
    ) -> Result<NearAiWalletLoginResult, RebornServicesError> {
        let _ = (caller, request);
        Err(llm_config::llm_config_unavailable())
    }

    /// Begin an OpenAI Codex device-code login; returns the user code + URL.
    async fn start_codex_login(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<CodexLoginStart, RebornServicesError> {
        let _ = caller;
        Err(llm_config::llm_config_unavailable())
    }

    async fn get_operator_setup(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        let _ = caller;
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn run_operator_setup(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorSetupRequest,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        let _ = (caller, request);
        Err(RebornServicesError::service_unavailable(false))
    }

    /// Return the effective operator config projection.
    ///
    /// Until the effective config backend is wired, read-only operator config/status/diagnostic
    /// surfaces intentionally return typed diagnostic payloads so the browser can explain
    /// what is unsupported. Mutating or side-effecting operator routes remain fail-closed
    /// with sanitized service-unavailable errors until their owning service is wired.
    async fn list_operator_config(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorConfigListResponse, RebornServicesError> {
        let _ = caller;
        Ok(RebornOperatorConfigListResponse {
            entries: Vec::new(),
            precedence: Vec::new(),
            diagnostics: vec![operator_config_surface_not_wired_diagnostic()],
        })
    }

    async fn get_operator_config_key(
        &self,
        caller: WebUiAuthenticatedCaller,
        key: String,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        let _ = (caller, key);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn set_operator_config_key(
        &self,
        caller: WebUiAuthenticatedCaller,
        key: String,
        request: RebornOperatorConfigSetRequest,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        let _ = (caller, key, request);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn validate_operator_config(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorConfigValidateRequest,
    ) -> Result<RebornOperatorConfigValidateResponse, RebornServicesError> {
        let _ = caller;
        let diagnostics = operator_config_validation_diagnostics(request.keys);
        Ok(RebornOperatorConfigValidateResponse {
            valid: diagnostics.is_empty(),
            diagnostics,
        })
    }

    async fn get_operator_diagnostics(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let _ = caller;
        Ok(operator_config_diagnostic_command_plane_response(
            RebornOperatorArea::Diagnostics,
        ))
    }

    async fn get_operator_status(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let _ = caller;
        Ok(operator_config_diagnostic_command_plane_response(
            RebornOperatorArea::Status,
        ))
    }

    async fn query_operator_logs(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornOperatorLogsQuery,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let _ = (caller, query);
        Err(RebornServicesError::service_unavailable(false))
    }

    async fn run_operator_service_lifecycle(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorServiceLifecycleRequest,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let _ = (caller, request);
        Err(RebornServicesError::service_unavailable(false))
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

/// Default facade implementation composed at the WebUI boundary.
#[derive(Clone)]
pub struct RebornServices {
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    inbound_attachments: Option<Arc<dyn InboundAttachmentLander>>,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    lifecycle_facade: Arc<dyn LifecycleProductFacade>,
    automation_facade: Arc<dyn AutomationProductFacade>,
    skills_facade: Arc<dyn SkillsProductFacade>,
    connectable_channels_facade: Arc<dyn ConnectableChannelsProductFacade>,
    outbound_preferences_facade: Arc<dyn OutboundPreferencesProductFacade>,
    operator_status: Arc<dyn OperatorStatusService>,
    operator_logs: Arc<dyn OperatorLogsService>,
    operator_service_lifecycle: Arc<dyn OperatorServiceLifecycleService>,
    approval_interactions: Arc<dyn ApprovalInteractionService>,
    auth_interactions: Arc<dyn AuthInteractionService>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    skill_activation_recorder: Option<Arc<SkillActivationRecorder>>,
    skill_activation_clearer: Option<Arc<SkillActivationClearer>>,
    llm_config: Option<Arc<dyn LlmConfigService>>,
    thread_operation_locks: Arc<ThreadOperationLocks>,
}

impl RebornServices {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            thread_service,
            turn_coordinator,
            inbound_attachments: None,
            event_stream: None,
            lifecycle_facade: Arc::new(UnsupportedLifecycleProductFacade::new_static(
                "reborn_lifecycle_facade_unwired",
            )),
            automation_facade: Arc::new(UnsupportedAutomationProductFacade::new_static()),
            skills_facade: Arc::new(UnsupportedSkillsProductFacade::new_static()),
            connectable_channels_facade: Arc::new(StaticConnectableChannelsProductFacade::default()),
            outbound_preferences_facade: Arc::new(
                UnsupportedOutboundPreferencesProductFacade::new_static(),
            ),
            operator_status: Arc::new(UnsupportedOperatorStatusService),
            operator_logs: Arc::new(UnsupportedOperatorLogsService),
            operator_service_lifecycle: Arc::new(UnsupportedOperatorServiceLifecycleService),
            approval_interactions: Arc::new(RejectingApprovalInteractionService),
            auth_interactions: Arc::new(RejectingAuthInteractionService),
            extension_credentials: None,
            skill_activation_recorder: None,
            skill_activation_clearer: None,
            llm_config: None,
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

    pub fn with_llm_config_service(mut self, llm_config: Arc<dyn LlmConfigService>) -> Self {
        self.llm_config = Some(llm_config);
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

    pub fn with_connectable_channels_facade(
        mut self,
        connectable_channels_facade: Arc<dyn ConnectableChannelsProductFacade>,
    ) -> Self {
        self.connectable_channels_facade = connectable_channels_facade;
        self
    }

    pub fn with_outbound_preferences_facade(
        mut self,
        outbound_preferences_facade: Arc<dyn OutboundPreferencesProductFacade>,
    ) -> Self {
        self.outbound_preferences_facade = outbound_preferences_facade;
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
}

#[async_trait]
impl RebornServicesApi for RebornServices {
    async fn get_operator_setup(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        let Some(llm_config) = &self.llm_config else {
            return Err(llm_config::llm_config_unavailable());
        };
        let snapshot = llm_config
            .snapshot(caller)
            .await
            .map_err(llm_config::map_llm_config_error)?;
        Ok(setup_response_from_llm_snapshot(snapshot, Vec::new()))
    }

    async fn run_operator_setup(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornOperatorSetupRequest,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        let Some(llm_config) = &self.llm_config else {
            return Err(llm_config::llm_config_unavailable());
        };

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

        let snapshot = match (request.provider_id, request.adapter) {
            (Some(provider_id), Some(adapter)) => llm_config
                .upsert_provider(
                    caller,
                    UpsertLlmProviderRequest {
                        id: provider_id,
                        name: None,
                        adapter,
                        base_url: request.base_url,
                        default_model: request.model.clone(),
                        api_key: request.api_key,
                        set_active: true,
                        model: request.model,
                    },
                )
                .await
                .map_err(llm_config::map_llm_config_error)?,
            (Some(provider_id), None) => llm_config
                .set_active(
                    caller,
                    SetActiveLlmRequest {
                        provider_id,
                        model: request.model,
                    },
                )
                .await
                .map_err(llm_config::map_llm_config_error)?,
            (None, _) => llm_config
                .snapshot(caller)
                .await
                .map_err(llm_config::map_llm_config_error)?,
        };

        Ok(setup_response_from_llm_snapshot(snapshot, Vec::new()))
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
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
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

    async fn submit_turn(
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
                self.clear_skill_activation_message(&scope, &accepted_message_ref)?;
                mark_message_deferred_busy_or_replay(
                    &*self.thread_service,
                    &thread_scope,
                    &handoff,
                    &client_action_id,
                )
                .await?;

                Ok(RebornSubmitTurnResponse::DeferredBusy {
                    thread_id: handoff.thread_id,
                    accepted_message_ref,
                    active_run_id: busy.active_run_id,
                    status: busy.status,
                    event_cursor: busy.event_cursor,
                })
            }
            Err(error) => {
                self.clear_skill_activation_message(&scope, &accepted_message_ref)?;
                Err(map_turn_error(error))
            }
        }
    }

    async fn delete_thread(
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

    async fn get_timeline(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let actor = caller.actor();
        let limit = clamp_timeline_limit(request.limit);
        let cursor = parse_timeline_cursor(request.cursor.as_deref())?;
        let scope = caller.turn_scope(thread_id);
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(actor.user_id.clone()))?;
        let history = match self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: scope.thread_id.clone(),
            })
            .await
        {
            Ok(history) => history,
            // When the session-scoped lookup fails with NotFound, try the
            // automation-trigger fallback: automation-trigger threads are
            // created under the trigger creator's scope, not the WebUI
            // caller's session scope, so the user-scoped lookup always misses
            // them. If the thread_id appears in any recent run of an
            // automation that belongs to this caller, we know the caller is
            // authorized (list_automations applies the same authorization).
            // We then re-fetch using the trigger-owned thread scope. Both
            // UnknownThread and ThreadScopeMismatch are treated as "not found
            // in my scope" — only those are eligible for the automation
            // fallback; backend/serialization errors propagate as-is.
            Err(
                SessionThreadError::UnknownThread { .. }
                | SessionThreadError::ThreadScopeMismatch { .. },
            ) => {
                // The primary user-scoped lookup missed. Try the automation-
                // trigger fallback; if it does not authorize the access it
                // returns the canonical NotFound error.
                let original_error =
                    RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false);
                self.try_automation_trigger_timeline_fallback(caller, &scope, original_error)
                    .await?
            }
            Err(err) => return Err(map_timeline_probe_error(err)),
        };

        let (messages, next_cursor) = paginate_timeline_messages(history.messages, limit, cursor);
        let summary_artifacts = cap_summary_artifacts(history.summary_artifacts);

        Ok(RebornTimelineResponse {
            thread: history.thread,
            messages,
            summary_artifacts,
            next_cursor,
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

    async fn cancel_run(
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

    async fn resolve_gate(
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
        Ok(state.into())
    }

    async fn list_threads(
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
        self.list_visible_threads_for_scope(scope, request).await
    }

    async fn list_automations(
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
        let automations = self
            .automation_facade
            .list_automations(caller, AutomationListRequest { limit, run_limit })
            .await?;
        Ok(RebornListAutomationsResponse { automations })
    }

    async fn list_connectable_channels(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        self.connectable_channels_facade
            .list_connectable_channels(caller)
            .await
    }

    async fn get_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        self.outbound_preferences_facade
            .get_outbound_preferences(caller)
            .await
    }

    async fn set_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        self.outbound_preferences_facade
            .set_outbound_preferences(caller, request)
            .await
    }

    async fn list_outbound_delivery_targets(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        self.outbound_preferences_facade
            .list_outbound_delivery_targets(caller)
            .await
    }

    async fn list_extensions(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionListResponse, RebornServicesError> {
        extensions::list_extensions(
            Arc::clone(&self.lifecycle_facade),
            self.extension_credentials.clone(),
            caller,
        )
        .await
    }

    async fn list_skills(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        self.skills_facade.list_skills(caller).await
    }

    async fn search_skills(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        self.skills_facade.search_skills(caller, query).await
    }

    async fn install_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
        content: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        self.skills_facade
            .install_skill(caller, name, content)
            .await
    }

    async fn read_skill_content(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        self.skills_facade.read_skill_content(caller, name).await
    }

    async fn update_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
        content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        self.skills_facade.update_skill(caller, name, content).await
    }

    async fn remove_skill(
        &self,
        caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        self.skills_facade.remove_skill(caller, name).await
    }

    async fn list_extension_registry(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
        extensions::list_extension_registry(self.lifecycle_facade.as_ref(), caller).await
    }

    async fn install_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        extensions::install_extension(self.lifecycle_facade.as_ref(), caller, package_ref).await
    }

    async fn activate_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        extensions::activate_extension(self.lifecycle_facade.as_ref(), caller, package_ref).await
    }

    async fn remove_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        extensions::remove_extension(self.lifecycle_facade.as_ref(), caller, package_ref).await
    }

    async fn setup_extension(
        &self,
        caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
        request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        lifecycle_setup::setup_extension(
            self.lifecycle_facade.as_ref(),
            self.extension_credentials.as_deref(),
            caller,
            package_ref,
            request,
        )
        .await
    }

    async fn get_operator_status(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let status = self.operator_status.status(caller).await?;
        Ok(RebornOperatorCommandPlaneResponse {
            area: RebornOperatorArea::Status,
            status: RebornOperatorSurfaceStatus::Available,
            message: "operator status is available".to_string(),
            operator_status: Some(status),
            logs: None,
            service_lifecycle: None,
            diagnostics: Vec::new(),
        })
    }

    async fn query_operator_logs(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornOperatorLogsQuery,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        let request = bounded_operator_logs_query(query);
        let logs = self.operator_logs.query_logs(caller, request).await?;
        Ok(RebornOperatorCommandPlaneResponse {
            area: RebornOperatorArea::Logs,
            status: RebornOperatorSurfaceStatus::Available,
            message: "operator logs query completed".to_string(),
            operator_status: None,
            logs: Some(logs),
            service_lifecycle: None,
            diagnostics: Vec::new(),
        })
    }

    async fn run_operator_service_lifecycle(
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

    async fn get_llm_config(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        service
            .snapshot(caller)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    async fn upsert_llm_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        validate_llm_base_url(request.base_url.as_deref())?;
        service
            .upsert_provider(caller, request)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    async fn delete_llm_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        service
            .delete_provider(caller, provider_id)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    async fn set_active_llm(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        let service = self
            .llm_config
            .as_ref()
            .ok_or_else(llm_config::llm_config_unavailable)?;
        service
            .set_active(caller, request)
            .await
            .map_err(llm_config::map_llm_config_error)
    }

    async fn test_llm_connection(
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

    async fn list_llm_models(
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

    async fn start_nearai_login(
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

    async fn start_codex_login(
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

    async fn complete_nearai_wallet_login(
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

impl RebornServices {
    async fn list_visible_threads_for_scope(
        &self,
        scope: ThreadScope,
        request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        let visible_limit = clamp_thread_list_limit(request.limit);
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
            visible_threads.extend(
                response
                    .threads
                    .into_iter()
                    .filter(|thread| !is_automation_trigger_thread(thread)),
            );
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

async fn mark_message_deferred_busy_or_replay(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
) -> Result<(), RebornServicesError> {
    match thread_service
        .mark_message_deferred_busy(thread_scope, &handoff.thread_id, handoff.message_id)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            reconcile_terminal_duplicate(
                thread_service,
                thread_scope,
                handoff,
                client_action_id,
                |replay| replay.status == MessageStatus::DeferredBusy,
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
impl RebornServices {
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
    async fn try_automation_trigger_timeline_fallback(
        &self,
        caller: WebUiAuthenticatedCaller,
        scope: &TurnScope,
        original_not_found_error: RebornServicesError,
    ) -> Result<ThreadHistory, RebornServicesError> {
        let access = self
            .check_automation_trigger_access(caller, scope, original_not_found_error)
            .await?;
        // Authorized: re-fetch the history using the TRUE stored scope
        // (owner_user_id = creator_user_id, not the caller's session user).
        let true_thread_scope = thread_scope_from_turn_scope(
            &access.scope,
            access.scope.explicit_owner_user_id().cloned(),
        )?;
        self.thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: true_thread_scope,
                thread_id: access.scope.thread_id.clone(),
            })
            .await
            .map_err(map_timeline_probe_error)
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
            WebUiGateResolution::Denied | WebUiGateResolution::Cancelled => {
                ApprovalInteractionDecision::Deny
            }
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
            ResolveApprovalInteractionResponse::Approved(response) => {
                Ok(RebornResolveGateResponse::Resumed(response.into()))
            }
            ResolveApprovalInteractionResponse::Denied(response) => {
                Ok(RebornResolveGateResponse::Cancelled(response.into()))
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
            WebUiGateResolution::Denied | WebUiGateResolution::Cancelled => {
                AuthInteractionDecision::Deny
            }
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
                    })
                    .await
                    .map_err(map_turn_error)?;
                Ok(RebornResolveGateResponse::Resumed(response.into()))
            }
            WebUiGateResolution::CredentialProvided { .. } => {
                Err(blocked_authentication_unavailable())
            }
            WebUiGateResolution::Denied | WebUiGateResolution::Cancelled => {
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

/// Default page size for [`RebornServicesApi::get_timeline`] when the
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

fn bounded_operator_logs_query(query: RebornOperatorLogsQuery) -> RebornLogQueryRequest {
    RebornLogQueryRequest {
        limit: Some(
            query
                .limit
                .unwrap_or(OPERATOR_LOGS_DEFAULT_LIMIT)
                .clamp(1, OPERATOR_LOGS_MAX_LIMIT),
        ),
        cursor: bounded_operator_logs_string(query.cursor, OPERATOR_LOGS_CURSOR_MAX_BYTES),
        level: query.level,
        target: bounded_operator_logs_string(query.target, OPERATOR_LOGS_TARGET_MAX_BYTES),
        thread_id: bounded_operator_logs_context_string(query.thread_id),
        run_id: bounded_operator_logs_context_string(query.run_id),
        turn_id: bounded_operator_logs_context_string(query.turn_id),
        tool_call_id: bounded_operator_logs_context_string(query.tool_call_id),
        tool_name: bounded_operator_logs_context_string(query.tool_name),
        source: bounded_operator_logs_context_string(query.source),
        tail: false,
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
