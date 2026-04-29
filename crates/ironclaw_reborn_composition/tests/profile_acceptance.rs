//! Acceptance tests #1 and #3 from issue #3026.
//!
//! These exercise the public composition root surface end-to-end at the
//! integration tier. The remaining acceptance tests (production-requires-
//! full-graph, config-layer-precedence, no-raw-secrets, shared-handle-wiring,
//! event-replay-wiring, postgres/libsql-parity, no-dual-source-of-truth,
//! rollback-switch) depend on substrate that has not yet merged and will be
//! added as their factories are wired.

use ironclaw_reborn_composition::{
    RebornBuildError, RebornBuildInput, RebornProfile, build_reborn_production_services,
};

fn input(profile: RebornProfile) -> RebornBuildInput {
    RebornBuildInput {
        profile,
        owner_id: "acceptance-test-owner".to_string(),
    }
}

/// Acceptance test #1 — default-off startup.
///
/// Given no Reborn config/profile switch, the legacy startup path remains
/// active and no partial Reborn services are exposed.
#[tokio::test]
async fn default_off_startup_exposes_no_partial_services() {
    // RebornProfile::default() == Disabled (verified in profile.rs unit
    // tests). Here we drive the full composition root entry point to prove
    // the disabled branch short-circuits before any factory runs.
    let services = build_reborn_production_services(input(RebornProfile::default()))
        .await
        .expect("disabled profile must succeed without touching substrate");

    assert_eq!(services.profile, RebornProfile::Disabled);
    // Every substrate slot is empty — there is no partial Reborn island
    // for channels/routes/tools to reach into.
    assert!(services.resource_governor.is_none());
    assert!(services.authorization.is_none());
    assert!(services.capability_lease_store.is_none());
    assert!(services.run_state_store.is_none());
    assert!(services.approval_request_store.is_none());
    assert!(services.event_log.is_none());
    assert!(services.audit_log.is_none());
    assert!(services.filesystem_root.is_none());
    assert!(services.extension_registry.is_none());
    assert!(
        !services.is_dev_only(),
        "Disabled is not a dev profile — it is the default-off legacy state"
    );
}

/// Acceptance test #3 — dev profile explicit fallback.
///
/// Given `RebornProfile::LocalDev`, in-memory backends are wired and the
/// readiness signal reports the dev profile (never "production-ready").
#[tokio::test]
async fn local_dev_profile_signals_dev_only() {
    let services = build_reborn_production_services(input(RebornProfile::LocalDev))
        .await
        .expect("local-dev must succeed with merged substrate");

    assert_eq!(services.profile, RebornProfile::LocalDev);
    assert!(services.is_dev_only(), "LocalDev must report dev-only");

    // Every merged substrate has a usable in-memory handle. The not-yet-
    // merged substrate stays empty (its gate factory tolerates missing
    // crates under LocalDev).
    assert!(services.resource_governor.is_some());
    assert!(services.event_log.is_some());
    assert!(services.audit_log.is_some());
    assert!(services.filesystem_root.is_some());
    assert!(services.run_state_store.is_some());
    assert!(services.approval_request_store.is_some());
    assert!(services.authorization.is_some());
    assert!(services.capability_lease_store.is_some());
    assert!(services.extension_registry.is_some());
}

/// Acceptance test #2 — production mode requires the full graph.
///
/// Given `RebornProfile::Production` and a missing required service, the
/// composition root fails before any traffic-serving surface can be exposed
/// and the diagnostic names the missing service. The diagnostic does not
/// leak host paths or credentials.
#[tokio::test]
async fn production_profile_fails_closed_on_missing_substrate() {
    let err = build_reborn_production_services(input(RebornProfile::Production))
        .await
        .expect_err("production must fail closed when required substrate is missing");

    let rendered = err.to_string();
    match err {
        RebornBuildError::SubstrateNotImplemented { service } => {
            assert!(
                !service.is_empty(),
                "missing-service diagnostic must name the service"
            );
            assert!(
                rendered.contains(service),
                "rendered error must include the service name; got {rendered}"
            );
        }
        other => panic!("expected SubstrateNotImplemented, got {other:?}"),
    }

    // Sanity: the redaction-safe Display contract from #3026's observability
    // section. Operators get an actionable name; nothing else.
    assert!(
        !rendered.contains("/Users/"),
        "diagnostic leaked a host path: {rendered}"
    );
    assert!(
        !rendered.contains("postgres://"),
        "diagnostic leaked a connection string: {rendered}"
    );
    assert!(
        !rendered.to_ascii_lowercase().contains("secret="),
        "diagnostic leaked a secret value: {rendered}"
    );
}

/// Acceptance test #11 — rollback switch.
///
/// Flipping the profile back to `Disabled` after a successful `LocalDev`
/// build returns a clean disabled graph with no partial service exposure.
/// The full rollback path against a live deployment (clearing channels,
/// loops, persisted Reborn-mode state) is covered by the binary-side
/// AppBuilder integration once Reborn-routed production paths exist.
#[tokio::test]
async fn rollback_to_disabled_clears_partial_services() {
    let dev = build_reborn_production_services(input(RebornProfile::LocalDev))
        .await
        .expect("local-dev build for rollback baseline");
    assert!(dev.resource_governor.is_some());

    let disabled = build_reborn_production_services(input(RebornProfile::Disabled))
        .await
        .expect("rollback to disabled must succeed");

    assert_eq!(disabled.profile, RebornProfile::Disabled);
    assert!(disabled.resource_governor.is_none());
    assert!(disabled.event_log.is_none());
    assert!(disabled.authorization.is_none());
}

/// Acceptance test #11 (full) — rollback signal across the full graph.
///
/// Builds `LocalDev` (full merged graph populated), rolls back to
/// `Disabled` (graph cleared), then forward to `LocalDev` again. The
/// rebuilt graph must be observably fresh — none of the previous run's
/// state can leak into it. Combined with the `validate()` rule that
/// rejects any wired slot under `Disabled`, this gives the rollback
/// contract two layers of protection.
#[tokio::test]
async fn rollback_round_trip_produces_fresh_graphs() {
    // Forward 1: full LocalDev graph. Capture the data-pointer addresses
    // through a thin-pointer cast — `Arc::as_ptr` over `dyn Trait`
    // returns a fat pointer that can't go to `usize` directly.
    let first = build_reborn_production_services(input(RebornProfile::LocalDev))
        .await
        .expect("first local-dev build");
    let first_governor_addr =
        Arc::as_ptr(first.resource_governor.as_ref().expect("governor")).cast::<()>() as usize;
    let first_event_log_addr =
        Arc::as_ptr(first.event_log.as_ref().expect("event log")).cast::<()>() as usize;
    drop(first);

    // Rollback: Disabled graph. validate() inside build also runs; if any
    // slot leaked through, it fails closed there.
    let disabled = build_reborn_production_services(input(RebornProfile::Disabled))
        .await
        .expect("rollback to disabled");
    assert_eq!(disabled.profile, RebornProfile::Disabled);
    assert!(!disabled.is_dev_only());

    // Forward 2: rebuild LocalDev. Each Arc must be a fresh allocation —
    // a static cache that handed back the previous build's handles
    // would leak state across rollback.
    let second = build_reborn_production_services(input(RebornProfile::LocalDev))
        .await
        .expect("second local-dev build");
    let second_governor_addr =
        Arc::as_ptr(second.resource_governor.as_ref().expect("governor")).cast::<()>() as usize;
    let second_event_log_addr =
        Arc::as_ptr(second.event_log.as_ref().expect("event log")).cast::<()>() as usize;

    assert_ne!(
        first_governor_addr, second_governor_addr,
        "rebuilt graph must allocate a fresh resource governor — \
         a cached handle would leak previous-run state across rollback"
    );
    assert_ne!(
        first_event_log_addr, second_event_log_addr,
        "rebuilt graph must allocate a fresh event log"
    );
}

/// Acceptance test #11 (full) — readiness signal flips with the rollback.
///
/// Operators must be able to observe that the graph is no longer in a
/// dev/non-disabled state immediately after rollback — without that
/// signal, the readiness surface (AC #14) would lie about cutover
/// status.
#[tokio::test]
async fn rollback_readiness_signal_flips_correctly() {
    let dev = build_reborn_production_services(input(RebornProfile::LocalDev))
        .await
        .unwrap();
    assert!(dev.is_dev_only(), "LocalDev must report dev-only");

    let disabled = build_reborn_production_services(input(RebornProfile::Disabled))
        .await
        .unwrap();
    assert!(
        !disabled.is_dev_only(),
        "Disabled is not a dev profile — it is the legacy default-off state"
    );

    // Sanity: a second forward to LocalDev re-asserts dev-only.
    let dev2 = build_reborn_production_services(input(RebornProfile::LocalDev))
        .await
        .unwrap();
    assert!(dev2.is_dev_only());
}

use std::sync::Arc;
