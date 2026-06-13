use std::{
    collections::HashMap,
    io::{self, Write},
    panic::AssertUnwindSafe,
    sync::Arc,
    time::Instant,
};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_auth::{
    CredentialAccountRecordSource, CredentialAccountService, CredentialRecoveryKind,
    CredentialRecoveryProjection, ProviderScope,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkMethod, ResourceScope, ResourceUsage,
    RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeCredentialTarget,
    RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};

use crate::gsuite::{
    credential::GoogleCredential,
    credential::{GoogleCredentialError, GoogleCredentialResolver},
    manifest::{
        GSUITE_REQUEST_BODY_LIMIT, GSUITE_RESPONSE_BODY_LIMIT, GSUITE_TIMEOUT_MS,
        GsuiteCapabilityOperation, GsuiteCapabilitySpec, find_gsuite_capability,
    },
    network::google_api_network_policy,
};

pub const CALENDAR_LIST_CALENDARS_CAPABILITY_ID: &str = "google-calendar.list_calendars";
pub const CALENDAR_LIST_EVENTS_CAPABILITY_ID: &str = "google-calendar.list_events";
pub const CALENDAR_GET_EVENT_CAPABILITY_ID: &str = "google-calendar.get_event";
pub const CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID: &str = "google-calendar.find_free_slots";
pub const CALENDAR_CREATE_EVENT_CAPABILITY_ID: &str = "google-calendar.create_event";
pub const CALENDAR_UPDATE_EVENT_CAPABILITY_ID: &str = "google-calendar.update_event";
pub const CALENDAR_DELETE_EVENT_CAPABILITY_ID: &str = "google-calendar.delete_event";
pub const CALENDAR_ADD_ATTENDEES_CAPABILITY_ID: &str = "google-calendar.add_attendees";
pub const CALENDAR_SET_REMINDER_CAPABILITY_ID: &str = "google-calendar.set_reminder";

pub const GMAIL_LIST_MESSAGES_CAPABILITY_ID: &str = "gmail.list_messages";
pub const GMAIL_GET_MESSAGE_CAPABILITY_ID: &str = "gmail.get_message";
pub const GMAIL_SEND_MESSAGE_CAPABILITY_ID: &str = "gmail.send_message";
pub const GMAIL_CREATE_DRAFT_CAPABILITY_ID: &str = "gmail.create_draft";
pub const GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID: &str = "gmail.reply_to_message";
pub const GMAIL_TRASH_MESSAGE_CAPABILITY_ID: &str = "gmail.trash_message";

const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

#[derive(Clone)]
pub struct GsuiteExecutor {
    resolver: Arc<GoogleCredentialResolver>,
    credential_stager: Arc<dyn GsuiteCredentialStager>,
}

impl GsuiteExecutor {
    pub fn new(
        accounts: Arc<dyn CredentialAccountService>,
        account_records: Arc<dyn CredentialAccountRecordSource>,
        credential_stager: Arc<dyn GsuiteCredentialStager>,
    ) -> Self {
        Self {
            resolver: Arc::new(GoogleCredentialResolver::new(accounts, account_records)),
            credential_stager,
        }
    }

    pub async fn dispatch(
        &self,
        request: GsuiteDispatchRequest<'_>,
    ) -> Result<GsuiteDispatchResult, GsuiteDispatchError> {
        let started = Instant::now();
        let (package, capability) = find_gsuite_capability(request.capability_id.as_str())
            .ok_or_else(|| {
                GsuiteDispatchError::new(RuntimeDispatchErrorKind::UndeclaredCapability)
            })?;
        let extension = ExtensionId::new(package.extension_id)
            .map_err(|_| GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend))?;
        let scopes = required_provider_scopes(capability)?;
        let credential = self
            .resolver
            .resolve(request.scope, &extension, &scopes)
            .await
            .map_err(map_credential_error)?;
        // Stage after parsing so a parse failure doesn't leave a staged credential
        // behind in the injection store.
        let execution = capability_execution(capability, request.input)?;
        self.stage_credential(&request, &credential).await?;
        let (response, network_egress_bytes) = match execution
            .execute(&request, &credential, self.credential_stager.as_ref())
            .await?
        {
            CapabilityExecutionOutcome::Response {
                response,
                network_egress_bytes,
            } => (response, network_egress_bytes),
            CapabilityExecutionOutcome::AuthExpired {
                network_egress_bytes,
            } => {
                self.resolver
                    .refresh(
                        request.scope,
                        &credential.account_scope,
                        &extension,
                        credential.account_id,
                    )
                    .await
                    .map_err(|error| {
                        add_network_usage(map_credential_error(error), network_egress_bytes)
                    })?;
                let refreshed = self
                    .resolver
                    .resolve_account(
                        request.scope,
                        &credential.account_scope,
                        &extension,
                        credential.account_id,
                        &scopes,
                    )
                    .await
                    .map_err(|error| {
                        add_network_usage(map_credential_error(error), network_egress_bytes)
                    })?;
                // Parse before staging for the same reason as the primary path:
                // a parse failure should not leave a credential staged.
                let retry_execution = capability_execution(capability, request.input)?;
                self.stage_credential(&request, &refreshed)
                    .await
                    .map_err(|error| add_network_usage(error, network_egress_bytes))?;
                match retry_execution
                    .execute(&request, &refreshed, self.credential_stager.as_ref())
                    .await
                    .map_err(|error| add_network_usage(error, network_egress_bytes))?
                {
                    CapabilityExecutionOutcome::Response {
                        response,
                        network_egress_bytes: retry_network_egress_bytes,
                    } => (
                        response,
                        network_egress_bytes.saturating_add(retry_network_egress_bytes),
                    ),
                    CapabilityExecutionOutcome::AuthExpired {
                        network_egress_bytes: retry_network_egress_bytes,
                    } => {
                        return Err(GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
                            .with_reason(GsuiteCredentialDispatchReason::BackendAuth)
                            .with_usage(ResourceUsage {
                                network_egress_bytes: network_egress_bytes
                                    .saturating_add(retry_network_egress_bytes),
                                ..ResourceUsage::default()
                            }));
                    }
                }
            }
        };
        let output = response_output(&response)?;
        let wall_clock_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let output_bytes = serde_json::to_vec(&output)
            .map(|body| body.len() as u64)
            .unwrap_or(0);
        Ok(GsuiteDispatchResult {
            output,
            usage: ResourceUsage {
                wall_clock_ms,
                output_bytes,
                network_egress_bytes,
                ..ResourceUsage::default()
            },
        })
    }

    async fn stage_credential(
        &self,
        request: &GsuiteDispatchRequest<'_>,
        credential: &GoogleCredential,
    ) -> Result<(), GsuiteDispatchError> {
        self.credential_stager
            .stage(GsuiteCredentialStageRequest {
                source_scope: &credential.access_secret_scope,
                target_scope: request.scope,
                capability_id: request.capability_id,
                access_secret: &credential.access_secret,
            })
            .await
            .map_err(|error| map_stage_error(error, credential.access_secret.clone()))
    }
}

pub struct GsuiteDispatchRequest<'a> {
    pub capability_id: &'a CapabilityId,
    pub scope: &'a ResourceScope,
    pub input: &'a Value,
    pub runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GsuiteDispatchResult {
    pub output: Value,
    pub usage: ResourceUsage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GsuiteCredentialDispatchReason {
    Recovery(Box<CredentialRecoveryProjection>),
    MissingScopes {
        missing_scopes: Vec<ProviderScope>,
    },
    MissingAccessSecret,
    AuthRequired {
        required_secrets: Vec<ironclaw_host_api::SecretHandle>,
    },
    BackendAuth,
    HostApi,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("GSuite capability dispatch failed: {kind}")]
pub struct GsuiteDispatchError {
    kind: RuntimeDispatchErrorKind,
    reason: Option<GsuiteCredentialDispatchReason>,
    usage: Option<ResourceUsage>,
}

impl GsuiteDispatchError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self {
            kind,
            reason: None,
            usage: None,
        }
    }

    pub fn with_reason(mut self, reason: GsuiteCredentialDispatchReason) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn reason(&self) -> Option<&GsuiteCredentialDispatchReason> {
        self.reason.as_ref()
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }

    /// Returns the secret handles the runtime auth gate must prompt for, or `None`
    /// if the error is an infrastructure failure rather than a user-actionable auth condition.
    ///
    /// `BackendAuth` and `HostApi` return `None`; all other reasons return `Some`.
    /// `AuthRequired` forwards its explicit handle list; the remaining auth reasons
    /// return an empty `Vec` (the caller reads [`Self::reason`] for richer context).
    /// `Recovery(Configured)` returns `None` because it signals a backend infrastructure
    /// failure — prompting the user to re-authenticate would be incorrect.
    pub fn auth_requirement(&self) -> Option<Vec<ironclaw_host_api::SecretHandle>> {
        match self.reason.as_ref()? {
            GsuiteCredentialDispatchReason::Recovery(recovery) => {
                match recovery.kind() {
                    // Backend infrastructure failure: do not trigger the auth gate.
                    CredentialRecoveryKind::Configured => None,
                    // User-actionable recovery: trigger auth gate with no specific handle.
                    CredentialRecoveryKind::SetupRequired
                    | CredentialRecoveryKind::ReauthorizeRequired
                    | CredentialRecoveryKind::AccountSelectionRequired => Some(Vec::new()),
                }
            }
            GsuiteCredentialDispatchReason::MissingScopes { .. }
            | GsuiteCredentialDispatchReason::MissingAccessSecret => Some(Vec::new()),
            GsuiteCredentialDispatchReason::AuthRequired { required_secrets } => {
                Some(required_secrets.clone())
            }
            GsuiteCredentialDispatchReason::BackendAuth
            | GsuiteCredentialDispatchReason::HostApi => None,
        }
    }

    pub fn is_auth_required(&self) -> bool {
        self.auth_requirement().is_some()
    }
}

pub struct GsuiteCredentialStageRequest<'a> {
    /// Scope where the resolved Google access-secret handle is stored.
    pub source_scope: &'a ResourceScope,
    /// Runtime invocation scope that receives the staged credential injection.
    pub target_scope: &'a ResourceScope,
    pub capability_id: &'a CapabilityId,
    pub access_secret: &'a ironclaw_host_api::SecretHandle,
}

/// Alias for [`ironclaw_host_api::CredentialStageError`].
///
/// The shared type lives in `ironclaw_host_api` so that both the GSuite staging
/// trait and the host-runtime staging layer use the same type without a
/// cross-crate conversion step.
pub type GsuiteCredentialStageError = ironclaw_host_api::CredentialStageError;

#[async_trait]
pub trait GsuiteCredentialStager: Send + Sync {
    async fn stage(
        &self,
        request: GsuiteCredentialStageRequest<'_>,
    ) -> Result<(), GsuiteCredentialStageError>;
}

enum CapabilityExecution {
    Single {
        method: NetworkMethod,
        url: String,
        body: Vec<u8>,
    },
    AddAttendees(CalendarAddAttendeesInput),
}

enum CapabilityExecutionOutcome {
    Response {
        response: ironclaw_host_api::RuntimeHttpEgressResponse,
        network_egress_bytes: u64,
    },
    AuthExpired {
        network_egress_bytes: u64,
    },
}

impl CapabilityExecution {
    async fn execute(
        self,
        request: &GsuiteDispatchRequest<'_>,
        credential: &GoogleCredential,
        stager: &dyn GsuiteCredentialStager,
    ) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
        match self {
            Self::Single { method, url, body } => {
                let response = execute_runtime_http(
                    runtime_request(request, credential.access_secret.clone(), method, url, body),
                    Arc::clone(&request.runtime_http_egress),
                )
                .await?;
                let network_egress_bytes = response.request_bytes;
                Ok(response_outcome(response, network_egress_bytes))
            }
            Self::AddAttendees(input) => {
                execute_add_attendees(request, credential, stager, input).await
            }
        }
    }
}

async fn execute_add_attendees(
    request: &GsuiteDispatchRequest<'_>,
    credential: &GoogleCredential,
    stager: &dyn GsuiteCredentialStager,
    input: CalendarAddAttendeesInput,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let url = input.event_path.url();
    let access_secret = credential.access_secret.clone();
    let current_response = execute_runtime_http(
        runtime_request(
            request,
            access_secret.clone(),
            NetworkMethod::Get,
            url.clone(),
            Vec::new(),
        ),
        Arc::clone(&request.runtime_http_egress),
    )
    .await?;
    let mut network_egress_bytes = current_response.request_bytes;
    if is_google_auth_expired_response(&current_response) {
        return Ok(CapabilityExecutionOutcome::AuthExpired {
            network_egress_bytes,
        });
    }
    let current = response_body_json(&current_response)
        .map_err(|error| add_network_usage(error, network_egress_bytes))?;
    let existing = current
        .get("attendees")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let attendees = merge_attendees(existing, input.attendees);
    // Re-stage before the PATCH: the staged-obligation store is one-shot,
    // so the GET egress consumed the first injection. Stage again so the
    // PATCH egress has its credential available.
    stager
        .stage(GsuiteCredentialStageRequest {
            source_scope: &credential.access_secret_scope,
            target_scope: request.scope,
            capability_id: request.capability_id,
            access_secret: &access_secret,
        })
        .await
        .map_err(|error| {
            add_network_usage(
                map_stage_error(error, access_secret.clone()),
                network_egress_bytes,
            )
        })?;
    let mut patch = runtime_request(
        request,
        access_secret,
        NetworkMethod::Patch,
        url,
        json_body(&json!({ "attendees": attendees }))
            .map_err(|error| add_network_usage(error, network_egress_bytes))?,
    );
    if let Some(etag) = response_etag(&current_response, &current) {
        patch.headers.push(("if-match".to_string(), etag));
    }
    let response = execute_runtime_http(patch, Arc::clone(&request.runtime_http_egress))
        .await
        .map_err(|error| add_network_usage(error, network_egress_bytes))?;
    network_egress_bytes = network_egress_bytes.saturating_add(response.request_bytes);
    Ok(response_outcome(response, network_egress_bytes))
}

fn response_outcome(
    response: ironclaw_host_api::RuntimeHttpEgressResponse,
    network_egress_bytes: u64,
) -> CapabilityExecutionOutcome {
    if is_google_auth_expired_response(&response) {
        CapabilityExecutionOutcome::AuthExpired {
            network_egress_bytes,
        }
    } else {
        CapabilityExecutionOutcome::Response {
            response,
            network_egress_bytes,
        }
    }
}

async fn execute_runtime_http(
    request: RuntimeHttpEgressRequest,
    egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, GsuiteDispatchError> {
    AssertUnwindSafe(egress.execute(request))
        .catch_unwind()
        .await
        .map_err(|_| {
            tracing::error!("GSuite runtime HTTP egress future panicked");
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
        })?
        .map_err(map_egress_error)
}

fn response_output(
    response: &ironclaw_host_api::RuntimeHttpEgressResponse,
) -> Result<Value, GsuiteDispatchError> {
    let body = response_body_json(response)?;
    Ok(json!({
        "status": response.status,
        "body": body,
        "redaction_applied": response.redaction_applied
    }))
}

fn response_body_json(
    response: &ironclaw_host_api::RuntimeHttpEgressResponse,
) -> Result<Value, GsuiteDispatchError> {
    if response.body.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_slice(&response.body)
            .map_err(|_| GsuiteDispatchError::new(RuntimeDispatchErrorKind::OutputDecode))
    }
}

fn is_google_auth_expired_response(
    response: &ironclaw_host_api::RuntimeHttpEgressResponse,
) -> bool {
    response.status == 401
}

fn required_provider_scopes(
    capability: &GsuiteCapabilitySpec,
) -> Result<Vec<ProviderScope>, GsuiteDispatchError> {
    capability
        .required_scopes
        .iter()
        .copied()
        .map(|scope| {
            ProviderScope::new(scope)
                .map_err(|_| GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend))
        })
        .collect()
}

fn capability_execution(
    capability: &GsuiteCapabilitySpec,
    input: &Value,
) -> Result<CapabilityExecution, GsuiteDispatchError> {
    use GsuiteCapabilityOperation as Operation;

    let single = |(method, url, body)| CapabilityExecution::Single { method, url, body };
    Ok(match capability.operation {
        Operation::CalendarListCalendars => single(calendar_list_calendars_request()),
        Operation::CalendarListEvents => single(calendar_list_events_request(input)?),
        Operation::CalendarGetEvent => single(calendar_get_event_request(input)?),
        Operation::CalendarFindFreeSlots => single(calendar_find_free_slots_request(input)?),
        Operation::CalendarCreateEvent => single(calendar_create_event_request(input)?),
        Operation::CalendarUpdateEvent => single(calendar_update_event_request(input)?),
        Operation::CalendarDeleteEvent => single(calendar_delete_event_request(input)?),
        Operation::CalendarAddAttendees => {
            CapabilityExecution::AddAttendees(CalendarAddAttendeesInput::parse(input)?)
        }
        Operation::CalendarSetReminder => single(calendar_set_reminder_request(input)?),
        Operation::GmailListMessages => single(gmail_list_messages_request(input)?),
        Operation::GmailGetMessage => single(gmail_get_message_request(input)?),
        Operation::GmailSendMessage => single(gmail_send_message_request(input)?),
        Operation::GmailCreateDraft => single(gmail_create_draft_request(input)?),
        Operation::GmailReplyToMessage => single(gmail_reply_to_message_request(input)?),
        Operation::GmailTrashMessage => single(gmail_trash_message_request(input)?),
    })
}

fn calendar_list_calendars_request() -> (NetworkMethod, String, Vec<u8>) {
    (
        NetworkMethod::Get,
        format!("{CALENDAR_API_BASE}/users/me/calendarList"),
        Vec::new(),
    )
}

struct CalendarEventsQuery {
    calendar_id: String,
    time_min: Option<String>,
    time_max: Option<String>,
    page_token: Option<String>,
    max_results: Option<String>,
}

impl CalendarEventsQuery {
    fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        Ok(Self {
            calendar_id: optional_str(input, "calendar_id")?
                .unwrap_or("primary")
                .to_string(),
            time_min: optional_query_value(input, "time_min")?,
            time_max: optional_query_value(input, "time_max")?,
            page_token: optional_query_value(input, "page_token")?,
            max_results: optional_query_value(input, "max_results")?,
        })
    }
}

struct CalendarEventPath {
    calendar_id: String,
    event_id: String,
}

impl CalendarEventPath {
    fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        Ok(Self {
            calendar_id: optional_str(input, "calendar_id")?
                .unwrap_or("primary")
                .to_string(),
            event_id: required_str(input, "event_id")?.to_string(),
        })
    }

    fn url(&self) -> String {
        format!(
            "{CALENDAR_API_BASE}/calendars/{}/events/{}",
            encode_segment(&self.calendar_id),
            encode_segment(&self.event_id)
        )
    }
}

struct CalendarAddAttendeesInput {
    event_path: CalendarEventPath,
    attendees: Vec<Value>,
}

impl CalendarAddAttendeesInput {
    fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        Ok(Self {
            event_path: CalendarEventPath::parse(input)?,
            attendees: required_array(input, "attendees")?
                .as_array()
                .ok_or_else(input_error)?
                .clone(),
        })
    }
}

struct GmailMessagesQuery {
    query: Option<String>,
    page_token: Option<String>,
    max_results: Option<String>,
    label_ids: Vec<String>,
}

impl GmailMessagesQuery {
    fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        Ok(Self {
            query: optional_query_value(input, "query")?,
            page_token: optional_query_value(input, "page_token")?,
            max_results: optional_query_value(input, "max_results")?,
            label_ids: optional_string_array(input, "label_ids")?,
        })
    }
}

struct GmailMessagePath {
    message_id: String,
}

impl GmailMessagePath {
    fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        Ok(Self {
            message_id: required_str(input, "message_id")?.to_string(),
        })
    }
}

fn calendar_list_events_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    let input = CalendarEventsQuery::parse(input)?;
    Ok((
        NetworkMethod::Get,
        calendar_events_url(&input, None),
        Vec::new(),
    ))
}

fn calendar_get_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Get,
        CalendarEventPath::parse(input)?.url(),
        Vec::new(),
    ))
}

fn calendar_find_free_slots_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Post,
        format!("{CALENDAR_API_BASE}/freeBusy"),
        json_body(input)?,
    ))
}

fn calendar_create_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    let query = CalendarEventsQuery {
        calendar_id: optional_str(input, "calendar_id")?
            .unwrap_or("primary")
            .to_string(),
        time_min: None,
        time_max: None,
        page_token: None,
        max_results: None,
    };
    Ok((
        NetworkMethod::Post,
        calendar_events_url(&query, None),
        json_body(required_object(input, "event")?)?,
    ))
}

fn calendar_update_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Patch,
        CalendarEventPath::parse(input)?.url(),
        json_body(required_object(input, "event")?)?,
    ))
}

fn calendar_delete_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Delete,
        CalendarEventPath::parse(input)?.url(),
        Vec::new(),
    ))
}

fn calendar_set_reminder_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Patch,
        CalendarEventPath::parse(input)?.url(),
        json_body(&json!({ "reminders": required_object(input, "reminders")? }))?,
    ))
}

fn gmail_list_messages_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Get,
        gmail_messages_url(&GmailMessagesQuery::parse(input)?),
        Vec::new(),
    ))
}

fn gmail_get_message_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Get,
        format!(
            "{GMAIL_API_BASE}/users/me/messages/{}?format=full",
            encode_segment(GmailMessagePath::parse(input)?.message_id.as_str())
        ),
        Vec::new(),
    ))
}

fn gmail_send_message_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Post,
        format!("{GMAIL_API_BASE}/users/me/messages/send"),
        json_body(required_object(input, "message")?)?,
    ))
}

fn gmail_create_draft_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Post,
        format!("{GMAIL_API_BASE}/users/me/drafts"),
        json_body(required_object(input, "draft")?)?,
    ))
}

fn gmail_reply_to_message_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Post,
        format!("{GMAIL_API_BASE}/users/me/messages/send"),
        json_body(required_object(input, "message")?)?,
    ))
}

fn gmail_trash_message_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Post,
        format!(
            "{GMAIL_API_BASE}/users/me/messages/{}/trash",
            encode_segment(GmailMessagePath::parse(input)?.message_id.as_str())
        ),
        Vec::new(),
    ))
}

fn runtime_request(
    request: &GsuiteDispatchRequest<'_>,
    access_secret: ironclaw_host_api::SecretHandle,
    method: NetworkMethod,
    url: String,
    body: Vec<u8>,
) -> RuntimeHttpEgressRequest {
    RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method,
        url,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body,
        network_policy: google_api_network_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle: access_secret,
            source: RuntimeCredentialSource::StagedObligation {
                capability_id: request.capability_id.clone(),
            },
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
        response_body_limit: Some(GSUITE_RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(GSUITE_TIMEOUT_MS),
    }
}

fn map_credential_error(error: GoogleCredentialError) -> GsuiteDispatchError {
    match error {
        GoogleCredentialError::Recovery(recovery) => {
            GsuiteDispatchError::new(map_recovery_kind(&recovery))
                .with_reason(GsuiteCredentialDispatchReason::Recovery(Box::new(recovery)))
        }
        GoogleCredentialError::MissingScopes { missing_scopes } => {
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client)
                .with_reason(GsuiteCredentialDispatchReason::MissingScopes { missing_scopes })
        }
        GoogleCredentialError::MissingAccessSecret => {
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client)
                .with_reason(GsuiteCredentialDispatchReason::MissingAccessSecret)
        }
        GoogleCredentialError::Auth(_) => {
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
                .with_reason(GsuiteCredentialDispatchReason::BackendAuth)
        }
        GoogleCredentialError::HostApi(_) => {
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
                .with_reason(GsuiteCredentialDispatchReason::HostApi)
        }
    }
}

fn map_stage_error(
    error: GsuiteCredentialStageError,
    required_secret: ironclaw_host_api::SecretHandle,
) -> GsuiteDispatchError {
    match error {
        GsuiteCredentialStageError::AuthRequired => GsuiteDispatchError::new(
            RuntimeDispatchErrorKind::Client,
        )
        .with_reason(GsuiteCredentialDispatchReason::AuthRequired {
            required_secrets: vec![required_secret],
        }),
        GsuiteCredentialStageError::Backend => {
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
                .with_reason(GsuiteCredentialDispatchReason::BackendAuth)
        }
    }
}

fn map_egress_error(error: RuntimeHttpEgressError) -> GsuiteDispatchError {
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OutputDecode,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    GsuiteDispatchError::new(kind).with_usage(ResourceUsage {
        network_egress_bytes: error.request_bytes(),
        ..ResourceUsage::default()
    })
}

fn map_recovery_kind(recovery: &CredentialRecoveryProjection) -> RuntimeDispatchErrorKind {
    match recovery.kind() {
        CredentialRecoveryKind::Configured => RuntimeDispatchErrorKind::Backend,
        CredentialRecoveryKind::SetupRequired
        | CredentialRecoveryKind::ReauthorizeRequired
        | CredentialRecoveryKind::AccountSelectionRequired => RuntimeDispatchErrorKind::Client,
    }
}

fn add_network_usage(error: GsuiteDispatchError, network_egress_bytes: u64) -> GsuiteDispatchError {
    let mut usage = error.usage().cloned().unwrap_or_default();
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(network_egress_bytes);
    error.with_usage(usage)
}

fn calendar_events_url(input: &CalendarEventsQuery, extra_query: Option<&str>) -> String {
    let calendar_id = encode_segment(&input.calendar_id);
    let mut url = format!("{CALENDAR_API_BASE}/calendars/{calendar_id}/events");
    let mut query = Vec::new();
    push_optional_query(&mut query, "timeMin", input.time_min.as_deref());
    push_optional_query(&mut query, "timeMax", input.time_max.as_deref());
    push_optional_query(&mut query, "pageToken", input.page_token.as_deref());
    push_optional_query(&mut query, "maxResults", input.max_results.as_deref());
    if let Some(extra) = extra_query {
        query.push(extra.to_string());
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    url
}

fn gmail_messages_url(input: &GmailMessagesQuery) -> String {
    let mut url = format!("{GMAIL_API_BASE}/users/me/messages");
    let mut query = Vec::new();
    push_optional_query(&mut query, "q", input.query.as_deref());
    push_optional_query(&mut query, "pageToken", input.page_token.as_deref());
    push_optional_query(&mut query, "maxResults", input.max_results.as_deref());
    for label_id in &input.label_ids {
        query.push(format!("labelIds={}", encode_percent(label_id)));
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    url
}

fn push_optional_query(query: &mut Vec<String>, query_key: &str, value: Option<&str>) {
    if let Some(value) = value {
        query.push(format!("{query_key}={}", encode_percent(value)));
    }
}

fn optional_query_value(input: &Value, key: &str) -> Result<Option<String>, GsuiteDispatchError> {
    Ok(match input.get(key) {
        Some(value) if value.is_string() => value.as_str().map(ToString::to_string),
        Some(value) if value.is_number() || value.is_boolean() => Some(value.to_string()),
        Some(_) => return Err(input_error()),
        None => None,
    })
}

fn optional_string_array(
    input: &Value,
    input_key: &str,
) -> Result<Vec<String>, GsuiteDispatchError> {
    let Some(value) = input.get(input_key) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(input_error)?;
    values
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToString::to_string)
                .ok_or_else(input_error)
        })
        .collect()
}

fn required_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, GsuiteDispatchError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(input_error)
}

fn optional_str<'a>(input: &'a Value, key: &str) -> Result<Option<&'a str>, GsuiteDispatchError> {
    input
        .get(key)
        .map(|value| value.as_str().ok_or_else(input_error))
        .transpose()
}

fn required_object<'a>(input: &'a Value, key: &str) -> Result<&'a Value, GsuiteDispatchError> {
    let value = input.get(key).ok_or_else(input_error)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(input_error())
    }
}

fn required_array<'a>(input: &'a Value, key: &str) -> Result<&'a Value, GsuiteDispatchError> {
    let value = input.get(key).ok_or_else(input_error)?;
    if value.is_array() {
        Ok(value)
    } else {
        Err(input_error())
    }
}

fn json_body(value: &Value) -> Result<Vec<u8>, GsuiteDispatchError> {
    let mut writer = BoundedJsonBody::new(GSUITE_REQUEST_BODY_LIMIT);
    serde_json::to_writer(&mut writer, value).map_err(|_| input_error())?;
    Ok(writer.into_inner())
}

struct BoundedJsonBody {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedJsonBody {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedJsonBody {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(buf.len()) > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "JSON request body exceeds GSuite request body limit",
            ));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn merge_attendees(mut existing: Vec<Value>, additions: Vec<Value>) -> Vec<Value> {
    let mut indexes_by_email = existing
        .iter()
        .enumerate()
        .filter_map(|(index, attendee)| {
            attendee
                .get("email")
                .and_then(Value::as_str)
                .map(|email| (email.to_ascii_lowercase(), index))
        })
        .collect::<HashMap<_, _>>();
    for addition in additions {
        let Some(email) = addition.get("email").and_then(Value::as_str) else {
            existing.push(addition.clone());
            continue;
        };
        let email = email.to_ascii_lowercase();
        match indexes_by_email.get(&email).copied() {
            Some(index) => existing[index] = addition.clone(),
            None => {
                indexes_by_email.insert(email, existing.len());
                existing.push(addition.clone());
            }
        }
    }
    existing
}

fn response_etag(
    response: &ironclaw_host_api::RuntimeHttpEgressResponse,
    body: &Value,
) -> Option<String> {
    response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("etag"))
        .map(|(_, value)| value.clone())
        .or_else(|| {
            body.get("etag")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn input_error() -> GsuiteDispatchError {
    GsuiteDispatchError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn encode_segment(segment: &str) -> String {
    encode_percent(segment)
}

fn encode_percent(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{HostApiError, RuntimeHttpEgressResponse};

    use super::*;

    #[test]
    fn map_credential_error_tests() {
        assert_eq!(
            map_credential_error(GoogleCredentialError::Recovery(
                ironclaw_auth::CredentialRecoveryProjection::setup_required(
                    ironclaw_auth::AuthProviderId::new("google").unwrap(),
                    ironclaw_auth::CredentialRecoveryReason::NoAccount,
                    Vec::new(),
                ),
            ))
            .kind(),
            RuntimeDispatchErrorKind::Client
        );
        assert_eq!(
            map_credential_error(GoogleCredentialError::Recovery(
                ironclaw_auth::CredentialRecoveryProjection::account_selection_required(
                    ironclaw_auth::AuthProviderId::new("google").unwrap(),
                    Vec::new(),
                ),
            ))
            .kind(),
            RuntimeDispatchErrorKind::Client
        );
        assert_eq!(
            map_credential_error(GoogleCredentialError::MissingAccessSecret).kind(),
            RuntimeDispatchErrorKind::Client
        );
        assert_eq!(
            map_credential_error(GoogleCredentialError::MissingAccessSecret).reason(),
            Some(&GsuiteCredentialDispatchReason::MissingAccessSecret)
        );
        let missing_scope =
            ProviderScope::new("https://www.googleapis.com/auth/gmail.modify").expect("scope");
        let missing_scopes_error = map_credential_error(GoogleCredentialError::MissingScopes {
            missing_scopes: vec![missing_scope.clone()],
        });
        assert_eq!(
            missing_scopes_error.kind(),
            RuntimeDispatchErrorKind::Client
        );
        assert_eq!(
            missing_scopes_error.reason(),
            Some(&GsuiteCredentialDispatchReason::MissingScopes {
                missing_scopes: vec![missing_scope],
            })
        );

        let backend_error = map_credential_error(GoogleCredentialError::Auth(
            ironclaw_auth::AuthProductError::BackendUnavailable,
        ));
        assert_eq!(backend_error.kind(), RuntimeDispatchErrorKind::Backend);
        assert_eq!(
            backend_error.reason(),
            Some(&GsuiteCredentialDispatchReason::BackendAuth)
        );
        let host_api_error = map_credential_error(GoogleCredentialError::HostApi(
            HostApiError::InvariantViolation {
                reason: "bad contract".to_string(),
            },
        ));
        assert_eq!(host_api_error.kind(), RuntimeDispatchErrorKind::Backend);
        assert_eq!(
            host_api_error.reason(),
            Some(&GsuiteCredentialDispatchReason::HostApi)
        );

        let configured_recovery = map_credential_error(GoogleCredentialError::Recovery(
            ironclaw_auth::CredentialRecoveryProjection::configured(
                ironclaw_auth::AuthProviderId::new("google").unwrap(),
                ironclaw_auth::CredentialAccountProjection {
                    id: ironclaw_auth::CredentialAccountId::new(),
                    provider: ironclaw_auth::AuthProviderId::new("google").unwrap(),
                    label: ironclaw_auth::CredentialAccountLabel::new("Google").unwrap(),
                    status: ironclaw_auth::CredentialAccountStatus::Configured,
                    ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                    owner_extension: None,
                    granted_extensions: Vec::new(),
                    secret_handle_count: 1,
                },
            ),
        ));
        assert_eq!(
            configured_recovery.kind(),
            RuntimeDispatchErrorKind::Backend
        );
    }

    #[test]
    fn auth_requirement_classifies_each_credential_dispatch_reason() {
        // Recovery(Configured) -> None: backend infra failure, must NOT trigger the
        // auth gate even though it carries a Recovery reason.  This is the S1 fix—
        // the old catch-all Recovery(_) arm incorrectly returned Some(vec![]) here.
        let configured_proj = ironclaw_auth::CredentialRecoveryProjection::configured(
            ironclaw_auth::AuthProviderId::new("google").unwrap(),
            ironclaw_auth::CredentialAccountProjection {
                id: ironclaw_auth::CredentialAccountId::new(),
                provider: ironclaw_auth::AuthProviderId::new("google").unwrap(),
                label: ironclaw_auth::CredentialAccountLabel::new("Google").unwrap(),
                status: ironclaw_auth::CredentialAccountStatus::Configured,
                ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                secret_handle_count: 1,
            },
        );
        let configured_recovery = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
            .with_reason(GsuiteCredentialDispatchReason::Recovery(Box::new(
                configured_proj,
            )));
        assert_eq!(
            configured_recovery.auth_requirement(),
            None,
            "Recovery(Configured) is a backend infra failure — must not trigger the auth gate"
        );
        assert!(!configured_recovery.is_auth_required());

        // Recovery(SetupRequired) -> Some(empty): user-actionable, triggers auth gate.
        // Richer projection is preserved via reason() per the doc-comment contract.
        let recovery = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client).with_reason(
            GsuiteCredentialDispatchReason::Recovery(Box::new(
                ironclaw_auth::CredentialRecoveryProjection::setup_required(
                    ironclaw_auth::AuthProviderId::new("google").unwrap(),
                    ironclaw_auth::CredentialRecoveryReason::NoAccount,
                    Vec::new(),
                ),
            )),
        );
        assert_eq!(recovery.auth_requirement(), Some(Vec::new()));
        assert!(recovery.is_auth_required());
        assert!(recovery.reason().is_some());

        // MissingScopes -> Some(empty): richer projection lives in reason().
        let scope =
            ProviderScope::new("https://www.googleapis.com/auth/gmail.modify").expect("scope");
        let missing_scopes = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client)
            .with_reason(GsuiteCredentialDispatchReason::MissingScopes {
                missing_scopes: vec![scope],
            });
        assert_eq!(missing_scopes.auth_requirement(), Some(Vec::new()));

        // MissingAccessSecret -> Some(empty).
        let missing_access = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client)
            .with_reason(GsuiteCredentialDispatchReason::MissingAccessSecret);
        assert_eq!(missing_access.auth_requirement(), Some(Vec::new()));

        // AuthRequired { required_secrets } -> Some with secrets forwarded.
        let handle = ironclaw_host_api::SecretHandle::new("google-access-token").unwrap();
        let auth_required = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client).with_reason(
            GsuiteCredentialDispatchReason::AuthRequired {
                required_secrets: vec![handle.clone()],
            },
        );
        assert_eq!(auth_required.auth_requirement(), Some(vec![handle]));

        // BackendAuth -> None (infra failure, not user-actionable).
        let backend_auth = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
            .with_reason(GsuiteCredentialDispatchReason::BackendAuth);
        assert_eq!(backend_auth.auth_requirement(), None);
        assert!(!backend_auth.is_auth_required());

        // HostApi -> None.
        let host_api = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend)
            .with_reason(GsuiteCredentialDispatchReason::HostApi);
        assert_eq!(host_api.auth_requirement(), None);

        // No reason at all -> None.
        let no_reason = GsuiteDispatchError::new(RuntimeDispatchErrorKind::Client);
        assert_eq!(no_reason.auth_requirement(), None);
    }

    #[test]
    fn map_egress_error_tests() {
        let cases = [
            (
                RuntimeHttpEgressError::Credential {
                    reason: "missing".to_string(),
                },
                RuntimeDispatchErrorKind::Client,
                0,
            ),
            (
                RuntimeHttpEgressError::Request {
                    reason: "denied".to_string(),
                    request_bytes: 11,
                    response_bytes: 0,
                },
                RuntimeDispatchErrorKind::InputEncode,
                11,
            ),
            (
                RuntimeHttpEgressError::Network {
                    reason: "policy_denied".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                RuntimeDispatchErrorKind::PolicyDenied,
                12,
            ),
            (
                RuntimeHttpEgressError::Network {
                    reason: "offline".to_string(),
                    request_bytes: 13,
                    response_bytes: 0,
                },
                RuntimeDispatchErrorKind::NetworkDenied,
                13,
            ),
            (
                RuntimeHttpEgressError::Response {
                    reason: "bad response".to_string(),
                    request_bytes: 14,
                    response_bytes: 1,
                },
                RuntimeDispatchErrorKind::OutputDecode,
                14,
            ),
            (
                RuntimeHttpEgressError::Network {
                    reason: ironclaw_host_api::RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED
                        .to_string(),
                    request_bytes: 15,
                    response_bytes: 1024,
                },
                RuntimeDispatchErrorKind::OutputTooLarge,
                15,
            ),
        ];

        for (error, expected_kind, expected_request_bytes) in cases {
            let mapped = map_egress_error(error);
            assert_eq!(mapped.kind(), expected_kind);
            assert_eq!(
                mapped
                    .usage()
                    .map(|usage| usage.network_egress_bytes)
                    .unwrap_or_default(),
                expected_request_bytes
            );
        }
    }

    #[test]
    fn is_google_auth_expired_response_only_matches_401() {
        let response = RuntimeHttpEgressResponse {
            status: 401,
            headers: Vec::new(),
            body: Vec::new(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        };
        assert!(is_google_auth_expired_response(&response));

        let response = RuntimeHttpEgressResponse {
            status: 403,
            headers: Vec::new(),
            body: Vec::new(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        };
        assert!(!is_google_auth_expired_response(&response));
    }

    #[test]
    fn input_validation_tests() {
        let input = json!({
            "string": "value",
            "object": {"nested": true},
            "array": [1],
        });

        assert_eq!(required_str(&input, "string").unwrap(), "value");
        assert!(matches!(
            required_str(&input, "missing").unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        ));
        assert!(matches!(
            required_str(&input, "object").unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        ));
        assert!(required_object(&input, "object").is_ok());
        assert!(required_object(&input, "array").is_err());
        assert!(required_array(&input, "array").is_ok());
        assert!(required_array(&input, "object").is_err());
        assert!(json_body(&input).is_ok());
    }

    #[test]
    fn url_building_tests() {
        assert_eq!(encode_percent("a b/c?d=e&f"), "a%20b%2Fc%3Fd%3De%26f");

        let calendar_events = calendar_events_url(
            &CalendarEventsQuery::parse(&json!({
                "calendar_id": "team calendar",
                "time_min": "2026-05-21T00:00:00Z",
                "max_results": 10,
            }))
            .unwrap(),
            None,
        );
        assert!(calendar_events.contains("/calendars/team%20calendar/events"));
        assert!(calendar_events.contains("timeMin=2026-05-21T00%3A00%3A00Z"));
        assert!(calendar_events.contains("maxResults=10"));

        let calendar_event = CalendarEventPath::parse(&json!({
            "calendar_id": "primary",
            "event_id": "evt/needs encoding",
        }))
        .unwrap()
        .url();
        assert!(calendar_event.ends_with("/events/evt%2Fneeds%20encoding"));

        let gmail_messages = gmail_messages_url(
            &GmailMessagesQuery::parse(&json!({
            "query": "is:unread from:ada",
            "label_ids": ["INBOX", "Team Label"],
            }))
            .unwrap(),
        );
        assert!(gmail_messages.contains("q=is%3Aunread%20from%3Aada"));
        assert!(gmail_messages.contains("labelIds=INBOX"));
        assert!(gmail_messages.contains("labelIds=Team%20Label"));
    }

    #[test]
    fn merge_attendees_deduplicates_email_case_insensitively() {
        let merged = merge_attendees(
            vec![
                serde_json::json!({"email": "Alice@Example.com", "name": "old"}),
                serde_json::json!({"email": "bob@example.com"}),
            ],
            vec![
                serde_json::json!({"email": "alice@example.com", "name": "new"}),
                serde_json::json!({"email": "carol@example.com"}),
            ],
        );

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0]["name"], "new");
        assert_eq!(merged[2]["email"], "carol@example.com");
    }

    #[test]
    fn merge_attendees_with_empty_existing_list() {
        let merged = merge_attendees(
            vec![],
            vec![serde_json::json!({"email": "alice@example.com"})],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["email"], "alice@example.com");
    }

    #[test]
    fn merge_attendees_appends_addition_without_email_field() {
        // An addition object that has no "email" key cannot be deduped and is
        // appended unconditionally.
        let merged = merge_attendees(
            vec![serde_json::json!({"email": "alice@example.com"})],
            vec![
                serde_json::json!({"displayName": "Bob"}), // no email
                serde_json::json!({"email": "carol@example.com"}),
            ],
        );
        assert_eq!(merged.len(), 3, "got {merged:?}");
        // Ordering: existing first, then no-email addition, then carol.
        assert_eq!(merged[0]["email"], "alice@example.com");
        assert_eq!(merged[1]["displayName"], "Bob");
        assert_eq!(merged[2]["email"], "carol@example.com");
    }

    #[test]
    fn merge_attendees_with_no_additions() {
        let existing = vec![serde_json::json!({"email": "alice@example.com"})];
        let merged = merge_attendees(existing.clone(), vec![]);
        assert_eq!(merged, existing);
    }

    #[test]
    fn response_etag_reads_case_insensitive_header_body_fallback_and_absent_case() {
        let response = RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("ETag".to_string(), "header-etag".to_string())],
            body: Vec::new(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        };
        assert_eq!(
            response_etag(&response, &serde_json::json!({"etag": "body-etag"})),
            Some("header-etag".to_string())
        );

        let response_without_header = RuntimeHttpEgressResponse {
            headers: Vec::new(),
            ..response
        };
        assert_eq!(
            response_etag(
                &response_without_header,
                &serde_json::json!({"etag": "body-etag"})
            ),
            Some("body-etag".to_string())
        );
        assert_eq!(
            response_etag(&response_without_header, &serde_json::json!({})),
            None
        );
    }
}
