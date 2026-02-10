//! Google Calendar API v3 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::near::agent::host;
use crate::types::*;

const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";

/// Make a Google Calendar API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("{}/{}", CALENDAR_API_BASE, path);

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Google Calendar API: {} {}", method, path),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Google Calendar API returned status {}: {}",
            response.status, body_text
        ));
    }

    // DELETE returns no content
    if response.body.is_empty() {
        return Ok(String::new());
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

/// Parse an event from the API's JSON response.
fn parse_event(v: &serde_json::Value) -> Event {
    Event {
        id: v["id"].as_str().unwrap_or("").to_string(),
        summary: v["summary"].as_str().unwrap_or("(no title)").to_string(),
        description: v["description"].as_str().map(|s| s.to_string()),
        location: v["location"].as_str().map(|s| s.to_string()),
        start: parse_event_time(&v["start"]),
        end: parse_event_time(&v["end"]),
        status: v["status"].as_str().unwrap_or("confirmed").to_string(),
        html_link: v["htmlLink"].as_str().map(|s| s.to_string()),
        attendees: v["attendees"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|a| Attendee {
                        email: a["email"].as_str().unwrap_or("").to_string(),
                        display_name: a["displayName"].as_str().map(|s| s.to_string()),
                        response_status: a["responseStatus"].as_str().map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        organizer: v.get("organizer").map(|o| Organizer {
            email: o["email"].as_str().unwrap_or("").to_string(),
            display_name: o["displayName"].as_str().map(|s| s.to_string()),
        }),
    }
}

fn parse_event_time(v: &serde_json::Value) -> EventTime {
    EventTime {
        date: v["date"].as_str().map(|s| s.to_string()),
        date_time: v["dateTime"].as_str().map(|s| s.to_string()),
        time_zone: v["timeZone"].as_str().map(|s| s.to_string()),
    }
}

/// List events from a calendar.
pub fn list_events(
    calendar_id: &str,
    time_min: Option<&str>,
    time_max: Option<&str>,
    max_results: u32,
    query: Option<&str>,
) -> Result<ListEventsResult, String> {
    let mut params = vec![
        format!("maxResults={}", max_results),
        "singleEvents=true".to_string(),
        "orderBy=startTime".to_string(),
    ];

    if let Some(t) = time_min {
        params.push(format!("timeMin={}", url_encode(t)));
    }
    if let Some(t) = time_max {
        params.push(format!("timeMax={}", url_encode(t)));
    }
    if let Some(q) = query {
        params.push(format!("q={}", url_encode(q)));
    }

    let path = format!(
        "calendars/{}/events?{}",
        url_encode(calendar_id),
        params.join("&")
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let events = parsed["items"]
        .as_array()
        .map(|arr| arr.iter().map(parse_event).collect())
        .unwrap_or_default();

    Ok(ListEventsResult {
        events,
        next_page_token: parsed["nextPageToken"].as_str().map(|s| s.to_string()),
    })
}

/// Get a single event by ID.
pub fn get_event(calendar_id: &str, event_id: &str) -> Result<EventResult, String> {
    let path = format!(
        "calendars/{}/events/{}",
        url_encode(calendar_id),
        url_encode(event_id)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(EventResult {
        event: parse_event(&parsed),
    })
}

/// Parameters for creating a calendar event.
pub struct CreateEventParams<'a> {
    pub calendar_id: &'a str,
    pub summary: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub start_datetime: Option<&'a str>,
    pub end_datetime: Option<&'a str>,
    pub start_date: Option<&'a str>,
    pub end_date: Option<&'a str>,
    pub timezone: Option<&'a str>,
    pub attendees: &'a [String],
}

/// Create a new event.
pub fn create_event(p: &CreateEventParams<'_>) -> Result<EventResult, String> {
    let mut event = serde_json::json!({
        "summary": p.summary,
    });

    if let Some(desc) = p.description {
        event["description"] = serde_json::Value::String(desc.to_string());
    }
    if let Some(loc) = p.location {
        event["location"] = serde_json::Value::String(loc.to_string());
    }

    // Build start/end, preferring datetime over date
    if let Some(dt) = p.start_datetime {
        let mut start = serde_json::json!({ "dateTime": dt });
        if let Some(tz) = p.timezone {
            start["timeZone"] = serde_json::Value::String(tz.to_string());
        }
        event["start"] = start;
    } else if let Some(d) = p.start_date {
        event["start"] = serde_json::json!({ "date": d });
    } else {
        return Err("Either start_datetime or start_date is required".to_string());
    }

    if let Some(dt) = p.end_datetime {
        let mut end = serde_json::json!({ "dateTime": dt });
        if let Some(tz) = p.timezone {
            end["timeZone"] = serde_json::Value::String(tz.to_string());
        }
        event["end"] = end;
    } else if let Some(d) = p.end_date {
        event["end"] = serde_json::json!({ "date": d });
    } else {
        return Err("Either end_datetime or end_date is required".to_string());
    }

    if !p.attendees.is_empty() {
        event["attendees"] = serde_json::json!(p
            .attendees
            .iter()
            .map(|e| serde_json::json!({ "email": e }))
            .collect::<Vec<_>>());
    }

    let body = serde_json::to_string(&event).map_err(|e| e.to_string())?;
    let path = format!("calendars/{}/events", url_encode(p.calendar_id));

    let response = api_call("POST", &path, Some(&body))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(EventResult {
        event: parse_event(&parsed),
    })
}

/// Parameters for updating a calendar event.
pub struct UpdateEventParams<'a> {
    pub calendar_id: &'a str,
    pub event_id: &'a str,
    pub summary: Option<&'a str>,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub start_datetime: Option<&'a str>,
    pub end_datetime: Option<&'a str>,
    pub start_date: Option<&'a str>,
    pub end_date: Option<&'a str>,
    pub timezone: Option<&'a str>,
    pub attendees: Option<&'a [String]>,
}

/// Update an existing event (PATCH for partial updates).
pub fn update_event(p: &UpdateEventParams<'_>) -> Result<EventResult, String> {
    let mut patch = serde_json::json!({});

    if let Some(s) = p.summary {
        patch["summary"] = serde_json::Value::String(s.to_string());
    }
    if let Some(d) = p.description {
        patch["description"] = serde_json::Value::String(d.to_string());
    }
    if let Some(l) = p.location {
        patch["location"] = serde_json::Value::String(l.to_string());
    }

    if let Some(dt) = p.start_datetime {
        let mut start = serde_json::json!({ "dateTime": dt });
        if let Some(tz) = p.timezone {
            start["timeZone"] = serde_json::Value::String(tz.to_string());
        }
        patch["start"] = start;
    } else if let Some(d) = p.start_date {
        patch["start"] = serde_json::json!({ "date": d });
    }

    if let Some(dt) = p.end_datetime {
        let mut end = serde_json::json!({ "dateTime": dt });
        if let Some(tz) = p.timezone {
            end["timeZone"] = serde_json::Value::String(tz.to_string());
        }
        patch["end"] = end;
    } else if let Some(d) = p.end_date {
        patch["end"] = serde_json::json!({ "date": d });
    }

    if let Some(att) = p.attendees {
        patch["attendees"] = serde_json::json!(att
            .iter()
            .map(|e| serde_json::json!({ "email": e }))
            .collect::<Vec<_>>());
    }

    let body = serde_json::to_string(&patch).map_err(|e| e.to_string())?;
    let path = format!(
        "calendars/{}/events/{}",
        url_encode(p.calendar_id),
        url_encode(p.event_id)
    );

    let response = api_call("PATCH", &path, Some(&body))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(EventResult {
        event: parse_event(&parsed),
    })
}

/// Delete an event.
pub fn delete_event(calendar_id: &str, event_id: &str) -> Result<DeleteResult, String> {
    let path = format!(
        "calendars/{}/events/{}",
        url_encode(calendar_id),
        url_encode(event_id)
    );

    api_call("DELETE", &path, None)?;

    Ok(DeleteResult {
        deleted: true,
        event_id: event_id.to_string(),
    })
}

/// Minimal percent-encoding for URL path segments and query values.
fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX[(b >> 4) as usize]));
                encoded.push(char::from(HEX[(b & 0x0F) as usize]));
            }
        }
    }
    encoded
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";
