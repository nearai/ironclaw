//! Helper functions for rendering job and thread status information.
//!
//! These helpers are used by the dashboard widget to display jobs and
//! engine threads. The sidebar widget that previously lived here has been
//! removed — all activity data now lives in the Dashboard tab.

use super::{JobStatus, ThreadStatus};

/// Compact type tag for engine thread types.
pub(crate) fn thread_type_tag(thread_type: &str) -> &str {
    match thread_type {
        "Foreground" => "[FG]",
        "Research" => "[R]",
        "Mission" => "[M]",
        _ => "[?]",
    }
}

/// Status icon for each job state.
pub(crate) fn job_icon(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "\u{25CB}",   // ○
        JobStatus::Running => "\u{25CF}",   // ●
        JobStatus::Completed => "\u{2713}", // ✓
        JobStatus::Failed => "\u{25CF}",    // ●
    }
}

/// Status icon for each thread state.
pub(crate) fn thread_icon(status: ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::Active => "\u{25CF}",    // ●
        ThreadStatus::Idle => "\u{25CB}",      // ○
        ThreadStatus::Completed => "\u{2713}", // ✓
        ThreadStatus::Failed => "\u{25CF}",    // ●
    }
}

/// Format a duration in seconds into a compact human-readable string.
pub(crate) fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{JobStatus, ThreadStatus};

    #[test]
    fn job_status_display() {
        assert_eq!(format!("{}", JobStatus::Pending), "pending");
        assert_eq!(format!("{}", JobStatus::Running), "running");
        assert_eq!(format!("{}", JobStatus::Completed), "done");
        assert_eq!(format!("{}", JobStatus::Failed), "failed");
    }

    #[test]
    fn thread_status_display() {
        assert_eq!(format!("{}", ThreadStatus::Active), "active");
        assert_eq!(format!("{}", ThreadStatus::Idle), "idle");
        assert_eq!(format!("{}", ThreadStatus::Completed), "done");
        assert_eq!(format!("{}", ThreadStatus::Failed), "failed");
    }

    #[test]
    fn format_uptime_seconds() {
        assert_eq!(format_uptime(30), "30s");
    }

    #[test]
    fn format_uptime_minutes() {
        assert_eq!(format_uptime(150), "2m");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(3720), "1h 2m");
    }

    #[test]
    fn format_uptime_exact_hour() {
        assert_eq!(format_uptime(7200), "2h");
    }

    #[test]
    fn job_icon_returns_correct_symbols() {
        assert_eq!(job_icon(JobStatus::Pending), "\u{25CB}");
        assert_eq!(job_icon(JobStatus::Running), "\u{25CF}");
        assert_eq!(job_icon(JobStatus::Completed), "\u{2713}");
        assert_eq!(job_icon(JobStatus::Failed), "\u{2717}");
    }

    #[test]
    fn thread_icon_returns_correct_symbols() {
        assert_eq!(thread_icon(ThreadStatus::Active), "\u{25CF}");
        assert_eq!(thread_icon(ThreadStatus::Idle), "\u{25CB}");
        assert_eq!(thread_icon(ThreadStatus::Completed), "\u{2713}");
        assert_eq!(thread_icon(ThreadStatus::Failed), "\u{2717}");
    }

    #[test]
    fn thread_type_tag_variants() {
        assert_eq!(thread_type_tag("Foreground"), "[FG]");
        assert_eq!(thread_type_tag("Research"), "[R]");
        assert_eq!(thread_type_tag("Mission"), "[M]");
        assert_eq!(thread_type_tag("Unknown"), "[?]");
    }
}
