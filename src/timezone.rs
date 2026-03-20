//! Timezone resolution and utilities.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

/// Resolve the effective timezone from a priority chain.
///
/// Priority: client_tz > user_setting > config_default > UTC
pub fn resolve_timezone(
    client_tz: Option<&str>,
    user_setting: Option<&str>,
    config_default: &str,
) -> Tz {
    // Try each in priority order, skipping invalid values
    for candidate in [client_tz, user_setting, Some(config_default)] {
        if let Some(tz) = candidate.and_then(parse_timezone) {
            return tz;
        }
    }
    Tz::UTC
}

/// Parse a timezone string (IANA name) into a `Tz`.
pub fn parse_timezone(s: &str) -> Option<Tz> {
    s.parse::<Tz>().ok()
}

/// Get today's date in the given timezone.
pub fn today_in_tz(tz: Tz) -> NaiveDate {
    Utc::now().with_timezone(&tz).date_naive()
}

/// Get the current time in the given timezone.
pub fn now_in_tz(tz: Tz) -> DateTime<Tz> {
    Utc::now().with_timezone(&tz)
}

/// Detect the system's timezone, falling back to UTC.
pub fn detect_system_timezone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|s| parse_timezone(&s))
        .unwrap_or(Tz::UTC)
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;

    use super::*;

    #[test]
    fn test_resolve_client_wins() {
        let tz = resolve_timezone(Some("America/New_York"), Some("Europe/London"), "UTC");
        assert_eq!(tz, chrono_tz::America::New_York);
    }

    #[test]
    fn test_resolve_user_setting_fallback() {
        let tz = resolve_timezone(None, Some("Europe/London"), "UTC");
        assert_eq!(tz, chrono_tz::Europe::London);
    }

    #[test]
    fn test_resolve_config_fallback() {
        let tz = resolve_timezone(None, None, "Asia/Tokyo");
        assert_eq!(tz, chrono_tz::Asia::Tokyo);
    }

    #[test]
    fn test_resolve_all_none_utc() {
        let tz = resolve_timezone(None, None, "UTC");
        assert_eq!(tz, Tz::UTC);
    }

    #[test]
    fn test_resolve_invalid_client_skipped() {
        let tz = resolve_timezone(Some("Fake/Zone"), Some("Europe/London"), "UTC");
        assert_eq!(tz, chrono_tz::Europe::London);
    }

    #[test]
    fn test_parse_valid() {
        assert_eq!(
            parse_timezone("America/Chicago"),
            Some(chrono_tz::America::Chicago)
        );
    }

    #[test]
    fn test_parse_invalid() {
        assert_eq!(parse_timezone("Fake/Zone"), None);
    }

    #[test]
    fn test_detect_system_tz() {
        // Should always return a valid Tz (at minimum UTC)
        let tz = detect_system_timezone();
        let _ = now_in_tz(tz); // Should not panic
    }

    #[test]
    fn test_today_in_tz_returns_valid_date() {
        let date = today_in_tz(Tz::UTC);
        // Verify it returns a valid date (year, month, day are all positive)
        assert!(date.year() > 0);
        assert!((1..=12).contains(&date.month()));
        assert!((1..=31).contains(&date.day()));
    }
}
