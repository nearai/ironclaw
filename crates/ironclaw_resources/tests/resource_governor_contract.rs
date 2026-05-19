use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use tempfile::tempdir;

use ironclaw_host_api::*;
use ironclaw_resources::*;
use rust_decimal_macros::dec;

#[derive(Clone)]
struct AlwaysFailingStore;

impl ResourceGovernorStore for AlwaysFailingStore {
    fn update<T, F>(&self, _update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        Err(ResourceError::Storage {
            reason: "forced durable write failure".to_string(),
        })
    }
}

#[test]
fn persistent_trait_set_limit_surfaces_storage_errors() {
    let governor: Arc<dyn ResourceGovernor> =
        Arc::new(PersistentResourceGovernor::new(AlwaysFailingStore));
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let error = governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap_err();

    assert!(
        matches!(error, ResourceError::Storage { reason } if reason == "forced durable write failure")
    );
}

#[test]
fn storage_errors_display_sanitized_message_without_backend_details() {
    let error = ResourceError::Storage {
        reason: "postgres://user:secret@localhost/db failed under /tmp/private".to_string(),
    };
    let rendered = error.to_string();

    assert_eq!(rendered, "resource governor storage error");
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("/tmp/private"));
}

#[test]
fn reserve_succeeds_when_budget_is_available() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                max_concurrency_slots: Some(2),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
fn reserve_with_id_uses_requested_identifier_and_rejects_duplicates() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let reservation_id = ResourceReservationId::new();
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };

    let reservation = governor
        .reserve_with_id(scope.clone(), estimate.clone(), reservation_id)
        .unwrap();

    assert_eq!(reservation.id, reservation_id);
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);
    assert!(matches!(
        governor.reserve_with_id(scope, estimate, reservation_id),
        Err(ResourceError::ReservationAlreadyExists { id }) if id == reservation_id
    ));
}

#[test]
fn reserve_with_id_rejects_negative_usd_estimates() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let err = governor
        .reserve_with_id(
            scope,
            ResourceEstimate {
                usd: Some(dec!(-100.00)),
                ..ResourceEstimate::default()
            },
            ResourceReservationId::new(),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative"
        }
    ));
    assert_eq!(governor.reserved_for(&account).usd, dec!(0));
}

#[test]
fn reconcile_rejects_negative_usd_actuals_without_closing_reservation() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.25)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let err = governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(-100.00),
                ..ResourceUsage::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative"
        }
    ));
    assert_eq!(governor.reserved_for(&account).usd, dec!(0.25));
    assert_eq!(governor.usage_for(&account).usd, dec!(0));
    assert!(governor.release(reservation.id).is_ok());
}

#[test]
fn usd_tally_saturates_instead_of_panicking_on_decimal_overflow() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(rust_decimal::Decimal::MAX),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(1)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    assert_eq!(
        governor.reserved_for(&account).usd,
        rust_decimal::Decimal::MAX
    );
}

#[test]
fn usd_limit_check_denies_instead_of_panicking_on_decimal_overflow() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(rust_decimal::Decimal::MAX),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(rust_decimal::Decimal::MAX),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(1)),
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
fn reserve_denies_when_usd_limit_would_be_exceeded() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(0.50)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_wall_clock_ms: Some(1_000),
                max_process_count: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
    governor
        .set_limit(
            account,
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
    assert!(matches!(
        governor.release(reservation.id),
        Err(ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        })
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).usd, dec!(0.20));
}

#[test]
fn release_frees_reserved_capacity_without_recording_spend() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
    governor
        .set_limit(
            tenant_account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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

#[test]
fn resource_governor_enforces_agent_scoped_limits_independently() {
    let governor = InMemoryResourceGovernor::new();
    let tenant = TenantId::new("tenant1").unwrap();
    let user = UserId::new("user1").unwrap();
    let agent_a = AgentId::new("agent-a").unwrap();
    let agent_b = AgentId::new("agent-b").unwrap();
    governor
        .set_limit(
            ResourceAccount::agent(tenant.clone(), user.clone(), None, agent_a.clone()),
            ResourceLimits {
                max_output_bytes: Some(10),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let estimate = ResourceEstimate {
        output_bytes: Some(8),
        ..ResourceEstimate::default()
    };
    governor
        .reserve(
            sample_scope_with_agent("tenant1", "user1", None, Some("agent-a")),
            estimate.clone(),
        )
        .unwrap();
    governor
        .reserve(
            sample_scope_with_agent("tenant1", "user1", None, Some("agent-b")),
            estimate.clone(),
        )
        .unwrap();

    let denial = governor
        .reserve(
            sample_scope_with_agent("tenant1", "user1", None, Some("agent-a")),
            estimate,
        )
        .unwrap_err();

    assert!(matches!(denial, ResourceError::LimitExceeded(_)));
    assert_eq!(
        governor.reserved_for(&ResourceAccount::agent(tenant, user, None, agent_a)),
        ResourceTally {
            output_bytes: 8,
            ..ResourceTally::default()
        }
    );
    assert_eq!(
        governor.reserved_for(&ResourceAccount::agent(
            TenantId::new("tenant1").unwrap(),
            UserId::new("user1").unwrap(),
            None,
            agent_b,
        )),
        ResourceTally {
            output_bytes: 8,
            ..ResourceTally::default()
        }
    );
}

#[test]
fn persistent_governor_reloads_active_holds_and_usage_from_store() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let active = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.20)),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let concurrency_denial = reloaded
        .reserve(
            scope.clone(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(
        concurrency_denial,
        ResourceError::LimitExceeded(denial)
            if denial.account == account
                && denial.dimension == ResourceDimension::ConcurrencySlots
                && denial.active_reserved == ResourceValue::Integer(1)
    ));

    reloaded
        .reconcile(
            active.id,
            ResourceUsage {
                usd: dec!(0.95),
                ..ResourceUsage::default()
            },
        )
        .unwrap();

    let reloaded_again = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let usd_denial = reloaded_again
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.10)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(
        usd_denial,
        ResourceError::LimitExceeded(denial)
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.current_usage == ResourceValue::Decimal(dec!(0.95))
    ));
}

#[test]
fn persistent_governor_serializes_concurrent_reservations_across_handles() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            account,
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        let mut scope = scope.clone();
        scope.invocation_id = InvocationId::new();
        handles.push(thread::spawn(move || {
            let governor =
                PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(path));
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
fn persistent_governor_writes_versioned_snapshot_schema() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id);

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            account,
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(snapshot["schema_version"], serde_json::json!(1));
}

#[test]
fn persistent_governor_upgrades_legacy_unversioned_snapshot() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(snapshot["schema_version"], serde_json::json!(1));
}

#[test]
fn persistent_governor_rejects_malformed_snapshot_with_storage_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(&path, "{not valid json").unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("malformed resource governor snapshot")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_snapshot_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 1,
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            },
            "unexpected": true
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_persisted_resource_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 1,
            "state": {
                "limits": [
                    [
                        { "Tenant": { "tenant_id": "tenant1" } },
                        { "max_usd": "1.00", "unexpected_limit": true }
                    ]
                ],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_reservation_scope_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .reserve(scope.clone(), ResourceEstimate::default())
        .unwrap();

    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    snapshot["state"]["reservations"].as_array_mut().unwrap()[0][1]["reservation"]["scope"]["unexpected_scope"] =
        serde_json::json!(true);
    fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = reloaded
        .try_set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_reservation_estimate_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.20)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    snapshot["state"]["reservations"].as_array_mut().unwrap()[0][1]["reservation"]["estimate"]["unexpected_estimate"] =
        serde_json::json!(true);
    fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = reloaded
        .try_set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_reservation_actual_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let reservation = governor
        .reserve(scope.clone(), ResourceEstimate::default())
        .unwrap();
    governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.20),
                ..ResourceUsage::default()
            },
        )
        .unwrap();

    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    snapshot["state"]["reservations"].as_array_mut().unwrap()[0][1]["actual"]["unexpected_actual"] =
        serde_json::json!(true);
    fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = reloaded
        .try_set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_partial_snapshot_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(&path, r#"{"schema_version": 1}"#).unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("missing field")
    ));
}

#[test]
fn persistent_governor_rejects_unsupported_snapshot_schema_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 999,
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason }
            if reason.contains("unsupported resource governor snapshot schema version 999")
    ));
}

/// Filesystem-backed reload contract — replaces the deleted libSQL /
/// Postgres `*_persistent_governor_reloads_active_holds_and_usage_from_store`
/// tests. Backend choice is now a property of the underlying
/// `RootFilesystem`; this test exercises the on-disk snapshot
/// round-trip through `ScopedFilesystem` so durability across reopen is
/// covered by the same surface (a single `FilesystemResourceGovernorStore`
/// constructed twice over the same backing store).
#[tokio::test]
async fn filesystem_persistent_governor_reloads_active_holds_and_usage_from_store() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = std::sync::Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = std::sync::Arc::new(ScopedFilesystem::with_fixed_view(
        std::sync::Arc::clone(&backend),
        mounts,
    ));

    let store = FilesystemResourceGovernorStore::new(std::sync::Arc::clone(&scoped));

    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let governor = PersistentResourceGovernor::new(store);
    governor
        .try_set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let active = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    // Reload from the same on-disk snapshot via a fresh
    // FilesystemResourceGovernorStore handle over the same ScopedFilesystem.
    let reloaded = PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(
        std::sync::Arc::clone(&scoped),
    ));
    let concurrency_denial = reloaded
        .reserve(
            scope.clone(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(
        concurrency_denial,
        ResourceError::LimitExceeded(denial)
            if denial.account == account && denial.dimension == ResourceDimension::ConcurrencySlots
    ));

    reloaded
        .reconcile(
            active.id,
            ResourceUsage {
                usd: dec!(0.95),
                ..ResourceUsage::default()
            },
        )
        .unwrap();
    let usd_denial = reloaded
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.10)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(
        usd_denial,
        ResourceError::LimitExceeded(denial)
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.current_usage == ResourceValue::Decimal(dec!(0.95))
    ));
}

fn sample_scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: project.map(|value| ProjectId::new(value).unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_scope_with_agent(
    tenant: &str,
    user: &str,
    project: Option<&str>,
    agent: Option<&str>,
) -> ResourceScope {
    let mut scope = sample_scope(tenant, user, project);
    scope.agent_id = agent.map(|id| AgentId::new(id).unwrap());
    scope
}

#[test]
fn project_and_agent_limits_both_apply_without_override() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope_with_agent("tenant1", "user1", Some("project1"), Some("agent1"));
    let project_account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    let agent_account = ResourceAccount::agent(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone(),
        scope.agent_id.clone().unwrap(),
    );

    governor
        .set_limit(
            project_account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(0.50)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
        .set_limit(
            agent_account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.75)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == project_account && denial.dimension == ResourceDimension::Usd
    ));

    governor
        .set_limit(
            project_account,
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
        .set_limit(
            agent_account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(0.50)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
            if denial.account == agent_account && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reservation_and_usage_are_charged_to_full_scope_cascade() {
    let governor = InMemoryResourceGovernor::new();
    let mut scope = sample_scope("tenant1", "user1", Some("project1"));
    scope.mission_id = Some(MissionId::new("mission1").unwrap());
    scope.thread_id = Some(ThreadId::new("thread1").unwrap());

    let accounts = ResourceAccount::cascade(&scope);
    assert_eq!(accounts.len(), 5);

    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.10)),
                output_bytes: Some(100),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    for account in &accounts {
        let reserved = governor.reserved_for(account);
        assert_eq!(reserved.usd, dec!(0.10));
        assert_eq!(reserved.output_bytes, 100);
        assert_eq!(reserved.concurrency_slots, 1);
    }

    governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.06),
                input_tokens: 11,
                output_tokens: 7,
                wall_clock_ms: 55,
                output_bytes: 80,
                network_egress_bytes: 9,
                process_count: 1,
            },
        )
        .unwrap();

    for account in &accounts {
        assert_eq!(governor.reserved_for(account), ResourceTally::default());
        let usage = governor.usage_for(account);
        assert_eq!(usage.usd, dec!(0.06));
        assert_eq!(usage.input_tokens, 11);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.wall_clock_ms, 55);
        assert_eq!(usage.output_bytes, 80);
        assert_eq!(usage.network_egress_bytes, 9);
        assert_eq!(usage.process_count, 1);
        assert_eq!(usage.concurrency_slots, 0);
    }
}

#[test]
fn project_limit_denies_leaf_even_when_tenant_allows() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let tenant = ResourceAccount::tenant(scope.tenant_id.clone());
    let project = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor
        .set_limit(
            tenant,
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
        .set_limit(
            project.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(1.50)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == project && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reconciled_usage_counts_against_future_reservations() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.20)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.80),
                ..ResourceUsage::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.30)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.current_usage == ResourceValue::Decimal(dec!(0.80))
                && denial.active_reserved == ResourceValue::Decimal(dec!(0))
                && denial.requested == ResourceValue::Decimal(dec!(0.30))
    ));
}

#[test]
fn active_reserved_and_usage_appear_in_denial_details() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let completed = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.40)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    governor
        .reconcile(
            completed.id,
            ResourceUsage {
                usd: dec!(0.40),
                ..ResourceUsage::default()
            },
        )
        .unwrap();

    governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.30)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.40)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.limit == ResourceValue::Decimal(dec!(1.00))
                && denial.current_usage == ResourceValue::Decimal(dec!(0.40))
                && denial.active_reserved == ResourceValue::Decimal(dec!(0.30))
                && denial.requested == ResourceValue::Decimal(dec!(0.40))
    ));
}

#[test]
fn actual_usage_above_estimate_is_recorded_and_blocks_future_work() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                usd: Some(dec!(0.20)),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.95),
                ..ResourceUsage::default()
            },
        )
        .unwrap();

    assert_eq!(governor.usage_for(&account).usd, dec!(0.95));
    assert!(matches!(
        governor.reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.10)),
                ..ResourceEstimate::default()
            },
        ),
        Err(ResourceError::LimitExceeded(denial))
            if denial.current_usage == ResourceValue::Decimal(dec!(0.95))
    ));
}

#[test]
fn closed_reservations_reject_cross_lifecycle_operations_with_status() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let reconciled = governor
        .reserve(scope.clone(), ResourceEstimate::default())
        .unwrap();
    governor
        .reconcile(reconciled.id, ResourceUsage::default())
        .unwrap();
    assert!(matches!(
        governor.release(reconciled.id),
        Err(ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        })
    ));

    let released = governor
        .reserve(scope, ResourceEstimate::default())
        .unwrap();
    governor.release(released.id).unwrap();
    assert!(matches!(
        governor.reconcile(released.id, ResourceUsage::default()),
        Err(ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        })
    ));
}

#[test]
fn non_usd_dimensions_can_deny_reservations() {
    assert_denied_dimension(
        ResourceLimits {
            max_input_tokens: Some(10),
            ..ResourceLimits::default()
        },
        ResourceEstimate {
            input_tokens: Some(11),
            ..ResourceEstimate::default()
        },
        ResourceDimension::InputTokens,
    );
    assert_denied_dimension(
        ResourceLimits {
            max_output_tokens: Some(10),
            ..ResourceLimits::default()
        },
        ResourceEstimate {
            output_tokens: Some(11),
            ..ResourceEstimate::default()
        },
        ResourceDimension::OutputTokens,
    );
    assert_denied_dimension(
        ResourceLimits {
            max_output_bytes: Some(10),
            ..ResourceLimits::default()
        },
        ResourceEstimate {
            output_bytes: Some(11),
            ..ResourceEstimate::default()
        },
        ResourceDimension::OutputBytes,
    );
    assert_denied_dimension(
        ResourceLimits {
            max_network_egress_bytes: Some(10),
            ..ResourceLimits::default()
        },
        ResourceEstimate {
            network_egress_bytes: Some(11),
            ..ResourceEstimate::default()
        },
        ResourceDimension::NetworkEgressBytes,
    );
    assert_denied_dimension(
        ResourceLimits {
            max_process_count: Some(1),
            ..ResourceLimits::default()
        },
        ResourceEstimate {
            process_count: Some(2),
            ..ResourceEstimate::default()
        },
        ResourceDimension::ProcessCount,
    );
}

fn assert_denied_dimension(
    limits: ResourceLimits,
    estimate: ResourceEstimate,
    expected: ResourceDimension,
) {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(account.clone(), limits).unwrap();

    let err = governor.reserve(scope, estimate).unwrap_err();
    assert!(matches!(
        err,
        ResourceError::LimitExceeded(denial)
            if denial.account == account && denial.dimension == expected
    ));
}
