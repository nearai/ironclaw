use std::{cmp::Ordering, sync::Arc};

use chrono::{DateTime, Duration, FixedOffset, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use futures_util::{StreamExt as _, stream};
use ironclaw_host_api::{
    NetworkMethod, RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse,
};
use regex::Regex;
use serde_json::{Value, json};

use crate::gsuite::credential::GoogleCredential;

use super::{
    CALENDAR_API_BASE, CapabilityExecutionOutcome, GMAIL_API_BASE, GsuiteDispatchError,
    GsuiteDispatchRequest, add_network_usage, calendar_events_collection_url, encode_percent,
    encode_segment, execute_runtime_http, input_error, is_google_auth_expired_response,
    optional_bool, optional_query_value, optional_str, optional_string_array, push_optional_query,
    response_body_json, runtime_request,
};

const DEFAULT_GMAIL_SUMMARY_LIMIT: u32 = 10;
const MAX_GMAIL_SUMMARY_LIMIT: u32 = 50;
const DEFAULT_AGENDA_LIMIT: u32 = 10;
const MAX_AGENDA_LIMIT: u32 = 100;
const DEFAULT_DAILY_BRIEF_EMAIL_LIMIT: u32 = 5;
const MAX_DAILY_BRIEF_EMAIL_LIMIT: u32 = 20;
const DEFAULT_PREVIEW_CHARS: usize = 500;
const MAX_PREVIEW_CHARS: usize = 4_000;
const MAX_CALENDARS: usize = 50;
const MAX_CONTEXT_TOOL_FANOUT_CONCURRENCY: usize = 8;
const DEFAULT_DAILY_BRIEF_EMAIL_QUERY: &str = "is:unread newer_than:7d";

pub(super) struct GmailFetchMessageSummariesInput {
    query: Option<String>,
    label_ids: Vec<String>,
    page_token: Option<String>,
    max_results: u32,
    body_preview_chars: usize,
}

impl GmailFetchMessageSummariesInput {
    pub(super) fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        Ok(Self {
            query: optional_query_value(input, "query")?,
            label_ids: optional_string_array(input, "label_ids")?,
            page_token: optional_query_value(input, "page_token")?,
            max_results: optional_u32(input, "max_results")?
                .unwrap_or(DEFAULT_GMAIL_SUMMARY_LIMIT)
                .clamp(1, MAX_GMAIL_SUMMARY_LIMIT),
            body_preview_chars: optional_usize(input, "body_preview_chars")?
                .unwrap_or(DEFAULT_PREVIEW_CHARS)
                .clamp(1, MAX_PREVIEW_CHARS),
        })
    }
}

pub(super) struct CalendarAgendaInput {
    calendar_id: Option<String>,
    calendar_ids: Vec<String>,
    include_all_calendars: bool,
    window: AgendaWindow,
    time_zone: FixedOffset,
    time_min: Option<String>,
    time_max: Option<String>,
    max_results: u32,
    query: Option<String>,
    description_chars: usize,
}

impl CalendarAgendaInput {
    pub(super) fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        let calendar_id = optional_str(input, "calendar_id")?.map(ToString::to_string);
        let calendar_ids = optional_string_array(input, "calendar_ids")?;
        if calendar_ids.len() > MAX_CALENDARS {
            return Err(input_error());
        }
        let include_all_calendars = optional_bool(input, "include_all_calendars")?.unwrap_or(false);
        let selector_count = usize::from(calendar_id.is_some())
            + usize::from(!calendar_ids.is_empty())
            + usize::from(include_all_calendars);
        if selector_count > 1 {
            return Err(input_error());
        }
        let time_min = optional_query_value(input, "time_min")?;
        let time_max = optional_query_value(input, "time_max")?;
        if time_min.is_some() != time_max.is_some() {
            return Err(input_error());
        }
        if let Some(time_min) = &time_min {
            parse_rfc3339_bound(time_min)?;
        }
        if let Some(time_max) = &time_max {
            parse_rfc3339_bound(time_max)?;
        }
        Ok(Self {
            calendar_id,
            calendar_ids,
            include_all_calendars,
            window: AgendaWindow::parse(optional_str(input, "window")?)?,
            time_zone: parse_fixed_offset(optional_str(input, "time_zone")?)?,
            time_min,
            time_max,
            max_results: optional_u32(input, "max_results")?
                .unwrap_or(DEFAULT_AGENDA_LIMIT)
                .clamp(1, MAX_AGENDA_LIMIT),
            query: optional_query_value(input, "query")?,
            description_chars: optional_usize(input, "description_chars")?
                .unwrap_or(DEFAULT_PREVIEW_CHARS)
                .clamp(0, MAX_PREVIEW_CHARS),
        })
    }
}

pub(super) struct CalendarMeetingPrepInput {
    agenda: CalendarAgendaInput,
    linked_resource_limit: usize,
}

impl CalendarMeetingPrepInput {
    pub(super) fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        let mut agenda = CalendarAgendaInput::parse(input)?;
        if input.get("window").is_none() && input.get("time_min").is_none() {
            agenda.window = AgendaWindow::Upcoming { days: 7 };
        }
        agenda.max_results = optional_u32(input, "max_results")?
            .unwrap_or(5)
            .clamp(1, 20);
        agenda.description_chars = optional_usize(input, "description_chars")?
            .unwrap_or(1_500)
            .clamp(0, MAX_PREVIEW_CHARS);
        Ok(Self {
            agenda,
            linked_resource_limit: optional_usize(input, "linked_resource_limit")?
                .unwrap_or(8)
                .clamp(0, 25),
        })
    }
}

pub(super) struct CalendarDailyBriefInput {
    agenda: CalendarAgendaInput,
    email_query: String,
    email_max_results: u32,
    body_preview_chars: usize,
}

impl CalendarDailyBriefInput {
    pub(super) fn parse(input: &Value) -> Result<Self, GsuiteDispatchError> {
        let mut agenda = CalendarAgendaInput::parse(input)?;
        let max_events = optional_u32(input, "max_events")?;
        let max_results = optional_u32(input, "max_results")?;
        if max_events.is_some() && max_results.is_some() {
            return Err(input_error());
        }
        agenda.max_results = max_events
            .or(max_results)
            .unwrap_or(agenda.max_results)
            .clamp(1, MAX_AGENDA_LIMIT);
        Ok(Self {
            agenda,
            email_query: optional_query_value(input, "email_query")?
                .unwrap_or_else(|| DEFAULT_DAILY_BRIEF_EMAIL_QUERY.to_string()),
            email_max_results: optional_u32(input, "email_max_results")?
                .unwrap_or(DEFAULT_DAILY_BRIEF_EMAIL_LIMIT)
                .clamp(1, MAX_DAILY_BRIEF_EMAIL_LIMIT),
            body_preview_chars: optional_usize(input, "body_preview_chars")?
                .unwrap_or(DEFAULT_PREVIEW_CHARS)
                .clamp(1, MAX_PREVIEW_CHARS),
        })
    }
}

pub(super) async fn execute_gmail_fetch_message_summaries(
    request: &GsuiteDispatchRequest<'_>,
    credential: &GoogleCredential,
    input: GmailFetchMessageSummariesInput,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let mut run = GoogleApiRun::new(request, credential);
    let body = fetch_gmail_summaries(&mut run, &input).await?;
    if let Some(auth_expired) = auth_expired_from_body(&body, run.network_egress_bytes()) {
        return Ok(auth_expired);
    }
    synthesized_outcome(body, &run)
}

pub(super) async fn execute_calendar_agenda(
    request: &GsuiteDispatchRequest<'_>,
    credential: &GoogleCredential,
    input: CalendarAgendaInput,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let mut run = GoogleApiRun::new(request, credential);
    let agenda = fetch_agenda(&mut run, &input, false).await?;
    if let Some(auth_expired) = auth_expired_from_body(&agenda, run.network_egress_bytes()) {
        return Ok(auth_expired);
    }
    synthesized_outcome(agenda, &run)
}

pub(super) async fn execute_calendar_meeting_prep(
    request: &GsuiteDispatchRequest<'_>,
    credential: &GoogleCredential,
    input: CalendarMeetingPrepInput,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let mut run = GoogleApiRun::new(request, credential);
    let agenda = fetch_agenda(&mut run, &input.agenda, true).await?;
    if let Some(auth_expired) = auth_expired_from_body(&agenda, run.network_egress_bytes()) {
        return Ok(auth_expired);
    }
    let events = agenda
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut meeting = events.first().cloned();
    let linked_resources = meeting
        .as_ref()
        .map(|event| linked_resources(event, input.linked_resource_limit))
        .unwrap_or_default();
    if let Some(fields) = meeting.as_mut().and_then(Value::as_object_mut) {
        fields.remove("_linkedResourceText");
    }
    let body = json!({
        "kind": "ironclaw#calendarMeetingPrep",
        "timeMin": agenda.get("timeMin").cloned().unwrap_or(Value::Null),
        "timeMax": agenda.get("timeMax").cloned().unwrap_or(Value::Null),
        "meeting": meeting,
        "linkedResources": linked_resources,
        "partialFailures": agenda.get("partialFailures").cloned().unwrap_or_else(|| json!([])),
    });
    synthesized_outcome(body, &run)
}

pub(super) async fn execute_calendar_daily_brief(
    request: &GsuiteDispatchRequest<'_>,
    credential: &GoogleCredential,
    input: CalendarDailyBriefInput,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let mut run = GoogleApiRun::new(request, credential);
    let agenda = fetch_agenda(&mut run, &input.agenda, false).await?;
    if let Some(auth_expired) = auth_expired_from_body(&agenda, run.network_egress_bytes()) {
        return Ok(auth_expired);
    }
    let email_input = GmailFetchMessageSummariesInput {
        query: Some(input.email_query),
        label_ids: Vec::new(),
        page_token: None,
        max_results: input.email_max_results,
        body_preview_chars: input.body_preview_chars,
    };
    let email_attention = fetch_gmail_summaries(&mut run, &email_input).await?;
    if let Some(auth_expired) = auth_expired_from_body(&email_attention, run.network_egress_bytes())
    {
        return Ok(auth_expired);
    }

    let mut partial_failures = scoped_partial_failures("calendar", &agenda);
    partial_failures.extend(scoped_partial_failures("gmail", &email_attention));
    let body = json!({
        "kind": "ironclaw#googleCalendarDailyBrief",
        "date": agenda.get("date").cloned().unwrap_or(Value::Null),
        "window": agenda.get("window").cloned().unwrap_or(Value::Null),
        "timeZone": agenda.get("timeZone").cloned().unwrap_or(Value::Null),
        "timeMin": agenda.get("timeMin").cloned().unwrap_or(Value::Null),
        "timeMax": agenda.get("timeMax").cloned().unwrap_or(Value::Null),
        "agenda": {
            "eventCount": agenda.get("eventCount").cloned().unwrap_or(Value::Null),
            "events": agenda.get("events").cloned().unwrap_or_else(|| json!([])),
            "calendarIds": agenda.get("calendarIds").cloned().unwrap_or_else(|| json!([])),
            "calendars": agenda.get("calendars").cloned().unwrap_or_else(|| json!([])),
        },
        "emailAttention": email_attention,
        "partialFailures": partial_failures,
    });
    synthesized_outcome(body, &run)
}

struct GoogleApiRun<'a, 'request> {
    request: &'a GsuiteDispatchRequest<'request>,
    credential: &'a GoogleCredential,
    network_egress_bytes: u64,
    redaction_applied: bool,
}

impl<'a, 'request> GoogleApiRun<'a, 'request> {
    fn new(request: &'a GsuiteDispatchRequest<'request>, credential: &'a GoogleCredential) -> Self {
        Self {
            request,
            credential,
            network_egress_bytes: 0,
            redaction_applied: false,
        }
    }

    fn prepare_get(&self, url: String) -> GoogleApiPreparedGet {
        GoogleApiPreparedGet {
            request: runtime_request(
                self.request,
                self.credential.access_secret.clone(),
                NetworkMethod::Get,
                url,
                Vec::new(),
            ),
            runtime_http_egress: Arc::clone(&self.request.runtime_http_egress),
        }
    }

    async fn get(&mut self, url: String) -> Result<RuntimeHttpEgressResponse, GsuiteDispatchError> {
        self.request(NetworkMethod::Get, url, Vec::new()).await
    }

    async fn request(
        &mut self,
        method: NetworkMethod,
        url: String,
        body: Vec<u8>,
    ) -> Result<RuntimeHttpEgressResponse, GsuiteDispatchError> {
        let response = execute_runtime_http(
            runtime_request(
                self.request,
                self.credential.access_secret.clone(),
                method,
                url,
                body,
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

    fn record_response_usage(&mut self, response: &RuntimeHttpEgressResponse) {
        self.network_egress_bytes = self
            .network_egress_bytes
            .saturating_add(response.request_bytes);
        self.redaction_applied |= response.redaction_applied;
    }
}

struct GoogleApiPreparedGet {
    request: RuntimeHttpEgressRequest,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
}

impl GoogleApiPreparedGet {
    async fn execute(self) -> Result<RuntimeHttpEgressResponse, GsuiteDispatchError> {
        execute_runtime_http(self.request, self.runtime_http_egress).await
    }
}

async fn fetch_gmail_summaries(
    run: &mut GoogleApiRun<'_, '_>,
    input: &GmailFetchMessageSummariesInput,
) -> Result<Value, GsuiteDispatchError> {
    let list_response = run.get(gmail_summary_list_url(input)).await?;
    if is_google_auth_expired_response(&list_response) {
        return Ok(auth_expired_marker());
    }
    let list_body = response_body_json(&list_response)
        .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
    if list_response.status != 200 {
        return Ok(json!({
            "kind": "ironclaw#gmailMessageSummaries",
            "query": input.query.clone(),
            "labelIds": input.label_ids.clone(),
            "resultSizeEstimate": Value::Null,
            "nextPageToken": Value::Null,
            "messages": [],
            "messageCount": 0,
            "unreadCount": 0,
            "partialFailures": [partial_failure("gmail_list", list_response.status, &list_body)],
        }));
    }
    let ids = message_ids(&list_body);
    let mut messages = Vec::new();
    let mut partial_failures = Vec::new();
    let mut requests = Vec::new();
    for (index, id) in ids.iter().take(input.max_results as usize).enumerate() {
        requests.push((index, id.clone(), run.prepare_get(gmail_metadata_url(id))));
    }
    let mut responses = stream::iter(requests)
        .map(|(index, id, request)| async move { (index, id, request.execute().await) })
        .buffer_unordered(MAX_CONTEXT_TOOL_FANOUT_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    responses.sort_by_key(|(index, _, _)| *index);
    let mut first_error = None;
    for (_, id, response) in responses {
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        run.record_response_usage(&response);
        if is_google_auth_expired_response(&response) {
            return Ok(auth_expired_marker());
        }
        let body = response_body_json(&response)
            .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
        if response.status != 200 {
            partial_failures.push(json!({
                "messageId": id,
                "status": response.status,
                "reason": safe_google_error_reason(&body),
            }));
            continue;
        }
        messages.push(compact_gmail_message(&body, input.body_preview_chars));
    }
    if let Some(error) = first_error {
        return Err(add_network_usage(error, run.network_egress_bytes()));
    }
    let unread_count = messages
        .iter()
        .filter(|message| {
            message
                .get("isUnread")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();

    let message_count = messages.len();
    Ok(json!({
        "kind": "ironclaw#gmailMessageSummaries",
        "query": input.query.clone(),
        "labelIds": input.label_ids.clone(),
        "resultSizeEstimate": list_body.get("resultSizeEstimate").cloned().unwrap_or(Value::Null),
        "nextPageToken": list_body.get("nextPageToken").cloned().unwrap_or(Value::Null),
        "messages": messages,
        "messageCount": message_count,
        "unreadCount": unread_count,
        "partialFailures": partial_failures,
    }))
}

async fn fetch_agenda(
    run: &mut GoogleApiRun<'_, '_>,
    input: &CalendarAgendaInput,
    include_link_source: bool,
) -> Result<Value, GsuiteDispatchError> {
    let (time_min, time_max, date_label) = agenda_bounds(input)?;
    let (calendar_ids, calendars, calendar_failure) = resolve_calendar_ids(run, input).await?;
    if let Some(response) = calendar_failure {
        if is_google_auth_expired_response(&response) {
            return Ok(auth_expired_marker());
        }
        let body = response_body_json(&response)
            .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
        return Ok(json!({
            "kind": "ironclaw#calendarAgenda",
            "date": date_label,
            "window": input.window.as_str(),
            "timeZone": format_fixed_offset(input.time_zone),
            "timeMin": time_min,
            "timeMax": time_max,
            "calendarIds": [],
            "calendars": [],
            "events": [],
            "eventCount": 0,
            "partialFailures": [partial_failure("calendar_discovery", response.status, &body)],
        }));
    }

    let mut events = Vec::new();
    let mut partial_failures = Vec::new();
    let mut urls = Vec::new();
    for (index, calendar_id) in calendar_ids.iter().take(MAX_CALENDARS).enumerate() {
        urls.push((
            index,
            calendar_id.clone(),
            agenda_events_url(input, calendar_id, &time_min, &time_max)?,
        ));
    }
    let mut requests = Vec::new();
    for (index, calendar_id, url) in urls {
        requests.push((index, calendar_id, run.prepare_get(url)));
    }
    let mut responses = stream::iter(requests)
        .map(|(index, calendar_id, request)| async move {
            (index, calendar_id, request.execute().await)
        })
        .buffer_unordered(MAX_CONTEXT_TOOL_FANOUT_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    responses.sort_by_key(|(index, _, _)| *index);
    let mut first_error = None;
    for (_, calendar_id, response) in responses {
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        run.record_response_usage(&response);
        if is_google_auth_expired_response(&response) {
            return Ok(auth_expired_marker());
        }
        let body = response_body_json(&response)
            .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
        if response.status != 200 {
            partial_failures.push(partial_failure(&calendar_id, response.status, &body));
            continue;
        }
        if let Some(items) = body.get("items").and_then(Value::as_array) {
            for event in items {
                events.push(compact_calendar_event(
                    event,
                    &calendar_id,
                    input.description_chars,
                    include_link_source,
                ));
            }
        }
    }
    if let Some(error) = first_error {
        return Err(add_network_usage(error, run.network_egress_bytes()));
    }
    events.sort_by(compare_event_start);
    events.truncate(input.max_results as usize);
    Ok(json!({
        "kind": "ironclaw#calendarAgenda",
        "date": date_label,
        "window": input.window.as_str(),
        "timeZone": format_fixed_offset(input.time_zone),
        "timeMin": time_min,
        "timeMax": time_max,
        "calendarIds": calendar_ids,
        "calendars": calendars,
        "events": events,
        "eventCount": events.len(),
        "partialFailures": partial_failures,
    }))
}

async fn resolve_calendar_ids(
    run: &mut GoogleApiRun<'_, '_>,
    input: &CalendarAgendaInput,
) -> Result<(Vec<String>, Vec<Value>, Option<RuntimeHttpEgressResponse>), GsuiteDispatchError> {
    if !input.include_all_calendars {
        if !input.calendar_ids.is_empty() {
            return Ok((input.calendar_ids.clone(), Vec::new(), None));
        }
        return Ok((
            vec![
                input
                    .calendar_id
                    .clone()
                    .unwrap_or_else(|| "primary".to_string()),
            ],
            Vec::new(),
            None,
        ));
    }

    let response = run
        .get(format!(
            "{CALENDAR_API_BASE}/users/me/calendarList?maxResults=250"
        ))
        .await?;
    if is_google_auth_expired_response(&response) || response.status != 200 {
        return Ok((Vec::new(), Vec::new(), Some(response)));
    }
    let body = response_body_json(&response)
        .map_err(|error| add_network_usage(error, run.network_egress_bytes()))?;
    let mut calendar_ids = Vec::new();
    let mut calendars = Vec::new();
    if let Some(items) = body.get("items").and_then(Value::as_array) {
        for calendar in items.iter().take(MAX_CALENDARS) {
            let Some(id) = calendar.get("id").and_then(Value::as_str) else {
                continue;
            };
            calendar_ids.push(id.to_string());
            calendars.push(json!({
                "id": id,
                "summary": calendar.get("summary").and_then(Value::as_str).unwrap_or(""),
                "primary": calendar.get("primary").and_then(Value::as_bool).unwrap_or(false),
            }));
        }
    }
    Ok((calendar_ids, calendars, None))
}

fn gmail_summary_list_url(input: &GmailFetchMessageSummariesInput) -> String {
    let mut query = vec![format!("maxResults={}", input.max_results)];
    push_optional_query(&mut query, "q", input.query.as_deref());
    push_optional_query(&mut query, "pageToken", input.page_token.as_deref());
    for label_id in &input.label_ids {
        query.push(format!("labelIds={}", encode_percent(label_id)));
    }
    format!("{GMAIL_API_BASE}/users/me/messages?{}", query.join("&"))
}

fn gmail_metadata_url(message_id: &str) -> String {
    format!(
        "{GMAIL_API_BASE}/users/me/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=To&metadataHeaders=Subject&metadataHeaders=Date",
        encode_segment(message_id)
    )
}

fn agenda_events_url(
    input: &CalendarAgendaInput,
    calendar_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<String, GsuiteDispatchError> {
    let mut query = vec![
        "singleEvents=true".to_string(),
        "orderBy=startTime".to_string(),
        format!("maxResults={}", input.max_results),
    ];
    push_optional_query(&mut query, "timeMin", Some(time_min));
    push_optional_query(&mut query, "timeMax", Some(time_max));
    push_optional_query(&mut query, "q", input.query.as_deref());
    Ok(calendar_events_collection_url(calendar_id, &query))
}

fn message_ids(body: &Value) -> Vec<String> {
    body.get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| message.get("id").and_then(Value::as_str))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn compact_gmail_message(message: &Value, body_preview_chars: usize) -> Value {
    let labels = message
        .get("labelIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let snippet = message.get("snippet").and_then(Value::as_str).unwrap_or("");
    json!({
        "id": message.get("id").and_then(Value::as_str).unwrap_or(""),
        "threadId": message.get("threadId").and_then(Value::as_str).unwrap_or(""),
        "from": gmail_header(message, "From"),
        "to": gmail_header(message, "To"),
        "subject": gmail_header(message, "Subject"),
        "date": gmail_header(message, "Date"),
        "snippet": snippet,
        "bodyPreview": truncate_chars(snippet, body_preview_chars),
        "labelIds": labels,
        "isUnread": labels.iter().any(|label| label.as_str() == Some("UNREAD")),
    })
}

fn gmail_header(message: &Value, name: &str) -> String {
    message
        .get("payload")
        .and_then(|payload| payload.get("headers"))
        .and_then(Value::as_array)
        .and_then(|headers| {
            headers.iter().find(|header| {
                header
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|candidate| candidate.eq_ignore_ascii_case(name))
                    .unwrap_or(false)
            })
        })
        .and_then(|header| header.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn compact_calendar_event(
    event: &Value,
    calendar_id: &str,
    description_chars: usize,
    include_link_source: bool,
) -> Value {
    let description = event
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let attendees = event
        .get("attendees")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(12)
                .map(compact_attendee)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut compact = json!({
        "id": event.get("id").and_then(Value::as_str).unwrap_or(""),
        "calendarId": calendar_id,
        "summary": event.get("summary").and_then(Value::as_str).unwrap_or("(No title)"),
        "start": event_time(event, "start"),
        "end": event_time(event, "end"),
        "location": event.get("location").and_then(Value::as_str).unwrap_or(""),
        "htmlLink": event.get("htmlLink").and_then(Value::as_str).unwrap_or(""),
        "hangoutLink": event.get("hangoutLink").and_then(Value::as_str).unwrap_or(""),
        "descriptionPreview": truncate_chars(description, description_chars),
        "attendeeCount": event.get("attendees").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "attendees": attendees,
    });
    if include_link_source && let Some(fields) = compact.as_object_mut() {
        fields.insert(
            "_linkedResourceText".to_string(),
            Value::String(linked_resource_text(event, description)),
        );
    }
    compact
}

fn compact_attendee(attendee: &Value) -> Value {
    json!({
        "email": attendee.get("email").and_then(Value::as_str).unwrap_or(""),
        "displayName": attendee.get("displayName").and_then(Value::as_str).unwrap_or(""),
        "responseStatus": attendee.get("responseStatus").and_then(Value::as_str).unwrap_or(""),
        "organizer": attendee.get("organizer").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn event_time(event: &Value, key: &str) -> Value {
    event
        .get(key)
        .and_then(|time| time.get("dateTime").or_else(|| time.get("date")))
        .cloned()
        .unwrap_or(Value::Null)
}

fn compare_event_start(left: &Value, right: &Value) -> Ordering {
    match (event_start_instant(left), event_start_instant(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => sortable_time(left).cmp(&sortable_time(right)),
    }
}

fn sortable_time(event: &Value) -> String {
    event
        .get("start")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn event_start_instant(event: &Value) -> Option<DateTime<Utc>> {
    let value = sortable_time(event);
    DateTime::parse_from_rfc3339(&value)
        .map(|time| time.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|time| Utc.from_utc_datetime(&time))
        })
}

fn linked_resources(event: &Value, limit: usize) -> Vec<Value> {
    let text = event
        .get("_linkedResourceText")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            [
                event
                    .get("descriptionPreview")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                event.get("location").and_then(Value::as_str).unwrap_or(""),
                event.get("htmlLink").and_then(Value::as_str).unwrap_or(""),
                event
                    .get("hangoutLink")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ]
            .join("\n")
        });
    let Ok(re) = Regex::new(r#"https://[^\s<>"']+"#) else {
        return Vec::new();
    };
    re.find_iter(&text)
        .take(limit)
        .map(|mat| {
            let url = mat.as_str();
            json!({
                "url": url,
                "kind": linked_resource_kind(url),
                "id": google_resource_id(url),
            })
        })
        .collect()
}

fn linked_resource_text(event: &Value, description: &str) -> String {
    [
        description,
        event.get("location").and_then(Value::as_str).unwrap_or(""),
        event.get("htmlLink").and_then(Value::as_str).unwrap_or(""),
        event
            .get("hangoutLink")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ]
    .join("\n")
}

fn linked_resource_kind(url: &str) -> &'static str {
    if url.contains("docs.google.com/document/") {
        "google_doc"
    } else if url.contains("docs.google.com/spreadsheets/") {
        "google_sheet"
    } else if url.contains("docs.google.com/presentation/") {
        "google_slide"
    } else if url.contains("drive.google.com/") {
        "google_drive"
    } else {
        "url"
    }
}

fn google_resource_id(url: &str) -> Option<String> {
    resource_id_after_marker(url, "/d/")
        .or_else(|| resource_id_after_marker(url, "/drive/folders/"))
        .or_else(|| query_param(url, "id"))
}

fn resource_id_after_marker(url: &str, marker: &str) -> Option<String> {
    let start = url.find(marker)? + marker.len();
    let tail = &url[start..];
    let id = tail.split(['/', '?', '&', '#']).next().unwrap_or("");
    non_empty_id(id)
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        let Some((candidate, value)) = pair.split_once('=') else {
            continue;
        };
        if candidate == key {
            return non_empty_id(value);
        }
    }
    None
}

fn non_empty_id(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn agenda_bounds(
    input: &CalendarAgendaInput,
) -> Result<(String, String, String), GsuiteDispatchError> {
    if let (Some(time_min), Some(time_max)) = (&input.time_min, &input.time_max) {
        return Ok((time_min.clone(), time_max.clone(), date_label(time_min)));
    }
    let now = Utc::now().with_timezone(&input.time_zone);
    let today = now.date_naive();
    let midnight = NaiveTime::from_hms_opt(0, 0, 0).ok_or_else(input_error)?;
    let (start, end) = match input.window {
        AgendaWindow::Today => (
            local_datetime(input.time_zone, today, midnight)?,
            local_datetime(input.time_zone, today + Duration::days(1), midnight)?,
        ),
        AgendaWindow::Tomorrow => (
            local_datetime(input.time_zone, today + Duration::days(1), midnight)?,
            local_datetime(input.time_zone, today + Duration::days(2), midnight)?,
        ),
        AgendaWindow::Week => (
            local_datetime(input.time_zone, today, midnight)?,
            local_datetime(input.time_zone, today + Duration::days(7), midnight)?,
        ),
        AgendaWindow::Upcoming { days } => (now, now + Duration::days(days.into())),
    };
    Ok((
        start.to_rfc3339(),
        end.to_rfc3339(),
        start.date_naive().to_string(),
    ))
}

fn local_datetime(
    offset: FixedOffset,
    date: NaiveDate,
    time: NaiveTime,
) -> Result<DateTime<FixedOffset>, GsuiteDispatchError> {
    match offset.from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(value) => Ok(value),
        _ => Err(input_error()),
    }
}

#[derive(Clone, Copy)]
enum AgendaWindow {
    Today,
    Tomorrow,
    Week,
    Upcoming { days: u32 },
}

impl AgendaWindow {
    fn parse(value: Option<&str>) -> Result<Self, GsuiteDispatchError> {
        match value.unwrap_or("today") {
            "today" => Ok(Self::Today),
            "tomorrow" => Ok(Self::Tomorrow),
            "week" | "this_week" => Ok(Self::Week),
            "upcoming" => Ok(Self::Upcoming { days: 7 }),
            _ => Err(input_error()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Tomorrow => "tomorrow",
            Self::Week => "week",
            Self::Upcoming { .. } => "upcoming",
        }
    }
}

fn parse_fixed_offset(value: Option<&str>) -> Result<FixedOffset, GsuiteDispatchError> {
    let Some(value) = value else {
        return FixedOffset::east_opt(0).ok_or_else(input_error);
    };
    if matches!(value, "UTC" | "Z" | "+00:00" | "-00:00") {
        return FixedOffset::east_opt(0).ok_or_else(input_error);
    }
    let sign = match value.as_bytes().first().copied() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return Err(input_error()),
    };
    let Some((hours, minutes)) = value[1..].split_once(':') else {
        return Err(input_error());
    };
    let hours: i32 = hours.parse().map_err(|error| {
        tracing::debug!(?error, time_zone = value, "invalid time zone hour");
        input_error()
    })?;
    let minutes: i32 = minutes.parse().map_err(|error| {
        tracing::debug!(?error, time_zone = value, "invalid time zone minute");
        input_error()
    })?;
    if hours > 23 || minutes > 59 {
        return Err(input_error());
    }
    FixedOffset::east_opt(sign * ((hours * 60 + minutes) * 60)).ok_or_else(input_error)
}

fn parse_rfc3339_bound(value: &str) -> Result<(), GsuiteDispatchError> {
    DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|error| {
            tracing::debug!(?error, value, "invalid RFC3339 calendar bound");
            input_error()
        })
}

fn format_fixed_offset(offset: FixedOffset) -> String {
    let seconds = offset.local_minus_utc();
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.abs();
    format!("{sign}{:02}:{:02}", seconds / 3600, (seconds % 3600) / 60)
}

fn optional_u32(input: &Value, key: &str) -> Result<Option<u32>, GsuiteDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    if let Some(number) = value.as_u64() {
        return u32::try_from(number).map(Some).map_err(|error| {
            tracing::debug!(?error, key, number, "numeric input is outside u32 range");
            input_error()
        });
    }
    if let Some(text) = value.as_str() {
        return text.parse::<u32>().map(Some).map_err(|error| {
            tracing::debug!(?error, key, text, "invalid numeric string input");
            input_error()
        });
    }
    Err(input_error())
}

fn optional_usize(input: &Value, key: &str) -> Result<Option<usize>, GsuiteDispatchError> {
    optional_u32(input, key).map(|value| value.map(|number| number as usize))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn partial_failure(scope: &str, status: u16, body: &Value) -> Value {
    json!({
        "scope": scope,
        "status": status,
        "reason": safe_google_error_reason(body),
    })
}

fn scoped_partial_failures(source: &str, body: &Value) -> Vec<Value> {
    body.get("partialFailures")
        .and_then(Value::as_array)
        .map(|failures| {
            failures
                .iter()
                .map(|failure| {
                    let mut failure = failure.clone();
                    if let Value::Object(fields) = &mut failure {
                        fields
                            .entry("source")
                            .or_insert_with(|| Value::String(source.to_string()));
                    }
                    failure
                })
                .collect()
        })
        .unwrap_or_default()
}

fn safe_google_error_reason(body: &Value) -> Option<String> {
    body.pointer("/error/status")
        .or_else(|| body.pointer("/error/errors/0/reason"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn date_label(time_min: &str) -> String {
    time_min.split('T').next().unwrap_or(time_min).to_string()
}

fn auth_expired_marker() -> Value {
    json!({"authExpired": true})
}

fn auth_expired_from_body(
    body: &Value,
    network_egress_bytes: u64,
) -> Option<CapabilityExecutionOutcome> {
    body.get("authExpired")
        .and_then(Value::as_bool)
        .and_then(|expired| {
            expired.then_some(CapabilityExecutionOutcome::AuthExpired {
                network_egress_bytes,
            })
        })
}

fn synthesized_outcome(
    body: Value,
    run: &GoogleApiRun<'_, '_>,
) -> Result<CapabilityExecutionOutcome, GsuiteDispatchError> {
    let response =
        synthesized_json_response(body, run.network_egress_bytes(), run.redaction_applied)?;
    Ok(CapabilityExecutionOutcome::Response {
        response,
        network_egress_bytes: run.network_egress_bytes(),
    })
}

fn synthesized_json_response(
    body: Value,
    request_bytes: u64,
    redaction_applied: bool,
) -> Result<RuntimeHttpEgressResponse, GsuiteDispatchError> {
    let body = serde_json::to_vec(&body).map_err(|error| {
        tracing::debug!(
            ?error,
            "failed to serialize synthesized GSuite context response"
        );
        GsuiteDispatchError::new(RuntimeDispatchErrorKind::OutputDecode)
    })?;
    Ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: Vec::new(),
        request_bytes,
        response_bytes: body.len() as u64,
        body,
        saved_body: None,
        redaction_applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_resource_id_stops_before_query_string() {
        assert_eq!(
            google_resource_id("https://docs.google.com/document/d/doc-123/edit?tab=t.0"),
            Some("doc-123".to_string())
        );
    }

    #[test]
    fn google_resource_id_extracts_drive_folder_and_query_id() {
        assert_eq!(
            google_resource_id("https://drive.google.com/drive/folders/folder-123?usp=sharing"),
            Some("folder-123".to_string())
        );
        assert_eq!(
            google_resource_id("https://drive.google.com/open?id=file-456&usp=drive_link"),
            Some("file-456".to_string())
        );
        assert_eq!(
            google_resource_id("https://drive.google.com/open?authuser&id=file-456"),
            Some("file-456".to_string())
        );
    }

    #[test]
    fn event_start_sort_uses_rfc3339_instants_before_string_fallback() {
        let earlier = json!({"start": "2026-01-01T00:30:00+02:00"});
        let later = json!({"start": "2025-12-31T23:00:00+00:00"});

        assert_eq!(compare_event_start(&earlier, &later), Ordering::Less);
    }

    #[test]
    fn calendar_agenda_input_rejects_one_sided_time_bounds() {
        let input = json!({"time_min": "2026-01-01T00:00:00Z"});

        assert!(CalendarAgendaInput::parse(&input).is_err());
    }

    #[test]
    fn calendar_agenda_input_rejects_invalid_time_bound_format() {
        let input = json!({
            "time_min": "2026-01-01",
            "time_max": "2026-01-02T00:00:00Z",
        });

        assert!(CalendarAgendaInput::parse(&input).is_err());
    }

    #[test]
    fn calendar_daily_brief_input_rejects_ambiguous_event_limits() {
        let input = json!({
            "max_events": 5,
            "max_results": 6,
        });

        assert!(CalendarDailyBriefInput::parse(&input).is_err());
    }

    #[test]
    fn parse_fixed_offset_accepts_utc_and_fixed_offsets() {
        assert_eq!(
            format_fixed_offset(parse_fixed_offset(Some("UTC")).unwrap()),
            "+00:00"
        );
        assert_eq!(
            format_fixed_offset(parse_fixed_offset(Some("+03:30")).unwrap()),
            "+03:30"
        );
        assert_eq!(
            format_fixed_offset(parse_fixed_offset(Some("-04:00")).unwrap()),
            "-04:00"
        );
    }

    #[test]
    fn agenda_window_accepts_supported_aliases() {
        assert_eq!(
            AgendaWindow::parse(Some("this_week")).unwrap().as_str(),
            "week"
        );
        assert!(AgendaWindow::parse(Some("month")).is_err());
    }
}
