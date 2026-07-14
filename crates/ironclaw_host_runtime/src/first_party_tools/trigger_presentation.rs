use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityDisplayText, CapabilityFinalReplyPresentation,
    truncate_capability_display_text,
};
use serde_json::Value;

use super::trigger_management::{
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID,
};

const ROUTINE_LIST_PREVIEW_LIMIT: usize = 10;
const ROUTINE_SUMMARY_MAX_BYTES: usize = 512;
const ROUTINE_PREVIEW_MAX_BYTES: usize = 4 * 1024;

const TRIGGER_CREATE_PROVIDER_ALIAS: &str = "builtin__trigger_create";
const TRIGGER_LIST_PROVIDER_ALIAS: &str = "builtin__trigger_list";
const TRIGGER_REMOVE_PROVIDER_ALIAS: &str = "builtin__trigger_remove";
const TRIGGER_PAUSE_PROVIDER_ALIAS: &str = "builtin__trigger_pause";
const TRIGGER_RESUME_PROVIDER_ALIAS: &str = "builtin__trigger_resume";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutineCapability {
    Create,
    List,
    Remove,
    Pause,
    Resume,
}

impl RoutineCapability {
    fn title(self) -> &'static str {
        match self {
            Self::List => "Routines",
            Self::Create | Self::Remove | Self::Pause | Self::Resume => "Routine",
        }
    }

    fn successful_output_summary(self) -> &'static str {
        match self {
            Self::Create => "Routine created",
            Self::List => "Routines listed",
            Self::Remove => "Routine removed",
            Self::Pause => "Routine paused",
            Self::Resume => "Routine resumed",
        }
    }

    fn output_summary(self, value: &Value) -> &'static str {
        let Some(field) = self.mutation_presence_field() else {
            return self.successful_output_summary();
        };
        match value.get(field).and_then(Value::as_bool) {
            Some(true) => self.successful_output_summary(),
            Some(false) => "Routine not found",
            None => "Routine status unavailable",
        }
    }

    fn mutation_presence_field(self) -> Option<&'static str> {
        match self {
            Self::Remove => Some("removed"),
            Self::Pause | Self::Resume => Some("updated"),
            Self::Create | Self::List => None,
        }
    }

    fn mutation_succeeded(self, value: &Value) -> bool {
        self.mutation_presence_field()
            .is_none_or(|field| value.get(field).and_then(Value::as_bool) == Some(true))
    }
}

/// Capability-owned routine input projection consumed by generic product
/// composition. This type deliberately contains display-safe data only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineInputPresentation {
    pub title: &'static str,
    pub subtitle: Option<String>,
    pub summary: CapabilityDisplayText,
}

/// Returns the capability-owned routine title for exact first-party ids and
/// the provider aliases emitted by the built-in provider.
pub fn routine_title(capability_id: &str) -> Option<&'static str> {
    routine_capability(capability_id).map(RoutineCapability::title)
}

/// Projects only allowlisted routine input fields for product display.
pub fn routine_input_presentation(
    capability_id: &str,
    value: &Value,
) -> Option<RoutineInputPresentation> {
    let operation = routine_capability(capability_id)?;
    let (subtitle, summary) = match operation {
        RoutineCapability::Create => {
            let subtitle = top_level_string(value, "name").map(bounded_value);
            let mut lines = Vec::with_capacity(3);
            if let Some(name) = subtitle.as_ref() {
                lines.push(format!("routine: {}", name.text));
            }
            if let Some(schedule) = routine_schedule_label(value.get("schedule")) {
                lines.push(format!("schedule: {schedule}"));
            }
            if let Some(timezone) = value
                .get("schedule")
                .and_then(|schedule| schedule.get("timezone"))
                .and_then(Value::as_str)
                .filter(|timezone| !timezone.is_empty())
            {
                lines.push(format!("timezone: {}", bounded_value(timezone).text));
            }
            let summary = if lines.is_empty() {
                bounded_summary("routine creation request")
            } else {
                bounded_summary(&lines.join("\n"))
            };
            (subtitle.map(|value| value.text), summary)
        }
        RoutineCapability::List => (None, bounded_summary("routine list request")),
        RoutineCapability::Remove => (None, bounded_summary("routine removal request")),
        RoutineCapability::Pause => (None, bounded_summary("routine pause request")),
        RoutineCapability::Resume => (None, bounded_summary("routine resume request")),
    };
    Some(RoutineInputPresentation {
        title: operation.title(),
        subtitle,
        summary,
    })
}

/// Projects the raw first-party trigger result into a bounded routine display
/// and a deterministic final-reply policy. Callers retain the raw result for
/// internal follow-up capability calls; product presentation consumes this
/// projection instead of reinterpreting trigger fields.
pub fn routine_output_presentation(
    capability_id: &str,
    output: &Value,
) -> Option<CapabilityDisplayOutputPreview> {
    let operation = routine_capability(capability_id)?;
    let mut truncated = false;
    let lines = match operation {
        RoutineCapability::List => routine_list_preview_lines(output, &mut truncated),
        RoutineCapability::Create
        | RoutineCapability::Remove
        | RoutineCapability::Pause
        | RoutineCapability::Resume => {
            routine_record_preview_lines(operation, output, &mut truncated)
        }
    };
    let summary = bounded_summary(operation.output_summary(output));
    truncated |= summary.truncated;
    let preview = truncate_capability_display_text(&lines.join("\n"), ROUTINE_PREVIEW_MAX_BYTES);
    truncated |= preview.truncated;
    let subtitle = if operation.mutation_succeeded(output) {
        output
            .get("trigger")
            .and_then(|trigger| top_level_string(trigger, "name"))
            .map(|name| bounded_value(name).text)
    } else {
        None
    };
    let final_reply_presentation = CapabilityFinalReplyPresentation::new(&preview.text);
    Some(CapabilityDisplayOutputPreview {
        output_summary: Some(summary.text),
        output_preview: preview.text,
        output_kind: "text".to_string(),
        subtitle,
        truncated,
        final_reply_presentation,
    })
}

fn routine_capability(capability_id: &str) -> Option<RoutineCapability> {
    match capability_id {
        TRIGGER_CREATE_CAPABILITY_ID | TRIGGER_CREATE_PROVIDER_ALIAS => {
            Some(RoutineCapability::Create)
        }
        TRIGGER_LIST_CAPABILITY_ID | TRIGGER_LIST_PROVIDER_ALIAS => Some(RoutineCapability::List),
        TRIGGER_REMOVE_CAPABILITY_ID | TRIGGER_REMOVE_PROVIDER_ALIAS => {
            Some(RoutineCapability::Remove)
        }
        TRIGGER_PAUSE_CAPABILITY_ID | TRIGGER_PAUSE_PROVIDER_ALIAS => {
            Some(RoutineCapability::Pause)
        }
        TRIGGER_RESUME_CAPABILITY_ID | TRIGGER_RESUME_PROVIDER_ALIAS => {
            Some(RoutineCapability::Resume)
        }
        _ => None,
    }
}

fn routine_record_preview_lines(
    operation: RoutineCapability,
    value: &Value,
    truncated: &mut bool,
) -> Vec<String> {
    let summary = operation.output_summary(value);
    let mut lines = vec![summary.to_string()];
    if !operation.mutation_succeeded(value) {
        return lines;
    }
    let Some(trigger) = value.get("trigger").filter(|trigger| trigger.is_object()) else {
        return lines;
    };
    if let Some(name) = top_level_string(trigger, "name") {
        let name = bounded_value(name);
        *truncated |= name.truncated;
        lines[0] = format!("{summary}: {}", name.text);
    }
    if matches!(
        operation,
        RoutineCapability::Create | RoutineCapability::Pause | RoutineCapability::Resume
    ) && let Some(schedule) = routine_schedule_label(trigger.get("schedule"))
    {
        lines.push(format!("Schedule: {schedule}"));
    }
    if matches!(
        operation,
        RoutineCapability::Create | RoutineCapability::Resume
    ) && let Some(next_run) = trigger
        .get("next_run_at")
        .and_then(Value::as_str)
        .and_then(format_utc_datetime)
    {
        lines.push(format!("Next run: {next_run}"));
    }
    lines
}

fn routine_list_preview_lines(value: &Value, truncated: &mut bool) -> Vec<String> {
    let Some(triggers) = value.get("triggers").and_then(Value::as_array) else {
        return vec!["Routines listed".to_string()];
    };
    let displayable = triggers
        .iter()
        .filter_map(|trigger| top_level_string(trigger, "name").map(|name| (trigger, name)))
        .collect::<Vec<_>>();
    let count = displayable.len();
    let visible_count = count.min(ROUTINE_LIST_PREVIEW_LIMIT);
    let overflow_count = usize::from(count > ROUTINE_LIST_PREVIEW_LIMIT);
    let mut lines = Vec::with_capacity(visible_count + overflow_count + 1);
    lines.push(match count {
        0 => "No routines found".to_string(),
        1 => "1 routine found".to_string(),
        count => format!("{count} routines found"),
    });
    for (trigger, name) in displayable.into_iter().take(ROUTINE_LIST_PREVIEW_LIMIT) {
        let name = bounded_value(name);
        *truncated |= name.truncated;
        let state = routine_state_label(trigger.get("state"));
        let schedule = routine_schedule_label(trigger.get("schedule"));
        let mut line = name.text;
        if state.is_some() || schedule.is_some() {
            line.push_str(" — ");
            if let Some(state) = state {
                line.push_str(state);
            }
            if let Some(schedule) = schedule {
                if state.is_some() {
                    line.push_str(", ");
                }
                line.push_str(schedule);
            }
        }
        lines.push(line);
    }
    if count > ROUTINE_LIST_PREVIEW_LIMIT {
        *truncated = true;
        lines.push(format!(
            "Showing first {ROUTINE_LIST_PREVIEW_LIMIT} routines"
        ));
    }
    lines
}

fn top_level_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn bounded_summary(value: &str) -> CapabilityDisplayText {
    truncate_capability_display_text(value, ROUTINE_SUMMARY_MAX_BYTES)
}

fn bounded_value(value: &str) -> CapabilityDisplayText {
    truncate_capability_display_text(value, ROUTINE_SUMMARY_MAX_BYTES / 2)
}

fn routine_schedule_label(schedule: Option<&Value>) -> Option<&'static str> {
    match schedule?.get("kind").and_then(Value::as_str)? {
        "cron" => Some("recurring"),
        "once" => Some("one-time"),
        _ => Some("scheduled"),
    }
}

fn routine_state_label(state: Option<&Value>) -> Option<&'static str> {
    match state?.as_str()? {
        "scheduled" => Some("active"),
        "paused" => Some("paused"),
        "completed" => Some("completed"),
        _ => Some("unknown"),
    }
}

fn format_utc_datetime(value: &str) -> Option<String> {
    let datetime = match chrono::DateTime::parse_from_rfc3339(value) {
        Ok(datetime) => datetime,
        Err(_) => {
            tracing::debug!("routine presentation omitted malformed next_run_at");
            return None;
        }
    };
    Some(
        datetime
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%d %H:%M UTC")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn third_party_suffixes_are_not_routine_capabilities() {
        for capability_id in ["acme.trigger_create", "acme__trigger_list", "trigger_pause"] {
            assert!(routine_title(capability_id).is_none(), "{capability_id}");
            assert!(
                routine_output_presentation(capability_id, &json!({})).is_none(),
                "{capability_id}"
            );
        }
    }

    #[test]
    fn list_filters_malformed_entries_before_counting_and_truncating() {
        let triggers = std::iter::once(json!({"trigger_id": "hidden-leading"}))
            .chain((0..11).map(|index| {
                json!({
                    "trigger_id": format!("hidden-{index}"),
                    "name": format!("Routine {index}"),
                    "state": "scheduled",
                    "schedule": {"kind": "cron", "expression": "0 8 * * *"}
                })
            }))
            .collect::<Vec<_>>();
        let presentation =
            routine_output_presentation(TRIGGER_LIST_CAPABILITY_ID, &json!({"triggers": triggers}))
                .expect("built-in routine projection");

        assert!(presentation.output_preview.contains("11 routines found"));
        assert!(presentation.output_preview.contains("Routine 0"));
        assert!(presentation.output_preview.contains("Routine 9"));
        assert!(!presentation.output_preview.contains("Routine 10"));
        assert!(
            presentation
                .output_preview
                .contains("Showing first 10 routines")
        );
        assert!(!presentation.output_preview.contains("hidden-"));
        assert!(!presentation.output_preview.contains("0 8 * * *"));
    }

    #[test]
    fn every_routine_verb_produces_a_deterministic_safe_final_reply() {
        for (capability_id, output, expected) in [
            (
                TRIGGER_CREATE_CAPABILITY_ID,
                json!({"trigger": {"name": "Morning weather", "schedule": {"kind": "cron", "expression": "0 8 * * *"}}}),
                "Routine created: Morning weather\nSchedule: recurring",
            ),
            (
                TRIGGER_LIST_CAPABILITY_ID,
                json!({"triggers": [{"trigger_id": "secret", "name": "Morning weather", "schedule": {"kind": "cron", "expression": "0 8 * * *"}}]}),
                "1 routine found\nMorning weather — recurring",
            ),
            (
                TRIGGER_REMOVE_CAPABILITY_ID,
                json!({"removed": true, "trigger": {"trigger_id": "secret", "name": "Morning weather"}}),
                "Routine removed: Morning weather",
            ),
            (
                TRIGGER_PAUSE_CAPABILITY_ID,
                json!({"updated": true, "trigger": {"trigger_id": "secret", "name": "Morning weather", "schedule": {"kind": "cron", "expression": "0 8 * * *"}}}),
                "Routine paused: Morning weather\nSchedule: recurring",
            ),
            (
                TRIGGER_RESUME_CAPABILITY_ID,
                json!({"updated": true, "trigger": {"trigger_id": "secret", "name": "Morning weather", "schedule": {"kind": "cron", "expression": "0 8 * * *"}}}),
                "Routine resumed: Morning weather\nSchedule: recurring",
            ),
        ] {
            let presentation = routine_output_presentation(capability_id, &output)
                .expect("built-in routine presentation");
            let final_reply = presentation
                .final_reply_presentation
                .expect("deterministic final reply");
            assert_eq!(final_reply.safe_reply(), expected);
            assert!(!final_reply.safe_reply().contains("secret"));
            assert!(!final_reply.safe_reply().contains("0 8 * * *"));
        }
    }

    #[test]
    fn mutation_final_replies_fail_closed_without_explicit_success() {
        for capability_id in [
            TRIGGER_REMOVE_CAPABILITY_ID,
            TRIGGER_PAUSE_CAPABILITY_ID,
            TRIGGER_RESUME_CAPABILITY_ID,
        ] {
            let presentation = routine_output_presentation(
                capability_id,
                &json!({"trigger": {"name": "Must not be confirmed"}}),
            )
            .expect("built-in routine presentation");
            assert_eq!(presentation.subtitle, None);
            assert_eq!(
                presentation
                    .final_reply_presentation
                    .expect("deterministic final reply")
                    .safe_reply(),
                "Routine status unavailable"
            );
        }
    }
}
