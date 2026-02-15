//! Google Calendar WASM Tool for IronClaw.
//!
//! Provides Google Calendar integration for viewing, creating, updating,
//! and deleting calendar events.
//!
//! # Capabilities Required
//!
//! - HTTP: `www.googleapis.com/calendar/v3/*` (GET, POST, PUT, PATCH, DELETE)
//! - Secrets: `google_calendar_token` (OAuth 2.0 token, injected automatically)
//!
//! # Supported Actions
//!
//! - `list_events`: List upcoming events with optional time range and search
//! - `get_event`: Get a specific event by ID
//! - `create_event`: Create a new calendar event
//! - `update_event`: Update an existing event (partial update)
//! - `delete_event`: Delete an event
//!
//! # Example Usage
//!
//! ```json
//! {"action": "list_events", "time_min": "2025-01-15T00:00:00Z", "max_results": 10}
//! ```

mod api;
mod types;

use types::GoogleCalendarAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct GoogleCalendarTool;

impl exports::near::agent::tool::Guest for GoogleCalendarTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_events", "get_event", "create_event", "update_event", "delete_event"],
                    "description": "The calendar operation to perform"
                },
                "calendar_id": {
                    "type": "string",
                    "description": "Calendar ID (default: 'primary')",
                    "default": "primary"
                },
                "event_id": {
                    "type": "string",
                    "description": "Event ID. Required for: get_event, update_event, delete_event"
                },
                "time_min": {
                    "type": "string",
                    "description": "Lower bound for event start time (RFC3339, e.g., '2025-01-15T00:00:00Z'). Used by: list_events"
                },
                "time_max": {
                    "type": "string",
                    "description": "Upper bound for event end time (RFC3339). Used by: list_events"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of events to return (default: 25). Used by: list_events",
                    "default": 25
                },
                "query": {
                    "type": "string",
                    "description": "Free text search terms to filter events. Used by: list_events"
                },
                "summary": {
                    "type": "string",
                    "description": "Event title. Required for: create_event. Optional for: update_event"
                },
                "description": {
                    "type": "string",
                    "description": "Event description. Used by: create_event, update_event"
                },
                "location": {
                    "type": "string",
                    "description": "Event location. Used by: create_event, update_event"
                },
                "start_datetime": {
                    "type": "string",
                    "description": "Start time (RFC3339, e.g., '2025-01-15T09:00:00-05:00'). For all-day events use start_date. Used by: create_event, update_event"
                },
                "end_datetime": {
                    "type": "string",
                    "description": "End time (RFC3339). For all-day events use end_date. Used by: create_event, update_event"
                },
                "start_date": {
                    "type": "string",
                    "description": "Start date for all-day events (e.g., '2025-01-15'). Used by: create_event, update_event"
                },
                "end_date": {
                    "type": "string",
                    "description": "End date for all-day events (exclusive, e.g., '2025-01-16'). Used by: create_event, update_event"
                },
                "timezone": {
                    "type": "string",
                    "description": "Timezone (e.g., 'America/New_York'). Used by: create_event, update_event"
                },
                "attendees": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Attendee email addresses. Used by: create_event, update_event"
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Google Calendar integration for viewing, creating, updating, and deleting calendar \
         events. Requires a Google Calendar OAuth token with the calendar.events scope. \
         Supports timed events, all-day events, attendees, locations, and free text search."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("google_oauth_token") {
        return Err(
            "Google OAuth token not configured. Run `ironclaw tool auth google-calendar` \
             to set up OAuth, or set the GOOGLE_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: GoogleCalendarAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Google Calendar action: {:?}", action),
    );

    let result = match action {
        GoogleCalendarAction::ListEvents {
            calendar_id,
            time_min,
            time_max,
            max_results,
            query,
        } => {
            let result = api::list_events(
                &calendar_id,
                time_min.as_deref(),
                time_max.as_deref(),
                max_results,
                query.as_deref(),
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleCalendarAction::GetEvent {
            calendar_id,
            event_id,
        } => {
            let result = api::get_event(&calendar_id, &event_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleCalendarAction::CreateEvent {
            calendar_id,
            summary,
            description,
            location,
            start_datetime,
            end_datetime,
            start_date,
            end_date,
            timezone,
            attendees,
        } => {
            let result = api::create_event(&api::CreateEventParams {
                calendar_id: &calendar_id,
                summary: &summary,
                description: description.as_deref(),
                location: location.as_deref(),
                start_datetime: start_datetime.as_deref(),
                end_datetime: end_datetime.as_deref(),
                start_date: start_date.as_deref(),
                end_date: end_date.as_deref(),
                timezone: timezone.as_deref(),
                attendees: &attendees,
            })?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleCalendarAction::UpdateEvent {
            calendar_id,
            event_id,
            summary,
            description,
            location,
            start_datetime,
            end_datetime,
            start_date,
            end_date,
            timezone,
            attendees,
        } => {
            let result = api::update_event(&api::UpdateEventParams {
                calendar_id: &calendar_id,
                event_id: &event_id,
                summary: summary.as_deref(),
                description: description.as_deref(),
                location: location.as_deref(),
                start_datetime: start_datetime.as_deref(),
                end_datetime: end_datetime.as_deref(),
                start_date: start_date.as_deref(),
                end_date: end_date.as_deref(),
                timezone: timezone.as_deref(),
                attendees: attendees.as_deref(),
            })?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleCalendarAction::DeleteEvent {
            calendar_id,
            event_id,
        } => {
            let result = api::delete_event(&calendar_id, &event_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

export!(GoogleCalendarTool);
