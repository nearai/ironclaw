use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// Model-visible runtime context for one loop execution.
///
/// First slice carries only time. The #4149 plan adds capability posture,
/// scoped-path semantics, and subagent narrowing as additional fields
/// rendered into the same prompt section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopRuntimeContext {
    /// Instant this loop execution started. Rendered at minute precision.
    pub loop_started_at_utc: DateTime<Utc>,
    /// IANA timezone name for the user (e.g. "America/Los_Angeles") when
    /// known. Never a guessed host timezone.
    pub user_timezone: Option<String>,
}

impl LoopRuntimeContext {
    pub fn render_model_content(&self) -> String {
        let utc = self.loop_started_at_utc.format("%Y-%m-%dT%H:%MZ");
        let local = self
            .user_timezone
            .as_deref()
            .and_then(|name| name.parse::<Tz>().ok().map(|tz| (name, tz)))
            .map(|(name, tz)| {
                let local = self.loop_started_at_utc.with_timezone(&tz);
                format!("{} ({}, {})", utc, local.format("%H:%M %a"), name)
            });
        match local {
            Some(stamped) => format!(
                "Current date/time at loop start: {stamped}. This was captured when \
                 this loop started; for the precise current time use the time \
                 capability if it is visible."
            ),
            None => format!(
                "Current date/time at loop start: {utc}. The user's timezone is \
                 unknown - if local time matters, ask the user or use the time \
                 capability if it is visible."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn stamp() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 6, 11, 21, 32, 47)
            .unwrap()
    }

    #[test]
    fn renders_utc_and_local_when_timezone_known() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: Some("America/Los_Angeles".to_string()),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("2026-06-11T21:32Z"),
            "minute-truncated UTC: {text}"
        );
        assert!(text.contains("14:32 Thu"), "local time + weekday: {text}");
        assert!(text.contains("America/Los_Angeles"), "{text}");
        assert!(text.contains("time capability"), "{text}");
        assert!(!text.contains(":47"), "seconds must be truncated: {text}");
    }

    #[test]
    fn renders_unknown_timezone_fallback() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
        };
        let text = ctx.render_model_content();
        assert!(text.contains("2026-06-11T21:32Z"), "{text}");
        assert!(text.contains("timezone is unknown"), "{text}");
        assert!(text.contains("ask the user"), "{text}");
    }

    #[test]
    fn invalid_timezone_falls_back_to_unknown() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: Some("Not/A_Zone".to_string()),
        };
        let text = ctx.render_model_content();
        assert!(text.contains("2026-06-11T21:32Z"), "{text}");
        assert!(text.contains("timezone is unknown"), "{text}");
        assert!(
            !text.contains("Not/A_Zone"),
            "invalid tz must not render: {text}"
        );
    }
}
