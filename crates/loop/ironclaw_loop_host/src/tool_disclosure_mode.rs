//! The on/off switch for progressive tool disclosure.
//!
//! Moved here from `ironclaw_turn_runner::runtime` with the disclosure decorator it
//! gates (WS3 runner sheds, PROPOSAL §6.7.3). The mode and the mechanism it
//! switches now live in one crate: `is_enabled()` is read at exactly one
//! production site — the runner's capability-port factory, deciding whether to
//! attach [`crate::ToolDisclosureCapabilityDecorator`] — so keeping the switch
//! in a different crate from the thing it switches was the split this shed
//! closes.

/// Environment variable that configures progressive disclosure.
pub const REBORN_TOOL_DISCLOSURE_ENV: &str = "REBORN_TOOL_DISCLOSURE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolDisclosureMode {
    /// Control arm: advertise every authorized schema.
    Off,
    /// Control arm: alphabetical preview and mandatory describe-style compact search results.
    Compact,
    /// Bounded complete signatures with the legacy alphabetical preview.
    Signatures,
    /// Production arm: namespace-aware preview and bounded signatures, without profile pins.
    #[default]
    Namespaces,
    /// Opt-in arm: namespace-aware preview, bounded signatures, and reviewed pins.
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

    /// Progressive tool disclosure defaults to namespace summaries for unset or empty
    /// configuration. Explicit `off` remains the rollback path, while
    /// unrecognized values fail closed to `Off`.
    fn from_raw(raw: Option<&str>) -> Self {
        match raw {
            Some(value) if value.eq_ignore_ascii_case("off") => Self::Off,
            Some(value) if value.eq_ignore_ascii_case("compact") => Self::Compact,
            Some(value) if value.eq_ignore_ascii_case("signatures") => Self::Signatures,
            Some(value) if value.eq_ignore_ascii_case("namespaces") => Self::Namespaces,
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

    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub(crate) fn includes_complete_signatures(self) -> bool {
        matches!(self, Self::Signatures | Self::Namespaces | Self::Bridged)
    }

    pub(crate) fn includes_namespace_summaries(self) -> bool {
        matches!(self, Self::Namespaces | Self::Bridged)
    }

    pub(crate) fn includes_profile_pins(self) -> bool {
        matches!(self, Self::Bridged)
    }
}

#[cfg(test)]
mod tests {
    use super::ToolDisclosureMode;

    #[test]
    fn tool_disclosure_mode_defaults_namespaces_with_off_kill_switch() {
        assert_eq!(
            ToolDisclosureMode::default(),
            ToolDisclosureMode::Namespaces
        );
        // Unset / empty use the production default. Invalid configuration and
        // explicit `off` fail closed to the rollback path.
        // `is_enabled()` is what gates whether the gateway attaches the decorator.
        assert!(
            ToolDisclosureMode::from_raw(None).is_enabled(),
            "unset must enable progressive disclosure"
        );
        assert!(ToolDisclosureMode::from_raw(Some("")).is_enabled());
        assert!(
            !ToolDisclosureMode::from_raw(Some("garbage")).is_enabled(),
            "unrecognized values must fail closed to Off"
        );
        for (raw, expected) in [
            ("compact", ToolDisclosureMode::Compact),
            ("COMPACT", ToolDisclosureMode::Compact),
            ("signatures", ToolDisclosureMode::Signatures),
            ("SIGNATURES", ToolDisclosureMode::Signatures),
            ("namespaces", ToolDisclosureMode::Namespaces),
            ("NAMESPACES", ToolDisclosureMode::Namespaces),
            ("bridged", ToolDisclosureMode::Bridged),
            ("BRIDGED", ToolDisclosureMode::Bridged),
        ] {
            assert_eq!(ToolDisclosureMode::from_raw(Some(raw)), expected);
        }
        assert!(
            !ToolDisclosureMode::from_raw(Some("off")).is_enabled(),
            "explicit REBORN_TOOL_DISCLOSURE=off disables disclosure"
        );
        assert!(ToolDisclosureMode::from_raw(Some("compact")).is_enabled());
        assert!(ToolDisclosureMode::from_raw(Some("signatures")).is_enabled());
        assert!(ToolDisclosureMode::from_raw(Some("namespaces")).is_enabled());
        assert!(!ToolDisclosureMode::Compact.includes_complete_signatures());
        assert!(ToolDisclosureMode::Signatures.includes_complete_signatures());
        assert!(!ToolDisclosureMode::Signatures.includes_namespace_summaries());
        assert!(ToolDisclosureMode::Namespaces.includes_namespace_summaries());
        assert!(!ToolDisclosureMode::Namespaces.includes_profile_pins());
        assert!(ToolDisclosureMode::Bridged.includes_profile_pins());
        // Per-variant gating is unchanged.
        assert!(!ToolDisclosureMode::Off.is_enabled());
        assert!(ToolDisclosureMode::Bridged.is_enabled());
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
