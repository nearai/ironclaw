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

    /// Parse a wire form produced by [`Self::as_str`]. The inverse is pinned by
    /// `installation_state_wire_form_matches_str`.
    pub fn from_wire(state: &str) -> Option<Self> {
        match state {
            "installed" => Some(Self::Installed),
            "configured" => Some(Self::Configured),
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

/// The public, user-actionable extension lifecycle state
/// (`docs/reborn/extension-runtime/overview.md` §6.1).
///
/// ```text
/// not a member                                        -> uninstalled
/// member + missing tenant setup, personal auth, or
///          pairing                                    -> setup_needed
/// member + every requirement ready                    -> active
/// ```
///
/// [`InstallationState`] is the *host's* internal checkpoint vocabulary
/// (`Installed`, `Configured`, `Disabled`, `Failed`, `Unsupported`, `Removed`)
/// used for recovery and diagnostics. Product surfaces must never expose those
/// checkpoints as additional user actions or resting states — there is no
/// public `installed` / `configured` / `failed` state and no Activate/Disable
/// action. Internal failures stay redacted diagnostics attached to
/// `setup_needed`; they never create a fourth product state.
///
/// [`Self::from_host_checkpoint`] collapses the checkpoint axis alone. A caller
/// -scoped surface must additionally fold in that caller's readiness (required
/// credentials, personal auth, channel pairing/connection) before publishing —
/// an extension whose host record is `Active` is still `setup_needed` for a
/// caller who has not connected it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePublicState {
    Uninstalled,
    SetupNeeded,
    Active,
}

impl LifecyclePublicState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uninstalled => "uninstalled",
            Self::SetupNeeded => "setup_needed",
            Self::Active => "active",
        }
    }

    /// Collapse a host-owned internal checkpoint onto the product contract.
    /// This is the checkpoint axis only — see the type docs.
    pub const fn from_host_checkpoint(state: InstallationState) -> Self {
        match state {
            InstallationState::Active => Self::Active,
            InstallationState::Removed => Self::Uninstalled,
            InstallationState::Installed
            | InstallationState::Configured
            | InstallationState::Disabled
            | InstallationState::Failed
            | InstallationState::Unsupported => Self::SetupNeeded,
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
            // The doc comment on `from_wire` claims this test pins the
            // inverse. It did not until now -- only `as_str` and the serde
            // form were asserted, so `from_wire` could drift from either.
            assert_eq!(
                InstallationState::from_wire(expected),
                Some(state),
                "from_wire must invert as_str for {expected}"
            );
        }
        assert_eq!(
            InstallationState::from_wire("not_a_state"),
            None,
            "an unknown wire value must not resolve to a checkpoint"
        );
    }
}
