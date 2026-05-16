//! Loop-driver configuration for the assembled Reborn runtime.
//!
//! `RebornDriverConfig` describes which loop drivers are registered in the
//! turn-runner's `DriverRegistry` at boot and which one is the implicit
//! default. Per-turn driver selection happens through
//! `requested_run_profile` on `SendMessageOptions`; that string drives the
//! run-profile resolver which yields a `ResolvedRunProfile.loop_driver`
//! descriptor; the worker then looks the descriptor up in the registry.
//!
//! This DTO exists primarily so that:
//!
//! 1. The composition root has a single point that controls "which drivers
//!    are wired" — important when planned-driver / custom-driver work
//!    lands without rewriting `build_reborn_runtime`.
//! 2. Operators can pick their default driver per deployment (e.g. text-only
//!    chat vs. planned-tool-using) without changing code.
//! 3. The boundary surface for "how does the runtime know about its
//!    drivers" stays in DTO form (cli/config can populate it from TOML
//!    in the future) rather than escaping through Arc<dyn AgentLoopDriver>.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RebornDriverChoice {
    /// Text-only host-managed model reply driver (the only driver wired
    /// end-to-end in the current composition).
    TextOnly,
    /// Planned driver. Recognized as a valid choice today; composition
    /// emits a clear `NotYetWired` boot error when selected because the
    /// planned-driver host wiring is a separate slice (#3651, #3036 sub
    /// "harness composition over planned driver"). Listed here so that
    /// operator-facing config validation can accept the name without
    /// silently rewriting to TextOnly.
    Planned,
}

impl RebornDriverChoice {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TextOnly => "text_only",
            Self::Planned => "planned",
        }
    }
}

impl std::fmt::Display for RebornDriverChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Driver registry shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornDriverConfig {
    /// The driver to register first and use when a turn omits
    /// `requested_run_profile`.
    pub default: RebornDriverChoice,
    /// Additional drivers to register so they can be picked per-turn via
    /// `SendMessageOptions::requested_run_profile`. The `default` is
    /// implicitly registered; do not list it again here.
    pub additional: Vec<RebornDriverChoice>,
}

impl RebornDriverConfig {
    /// Text-only single-driver shape used by the standalone CLI today.
    pub const fn text_only_only() -> Self {
        Self {
            default: RebornDriverChoice::TextOnly,
            additional: Vec::new(),
        }
    }

    pub fn with_default(mut self, choice: RebornDriverChoice) -> Self {
        self.default = choice;
        self
    }

    pub fn with_additional(mut self, choice: RebornDriverChoice) -> Self {
        if choice != self.default && !self.additional.contains(&choice) {
            self.additional.push(choice);
        }
        self
    }

    /// Every driver this config asks the composition to register
    /// (default + additional, deduplicated).
    pub fn all_choices(&self) -> Vec<RebornDriverChoice> {
        let mut result = vec![self.default];
        for choice in &self.additional {
            if !result.contains(choice) {
                result.push(*choice);
            }
        }
        result
    }
}

impl Default for RebornDriverConfig {
    fn default() -> Self {
        Self::text_only_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_only_only_has_no_additional() {
        let config = RebornDriverConfig::text_only_only();
        assert_eq!(config.default, RebornDriverChoice::TextOnly);
        assert!(config.additional.is_empty());
        assert_eq!(config.all_choices(), vec![RebornDriverChoice::TextOnly]);
    }

    #[test]
    fn additional_does_not_duplicate_default() {
        let config = RebornDriverConfig::text_only_only()
            .with_additional(RebornDriverChoice::TextOnly);
        assert!(config.additional.is_empty());
    }

    #[test]
    fn additional_dedupes() {
        let config = RebornDriverConfig::text_only_only()
            .with_additional(RebornDriverChoice::Planned)
            .with_additional(RebornDriverChoice::Planned);
        assert_eq!(config.additional, vec![RebornDriverChoice::Planned]);
        assert_eq!(
            config.all_choices(),
            vec![RebornDriverChoice::TextOnly, RebornDriverChoice::Planned]
        );
    }

    #[test]
    fn wire_strings_are_stable() {
        assert_eq!(RebornDriverChoice::TextOnly.as_str(), "text_only");
        assert_eq!(RebornDriverChoice::Planned.as_str(), "planned");
    }
}
