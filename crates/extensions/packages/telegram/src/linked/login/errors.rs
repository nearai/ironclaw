//! Mapping Telegram's failures onto the closed device-link vocabulary.
//!
//! Split out of `login.rs` because it is a table, not a flow — and because the
//! judgement it encodes is the one a reviewer most needs to read on its own:
//! `restartable` decides whether the card offers the user another attempt, and
//! getting it wrong is invisible in manual testing (a terminal failure looks
//! like a retryable one until the user has tried five times).

use grammers_client::InvocationError;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkError, DeviceLinkErrorCode, DeviceLinkStep,
};

use crate::linked::session_store::SessionStoreError;
use crate::linked::transport::TransportError;

/// Terminal vendor conditions that must stop a scan rather than back off.
///
/// `None` means "keep polling with backoff". Only the three conditions a user
/// can act on end the scan: the account cannot be linked, the vendor declined,
/// or the token window closed. Everything else — including a 5xx — is a
/// transient the poll loop already handles.
pub(super) fn fatal_step(error: &TransportError) -> Option<DeviceLinkStep> {
    let TransportError::Rpc { code, name, .. } = error else {
        return None;
    };
    let (mapped, restartable) = rpc_disposition(name, *code);
    match mapped {
        DeviceLinkErrorCode::AccountUnavailable | DeviceLinkErrorCode::Declined => {
            Some(DeviceLinkStep::Failed {
                code: mapped,
                restartable,
            })
        }
        DeviceLinkErrorCode::Expired => Some(DeviceLinkStep::Failed {
            code: mapped,
            restartable: true,
        }),
        _ => None,
    }
}

pub(super) fn custody_error(error: SessionStoreError) -> DeviceLinkError {
    match error {
        SessionStoreError::Custody(custody) => DeviceLinkError::Custody(custody),
        SessionStoreError::CasExhausted { .. } => DeviceLinkError::Internal {
            reason: "the linked session could not be stored without clobbering a concurrent write",
        },
        SessionStoreError::Blob(_) => DeviceLinkError::Internal {
            reason: "the stored linked session could not be read",
        },
        SessionStoreError::RefusedDcOption { .. } => DeviceLinkError::Internal {
            reason: "telegram named an unacceptable datacenter address",
        },
        SessionStoreError::Poisoned => DeviceLinkError::Internal {
            reason: "the linked session mirror lock was poisoned",
        },
    }
}

pub(super) fn vendor_error(error: TransportError) -> DeviceLinkError {
    match &error {
        TransportError::Rpc { code, name, .. } => {
            let (mapped, restartable) = rpc_disposition(name, *code);
            DeviceLinkError::Vendor {
                code: mapped,
                restartable,
            }
        }
        TransportError::Session => DeviceLinkError::Internal {
            reason: "linked-session custody failed underneath a telegram request",
        },
        _ => DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::VendorUnavailable,
            restartable: true,
        },
    }
}

pub(super) fn invocation_error(error: InvocationError) -> DeviceLinkError {
    match error {
        InvocationError::Rpc(rpc) => {
            let (code, restartable) = rpc_disposition(&rpc.name, rpc.code);
            DeviceLinkError::Vendor { code, restartable }
        }
        _ => DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::VendorUnavailable,
            restartable: true,
        },
    }
}

/// Map a Telegram error name onto the closed host vocabulary.
///
/// The distinction that matters most is `restartable`: a rate limit can
/// succeed on a later attempt, a deactivated account never can, and prompting
/// forever for the latter is the behaviour §4.5 exists to forbid.
pub(super) fn rpc_disposition(name: &str, code: i32) -> (DeviceLinkErrorCode, bool) {
    if name.starts_with("FLOOD_WAIT")
        || name.ends_with("_FLOOD")
        || name == "PHONE_PASSWORD_FLOOD"
        || code == 420
    {
        return (DeviceLinkErrorCode::RateLimited, true);
    }
    match name {
        "USER_DEACTIVATED"
        | "USER_DEACTIVATED_BAN"
        | "PHONE_NUMBER_BANNED"
        | "AUTH_KEY_DUPLICATED"
        | "UPDATE_APP_TO_LOGIN" => (DeviceLinkErrorCode::AccountUnavailable, false),
        "AUTH_TOKEN_EXPIRED" | "PHONE_CODE_EXPIRED" => (DeviceLinkErrorCode::Expired, true),
        "AUTH_TOKEN_INVALID"
        | "AUTH_TOKEN_ALREADY_ACCEPTED"
        | "SESSION_REVOKED"
        | "AUTH_KEY_UNREGISTERED" => (DeviceLinkErrorCode::Declined, true),
        "PHONE_NUMBER_INVALID"
        | "PHONE_NUMBER_UNOCCUPIED"
        | "PHONE_CODE_INVALID"
        | "PHONE_CODE_EMPTY"
        | "PASSWORD_HASH_INVALID"
        | "SRP_ID_INVALID"
        | "SRP_PASSWORD_CHANGED" => (DeviceLinkErrorCode::InvalidInput, true),
        _ if code >= 500 => (DeviceLinkErrorCode::VendorUnavailable, true),
        _ => (DeviceLinkErrorCode::InvalidInput, true),
    }
}

#[cfg(test)]
#[path = "../../tests/linked_login_errors.rs"]
mod tests;
