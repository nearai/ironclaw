use std::{borrow::Cow, collections::HashMap, sync::Arc};

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use ironclaw_host_api::{NetworkMethod, RuntimeDispatchErrorKind, RuntimeHttpEgressResponse};
use serde_json::{Value, json};

use crate::gsuite::credential::GoogleCredential;

use super::{
    CALENDAR_API_BASE, CapabilityExecutionOutcome, GsuiteCredentialStageRequest,
    GsuiteCredentialStager, GsuiteDispatchError, GsuiteDispatchRequest, add_network_usage,
    calendar_events_collection_url, encode_percent, execute_runtime_http, input_error,
    is_google_auth_expired_response, map_stage_error, optional_bool, optional_query_value,
    optional_str, optional_string_array, push_optional_query, response_body_json, runtime_request,
};

const DEFAULT_MAX_RESULTS: &str = "25";
const MAX_CALENDARS: usize = 50;
const SAFE_GOOGLE_ERROR_REASONS: &[&str] = &[
    "ABORTED",
    "ALREADY_EXISTS",
    "CANCELLED",
    "DATA_LOSS",
    "DEADLINE_EXCEEDED",
    "FAILED_PRECONDITION",
    "INTERNAL",
    "INVALID_ARGUMENT",
    "NOT_FOUND",
    "OUT_OF_RANGE",
    "PERMISSION_DENIED",
    "RESOURCE_EXHAUSTED",
    "UNAUTHENTICATED",
    "UNAVAILABLE",
    "UNKNOWN",
    "authError",
    "backendError",
    "dailyLimitExceeded",
    "forbidden",
    "invalid",
    "keyInvalid",
    "notFound",
    "quotaExceeded",
    "rateLimitExceeded",
    "required",
    "userRateLimitExceeded",
];

pub(super) struct CalendarEventsQuery {
    calendar_id: String,
    calendar_ids: Vec<String>,
    include_all_calendars: bool,
    time_min: Option<String>,
    time_max: Option<String>,
    page_token: Option<String>,
    page_tokens: HashMap<String, String>,
    max_results: Option<String>,
    query: Option<String>,
}

impl CalendarEventsQuery {
    pub(super) fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        let calendar_id = optional_str(input, "calendar_id")?;
        let calendar_ids = optional_string_array(input, "calendar_ids")?;
        if calendar_ids.len() > MAX_CALENDARS {
            return Err(input_error());
        }
        let include_all_calendars = optional_bool(input, "include_all_calendars")?.unwrap_or(false);
        let calendar_selector_count = usize::from(calendar_id.is_some())
            + usize::from(!calendar_ids.is_empty())
            + usize::from(include_all_calendars);
        if calendar_selector_count > 1 {
            return Err(input_error());
        }
        let aggregate_request = include_all_calendars || !calendar_ids.is_empty();
        let page_token = optional_query_value(input, "page_token")?;
        if page_token.is_some() && aggregate_request {
            return Err(input_error());
        }
        let page_tokens = optional_page_tokens(input)?;
        if !page_tokens.is_empty() && !aggregate_request {
            return Err(input_error());
        }
        Ok(Self {
            calendar_id: calendar_id.unwrap_or("primary").to_string(),
            calendar_ids,
            include_all_calendars,
            time_min: optional_query_value(input, "time_min")?,
            time_max: optional_query_value(input, "time_max")?,
            page_token,
            page_tokens,
            max_results: optional_query_value(input, "max_results")?,
            query: optional_query_value(input, "query")?,
        })
    }

    fn requires_aggregate_response(&self) -> bool {
        self.include_all_calendars || !self.calendar_ids.is_empty()
    }

    fn target_calendar_ids(&self) -> Vec<String> {
        if self.calendar_ids.is_empty() {
            vec![self.calendar_id.clone()]
        } else {
            self.calendar_ids.clone()
        }
    }

    fn page_token_for(&self, calendar_id: &str) -> Option<&str> {
        self.page_tokens
            .get(calendar_id)
            .map(String::as_str)
            .or(self.page_token.as_deref())
    }
}

pub(super) async fn execute(
    request: &GsuiteDispatchRequest<'_>,
    credential: &GoogleCredential,
    stager: &dyn GsuiteCredentialStager,
    input: CalendarEventsQuery,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let mut run = CalendarListEventsRun::new(request, credential, stager);
    if !input.requires_aggregate_response() {
        let response = run
            .get(list_events_url(&input, &input.calendar_id)?)
            .await?;
        return Ok(super::response_outcome(
            response,
            run.network_egress_bytes(),
        ));
    }

    let (calendar_ids, calendars, calendar_discovery_truncated) =
        match resolve_calendar_ids(&mut run, &input).await? {
            CalendarIdResolution::Ready {
                calendar_ids,
                calendars,
                truncated,
            } => (calendar_ids, calendars, truncated),
            CalendarIdResolution::AuthExpired => {
                return Ok(CapabilityExecutionOutcome::AuthExpired {
                    network_egress_bytes: run.network_egress_bytes(),
                });
            }
            CalendarIdResolution::DiscoveryFailed { response } => {
                return Ok(CapabilityExecutionOutcome::Response {
                    response,
                    network_egress_bytes: run.network_egress_bytes(),
                });
            }
        };

    let mut items = Vec::new();
    let mut next_page_tokens = serde_json::Map::new();
    let mut partial_failures = Vec::new();
    for calendar_id in calendar_ids.iter().take(MAX_CALENDARS) {
        let response = run.get(list_events_url(&input, calendar_id)?).await?;
        if is_google_auth_expired_response(&response) {
            return Ok(CapabilityExecutionOutcome::AuthExpired {
                network_egress_bytes: run.network_egress_bytes(),
            });
        }
        let body = response_body_json(&response)
            .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
        if response.status != 200 {
            partial_failures.push(sanitized_partial_failure(
                calendar_id,
                response.status,
                &body,
            ));
            continue;
        }
        if let Some(token) = body.get("nextPageToken").and_then(Value::as_str) {
            next_page_tokens.insert(calendar_id.clone(), Value::String(token.to_string()));
        }
        if let Some(event_items) = body.get("items").and_then(Value::as_array) {
            for event in event_items {
                items.push(with_calendar_id(event.clone(), calendar_id));
            }
        }
    }
    items.sort_by_cached_key(event_start_key);

    let body = json!({
        "kind": "ironclaw#calendarEvents",
        "summary": "Google Calendar events",
        "calendarIds": calendar_ids,
        "calendars": calendars,
        "calendarDiscoveryTruncated": calendar_discovery_truncated,
        "items": items,
        "nextPageTokens": next_page_tokens,
        "partialFailures": partial_failures,
    });
    let response =
        synthesized_json_response(body, run.network_egress_bytes(), run.redaction_applied())
            .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
    Ok(CapabilityExecutionOutcome::Response {
        response,
        network_egress_bytes: run.network_egress_bytes(),
    })
}

struct CalendarListEventsRun<'a, 'request> {
    request: &'a GsuiteDispatchRequest<'request>,
    credential: &'a GoogleCredential,
    stager: &'a dyn GsuiteCredentialStager,
    credential_staged: bool,
    network_egress_bytes: u64,
    redaction_applied: bool,
}

impl<'a, 'request> CalendarListEventsRun<'a, 'request> {
    fn new(
        request: &'a GsuiteDispatchRequest<'request>,
        credential: &'a GoogleCredential,
        stager: &'a dyn GsuiteCredentialStager,
    ) -> Self {
        Self {
            request,
            credential,
            stager,
            // GsuiteExecutor stages once before capability execution starts.
            credential_staged: true,
            network_egress_bytes: 0,
            redaction_applied: false,
        }
    }

    async fn get(&mut self, url: String) -> Result<RuntimeHttpEgressResponse, GsuiteDispatchError> {
        self.stage_credential_if_needed().await?;
        self.credential_staged = false;
        let response = execute_runtime_http(
            runtime_request(
                self.request,
                self.credential.access_secret.clone(),
                NetworkMethod::Get,
                url,
                Vec::new(),
            ),
            Arc::clone(&self.request.runtime_http_egress),
        )
        .await
        .map_err(|error| add_network_usage(error, self.network_egress_bytes))?;
        self.network_egress_bytes = self
            .network_egress_bytes
            .saturating_add(response.request_bytes);
        self.redaction_applied |= response.redaction_applied;
        Ok(response)
    }

    fn network_egress_bytes(&self) -> u64 {
        self.network_egress_bytes
    }

    fn redaction_applied(&self) -> bool {
        self.redaction_applied
    }

    async fn stage_credential_if_needed(&mut self) -> Result<(), GsuiteDispatchError> {
        if self.credential_staged {
            return Ok(());
        }
        self.stager
            .stage(GsuiteCredentialStageRequest {
                source_scope: &self.credential.access_secret_scope,
                target_scope: self.request.scope,
                capability_id: self.request.capability_id,
                access_secret: &self.credential.access_secret,
            })
            .await
            .map_err(|error| {
                add_network_usage(
                    map_stage_error(error, self.credential.access_secret.clone()),
                    self.network_egress_bytes,
                )
            })
    }
}

enum CalendarIdResolution {
    Ready {
        calendar_ids: Vec<String>,
        calendars: Vec<Value>,
        truncated: bool,
    },
    AuthExpired,
    DiscoveryFailed {
        response: ironclaw_host_api::RuntimeHttpEgressResponse,
    },
}

async fn resolve_calendar_ids(
    run: &mut CalendarListEventsRun<'_, '_>,
    input: &CalendarEventsQuery,
) -> Result<CalendarIdResolution, GsuiteDispatchError> {
    if !input.include_all_calendars {
        return Ok(CalendarIdResolution::Ready {
            calendar_ids: input.target_calendar_ids(),
            calendars: Vec::new(),
            truncated: false,
        });
    }

    let mut calendars = Vec::new();
    let mut calendar_ids = Vec::new();
    let mut truncated = false;
    let mut page_token = None;
    loop {
        let response = run
            .get(list_calendars_page_url(page_token.as_deref()))
            .await?;
        if is_google_auth_expired_response(&response) {
            return Ok(CalendarIdResolution::AuthExpired);
        }
        if response.status != 200 {
            return Ok(CalendarIdResolution::DiscoveryFailed { response });
        }
        let body = response_body_json(&response)
            .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
        if let Some(items) = body.get("items").and_then(Value::as_array) {
            for calendar in items {
                if calendar_ids.len() >= MAX_CALENDARS {
                    truncated = true;
                    break;
                }
                let Some(id) = calendar.get("id").and_then(Value::as_str) else {
                    continue;
                };
                calendar_ids.push(id.to_string());
                calendars.push(calendar.clone());
            }
        }
        page_token = body
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if page_token.is_none() || calendar_ids.len() >= MAX_CALENDARS {
            if page_token.is_some() && calendar_ids.len() >= MAX_CALENDARS {
                truncated = true;
            }
            break;
        }
    }

    Ok(CalendarIdResolution::Ready {
        calendar_ids,
        calendars,
        truncated,
    })
}

fn list_events_url(
    input: &CalendarEventsQuery,
    calendar_id: &str,
) -> Result<String, GsuiteDispatchError> {
    Ok(calendar_events_collection_url(
        calendar_id,
        &list_events_query(input, calendar_id)?,
    ))
}

fn list_events_query(
    input: &CalendarEventsQuery,
    calendar_id: &str,
) -> Result<Vec<String>, GsuiteDispatchError> {
    let mut query = vec![
        "singleEvents=true".to_string(),
        "orderBy=startTime".to_string(),
    ];
    let page_token = input.page_token_for(calendar_id);
    let generated_time_min;
    let time_min = match input.time_min.as_deref() {
        Some(time_min) => Some(Cow::Borrowed(time_min)),
        None if page_token.is_some() => None,
        None => {
            generated_time_min = current_utc_rfc3339();
            Some(Cow::Owned(generated_time_min))
        }
    };
    push_optional_query(&mut query, "timeMin", time_min.as_deref());
    push_optional_query(&mut query, "timeMax", input.time_max.as_deref());
    push_optional_query(&mut query, "pageToken", page_token);
    push_optional_query(
        &mut query,
        "maxResults",
        Some(input.max_results.as_deref().unwrap_or(DEFAULT_MAX_RESULTS)),
    );
    push_optional_query(&mut query, "q", input.query.as_deref());
    Ok(query)
}

fn list_calendars_page_url(page_token: Option<&str>) -> String {
    let mut url = format!("{CALENDAR_API_BASE}/users/me/calendarList?maxResults=250");
    if let Some(page_token) = page_token {
        url.push_str("&pageToken=");
        url.push_str(&encode_percent(page_token));
    }
    url
}

fn optional_page_tokens(input: &Value) -> Result<HashMap<String, String>, GsuiteDispatchError> {
    let Some(value) = input.get("page_tokens") else {
        return Ok(HashMap::new());
    };
    let object = value.as_object().ok_or_else(input_error)?;
    if object.len() > MAX_CALENDARS {
        return Err(input_error());
    }
    object
        .iter()
        .map(|(calendar_id, token)| {
            let token = token.as_str().ok_or_else(input_error)?;
            if calendar_id.is_empty() || token.is_empty() {
                return Err(input_error());
            }
            Ok((calendar_id.clone(), token.to_string()))
        })
        .collect()
}

fn with_calendar_id(mut event: Value, calendar_id: &str) -> Value {
    if let Some(object) = event.as_object_mut() {
        object.insert(
            "calendarId".to_string(),
            Value::String(calendar_id.to_string()),
        );
        event
    } else {
        json!({ "calendarId": calendar_id, "event": event })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct EventStartKey {
    missing: bool,
    epoch_millis: i64,
    raw: String,
}

fn event_start_key(event: &Value) -> EventStartKey {
    let raw = event
        .get("start")
        .and_then(|start| {
            start
                .get("dateTime")
                .or_else(|| start.get("date"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_string();
    let epoch_millis = parse_event_start_epoch_millis(&raw);
    EventStartKey {
        missing: epoch_millis.is_none(),
        epoch_millis: epoch_millis.unwrap_or(i64::MAX),
        raw,
    }
}

fn parse_event_start_epoch_millis(raw: &str) -> Option<i64> {
    if let Ok(date_time) = DateTime::parse_from_rfc3339(raw) {
        return Some(date_time.timestamp_millis());
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date_time| date_time.and_utc().timestamp_millis())
}

fn sanitized_partial_failure(calendar_id: &str, status: u16, body: &Value) -> Value {
    let mut failure = serde_json::Map::from_iter([
        (
            "calendarId".to_string(),
            Value::String(calendar_id.to_string()),
        ),
        (
            "status".to_string(),
            Value::Number(serde_json::Number::from(status)),
        ),
    ]);
    if let Some(reason) = safe_google_error_reason(body) {
        failure.insert("reason".to_string(), Value::String(reason.to_string()));
    }
    Value::Object(failure)
}

fn safe_google_error_reason(body: &Value) -> Option<&str> {
    [
        body.pointer("/error/status").and_then(Value::as_str),
        body.pointer("/error/errors/0/reason")
            .and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|reason| SAFE_GOOGLE_ERROR_REASONS.contains(reason))
}

fn synthesized_json_response(
    body: Value,
    request_bytes: u64,
    redaction_applied: bool,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, GsuiteDispatchError> {
    let body = serde_json::to_vec(&body).map_err(|error| {
        tracing::debug!(?error, "failed to serialize synthesized GSuite response");
        GsuiteDispatchError::new(RuntimeDispatchErrorKind::OutputDecode)
    })?;
    Ok(ironclaw_host_api::RuntimeHttpEgressResponse {
        status: 200,
        headers: Vec::new(),
        request_bytes,
        response_bytes: body.len() as u64,
        body,
        saved_body: None,
        redaction_applied,
    })
}

fn current_utc_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
