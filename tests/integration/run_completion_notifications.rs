//! Whole-path evidence for run-completion notifications (2026-08-13 design
//! §5.2): a REAL turn through the production workflow commits to the REAL
//! process journal, the production `RunCompletionJournalObserver` observes
//! that commit and hands it to the product ingest, and exactly one durable
//! unread notice lands in the owner's notice store with the run's identity.
//!
//! The observer registration, its filtering, and the ingest's eligibility
//! rules are the production code; the single fake is the scripted model at
//! the vendor-SDK seam (`tests/integration/CLAUDE.md`). The notice store
//! rides an in-memory filesystem because backend selection is composition's
//! concern, not this scenario's.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use ironclaw_assistant::run_completions::RunCompletionSurfaceServices;
use ironclaw_assistant::run_completions::ingest::RunCompletionIngest;
use ironclaw_assistant::run_completions::observer::RunCompletionJournalObserver;
use ironclaw_assistant::run_completions::records::{CompletionDeliveryState, RunCompletionNotice};
use ironclaw_assistant::run_completions::store::{
    RUN_NOTICES_MOUNT_ALIAS, RunCompletionNoticeStore, RunCompletionNotices, RunCompletionOwner,
};
use ironclaw_assistant::run_completions::stream::RunCompletionStreamHub;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
use ironclaw_host_api::path::{MountAlias, VirtualPath};
use ironclaw_host_api::resource::ResourceScope;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

/// The product notice services over an in-memory `/run-notices` per-user
/// mount plus the `/tenant-shared` due-owner registry — the same mount shape
/// composition registers, minus the backend choice.
fn notice_services() -> Arc<RunCompletionSurfaceServices> {
    let store = Arc::new(RunCompletionNoticeStore::new(Arc::new(
        ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), |scope: &ResourceScope| {
            MountView::new(vec![
                MountGrant::new(
                    MountAlias::new(RUN_NOTICES_MOUNT_ALIAS)?,
                    VirtualPath::new(format!(
                        "/tenants/{}/users/{}/run-notices",
                        scope.tenant_id, scope.user_id
                    ))?,
                    MountPermissions::read_write_list_delete(),
                ),
                MountGrant::new(
                    MountAlias::new("/tenant-shared")?,
                    VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id))?,
                    MountPermissions::read_write(),
                ),
            ])
        }),
    ))) as Arc<dyn RunCompletionNotices>;
    let hub = Arc::new(RunCompletionStreamHub::new(Arc::clone(&store)));
    Arc::new(RunCompletionSurfaceServices::new(
        store,
        hub,
        Arc::new(ironclaw_notifications::NoopNotificationInboxStore),
    ))
}

/// Observer delivery is durable and asynchronous (cursor-tracked, retried),
/// so the notice appears shortly after the turn's terminal commit rather
/// than synchronously with `submit_turn` returning.
async fn wait_for_unread(
    services: &RunCompletionSurfaceServices,
    owner: &RunCompletionOwner,
    expected: usize,
) -> Vec<RunCompletionNotice> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let unread = services
            .notices
            .unread_snapshot(owner)
            .await
            .expect("unread snapshot reads");
        if unread.len() >= expected {
            return unread;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {expected} unread run-completion notice(s), observed {} before the deadline",
            unread.len()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn completed_user_turn_creates_one_unread_notice_through_the_real_journal_observer() {
    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::text("first completed reply"),
            RebornScriptedReply::text("second completed reply"),
        ])
        .build()
        .await
        .expect("harness builds");
    let services = notice_services();
    let ingest = Arc::new(RunCompletionIngest::new(
        Arc::clone(&services),
        h.thread_harness.service.clone(),
    ));
    // The production observer, registered on the production journal the
    // harness's turn runtime commits to — exactly composition's wiring.
    h._shared
        .process_system
        .subscribe_process_observer(Arc::new(RunCompletionJournalObserver::new(Arc::clone(
            &ingest,
        ))))
        .expect("observer registers");
    let owner = RunCompletionOwner {
        tenant_id: h.binding.tenant_id.clone(),
        user_id: h.binding.actor_user_id.clone(),
    };

    let run_id = h
        .submit_turn("finish and notify me")
        .await
        .expect("turn completes");

    let unread = wait_for_unread(&services, &owner, 1).await;
    assert_eq!(unread.len(), 1, "exactly one notice per completed run");
    let notice = &unread[0];
    assert_eq!(
        notice.run_id,
        run_id.to_string(),
        "the notice names the exact run"
    );
    assert_eq!(
        notice.thread_id,
        h.binding.thread_id.as_str(),
        "the notice names the thread the reply landed in"
    );
    assert_eq!(notice.owner_user_id, h.binding.actor_user_id.as_str());
    assert!(!notice.is_read(), "a fresh completion is unread");
    assert!(
        matches!(
            notice.delivery,
            CompletionDeliveryState::PendingArbitration { .. }
        ),
        "a fresh completion awaits arbitration: {:?}",
        notice.delivery
    );
    assert_eq!(
        ingest.anomaly_count(),
        0,
        "the finalized reply resolved for the exact run"
    );

    // A second completion extends the owner's monotonic sequence; the first
    // notice is untouched (no rewrite, no duplicate).
    let second_run_id = h
        .submit_turn("and again")
        .await
        .expect("second turn completes");
    let unread = wait_for_unread(&services, &owner, 2).await;
    assert_eq!(unread.len(), 2, "one notice per run, none duplicated");
    assert_eq!(unread[0].run_id, run_id.to_string(), "oldest first");
    assert_eq!(unread[1].run_id, second_run_id.to_string());
    assert!(
        unread[0].sequence < unread[1].sequence,
        "the owner sequence is monotonic across runs"
    );
}
