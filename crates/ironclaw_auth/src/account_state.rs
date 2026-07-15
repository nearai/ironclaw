//! The auth-account state machine (one enum, every vendor).
//!
//! `docs/reborn/extension-runtime/overview.md` §6.3: the machine is owned by
//! the auth engine; recipes affect HTTP details only, never states or
//! transitions. The enum is re-exported by `ironclaw_extension_host::state`
//! so the two standard state machines stay discoverable together, but the
//! definition lives here with the engine that drives it.
//!
//! ```text
//! Disconnected ──start flow──▶ Authenticating ──callback ok──▶ Connected
//!       ▲                            │ TTL/denied/error              │
//!       │◀───────────────────────────┘                               │
//!       │                                     refresh failure/expiry ▼
//!       │◀────────── disconnect / removal ──────────── Connected / Expired
//! ```
//!
//! `Refreshing` is deliberately not a state: it is internal to the engine and
//! never observable on the wire. Neither is a `Revoking` window: disconnect and
//! removal delete the account synchronously (`Revoked`/`Missing` project to
//! `Disconnected`), so no in-progress revoking state is ever produced or
//! observed on the wire.

use serde::{Deserialize, Serialize};

use crate::credential::CredentialAccountStatus;
use crate::flow::AuthFlowStatus;

/// The auth-account state (one enum, every vendor; overview §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAccountState {
    Disconnected,
    Authenticating,
    Connected,
    Expired,
}

impl AuthAccountState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Authenticating => "authenticating",
            Self::Connected => "connected",
            Self::Expired => "expired",
        }
    }

    /// Whether a transition from `self` to `next` is legal (overview §6.3).
    ///
    /// Disconnect and removal delete the account synchronously and project to
    /// `Disconnected`; there is no observable `Revoking` window.
    pub fn can_transition_to(self, next: AuthAccountState) -> bool {
        use AuthAccountState::*;
        matches!(
            (self, next),
            (Disconnected, Authenticating)
                | (Authenticating, Connected)   // callback ok
                | (Authenticating, Disconnected) // TTL / denied / error
                | (Connected, Expired)          // refresh failure / expiry
                | (Connected, Disconnected)     // disconnect / removal
                | (Expired, Disconnected)       // disconnect / removal
                | (Expired, Authenticating) // re-auth from expired
        )
    }
}

/// Typed reason for the last transition into a non-`connected` state. The
/// wire carries exactly these categories; vendor response bodies are never
/// stored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAccountLastError {
    /// The auth flow's TTL elapsed before the vendor callback arrived.
    FlowExpired,
    /// The vendor denied authorization (user declined or scopes rejected).
    VendorDenied,
    /// Token exchange with the vendor failed.
    ExchangeFailed,
    /// On-demand refresh failed transiently.
    RefreshFailed,
    /// The vendor permanently revoked the grant (`invalid_grant`).
    GrantRevoked,
    /// The `api_key` validation probe rejected the stored key.
    ValidationProbeFailed,
    /// The credential was removed or never configured.
    CredentialMissing,
}

/// Projection of the durable account/flow records into the standard state
/// machine. Storage is reused (`CredentialAccountStatus` rows are the durable
/// representation); this mapping is total so the wire can expose exactly the
/// §6.3 enum without a second persisted state column.
pub fn project_auth_account_state(
    account_status: Option<CredentialAccountStatus>,
    active_flow_status: Option<AuthFlowStatus>,
) -> (AuthAccountState, Option<AuthAccountLastError>) {
    // A live (non-terminal) flow means the user is mid-authentication,
    // regardless of what an older account row says.
    if matches!(
        active_flow_status,
        Some(
            AuthFlowStatus::Pending
                | AuthFlowStatus::AwaitingUser
                | AuthFlowStatus::CallbackReceived
                | AuthFlowStatus::Completing
        )
    ) {
        return (AuthAccountState::Authenticating, None);
    }
    match account_status {
        Some(CredentialAccountStatus::Configured) => (AuthAccountState::Connected, None),
        Some(CredentialAccountStatus::Expired) => (
            AuthAccountState::Expired,
            Some(AuthAccountLastError::RefreshFailed),
        ),
        Some(CredentialAccountStatus::RefreshFailed) => (
            AuthAccountState::Expired,
            Some(AuthAccountLastError::RefreshFailed),
        ),
        Some(CredentialAccountStatus::Revoked) => (
            AuthAccountState::Disconnected,
            Some(AuthAccountLastError::GrantRevoked),
        ),
        Some(CredentialAccountStatus::Missing) => (
            AuthAccountState::Disconnected,
            Some(AuthAccountLastError::CredentialMissing),
        ),
        Some(CredentialAccountStatus::Inactive | CredentialAccountStatus::PendingSetup) | None => {
            match active_flow_status {
                Some(AuthFlowStatus::Expired) => (
                    AuthAccountState::Disconnected,
                    Some(AuthAccountLastError::FlowExpired),
                ),
                Some(AuthFlowStatus::Failed) => (
                    AuthAccountState::Disconnected,
                    Some(AuthAccountLastError::VendorDenied),
                ),
                _ => (AuthAccountState::Disconnected, None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_account_state_wire_form_matches_str() {
        for (state, expected) in [
            (AuthAccountState::Disconnected, "disconnected"),
            (AuthAccountState::Authenticating, "authenticating"),
            (AuthAccountState::Connected, "connected"),
            (AuthAccountState::Expired, "expired"),
        ] {
            assert_eq!(state.as_str(), expected);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::Value::String(expected.to_string())
            );
        }
    }

    #[test]
    fn legal_transitions_only() {
        use AuthAccountState::*;
        assert!(Disconnected.can_transition_to(Authenticating));
        assert!(Authenticating.can_transition_to(Connected));
        assert!(Authenticating.can_transition_to(Disconnected));
        assert!(Connected.can_transition_to(Expired));
        assert!(Connected.can_transition_to(Disconnected));
        assert!(Expired.can_transition_to(Disconnected));
        assert!(Expired.can_transition_to(Authenticating));
        // Illegal jumps.
        assert!(!Disconnected.can_transition_to(Connected));
        assert!(!Connected.can_transition_to(Authenticating));
        assert!(!Expired.can_transition_to(Connected));
    }

    #[test]
    fn projection_prefers_live_flow_then_account_status() {
        assert_eq!(
            project_auth_account_state(
                Some(CredentialAccountStatus::Configured),
                Some(AuthFlowStatus::AwaitingUser),
            ),
            (AuthAccountState::Authenticating, None)
        );
        assert_eq!(
            project_auth_account_state(Some(CredentialAccountStatus::Configured), None),
            (AuthAccountState::Connected, None)
        );
        assert_eq!(
            project_auth_account_state(Some(CredentialAccountStatus::RefreshFailed), None),
            (
                AuthAccountState::Expired,
                Some(AuthAccountLastError::RefreshFailed)
            )
        );
        assert_eq!(
            project_auth_account_state(Some(CredentialAccountStatus::Revoked), None),
            (
                AuthAccountState::Disconnected,
                Some(AuthAccountLastError::GrantRevoked)
            )
        );
        // Flow TTL expiry with no configured account lands in `disconnected`
        // with a typed reason (AUTH-10).
        assert_eq!(
            project_auth_account_state(None, Some(AuthFlowStatus::Expired)),
            (
                AuthAccountState::Disconnected,
                Some(AuthAccountLastError::FlowExpired)
            )
        );
        // Vendor denial lands in `disconnected` with a typed reason (AUTH-10).
        assert_eq!(
            project_auth_account_state(None, Some(AuthFlowStatus::Failed)),
            (
                AuthAccountState::Disconnected,
                Some(AuthAccountLastError::VendorDenied)
            )
        );
    }
}
