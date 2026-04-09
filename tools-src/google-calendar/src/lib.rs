//! Google Calendar WASM Tool for IronClaw.
//!
//! Provides Google Calendar integration for viewing, creating, updating,
//! and deleting calendar events.
//!
//! # Capabilities Required
//!
//! - HTTP: `www.googleapis.com/calendar/v3/*` (GET, POST, PUT, PATCH, DELETE)
//! - Secrets: `google_oauth_token` (OAuth 2.0 token, injected automatically)
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
        // Derived from `GoogleCalendarAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract.
        let schema = schemars::schema_for!(types::GoogleCalendarAction);
        serde_json::to_string(&schema).expect("schema serialization is infallible")
    }

    fn description() -> String {
        "Google Calendar integration for viewing, creating, updating, and deleting calendar \
         events. Requires a Google Calendar OAuth token with the calendar.events scope. \
         Supports timed events, all-day events, attendees, locations, and free text search. \
         To discover all available API operations, use http GET to fetch \
         <https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest> (public, no auth needed)."
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
