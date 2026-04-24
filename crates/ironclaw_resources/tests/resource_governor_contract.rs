use std::sync::{Arc, Barrier};
use std::thread;

use ironclaw_host_api::*;
use ironclaw_resources::*;
use rust_decimal_macros::dec;

#[test]
fn reserve_succeeds_when_budget_is_available() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_usd: Some(dec!(1.00)),
            max_concurrency_slots: Some(2),
            ..ResourceLimits::default()
        },
    );

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.25)),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    assert_eq!(reservation.scope, scope);
    assert_eq!(reservation.estimate.usd, Some(dec!(0.25)));
    assert_eq!(governor.reserved_for(&account).usd, dec!(0.25));
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);
}

#[test]
fn reserve_denies_when_usd_limit_would_be_exceeded() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_usd: Some(dec!(0.50)),
            ..ResourceLimits::default()
        },
    );

    let err = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.75)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == account && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reserve_denies_runtime_quota_even_without_usd() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_wall_clock_ms: Some(1_000),
            max_process_count: Some(1),
            ..ResourceLimits::default()
        },
    );

    let err = governor
        .reserve(
            scope,
            ResourceEstimate {
                wall_clock_ms: Some(2_000),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == account && denial.dimension == ResourceDimension::WallClockMs
    ));
}

#[test]
fn active_reservations_consume_concurrency_until_released() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );

    let first = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    let second = governor.reserve(
        scope.clone(),
        ResourceEstimate {
            concurrency_slots: Some(1),
            ..ResourceEstimate::default()
        },
    );
    assert!(matches!(
        second,
        Err(ResourceError::LimitExceeded(denial))
            if denial.dimension == ResourceDimension::ConcurrencySlots
    ));

    governor.release(first.id).unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 0);

    governor
        .reserve(
            scope,
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
}

#[test]
fn concurrent_reservations_cannot_oversubscribe_scope() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor.set_limit(
        account,
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );

    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let governor = Arc::clone(&governor);
        let barrier = Arc::clone(&barrier);
        let mut scope = scope.clone();
        scope.invocation_id = InvocationId::new();
        handles.push(thread::spawn(move || {
            barrier.wait();
            governor
                .reserve(
                    scope,
                    ResourceEstimate {
                        concurrency_slots: Some(1),
                        ..ResourceEstimate::default()
                    },
                )
                .is_ok()
        }));
    }

    let successes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|success| *success)
        .count();
    assert_eq!(successes, 1);
}

#[test]
fn reconcile_records_actual_usage_and_closes_reservation() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_usd: Some(dec!(1.00)),
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );

    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.75)),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let receipt = governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.20),
                input_tokens: 10,
                output_tokens: 20,
                wall_clock_ms: 100,
                output_bytes: 50,
                network_egress_bytes: 0,
                process_count: 1,
            },
        )
        .unwrap();

    assert_eq!(receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).usd, dec!(0.20));
    assert_eq!(governor.usage_for(&account).input_tokens, 10);
    assert!(matches!(
        governor.reconcile(reservation.id, ResourceUsage::default()),
        Err(ResourceError::ReservationClosed { .. })
    ));
}

#[test]
fn release_frees_reserved_capacity_without_recording_spend() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_usd: Some(dec!(1.00)),
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );

    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.75)),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let receipt = governor.release(reservation.id).unwrap();
    assert_eq!(receipt.status, ReservationStatus::Released);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert!(matches!(
        governor.release(reservation.id),
        Err(ResourceError::ReservationClosed { .. })
    ));
}

#[test]
fn unknown_reservation_cannot_be_reconciled_or_released() {
    let governor = InMemoryResourceGovernor::new();
    let unknown = ResourceReservationId::new();

    assert!(matches!(
        governor.reconcile(unknown, ResourceUsage::default()),
        Err(ResourceError::UnknownReservation { id }) if id == unknown
    ));
    assert!(matches!(
        governor.release(unknown),
        Err(ResourceError::UnknownReservation { id }) if id == unknown
    ));
}

#[test]
fn tenant_limit_applies_across_projects() {
    let governor = InMemoryResourceGovernor::new();
    let project_a = sample_scope("tenant1", "user1", Some("project_a"));
    let project_b = sample_scope("tenant1", "user1", Some("project_b"));
    let tenant_account = ResourceAccount::tenant(project_a.tenant_id.clone());
    governor.set_limit(
        tenant_account.clone(),
        ResourceLimits {
            max_usd: Some(dec!(1.00)),
            ..ResourceLimits::default()
        },
    );

    governor
        .reserve(
            project_a,
            ResourceEstimate {
                usd: Some(dec!(0.75)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(
            project_b,
            ResourceEstimate {
                usd: Some(dec!(0.50)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == tenant_account && denial.dimension == ResourceDimension::Usd
    ));
}

fn sample_scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        project_id: project.map(|value| ProjectId::new(value).unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}
