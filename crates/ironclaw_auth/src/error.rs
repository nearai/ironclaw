use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable sanitized auth error vocabulary for product surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorCode {
    #[error("unknown_or_expired_flow")]
    UnknownOrExpiredFlow,
    #[error("cross_scope_denied")]
    CrossScopeDenied,
    #[error("provider_denied")]
    ProviderDenied,
    #[error("token_exchange_failed")]
    TokenExchangeFailed,
    #[error("refresh_failed")]
    RefreshFailed,
    #[error("credential_missing")]
    CredentialMissing,
    #[error("account_selection_required")]
    AccountSelectionRequired,
    #[error("backend_unavailable")]
    BackendUnavailable,
    /// A compare-and-swap precondition failed; the caller should re-read and
    /// retry if appropriate.
    #[error("backend_conflict")]
    BackendConflict,
    #[error("malformed_callback")]
    MalformedCallback,
    #[error("canceled")]
    Canceled,
    #[error("flow_already_terminal")]
    FlowAlreadyTerminal,
    #[error("invalid_request")]
    InvalidRequest,
}

/// Product auth failures. Error messages are stable and sanitized; raw
/// provider bodies, raw tokens, and backend internals must not be stored here.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthProductError {
    #[error("auth flow is unknown or expired")]
    UnknownOrExpiredFlow,
    #[error("auth record belongs to another scope")]
    CrossScopeDenied,
    #[error("auth callback is malformed")]
    MalformedCallback,
    #[error("provider denied authorization")]
    ProviderDenied,
    #[error("token exchange failed")]
    TokenExchangeFailed,
    #[error("token refresh failed")]
    RefreshFailed,
    #[error("credential is missing")]
    CredentialMissing,
    #[error("account selection required")]
    AccountSelectionRequired,
    #[error("backend unavailable")]
    BackendUnavailable,
    /// A compare-and-swap precondition failed; the caller should re-read and
    /// retry if the operation is safe to retry.
    #[error("backend conflict (CAS precondition failed)")]
    BackendConflict,
    #[error("auth flow was canceled")]
    Canceled,
    #[error("auth flow is already terminal")]
    FlowAlreadyTerminal,
    #[error("invalid auth request: {reason}")]
    InvalidRequest { reason: String },
}

impl AuthProductError {
    pub(crate) fn invalid_request(reason: impl Into<String>) -> Self {
        Self::InvalidRequest {
            reason: reason.into(),
        }
    }

    pub fn code(&self) -> AuthErrorCode {
        match self {
            Self::UnknownOrExpiredFlow => AuthErrorCode::UnknownOrExpiredFlow,
            Self::CrossScopeDenied => AuthErrorCode::CrossScopeDenied,
            Self::MalformedCallback => AuthErrorCode::MalformedCallback,
            Self::ProviderDenied => AuthErrorCode::ProviderDenied,
            Self::TokenExchangeFailed => AuthErrorCode::TokenExchangeFailed,
            Self::RefreshFailed => AuthErrorCode::RefreshFailed,
            Self::CredentialMissing => AuthErrorCode::CredentialMissing,
            Self::AccountSelectionRequired => AuthErrorCode::AccountSelectionRequired,
            Self::BackendUnavailable => AuthErrorCode::BackendUnavailable,
            Self::BackendConflict => AuthErrorCode::BackendConflict,
            Self::Canceled => AuthErrorCode::Canceled,
            Self::FlowAlreadyTerminal => AuthErrorCode::FlowAlreadyTerminal,
            Self::InvalidRequest { .. } => AuthErrorCode::InvalidRequest,
        }
    }
}
