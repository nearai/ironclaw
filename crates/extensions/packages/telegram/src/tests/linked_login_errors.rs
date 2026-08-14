//! Tests for the device-link error table.
//!
//! The load-bearing assertion in every case below is `restartable`: whether the
//! card offers the user another attempt. A terminal failure reported as
//! restartable re-prompts forever for something that can never succeed, and a
//! restartable one reported as terminal strands a user who only hit a rate
//! limit. Neither is visible in a manual pass.

use super::*;

// ---------------------------------------------------------------------------
// Error classification — `restartable` is the load-bearing bit
// ---------------------------------------------------------------------------

#[test]
fn a_deactivated_account_is_terminal_and_never_re_prompted() {
    for name in [
        "USER_DEACTIVATED",
        "USER_DEACTIVATED_BAN",
        "PHONE_NUMBER_BANNED",
        "AUTH_KEY_DUPLICATED",
    ] {
        let (code, restartable) = rpc_disposition(name, 400);
        assert_eq!(
            code,
            DeviceLinkErrorCode::AccountUnavailable,
            "{name} must be account-unavailable"
        );
        assert!(
            !restartable,
            "{name} can never succeed on a retry, so the card must not offer one"
        );
    }
}

#[test]
fn a_rate_limit_is_restartable_whatever_shape_it_arrives_in() {
    for (name, code) in [
        ("FLOOD_WAIT", 420),
        ("PHONE_PASSWORD_FLOOD", 400),
        ("SOMETHING_FLOOD", 400),
        ("UNRECOGNIZED", 420),
    ] {
        let (mapped, restartable) = rpc_disposition(name, code);
        assert_eq!(mapped, DeviceLinkErrorCode::RateLimited, "{name}");
        assert!(restartable, "{name}");
    }
}

#[test]
fn rejected_user_input_is_restartable_and_never_terminal() {
    for name in [
        "PHONE_NUMBER_INVALID",
        "PHONE_CODE_INVALID",
        "PHONE_CODE_EMPTY",
        "PASSWORD_HASH_INVALID",
        "SRP_ID_INVALID",
    ] {
        let (code, restartable) = rpc_disposition(name, 400);
        assert_eq!(code, DeviceLinkErrorCode::InvalidInput, "{name}");
        assert!(restartable, "{name}");
    }
}

#[test]
fn an_expired_token_and_a_revoked_session_are_distinguished() {
    assert_eq!(
        rpc_disposition("AUTH_TOKEN_EXPIRED", 400).0,
        DeviceLinkErrorCode::Expired
    );
    assert_eq!(
        rpc_disposition("SESSION_REVOKED", 401).0,
        DeviceLinkErrorCode::Declined
    );
    assert_eq!(
        rpc_disposition("AUTH_TOKEN_ALREADY_ACCEPTED", 400).0,
        DeviceLinkErrorCode::Declined
    );
}

#[test]
fn a_server_side_failure_is_vendor_unavailable() {
    let (code, restartable) = rpc_disposition("INTERNAL_SERVER_ERROR", 500);
    assert_eq!(code, DeviceLinkErrorCode::VendorUnavailable);
    assert!(restartable);
}

#[test]
fn an_unknown_outcome_on_a_write_never_becomes_a_terminal_link_failure() {
    // The transport reports "may have executed"; the flow must not translate
    // that into something the card presents as unrecoverable.
    let error = vendor_error(TransportError::OutcomeUnknown);
    assert!(error.restartable());
    assert_eq!(error.code(), DeviceLinkErrorCode::VendorUnavailable);
}

#[test]
fn custody_failures_stay_custody_failures() {
    let error = custody_error(SessionStoreError::Custody(LinkedSessionErrorForTest::make()));
    assert_eq!(error.code(), DeviceLinkErrorCode::CustodyFailed);
    assert!(
        !error.restartable(),
        "a custody failure is not fixed by asking the user to scan again"
    );
}

/// Local helper: `LinkedSessionError` has no test constructor, and naming the
/// variant here keeps the assertion above readable.
struct LinkedSessionErrorForTest;

impl LinkedSessionErrorForTest {
    fn make() -> ironclaw_extension_contracts::linked_session::LinkedSessionError {
        ironclaw_extension_contracts::linked_session::LinkedSessionError::Unavailable {
            reason: "test",
        }
    }
}
