//! Fault profile -> failure kind -> fate (#6524 workstream 6).
//!
//! The e2e suite declares reusable provider fault profiles in Python
//! (`tests/e2e/provider_fault_proxy.py`); the runtime decides what a failure
//! means in Rust (`FailureKind::fate`). Before these tests the two never met:
//! the e2e matrix asserted a *string* in a preview payload, and the Rust tests
//! enumerated `FailureKind::ALL` synthetically without ever seeing a profile.
//! So no test said what `http_429` or `expired_credential` does to a run.
//!
//! That gap is not hypothetical. `expired_credential` is the only profile
//! whose fate is `Park` (a re-auth gate rather than a failure), and it is
//! explicitly excluded from the e2e matrix because it does not produce the
//! `status == "failed"` shape that test asserts. Its whole path was untested
//! on both sides.
//!
//! The chain each case below drives, all of it real code:
//!
//! ```text
//! profile status                          provider_fault_proxy.py
//!   -> "github_api_error_status_{status}"  request.rs:45
//!   -> guest kind tag                      github wasm-src lib.rs:55
//!   -> WasmGuestErrorKind                  wasm_guest_error_kind
//!   -> DispatchError                       wasm_guest_dispatch_error
//!   -> FailureKind                         From<DispatchFailureKind>
//!   -> FailureFate                         FailureKind::fate
//! ```
//!
//! Statuses are read from the Python catalogue rather than restated here, so
//! a profile that changes status is classified as its new status rather than
//! silently keeping the old expectation.

use ironclaw_host_api::{CapabilityId, DispatchError, FailureFate, FailureKind};

use super::wasm_guest_dispatch_error;

/// The e2e fault-profile catalogue, as source text.
const FAULT_PROFILE_CATALOGUE: &str =
    include_str!("../../../../../tests/e2e/provider_fault_proxy.py");

/// What a profile is expected to mean to a run.
///
/// Carries no status or guest-kind column on purpose: both are derived, the
/// first from the catalogue and the second from the guest's own classifier.
/// Restating them here would let this table and production drift apart while
/// every assertion still passed.
struct FaultProfileFate {
    /// Name as declared in `tests/e2e/provider_fault_proxy.py`.
    profile: &'static str,
    expected_failure_kind: FailureKind,
    expected_fate: FailureFate,
}

/// Profiles whose classification is decided by an HTTP status, so the whole
/// chain is determined by code rather than by a live transport.
const STATUS_PROFILE_FATES: &[FaultProfileFate] = &[
    // 401 is the only route to a re-auth gate. If this row ever becomes
    // `ModelVisible`, an expired token stops prompting the user to reconnect
    // and starts being reported to the model as a tool failure.
    FaultProfileFate {
        profile: "http_401",
        expected_failure_kind: FailureKind::AuthRequired,
        expected_fate: FailureFate::Park,
    },
    FaultProfileFate {
        profile: "expired_credential",
        expected_failure_kind: FailureKind::AuthRequired,
        expected_fate: FailureFate::Park,
    },
    // 403 and 429 are deliberately NOT retried on the capability path. This
    // disagrees with the LLM path, where a rate limit IS retried with backoff
    // (`ironclaw_llm::retry::is_retryable`). The asymmetry is intentional --
    // a provider 429 usually needs a changed plan, not a faster loop -- but it
    // is asserted here so it stays a decision.
    FaultProfileFate {
        profile: "http_403",
        expected_failure_kind: FailureKind::Client,
        expected_fate: FailureFate::ModelVisible,
    },
    FaultProfileFate {
        profile: "wrong_scope",
        expected_failure_kind: FailureKind::Client,
        expected_fate: FailureFate::ModelVisible,
    },
    FaultProfileFate {
        profile: "http_429",
        expected_failure_kind: FailureKind::Client,
        expected_fate: FailureFate::ModelVisible,
    },
    FaultProfileFate {
        profile: "http_400",
        expected_failure_kind: FailureKind::OperationFailed,
        expected_fate: FailureFate::ModelVisible,
    },
    FaultProfileFate {
        profile: "http_404",
        expected_failure_kind: FailureKind::OperationFailed,
        expected_fate: FailureFate::ModelVisible,
    },
    FaultProfileFate {
        profile: "http_409",
        expected_failure_kind: FailureKind::OperationFailed,
        expected_fate: FailureFate::ModelVisible,
    },
    // 5xx reaching the model rather than being retried is the surprising row.
    // The guest collapses every non-special status to `operation_failed`, so a
    // 500 is indistinguishable from a 404 by the time the runtime sees it, and
    // the loop's `Retry` fate is never reached. Retrying a provider 5xx would
    // need the guest to emit a distinct kind first.
    FaultProfileFate {
        profile: "http_500",
        expected_failure_kind: FailureKind::OperationFailed,
        expected_fate: FailureFate::ModelVisible,
    },
    FaultProfileFate {
        profile: "http_503",
        expected_failure_kind: FailureKind::OperationFailed,
        expected_fate: FailureFate::ModelVisible,
    },
];

/// Profiles that break the transport instead of returning a usable status.
///
/// Their classification runs through `sanitize_host_error`, which matches on
/// the *host's* error text, so the resulting kind cannot be derived from the
/// profile declaration -- it takes a running host to know. They are listed
/// rather than omitted so the partition assertion below stays honest about
/// what is and is not pinned here; their coverage is the e2e
/// `PROVIDER_FAULT_CASES` matrix.
///
/// Note this is not "profiles without a status": `malformed_json`,
/// `truncated_response` and `missing_field` all answer 200. What they have in
/// common is that the status is not what decides the outcome.
const TRANSPORT_SHAPED_PROFILES: &[&str] = &[
    "timeout",
    "connection_reset",
    "malformed_json",
    "truncated_response",
    "missing_field",
    "lost_acknowledgement",
];

/// Every profile declared in the catalogue, with its status when it has one.
///
/// Entries look like:
///
/// ```text
///     "http_400": ProviderFaultProfile(
///         name="http_400",
///         action="respond",
///         status=400,
///     ),
/// ```
fn catalogue_profiles() -> Vec<(String, Option<u16>)> {
    let mut profiles = Vec::new();
    let mut current: Option<(String, Option<u16>)> = None;
    for line in FAULT_PROFILE_CATALOGUE.lines() {
        let trimmed = line.trim();
        if let Some(key) = trimmed.strip_suffix(": ProviderFaultProfile(") {
            if let Some(entry) = current.take() {
                profiles.push(entry);
            }
            let name = key.trim().trim_matches('"').to_string();
            current = Some((name, None));
            continue;
        }
        if let Some((_, status)) = current.as_mut()
            && let Some(value) = trimmed.strip_prefix("status=")
            && let Ok(parsed) = value.trim_end_matches(',').parse::<u16>()
        {
            *status = Some(parsed);
        }
    }
    if let Some(entry) = current.take() {
        profiles.push(entry);
    }
    profiles
}

/// The kind tag the GitHub guest emits for an HTTP status.
///
/// Mirrors `guest_error_kind` in the extension's wasm source, which is a
/// separate crate compiled to wasm and so cannot be linked into a host test.
/// The status side is not guesswork: `request.rs:45` builds the code as
/// `github_api_error_status_{status}` for every non-2xx.
fn guest_kind_for_status(status: u16) -> &'static str {
    match status {
        401 => "auth_required",
        403 | 429 => "client",
        _ => "operation_failed",
    }
}

/// The wire payload the GitHub guest emits for a failed request
/// (`{"code": ..., "kind": ...}`, github wasm-src lib.rs:47-52).
fn guest_error_payload(status: u16) -> String {
    serde_json::json!({
        "code": format!("github_api_error_status_{status}"),
        "kind": guest_kind_for_status(status),
    })
    .to_string()
}

/// The catalogue status for a profile, or a failure naming the profile.
fn catalogue_status(profile: &str) -> u16 {
    let profiles = catalogue_profiles();
    let (_, status) = profiles
        .iter()
        .find(|(name, _)| name == profile)
        .unwrap_or_else(|| {
            panic!(
                "fault profile `{profile}` is classified here but no longer exists in the catalogue"
            )
        });
    status.unwrap_or_else(|| {
        panic!(
            "fault profile `{profile}` is classified by status but declares none in the catalogue"
        )
    })
}

#[test]
fn status_fault_profiles_classify_to_their_declared_fate() {
    let capability = CapabilityId::new("github.list_issues").expect("a legal capability id");

    for case in STATUS_PROFILE_FATES {
        let status = catalogue_status(case.profile);
        let payload = guest_error_payload(status);
        let dispatch = wasm_guest_dispatch_error(&payload, &capability);
        let failure_kind: FailureKind = dispatch.failure_kind().into();

        assert_eq!(
            failure_kind, case.expected_failure_kind,
            "fault profile `{}` (HTTP {status}) classified as {failure_kind:?}",
            case.profile
        );
        assert_eq!(
            failure_kind.fate(),
            case.expected_fate,
            "fault profile `{}` (HTTP {status}) has fate {:?}",
            case.profile,
            failure_kind.fate()
        );
        // Retryability is defined as `fate() == Retry`, so asserting it
        // separately catches the two drifting apart.
        assert_eq!(
            failure_kind.is_retryable(),
            case.expected_fate == FailureFate::Retry,
            "fault profile `{}` retryability disagrees with its fate",
            case.profile
        );
    }
}

#[test]
fn a_401_parks_for_reauth_and_is_never_reported_as_a_tool_failure() {
    let capability = CapabilityId::new("github.list_issues").expect("a legal capability id");
    let payload = guest_error_payload(catalogue_status("expired_credential"));

    // The variant matters, not just the fate: a re-auth gate needs the
    // capability and credential requirements carried through, which only
    // `DispatchError::AuthRequired` does.
    let dispatch = wasm_guest_dispatch_error(&payload, &capability);
    assert!(
        matches!(dispatch, DispatchError::AuthRequired { .. }),
        "an expired credential must produce an auth gate, got {dispatch:?}"
    );

    let failure_kind: FailureKind = dispatch.failure_kind().into();
    assert_eq!(failure_kind.fate(), FailureFate::Park);
    assert!(
        !failure_kind.is_retryable(),
        "parking for re-auth must not also retry the same call"
    );
}

#[test]
fn every_declared_fault_profile_is_classified_exactly_once() {
    let declared = catalogue_profiles();
    // Guard against a silent parse failure: an empty or truncated list would
    // make every assertion below vacuous.
    assert!(
        declared.len() >= 10,
        "parsed only {} profiles from the catalogue; the parser has drifted \
         from the file's shape and this gate would pass vacuously",
        declared.len()
    );

    let classified_by_status: Vec<&str> = STATUS_PROFILE_FATES
        .iter()
        .map(|case| case.profile)
        .collect();

    // Forward: nothing in the catalogue is unaccounted for.
    for (profile, _) in &declared {
        let in_status = classified_by_status.contains(&profile.as_str());
        let in_transport = TRANSPORT_SHAPED_PROFILES.contains(&profile.as_str());
        assert!(
            in_status || in_transport,
            "fault profile `{profile}` is declared in \
             tests/e2e/provider_fault_proxy.py but no test says what it means \
             to a run. Add it to STATUS_PROFILE_FATES with its expected \
             FailureKind and fate, or to TRANSPORT_SHAPED_PROFILES if its \
             classification depends on live host error text."
        );
        assert!(
            !(in_status && in_transport),
            "fault profile `{profile}` is in both lists; it cannot be both \
             status-classified and transport-shaped"
        );
    }

    // Backward: nothing here refers to a profile that no longer exists. A
    // stale row would otherwise keep asserting against a deleted profile and
    // read as coverage.
    let declared_names: Vec<&str> = declared.iter().map(|(name, _)| name.as_str()).collect();
    for profile in classified_by_status
        .iter()
        .chain(TRANSPORT_SHAPED_PROFILES.iter())
    {
        assert!(
            declared_names.contains(profile),
            "fault profile `{profile}` is classified here but no longer exists \
             in tests/e2e/provider_fault_proxy.py; remove the stale row"
        );
    }

    // Unique: a duplicate row would let two different expectations both claim
    // to cover the same profile.
    let mut seen = classified_by_status.clone();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(before, seen.len(), "duplicate rows in STATUS_PROFILE_FATES");
}
