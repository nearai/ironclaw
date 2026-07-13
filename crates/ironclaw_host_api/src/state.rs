//! The installation lifecycle state machine (one enum, every extension;
//! `docs/reborn/extension-runtime/overview.md` §6.1).
//!
//! This enum is the host-owned installation-lifecycle vocabulary. It lives in
//! `ironclaw_host_api` — the crate every Reborn system-service and the product
//! wire already depend on — so both the `ExtensionHost` (the only writer, in
//! `ironclaw_extension_host`, which re-exports this type) and the product-facing
//! extensions wire (`ironclaw_product_workflow`) name the *same* enum without a
//! new dependency edge. No extension or vendor may introduce a state, so the
//! definition is generic and nothing downstream extends it.
//!
//! The companion auth-account state machine (§6.3) lives in `ironclaw_auth`
//! next to the engine that drives it (`ironclaw_auth::AuthAccountState`); the
//! two are re-exported together by `ironclaw_extension_host::state`.

use serde::{Deserialize, Serialize};

/// The installation lifecycle state (one enum, every extension).
///
/// ```text
/// Installed ──activate──▶ Activating ──publish──▶ Active
///     ▲                        │ failure                │
///     └────────────────────────┘                        │ deactivate/upgrade
///                                                       ▼
/// Removed ◀──done── Removing ◀──remove── Installed ◀── Deactivating (drain)
///                      │ cleanup failure
///                      ▼
///               RemovalPending ──retry──▶ Removing
/// ```
///
/// `Activating`, `Deactivating`, and `Removing` are transient and persisted,
/// so a crash mid-transition resumes deterministically at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    Installed,
    Activating,
    Active,
    Deactivating,
    Removing,
    RemovalPending,
    Removed,
}

impl InstallationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Activating => "activating",
            Self::Active => "active",
            Self::Deactivating => "deactivating",
            Self::Removing => "removing",
            Self::RemovalPending => "removal_pending",
            Self::Removed => "removed",
        }
    }

    /// Transient states resume deterministically at startup.
    pub fn is_transient(self) -> bool {
        matches!(self, Self::Activating | Self::Deactivating | Self::Removing)
    }

    /// The deterministic crash-resume target for a state observed at startup:
    /// a transient state resolves to where its interrupted operation must
    /// re-drive from.
    pub fn resume_target(self) -> InstallationState {
        match self {
            // Activation was interrupted before publish; it publishes nothing,
            // so resume from Installed and let activation re-drive.
            Self::Activating => Self::Installed,
            // Deactivation drains; resume as Active and re-run deactivate.
            Self::Deactivating => Self::Active,
            // Removal is idempotent and must complete; re-drive Removing.
            Self::Removing => Self::Removing,
            // Terminal/steady states resume as themselves.
            other => other,
        }
    }

    /// Whether a transition from `self` to `next` is legal (overview §6.1).
    pub fn can_transition_to(self, next: InstallationState) -> bool {
        use InstallationState::*;
        matches!(
            (self, next),
            (Installed, Activating)
                | (Activating, Active)
                | (Activating, Installed) // activation failure
                | (Active, Deactivating)
                | (Active, Removing) // remove while active runs deactivate-drain internally
                | (Deactivating, Installed)
                | (Installed, Removing)
                | (Removing, Removed)
                | (Removing, RemovalPending) // cleanup failure
                | (RemovalPending, Removing) // retry
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_state_wire_form_matches_str() {
        for (state, expected) in [
            (InstallationState::Installed, "installed"),
            (InstallationState::Activating, "activating"),
            (InstallationState::Active, "active"),
            (InstallationState::Deactivating, "deactivating"),
            (InstallationState::Removing, "removing"),
            (InstallationState::RemovalPending, "removal_pending"),
            (InstallationState::Removed, "removed"),
        ] {
            assert_eq!(state.as_str(), expected);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::Value::String(expected.to_string())
            );
        }
    }

    #[test]
    fn transient_states_resume_deterministically() {
        assert_eq!(
            InstallationState::Activating.resume_target(),
            InstallationState::Installed
        );
        assert_eq!(
            InstallationState::Deactivating.resume_target(),
            InstallationState::Active
        );
        assert_eq!(
            InstallationState::Removing.resume_target(),
            InstallationState::Removing
        );
        assert!(InstallationState::Activating.is_transient());
        assert!(!InstallationState::Active.is_transient());
        assert!(!InstallationState::RemovalPending.is_transient());
    }

    #[test]
    fn legal_transitions_only() {
        use InstallationState::*;
        assert!(Installed.can_transition_to(Activating));
        assert!(Activating.can_transition_to(Active));
        assert!(Activating.can_transition_to(Installed));
        assert!(Removing.can_transition_to(RemovalPending));
        assert!(RemovalPending.can_transition_to(Removing));
        // Illegal jumps.
        assert!(!Installed.can_transition_to(Active));
        assert!(!Active.can_transition_to(Removed));
        assert!(!Removed.can_transition_to(Active));
        assert!(!RemovalPending.can_transition_to(Active));
    }
}
