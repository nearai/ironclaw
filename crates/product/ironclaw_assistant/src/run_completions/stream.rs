//! The owner-scoped run-completion stream hub (2026-08-13 design §7.6).
//!
//! The user-completion source is the durable notice store, not the runtime
//! event-log partition: the store's per-owner sequence is the stream's one
//! ordered cursor domain, and this hub only fans out live wake-ups. On
//! subscribe, replay comes from `list_after` (durable), then live events
//! ride a bounded per-owner broadcast; a lagged subscriber rebases from the
//! bounded unread snapshot rather than blocking completion commits.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use ironclaw_product_contracts::run_completions::{
    RUN_COMPLETION_CLEAR_SCHEMA, RUN_COMPLETION_GRANT_SCHEMA, RUN_COMPLETION_NOTICE_SCHEMA,
    RunCompletionClearEvent, RunCompletionGrantEvent, RunCompletionGrantSurface,
    RunCompletionNoticeEvent, RunCompletionStreamEvent,
};
use tokio::sync::broadcast;

use super::records::{CompletionDeliveryState, CompletionSurface, RunCompletionNotice};
use super::store::{RunCompletionNotices, RunCompletionOwner, RunCompletionStoreError};

/// Bounded per-owner live buffer. A subscriber that falls further behind
/// receives a lag signal and rebases from the durable snapshot.
const OWNER_CHANNEL_CAPACITY: usize = 64;

/// One live item on an owner's stream: the event plus its sequence member.
#[derive(Debug, Clone)]
pub struct SequencedCompletionEvent {
    pub sequence: u64,
    pub event: RunCompletionStreamEvent,
}

pub struct RunCompletionStreamHub {
    notices: Arc<dyn RunCompletionNotices>,
    senders: Mutex<HashMap<String, broadcast::Sender<SequencedCompletionEvent>>>,
}

impl RunCompletionStreamHub {
    pub fn new(notices: Arc<dyn RunCompletionNotices>) -> Self {
        Self {
            notices,
            senders: Mutex::new(HashMap::new()),
        }
    }

    fn owner_key(owner: &RunCompletionOwner) -> String {
        format!(
            "{}\u{1f}{}",
            owner.tenant_id.as_str(),
            owner.user_id.as_str()
        )
    }

    fn sender(&self, owner: &RunCompletionOwner) -> broadcast::Sender<SequencedCompletionEvent> {
        let mut senders = self
            .senders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        senders
            .entry(Self::owner_key(owner))
            .or_insert_with(|| broadcast::channel(OWNER_CHANNEL_CAPACITY).0)
            .clone()
    }

    /// Subscribe to one owner's completion stream: durable replay after
    /// `after_sequence`, then the live receiver. The receiver is registered
    /// before the replay read so an event written between the two is
    /// delivered (possibly twice — stable notice IDs collapse duplicates,
    /// per the design's at-least-once preference).
    pub async fn subscribe(
        &self,
        owner: &RunCompletionOwner,
        after_sequence: Option<u64>,
        replay_limit: usize,
    ) -> Result<
        (
            Vec<SequencedCompletionEvent>,
            broadcast::Receiver<SequencedCompletionEvent>,
        ),
        RunCompletionStoreError,
    > {
        let receiver = self.sender(owner).subscribe();
        let replayed = self
            .notices
            .list_after(owner, after_sequence, replay_limit)
            .await?;
        let events = self.project_batch(owner, &replayed).await;
        Ok((events, receiver))
    }

    /// The bounded rebase snapshot (§7.6): unread/unsettled notices only.
    pub async fn rebase_snapshot(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<Vec<SequencedCompletionEvent>, RunCompletionStoreError> {
        let unread = self.notices.unread_snapshot(owner).await?;
        Ok(self.project_batch(owner, &unread).await)
    }

    /// Project a batch of notices, querying each distinct thread's unread
    /// count once — the count is identical for every notice of a thread, and
    /// per-notice queries would make a 250-notice replay issue 250 ordered
    /// index scans.
    async fn project_batch(
        &self,
        owner: &RunCompletionOwner,
        notices: &[RunCompletionNotice],
    ) -> Vec<SequencedCompletionEvent> {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut events = Vec::with_capacity(notices.len());
        for notice in notices {
            let unread_count = match counts.get(&notice.thread_id) {
                Some(count) => *count,
                None => {
                    let count = match self
                        .notices
                        .unread_for_thread(owner, &notice.thread_id, 99)
                        .await
                    {
                        Ok(thread_notices) => thread_notices.len(),
                        Err(error) => {
                            // silent-ok: grouped-copy count only; logged.
                            tracing::debug!(
                                target: "ironclaw::reborn::run_completions",
                                %error,
                                "per-thread unread count unavailable; using notice-local floor",
                            );
                            usize::from(!notice.is_read())
                        }
                    };
                    counts.insert(notice.thread_id.clone(), count);
                    count
                }
            };
            events.push(SequencedCompletionEvent {
                sequence: notice.sequence,
                event: RunCompletionStreamEvent::Notice(RunCompletionNoticeEvent {
                    schema: RUN_COMPLETION_NOTICE_SCHEMA.to_string(),
                    sequence: notice.sequence.to_string(),
                    notice_id: notice.notice_id.clone(),
                    run_id: notice.run_id.clone(),
                    thread_id: notice.thread_id.clone(),
                    thread_tag: notice.thread_tag.clone(),
                    completed_at: notice.completed_at.to_rfc3339(),
                    read: notice.is_read(),
                    unread_count_for_thread: u16::try_from(unread_count).unwrap_or(u16::MAX),
                }),
            });
        }
        events
    }

    /// Project one durable notice into its wire event, joining the bounded
    /// per-thread unread count.
    pub async fn notice_event(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
    ) -> RunCompletionNoticeEvent {
        let unread_count = match self
            .notices
            .unread_for_thread(owner, &notice.thread_id, 99)
            .await
        {
            Ok(notices) => notices.len(),
            Err(error) => {
                // silent-ok: the count only feeds grouped badge copy; the
                // notice itself still delivers. Logged so a wrong badge has
                // a server-side trace.
                tracing::debug!(
                    target: "ironclaw::reborn::run_completions",
                    %error,
                    "per-thread unread count unavailable; using notice-local floor",
                );
                usize::from(!notice.is_read())
            }
        };
        RunCompletionNoticeEvent {
            schema: RUN_COMPLETION_NOTICE_SCHEMA.to_string(),
            sequence: notice.sequence.to_string(),
            notice_id: notice.notice_id.clone(),
            run_id: notice.run_id.clone(),
            thread_id: notice.thread_id.clone(),
            thread_tag: notice.thread_tag.clone(),
            completed_at: notice.completed_at.to_rfc3339(),
            read: notice.is_read(),
            unread_count_for_thread: u16::try_from(unread_count).unwrap_or(u16::MAX),
        }
    }

    /// Wake connected pages after a notice write (create or replay).
    pub async fn notice_written(&self, owner: &RunCompletionOwner, notice: &RunCompletionNotice) {
        let event = self.notice_event(owner, notice).await;
        let _ = self.sender(owner).send(SequencedCompletionEvent {
            sequence: notice.sequence,
            event: RunCompletionStreamEvent::Notice(event),
        });
    }

    /// Publish a grant to every connected page; only the named worker
    /// applies it (§5.6).
    pub fn publish_grant(&self, owner: &RunCompletionOwner, notice: &RunCompletionNotice) {
        let CompletionDeliveryState::Granted {
            grant_id,
            browser_instance_id,
            surface,
            state_revision,
            expires_at,
            ..
        } = &notice.delivery
        else {
            return;
        };
        let surface = match surface {
            CompletionSurface::NoSurfaceWatchingThread => {
                RunCompletionGrantSurface::NoSurfaceWatchingThread
            }
            CompletionSurface::InApp => RunCompletionGrantSurface::InApp,
            CompletionSurface::LocalOs => RunCompletionGrantSurface::LocalOs,
            // Web Push is never presented through a browser grant.
            CompletionSurface::WebAppPush => return,
        };
        let _ = self.sender(owner).send(SequencedCompletionEvent {
            sequence: notice.sequence,
            event: RunCompletionStreamEvent::Grant(RunCompletionGrantEvent {
                schema: RUN_COMPLETION_GRANT_SCHEMA.to_string(),
                sequence: notice.sequence.to_string(),
                notice_id: notice.notice_id.clone(),
                grant_id: grant_id.clone(),
                browser_instance_id: browser_instance_id.clone(),
                state_revision: *state_revision,
                surface,
                expires_at: expires_at.to_rfc3339(),
            }),
        });
    }

    /// Publish a clear after a durable read transition (§9.3).
    pub fn publish_clear(&self, owner: &RunCompletionOwner, notice: &RunCompletionNotice) {
        let read_at = match &notice.read {
            super::records::CompletionReadState::Read { read_at, .. } => read_at.to_rfc3339(),
            super::records::CompletionReadState::Unread => return,
        };
        let _ = self.sender(owner).send(SequencedCompletionEvent {
            sequence: notice.sequence,
            event: RunCompletionStreamEvent::Clear(RunCompletionClearEvent {
                schema: RUN_COMPLETION_CLEAR_SCHEMA.to_string(),
                sequence: notice.sequence.to_string(),
                notice_id: notice.notice_id.clone(),
                thread_id: notice.thread_id.clone(),
                thread_tag: notice.thread_tag.clone(),
                read_at,
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_completions::records::{
        CompletionDeliveryState, CompletionReadEvidence, CompletionReadState, CompletionSurface,
        RUN_COMPLETION_NOTICE_VERSION,
    };
    use crate::run_completions::store::{NewRunCompletionNotice, RunCompletionNoticeStore};

    use chrono::{Duration as ChronoDuration, Utc};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_host_api::resource::ResourceScope;

    fn hub() -> (RunCompletionStreamHub, Arc<dyn RunCompletionNotices>) {
        let store = Arc::new(RunCompletionNoticeStore::new(Arc::new(
            ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), |scope: &ResourceScope| {
                MountView::new(vec![
                    MountGrant::new(
                        MountAlias::new(crate::run_completions::store::RUN_NOTICES_MOUNT_ALIAS)?,
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
        (RunCompletionStreamHub::new(Arc::clone(&store)), store)
    }

    /// Seed one durable notice so the ordered indexes exist before the
    /// subscription replay queries them (create_notice owns ensure_index).
    async fn seed(store: &Arc<dyn RunCompletionNotices>) {
        store
            .create_notice(
                &owner(),
                NewRunCompletionNotice {
                    notice_id: "rcn-seed".to_string(),
                    run_id: "run-seed".to_string(),
                    thread_id: "thread-seed".to_string(),
                    agent_id: Some("agent-alpha".to_string()),
                    project_id: None,
                    thread_tag: "rct-seed".to_string(),
                    terminal_projection_ref: "run-completion/rcn-seed".to_string(),
                    completed_at: Utc::now(),
                    arbitration_closes_at: Utc::now() + ChronoDuration::seconds(1),
                },
            )
            .await
            .expect("seed notice");
    }

    fn owner() -> RunCompletionOwner {
        RunCompletionOwner {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
        }
    }

    fn granted_notice(surface: CompletionSurface) -> RunCompletionNotice {
        RunCompletionNotice {
            version: RUN_COMPLETION_NOTICE_VERSION,
            notice_id: "rcn-hub".to_string(),
            sequence: 7,
            tenant_id: "tenant-alpha".to_string(),
            owner_user_id: "user-alpha".to_string(),
            run_id: "run-hub".to_string(),
            thread_id: "thread-hub".to_string(),
            agent_id: Some("agent-alpha".to_string()),
            project_id: None,
            thread_tag: "rct-hub".to_string(),
            terminal_projection_ref: "run-completion/rcn-hub".to_string(),
            completed_at: Utc::now(),
            delivery: CompletionDeliveryState::Granted {
                grant_id: "rcg-1".to_string(),
                browser_instance_id: "browser-a".to_string(),
                surface,
                state_revision: 3,
                expires_at: Utc::now() + ChronoDuration::seconds(2),
                grants_issued: 1,
            },
            read: CompletionReadState::Unread,
            intents: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn publish_grant_emits_only_for_browser_surfaces() {
        let (hub, store) = hub();
        let owner = owner();
        seed(&store).await;
        let (_replay, mut receiver) = hub.subscribe(&owner, None, 10).await.expect("subscribe");

        hub.publish_grant(&owner, &granted_notice(CompletionSurface::InApp));
        let event = receiver.recv().await.expect("grant delivered");
        match event.event {
            RunCompletionStreamEvent::Grant(grant) => {
                assert_eq!(grant.grant_id, "rcg-1");
                assert_eq!(grant.browser_instance_id, "browser-a");
                assert_eq!(
                    grant.surface,
                    ironclaw_product_contracts::run_completions::RunCompletionGrantSurface::InApp
                );
            }
            other => panic!("expected a grant event, got {other:?}"),
        }

        // A push-owned surface is never a browser grant (§5.6): nothing to
        // apply client-side, so nothing is published.
        hub.publish_grant(&owner, &granted_notice(CompletionSurface::WebAppPush));
        // A non-granted state publishes nothing either.
        let mut pending = granted_notice(CompletionSurface::InApp);
        pending.delivery = CompletionDeliveryState::NoExternalTarget {
            settled_at: Utc::now(),
        };
        hub.publish_grant(&owner, &pending);
        assert!(
            matches!(
                receiver.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "no grant frame may be emitted for push-owned or settled states"
        );
    }

    #[tokio::test]
    async fn publish_clear_emits_only_for_read_notices() {
        let (hub, store) = hub();
        let owner = owner();
        seed(&store).await;
        let (_replay, mut receiver) = hub.subscribe(&owner, None, 10).await.expect("subscribe");

        let mut unread = granted_notice(CompletionSurface::InApp);
        hub.publish_clear(&owner, &unread);
        let _ = &unread;
        assert!(
            matches!(
                receiver.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "an unread notice never clears"
        );

        unread.read = CompletionReadState::Read {
            read_at: Utc::now(),
            evidence: CompletionReadEvidence::ReplyRendered {
                browser_instance_id: "browser-a".to_string(),
            },
        };
        hub.publish_clear(&owner, &unread);
        let event = receiver.recv().await.expect("clear delivered");
        match event.event {
            RunCompletionStreamEvent::Clear(clear) => {
                assert_eq!(clear.notice_id, "rcn-hub");
                assert_eq!(clear.thread_tag, "rct-hub");
            }
            other => panic!("expected a clear event, got {other:?}"),
        }
    }
}
