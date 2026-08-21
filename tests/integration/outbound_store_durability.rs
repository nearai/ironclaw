//! W6-COLD-SPOTS: `OutboundStateStore` (`outbound_preferences`
//! role) survives a real process-level reopen. Mirrors `standalone_outbound_store` (factory.rs);
//! see docs/internal/plans/2026-07-04-w6-cold-spots-plan.md.
//!
//! `ThreadNotificationPolicy`/`DeliveredGateRouteStore`/
//! `TriggeredRunDeliveryStore` excluded — not covered here. Deferred until
//! PR #5656.

use ironclaw_composition::{RebornRuntimeInput, build_runtime};
use ironclaw_config::RebornStoragePaths;
use ironclaw_outbound::{
    CommunicationModality, CommunicationPreferenceKey, CommunicationPreferenceRecord,
};

/// Write survives a fresh libsql reopen of the same on-disk file. Failure
/// class of PR #4782 (two stores over different mount views).
#[tokio::test]
async fn filesystem_outbound_state_store_persists_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_paths = RebornStoragePaths::from_installation_root(dir.path().join("reborn-home"));
    std::fs::create_dir_all(storage_paths.state_root()).expect("create state root");
    std::fs::write(
        storage_paths
            .state_root()
            .join(".reborn-local-dev-secrets-master-key"),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
    )
    .expect("seed test secrets master key");
    let services = build_runtime(RebornRuntimeInput::from_build_input(
        ironclaw_composition::local_filesystem_build_input(
            "w6-outbound-durability",
            storage_paths.installation_root().to_path_buf(),
        ),
    ))
    .await
    .expect("services build");

    let store = services
        .standalone_outbound_preferences_for_test()
        .expect("local-dev outbound_preferences wired");
    let installation_root = std::fs::canonicalize(storage_paths.installation_root())
        .expect("canonicalize Reborn installation root");

    let tenant = ironclaw_host_api::ids::TenantId::new("w6-outbound-tenant").unwrap();
    let user = ironclaw_host_api::ids::UserId::new("w6-outbound-user").unwrap();
    let key = CommunicationPreferenceKey::personal(tenant.clone(), user.clone());

    // Non-vacuity guard (before-write): a fresh scope has no row at all yet.
    let before_write = store
        .load_communication_preference(key.clone())
        .await
        .expect("load before write");
    assert!(
        before_write.is_none(),
        "expected no preference row before the write, found: {before_write:?}"
    );

    store
        .put_communication_preference(CommunicationPreferenceRecord {
            scope: key.scope.clone(),
            legacy_notification_target: None,
            default_modality: Some(CommunicationModality::Voice), // distinctive, non-default
            // Distinctive, non-default: the reopen assert below proves the
            // stored set itself survives, not just the row (an empty vec is
            // indistinguishable from the deserialization default).
            notification_targets: vec![
                ironclaw_outbound::OutboundDeliveryTargetId::new("slack:durability-dm")
                    .expect("target id"),
            ],
            updated_at: chrono::Utc::now(),
            updated_by: user.clone(),
        })
        .await
        .expect("write preference");

    // Reopen: a genuinely fresh store over a NEW libsql connection to the
    // same on-disk file — not the same Arc as `store` above. Drop the live
    // runtime first so this models a process restart and releases libSQL's
    // connection resources before the independent reopen.
    drop(store);
    drop(services);
    let reopened =
        ironclaw_composition::test_support::open_standalone_outbound_preferences_store_for_test(
            &installation_root,
        )
        .await
        .expect("reopen outbound store");

    let record = reopened
        .load_communication_preference(key)
        .await
        .expect("load after reopen")
        .expect("record survived reopen");
    assert_eq!(
        record.record.default_modality,
        Some(CommunicationModality::Voice)
    );
    assert_eq!(
        record
            .record
            .notification_targets
            .iter()
            .map(|target| target.as_str())
            .collect::<Vec<_>>(),
        vec!["slack:durability-dm"],
        "the stored notification set must survive a fresh-connection reopen"
    );
    assert_eq!(record.record.updated_by, user);
}
