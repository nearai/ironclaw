//! The on/off switch for progressive tool disclosure.
//!
//! Moved here from `ironclaw_runner::runtime` with the disclosure decorator it
//! gates (WS3 runner sheds, PROPOSAL §6.7.3). The mode and the mechanism it
//! switches now live in one crate: `is_bridged()` is read at exactly one
//! production site — the runner's capability-port factory, deciding whether to
//! attach [`crate::ToolDisclosureCapabilityDecorator`] — so keeping the switch
//! in a different crate from the thing it switches was the split this shed
//! closes.

/// Environment variable that configures progressive disclosure.
pub const REBORN_TOOL_DISCLOSURE_ENV: &str = "REBORN_TOOL_DISCLOSURE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolDisclosureMode {
    Off,
    #[default]
    Bridged,
}

impl ToolDisclosureMode {
    pub fn from_env() -> Self {
        match std::env::var(REBORN_TOOL_DISCLOSURE_ENV) {
            Ok(value) => Self::from_raw(Some(&value)),
            Err(std::env::VarError::NotPresent) => Self::from_raw(None),
            // Don't silently `.ok()`-drop a NotUnicode read: the var is set but
            // unreadable (a misconfiguration). Record it at the REPL-safe debug
            // level and fail closed even if the unset default changes later.
            Err(std::env::VarError::NotUnicode(_)) => {
                tracing::debug!(
                    target: "ironclaw::reborn::runtime",
                    env = REBORN_TOOL_DISCLOSURE_ENV,
                    "REBORN_TOOL_DISCLOSURE is set but not valid UTF-8; falling back to Off"
                );
                Self::Off
            }
        }
    }

    /// Progressive tool disclosure defaults to bridged for unset or empty
    /// configuration. Explicit `off` remains the rollback path, while
    /// unrecognized values fail closed to `Off`.
    fn from_raw(raw: Option<&str>) -> Self {
        match raw {
            Some(value) if value.eq_ignore_ascii_case("off") => Self::Off,
            Some(value) if value.eq_ignore_ascii_case("bridged") => Self::Bridged,
            Some(value) if !value.is_empty() => {
                tracing::debug!(
                    target: "ironclaw::reborn::runtime",
                    env = REBORN_TOOL_DISCLOSURE_ENV,
                    "unrecognized REBORN_TOOL_DISCLOSURE value; falling back to Off"
                );
                Self::Off
            }
            // Unset / empty follows the production default.
            _ => Self::default(),
        }
    }

    pub fn is_bridged(self) -> bool {
        matches!(self, Self::Bridged)
    }
}

#[cfg(test)]
mod tests {
    use super::ToolDisclosureMode;

    #[test]
    fn tool_disclosure_mode_defaults_bridged_with_off_kill_switch() {
        assert_eq!(ToolDisclosureMode::default(), ToolDisclosureMode::Bridged);
        // Unset / empty use the production default. Invalid configuration and
        // explicit `off` fail closed to the rollback path.
        // `is_bridged()` is what gates whether the gateway attaches the decorator.
        assert!(
            ToolDisclosureMode::from_raw(None).is_bridged(),
            "unset must enable progressive disclosure"
        );
        assert!(ToolDisclosureMode::from_raw(Some("")).is_bridged());
        assert!(
            !ToolDisclosureMode::from_raw(Some("garbage")).is_bridged(),
            "unrecognized values must fail closed to Off"
        );
        assert!(ToolDisclosureMode::from_raw(Some("bridged")).is_bridged());
        assert!(ToolDisclosureMode::from_raw(Some("BRIDGED")).is_bridged());
        assert!(
            !ToolDisclosureMode::from_raw(Some("off")).is_bridged(),
            "explicit REBORN_TOOL_DISCLOSURE=off disables disclosure"
        );
        // Per-variant gating is unchanged.
        assert!(!ToolDisclosureMode::Off.is_bridged());
        assert!(ToolDisclosureMode::Bridged.is_bridged());
    }

    #[cfg(unix)]
    #[test]
    fn tool_disclosure_mode_non_unicode_env_fails_closed() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::process::Command;

        use super::REBORN_TOOL_DISCLOSURE_ENV;

        const CHILD_MARKER: &str = "IRONCLAW_NON_UNICODE_DISCLOSURE_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_some() {
            assert_eq!(
                ToolDisclosureMode::from_env(),
                ToolDisclosureMode::Off,
                "non-Unicode configuration must fail closed"
            );
            return;
        }

        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                // Self-referential: the child re-runs *this* test by name. The
                // module path moved with the type (runner `runtime::tests::` ->
                // loop_host `tool_disclosure_mode::tests::`), so this literal
                // moved with it. It is the only content edit any moved test
                // carries in this shed.
                "tool_disclosure_mode::tests::tool_disclosure_mode_non_unicode_env_fails_closed",
                "--test-threads=1",
            ])
            .env(CHILD_MARKER, "1")
            .env(REBORN_TOOL_DISCLOSURE_ENV, OsString::from_vec(vec![0xff]))
            .output()
            .expect("spawn isolated non-Unicode environment test");
        assert!(
            output.status.success(),
            "isolated non-Unicode environment test failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
