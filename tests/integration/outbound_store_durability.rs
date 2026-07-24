//! W6-COLD-SPOTS: `FilesystemOutboundStateStore` (`outbound_preferences`
//! role) survives a real process-level reopen. Mirrors `local_dev_outbound_store` (factory.rs);
//! see docs/plans/2026-07-04-w6-cold-spots-plan.md.
//!
//! `ThreadNotificationPolicy`/`DeliveredGateRouteStore`/
//! `TriggeredRunDeliveryStore` excluded — not covered here. Deferred until
//! PR #5656.

use ironclaw_outbound::{
    CommunicationModality, CommunicationPreferenceKey, CommunicationPreferenceRecord,
};
use ironclaw_reborn_composition::{RebornRuntimeInput, build_runtime};

/// Write survives a fresh libsql reopen of the same on-disk file. Failure
/// class of PR #4782 (two stores over different mount views).
#[tokio::test]
async fn filesystem_outbound_state_store_persists_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime(RebornRuntimeInput::from_build_input(
        ironclaw_reborn_composition::local_dev_build_input(
            "w6-outbound-durability",
            dir.path().join("local-dev"),
        ),
    ))
    .await
    .expect("services build");

    let store = services
        .local_dev_outbound_preferences_for_test()
        .expect("local-dev outbound_preferences wired");
    // The runtime canonicalizes the local-dev storage root at build time
    // (`canonicalize_local_dev_path` == `std::fs::canonicalize`), so reproduce
    // the exact on-disk path the store was opened over from this test's own
    // input path rather than reaching for a removed runtime accessor. The
    // build already created (and canonicalized) this directory.
    let storage_root = std::fs::canonicalize(dir.path().join("local-dev"))
        .expect("canonicalize local-dev storage root");

    let tenant = ironclaw_host_api::TenantId::new("w6-outbound-tenant").unwrap();
    let user = ironclaw_host_api::UserId::new("w6-outbound-user").unwrap();
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
            final_reply_target: None,
            progress_target: None,
            approval_prompt_target: None,
            auth_prompt_target: None,
            default_modality: Some(CommunicationModality::Voice), // distinctive, non-default
            updated_at: chrono::Utc::now(),
            updated_by: user.clone(),
        })
        .await
        .expect("write preference");

    // Reopen: a genuinely fresh store over a NEW libsql connection to the
    // same on-disk file — not the same Arc as `store` above.
    let reopened =
        ironclaw_reborn_composition::test_support::open_local_dev_outbound_preferences_store_for_test(
            &storage_root,
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
    assert_eq!(record.record.updated_by, user);
}
