use std::{collections::HashMap, sync::Arc, time::Instant};

use ironclaw_auth::{
    CredentialAccountService, GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE,
    GOOGLE_GMAIL_MODIFY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE, ProviderScope,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkMethod, ResourceScope, ResourceUsage,
    RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeCredentialTarget,
    RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};

use crate::gsuite::{
    credential::{GoogleCredentialError, GoogleCredentialResolver},
    manifest::{
        CALENDAR_EXTENSION_ID, GMAIL_EXTENSION_ID, GSUITE_RESPONSE_BODY_LIMIT, GSUITE_TIMEOUT_MS,
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
}

impl GsuiteExecutor {
    pub fn new(accounts: Arc<dyn CredentialAccountService>) -> Self {
        Self {
            resolver: Arc::new(GoogleCredentialResolver::new(accounts)),
        }
    }

    pub async fn dispatch(
        &self,
        request: GsuiteDispatchRequest<'_>,
    ) -> Result<GsuiteDispatchResult, GsuiteDispatchError> {
        let started = Instant::now();
        let kind = GsuiteCapability::from_id(request.capability_id.as_str()).ok_or_else(|| {
            GsuiteDispatchError::new(RuntimeDispatchErrorKind::UndeclaredCapability)
        })?;
        let extension = ExtensionId::new(kind.extension_id())
            .map_err(|_| GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend))?;
        let scopes = kind.required_scopes()?;
        let credential = self
            .resolver
            .resolve(request.scope, &extension, &scopes)
            .await
            .map_err(map_credential_error)?;
        let (response, network_egress_bytes) =
            if matches!(kind, GsuiteCapability::CalendarAddAttendees) {
                execute_add_attendees(&request, credential.access_secret).await?
            } else {
                let api_request = kind.request(&request, credential.access_secret)?;
                let response =
                    execute_runtime_http(api_request, Arc::clone(&request.runtime_http_egress))
                        .await?;
                let network_egress_bytes = response.request_bytes;
                (response, network_egress_bytes)
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("GSuite capability dispatch failed: {kind}")]
pub struct GsuiteDispatchError {
    kind: RuntimeDispatchErrorKind,
    usage: Option<ResourceUsage>,
}

impl GsuiteDispatchError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    pub fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

#[derive(Debug, Clone, Copy)]
enum GsuiteCapability {
    CalendarListCalendars,
    CalendarListEvents,
    CalendarGetEvent,
    CalendarFindFreeSlots,
    CalendarCreateEvent,
    CalendarUpdateEvent,
    CalendarDeleteEvent,
    CalendarAddAttendees,
    CalendarSetReminder,
    GmailListMessages,
    GmailGetMessage,
    GmailSendMessage,
    GmailCreateDraft,
    GmailReplyToMessage,
    GmailTrashMessage,
}

async fn execute_add_attendees(
    request: &GsuiteDispatchRequest<'_>,
    access_secret: ironclaw_host_api::SecretHandle,
) -> Result<(ironclaw_host_api::RuntimeHttpEgressResponse, u64), GsuiteDispatchError> {
    let url = calendar_event_url(request.input)?;
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
    let current = response_body_json(&current_response)
        .map_err(|error| add_network_usage(error, network_egress_bytes))?;
    let existing = current
        .get("attendees")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let attendees = merge_attendees(
        existing,
        required_array(request.input, "attendees")
            .map_err(|error| add_network_usage(error, network_egress_bytes))?,
    );
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
    Ok((response, network_egress_bytes))
}

async fn execute_runtime_http(
    request: RuntimeHttpEgressRequest,
    egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, GsuiteDispatchError> {
    tokio::task::spawn_blocking(move || egress.execute(request))
        .await
        .map_err(|_| GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend))?
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

impl GsuiteCapability {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            CALENDAR_LIST_CALENDARS_CAPABILITY_ID => Some(Self::CalendarListCalendars),
            CALENDAR_LIST_EVENTS_CAPABILITY_ID => Some(Self::CalendarListEvents),
            CALENDAR_GET_EVENT_CAPABILITY_ID => Some(Self::CalendarGetEvent),
            CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID => Some(Self::CalendarFindFreeSlots),
            CALENDAR_CREATE_EVENT_CAPABILITY_ID => Some(Self::CalendarCreateEvent),
            CALENDAR_UPDATE_EVENT_CAPABILITY_ID => Some(Self::CalendarUpdateEvent),
            CALENDAR_DELETE_EVENT_CAPABILITY_ID => Some(Self::CalendarDeleteEvent),
            CALENDAR_ADD_ATTENDEES_CAPABILITY_ID => Some(Self::CalendarAddAttendees),
            CALENDAR_SET_REMINDER_CAPABILITY_ID => Some(Self::CalendarSetReminder),
            GMAIL_LIST_MESSAGES_CAPABILITY_ID => Some(Self::GmailListMessages),
            GMAIL_GET_MESSAGE_CAPABILITY_ID => Some(Self::GmailGetMessage),
            GMAIL_SEND_MESSAGE_CAPABILITY_ID => Some(Self::GmailSendMessage),
            GMAIL_CREATE_DRAFT_CAPABILITY_ID => Some(Self::GmailCreateDraft),
            GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID => Some(Self::GmailReplyToMessage),
            GMAIL_TRASH_MESSAGE_CAPABILITY_ID => Some(Self::GmailTrashMessage),
            _ => None,
        }
    }

    fn extension_id(self) -> &'static str {
        match self {
            Self::CalendarListCalendars
            | Self::CalendarListEvents
            | Self::CalendarGetEvent
            | Self::CalendarFindFreeSlots
            | Self::CalendarCreateEvent
            | Self::CalendarUpdateEvent
            | Self::CalendarDeleteEvent
            | Self::CalendarAddAttendees
            | Self::CalendarSetReminder => CALENDAR_EXTENSION_ID,
            Self::GmailListMessages
            | Self::GmailGetMessage
            | Self::GmailSendMessage
            | Self::GmailCreateDraft
            | Self::GmailReplyToMessage
            | Self::GmailTrashMessage => GMAIL_EXTENSION_ID,
        }
    }

    fn required_scopes(self) -> Result<Vec<ProviderScope>, GsuiteDispatchError> {
        let scopes = match self {
            Self::CalendarListCalendars
            | Self::CalendarListEvents
            | Self::CalendarGetEvent
            | Self::CalendarFindFreeSlots => vec![GOOGLE_CALENDAR_READONLY_SCOPE],
            Self::CalendarCreateEvent
            | Self::CalendarUpdateEvent
            | Self::CalendarDeleteEvent
            | Self::CalendarAddAttendees
            | Self::CalendarSetReminder => vec![GOOGLE_CALENDAR_EVENTS_SCOPE],
            Self::GmailListMessages | Self::GmailGetMessage => vec![GOOGLE_GMAIL_READONLY_SCOPE],
            Self::GmailSendMessage => vec![GOOGLE_GMAIL_SEND_SCOPE],
            Self::GmailCreateDraft | Self::GmailTrashMessage => vec![GOOGLE_GMAIL_MODIFY_SCOPE],
            Self::GmailReplyToMessage => vec![GOOGLE_GMAIL_SEND_SCOPE],
        };
        scopes
            .into_iter()
            .map(|scope| {
                ProviderScope::new(scope)
                    .map_err(|_| GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend))
            })
            .collect()
    }

    fn request(
        self,
        request: &GsuiteDispatchRequest<'_>,
        access_secret: ironclaw_host_api::SecretHandle,
    ) -> Result<RuntimeHttpEgressRequest, GsuiteDispatchError> {
        let (method, url, body) = self.request_parts(request.input)?;
        Ok(runtime_request(request, access_secret, method, url, body))
    }

    fn request_parts(
        self,
        input: &Value,
    ) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
        match self {
            Self::CalendarListCalendars => calendar_list_calendars_request(),
            Self::CalendarListEvents => calendar_list_events_request(input),
            Self::CalendarGetEvent => calendar_get_event_request(input),
            Self::CalendarFindFreeSlots => calendar_find_free_slots_request(input),
            Self::CalendarCreateEvent => calendar_create_event_request(input),
            Self::CalendarUpdateEvent => calendar_update_event_request(input),
            Self::CalendarDeleteEvent => calendar_delete_event_request(input),
            Self::CalendarAddAttendees => {
                Err(GsuiteDispatchError::new(RuntimeDispatchErrorKind::Backend))
            }
            Self::CalendarSetReminder => calendar_set_reminder_request(input),
            Self::GmailListMessages => gmail_list_messages_request(input),
            Self::GmailGetMessage => gmail_get_message_request(input),
            Self::GmailSendMessage => gmail_send_message_request(input),
            Self::GmailCreateDraft => gmail_create_draft_request(input),
            Self::GmailReplyToMessage => gmail_reply_to_message_request(input),
            Self::GmailTrashMessage => gmail_trash_message_request(input),
        }
    }
}

fn calendar_list_calendars_request() -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError>
{
    Ok((
        NetworkMethod::Get,
        format!("{CALENDAR_API_BASE}/users/me/calendarList"),
        Vec::new(),
    ))
}

fn calendar_list_events_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Get,
        calendar_events_url(input, None)?,
        Vec::new(),
    ))
}

fn calendar_get_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((NetworkMethod::Get, calendar_event_url(input)?, Vec::new()))
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
    Ok((
        NetworkMethod::Post,
        calendar_events_url(input, None)?,
        json_body(required_object(input, "event")?)?,
    ))
}

fn calendar_update_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Patch,
        calendar_event_url(input)?,
        json_body(required_object(input, "event")?)?,
    ))
}

fn calendar_delete_event_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Delete,
        calendar_event_url(input)?,
        Vec::new(),
    ))
}

fn calendar_set_reminder_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Patch,
        calendar_event_url(input)?,
        json_body(&json!({ "reminders": required_object(input, "reminders")? }))?,
    ))
}

fn gmail_list_messages_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((NetworkMethod::Get, gmail_messages_url(input)?, Vec::new()))
}

fn gmail_get_message_request(
    input: &Value,
) -> Result<(NetworkMethod, String, Vec<u8>), GsuiteDispatchError> {
    Ok((
        NetworkMethod::Get,
        format!(
            "{GMAIL_API_BASE}/users/me/messages/{}?format=full",
            encode_segment(required_str(input, "message_id")?)
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
            encode_segment(required_str(input, "message_id")?)
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
        timeout_ms: Some(GSUITE_TIMEOUT_MS),
    }
}

fn map_credential_error(error: GoogleCredentialError) -> GsuiteDispatchError {
    let kind = match error {
        GoogleCredentialError::Missing
        | GoogleCredentialError::AccountSelectionRequired
        | GoogleCredentialError::NotConfigured
        | GoogleCredentialError::MissingAccessSecret
        | GoogleCredentialError::MissingScopes => RuntimeDispatchErrorKind::Client,
        GoogleCredentialError::Auth(_) | GoogleCredentialError::HostApi(_) => {
            RuntimeDispatchErrorKind::Backend
        }
    };
    GsuiteDispatchError::new(kind)
}

fn map_egress_error(error: RuntimeHttpEgressError) -> GsuiteDispatchError {
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
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

fn add_network_usage(error: GsuiteDispatchError, network_egress_bytes: u64) -> GsuiteDispatchError {
    let mut usage = error.usage().cloned().unwrap_or_default();
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(network_egress_bytes);
    error.with_usage(usage)
}

fn calendar_events_url(
    input: &Value,
    extra_query: Option<&str>,
) -> Result<String, GsuiteDispatchError> {
    let calendar_id = encode_segment(optional_str(input, "calendar_id")?.unwrap_or("primary"));
    let mut url = format!("{CALENDAR_API_BASE}/calendars/{calendar_id}/events");
    let mut query = Vec::new();
    push_query(input, &mut query, "time_min", "timeMin");
    push_query(input, &mut query, "time_max", "timeMax");
    push_query(input, &mut query, "page_token", "pageToken");
    push_query(input, &mut query, "max_results", "maxResults");
    if let Some(extra) = extra_query {
        query.push(extra.to_string());
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    Ok(url)
}

fn calendar_event_url(input: &Value) -> Result<String, GsuiteDispatchError> {
    let calendar_id = encode_segment(optional_str(input, "calendar_id")?.unwrap_or("primary"));
    let event_id = encode_segment(required_str(input, "event_id")?);
    Ok(format!(
        "{CALENDAR_API_BASE}/calendars/{calendar_id}/events/{event_id}"
    ))
}

fn gmail_messages_url(input: &Value) -> Result<String, GsuiteDispatchError> {
    let mut url = format!("{GMAIL_API_BASE}/users/me/messages");
    let mut query = Vec::new();
    push_query(input, &mut query, "query", "q");
    push_query(input, &mut query, "page_token", "pageToken");
    push_query(input, &mut query, "max_results", "maxResults");
    push_repeated_string_query(input, &mut query, "label_ids", "labelIds")?;
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    Ok(url)
}

fn push_query(input: &Value, query: &mut Vec<String>, input_key: &str, query_key: &str) {
    if let Some(value) = input.get(input_key) {
        let value = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        query.push(format!("{query_key}={}", encode_percent(&value)));
    }
}

fn push_repeated_string_query(
    input: &Value,
    query: &mut Vec<String>,
    input_key: &str,
    query_key: &str,
) -> Result<(), GsuiteDispatchError> {
    let Some(value) = input.get(input_key) else {
        return Ok(());
    };
    let values = value.as_array().ok_or_else(input_error)?;
    for item in values {
        let item = item.as_str().ok_or_else(input_error)?;
        query.push(format!("{query_key}={}", encode_percent(item)));
    }
    Ok(())
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
    serde_json::to_vec(value).map_err(|_| input_error())
}

fn merge_attendees(mut existing: Vec<Value>, additions: &Value) -> Vec<Value> {
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
    let Some(additions) = additions.as_array() else {
        return existing;
    };
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
    use ironclaw_host_api::HostApiError;

    use super::*;

    #[test]
    fn map_credential_error_tests() {
        for error in [
            GoogleCredentialError::Missing,
            GoogleCredentialError::AccountSelectionRequired,
            GoogleCredentialError::NotConfigured,
            GoogleCredentialError::MissingAccessSecret,
            GoogleCredentialError::MissingScopes,
        ] {
            assert_eq!(
                map_credential_error(error).kind(),
                RuntimeDispatchErrorKind::Client
            );
        }

        assert_eq!(
            map_credential_error(GoogleCredentialError::Auth(
                ironclaw_auth::AuthProductError::BackendUnavailable,
            ))
            .kind(),
            RuntimeDispatchErrorKind::Backend
        );
        assert_eq!(
            map_credential_error(GoogleCredentialError::HostApi(
                HostApiError::InvariantViolation {
                    reason: "bad contract".to_string(),
                },
            ))
            .kind(),
            RuntimeDispatchErrorKind::Backend
        );
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
                    reason: "offline".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                RuntimeDispatchErrorKind::NetworkDenied,
                12,
            ),
            (
                RuntimeHttpEgressError::Response {
                    reason: "bad response".to_string(),
                    request_bytes: 13,
                    response_bytes: 1,
                },
                RuntimeDispatchErrorKind::OutputDecode,
                13,
            ),
            (
                RuntimeHttpEgressError::Network {
                    reason: ironclaw_host_api::RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED
                        .to_string(),
                    request_bytes: 14,
                    response_bytes: 1024,
                },
                RuntimeDispatchErrorKind::OutputTooLarge,
                14,
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
            &json!({
                "calendar_id": "team calendar",
                "time_min": "2026-05-21T00:00:00Z",
                "max_results": 10,
            }),
            None,
        )
        .unwrap();
        assert!(calendar_events.contains("/calendars/team%20calendar/events"));
        assert!(calendar_events.contains("timeMin=2026-05-21T00%3A00%3A00Z"));
        assert!(calendar_events.contains("maxResults=10"));

        let calendar_event = calendar_event_url(&json!({
            "calendar_id": "primary",
            "event_id": "evt/needs encoding",
        }))
        .unwrap();
        assert!(calendar_event.ends_with("/events/evt%2Fneeds%20encoding"));

        let gmail_messages = gmail_messages_url(&json!({
            "query": "is:unread from:ada",
            "label_ids": ["INBOX", "Team Label"],
        }))
        .unwrap();
        assert!(gmail_messages.contains("q=is%3Aunread%20from%3Aada"));
        assert!(gmail_messages.contains("labelIds=INBOX"));
        assert!(gmail_messages.contains("labelIds=Team%20Label"));
    }
}
