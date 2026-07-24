//! The installation-state projection (one enum, every extension;
//! `docs/reborn/extension-runtime/overview.md` §6.1).
//!
//! This enum is the host-owned installation-lifecycle vocabulary. It lives in
//! `ironclaw_host_api` — the crate every Reborn system-service and the product
//! wire already depend on — so both the `ExtensionHost` (in
//! `ironclaw_extension_host`, which re-exports this type and writes the record
//! subset `{Installed, Active, Failed}`) and the product-facing extensions wire
//! (`ironclaw_product`) name the *same* enum without a new dependency
//! edge. No extension or vendor may introduce a state, so the definition is
//! generic and nothing downstream extends it.
//!
//! It is an **honest internal projection**, not a durable user lifecycle.
//! User state is derived from installation membership plus manifest-declared
//! personal setup readiness. The host persists only the working
//! subset it can prove — `Installed` (staged), `Active` (serving), and `Failed`
//! (activation failed, carries `last_error`) — while `Configured`, `Disabled`,
//! `Unsupported` are derived at projection time and `Removed` is an
//! action-response signal (removal deletes the record; it is never a resting
//! state).
//!
//! The companion auth-account state machine (§6.3) lives in `ironclaw_auth`
//! next to the engine that drives it (`ironclaw_auth::AuthAccountState`); the
//! two are re-exported together by `ironclaw_extension_host::state`.

use serde::{Deserialize, Serialize};

/// The installation-state projection (one enum, every extension).
///
/// ```text
///                     activate ok
///   Installed ─────────────────────────▶ Active
///      │  ▲                                 │
///      │  └────────── deactivate ───────────┘
///      │                                    │
///      │ activate fails (non-auth)          │ activate fails (non-auth)
///      ▼                                    ▼
///     Failed ◀─────────────────────────────┘   (carries last_error; no auto-retry)
///
///   Derived (never host-persisted): Configured (creds present, not active),
///   Disabled (user turned it off), Unsupported (runtime cannot serve).
///   Removed is an action-response signal only — removal drops the record.
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    /// Installed, not active, no required credentials outstanding.
    Installed,
    /// Installed with required credentials present but not yet active (derived).
    Configured,
    /// Enabled and serving (in the host active-set).
    Active,
    /// A runtime-internal disabled working record. User disable is removal,
    /// never this state.
    Disabled,
    /// Terminal non-auth activation failure (activation failed with a
    /// `last_error`). Does not auto-retry; distinct from pristine `Installed`.
    /// Auth-rejection failures are represented by the auth-account axis
    /// (`AuthAccountState`), not here.
    Failed,
    /// The runtime cannot service this extension's lifecycle.
    Unsupported,
    /// Action-response signal that a removal completed and dropped the record.
    /// Never a resting state.
    Removed,
}

impl InstallationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Configured => "configured",
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
            Self::Removed => "removed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_state_wire_form_matches_str() {
        for (state, expected) in [
            (InstallationState::Installed, "installed"),
            (InstallationState::Configured, "configured"),
            (InstallationState::Active, "active"),
            (InstallationState::Disabled, "disabled"),
            (InstallationState::Failed, "failed"),
            (InstallationState::Unsupported, "unsupported"),
            (InstallationState::Removed, "removed"),
        ] {
            assert_eq!(state.as_str(), expected);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::Value::String(expected.to_string())
            );
        }
    }
}
