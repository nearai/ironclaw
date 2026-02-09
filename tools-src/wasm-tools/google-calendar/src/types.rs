//! Types for Google Calendar API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Google Calendar tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GoogleCalendarAction {
    /// List events from a calendar.
    ListEvents {
        /// Calendar ID (default: "primary").
        #[serde(default = "default_calendar_id")]
        calendar_id: String,
        /// Lower bound (RFC3339 timestamp) for filtering by start time.
        #[serde(default)]
        time_min: Option<String>,
        /// Upper bound (RFC3339 timestamp) for filtering by end time.
        #[serde(default)]
        time_max: Option<String>,
        /// Maximum number of events to return (default: 25).
        #[serde(default = "default_max_results")]
        max_results: u32,
        /// Free text search terms to filter events.
        #[serde(default)]
        query: Option<String>,
    },

    /// Get a single event by ID.
    GetEvent {
        /// Calendar ID (default: "primary").
        #[serde(default = "default_calendar_id")]
        calendar_id: String,
        /// The event ID.
        event_id: String,
    },

    /// Create a new event.
    CreateEvent {
        /// Calendar ID (default: "primary").
        #[serde(default = "default_calendar_id")]
        calendar_id: String,
        /// Event title.
        summary: String,
        /// Event description.
        #[serde(default)]
        description: Option<String>,
        /// Event location.
        #[serde(default)]
        location: Option<String>,
        /// Start time as RFC3339 timestamp (e.g., "2025-01-15T09:00:00-05:00").
        /// For all-day events, use date format "2025-01-15" in `start_date` instead.
        #[serde(default)]
        start_datetime: Option<String>,
        /// End time as RFC3339 timestamp.
        #[serde(default)]
        end_datetime: Option<String>,
        /// Start date for all-day events (e.g., "2025-01-15").
        #[serde(default)]
        start_date: Option<String>,
        /// End date for all-day events (exclusive, e.g., "2025-01-16" for a single day).
        #[serde(default)]
        end_date: Option<String>,
        /// Timezone (e.g., "America/New_York"). Used with datetime fields.
        #[serde(default)]
        timezone: Option<String>,
        /// Attendee email addresses.
        #[serde(default)]
        attendees: Vec<String>,
    },

    /// Update an existing event (partial update via PATCH).
    UpdateEvent {
        /// Calendar ID (default: "primary").
        #[serde(default = "default_calendar_id")]
        calendar_id: String,
        /// The event ID to update.
        event_id: String,
        /// New event title.
        #[serde(default)]
        summary: Option<String>,
        /// New event description.
        #[serde(default)]
        description: Option<String>,
        /// New event location.
        #[serde(default)]
        location: Option<String>,
        /// New start datetime (RFC3339).
        #[serde(default)]
        start_datetime: Option<String>,
        /// New end datetime (RFC3339).
        #[serde(default)]
        end_datetime: Option<String>,
        /// New start date for all-day events.
        #[serde(default)]
        start_date: Option<String>,
        /// New end date for all-day events.
        #[serde(default)]
        end_date: Option<String>,
        /// Timezone for datetime fields.
        #[serde(default)]
        timezone: Option<String>,
        /// Replace attendees list with these email addresses.
        #[serde(default)]
        attendees: Option<Vec<String>>,
    },

    /// Delete an event.
    DeleteEvent {
        /// Calendar ID (default: "primary").
        #[serde(default = "default_calendar_id")]
        calendar_id: String,
        /// The event ID to delete.
        event_id: String,
    },
}

fn default_calendar_id() -> String {
    "primary".to_string()
}

fn default_max_results() -> u32 {
    25
}

/// A Google Calendar event.
#[derive(Debug, Serialize)]
pub struct Event {
    pub id: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub start: EventTime,
    pub end: EventTime,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_link: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizer: Option<Organizer>,
}

/// Event start/end time. Either `date` (all-day) or `date_time` (timed).
#[derive(Debug, Serialize)]
pub struct EventTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

/// An event attendee.
#[derive(Debug, Serialize)]
pub struct Attendee {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<String>,
}

/// Event organizer.
#[derive(Debug, Serialize)]
pub struct Organizer {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Result from list_events.
#[derive(Debug, Serialize)]
pub struct ListEventsResult {
    pub events: Vec<Event>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

/// Result from create/update operations.
#[derive(Debug, Serialize)]
pub struct EventResult {
    pub event: Event,
}

/// Result from delete_event.
#[derive(Debug, Serialize)]
pub struct DeleteResult {
    pub deleted: bool,
    pub event_id: String,
}
