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
        let mut events = Vec::with_capacity(replayed.len());
        for notice in &replayed {
            events.push(SequencedCompletionEvent {
                sequence: notice.sequence,
                event: RunCompletionStreamEvent::Notice(self.notice_event(owner, notice).await),
            });
        }
        Ok((events, receiver))
    }

    /// The bounded rebase snapshot (§7.6): unread/unsettled notices only.
    pub async fn rebase_snapshot(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<Vec<SequencedCompletionEvent>, RunCompletionStoreError> {
        let unread = self.notices.unread_snapshot(owner).await?;
        let mut events = Vec::with_capacity(unread.len());
        for notice in &unread {
            events.push(SequencedCompletionEvent {
                sequence: notice.sequence,
                event: RunCompletionStreamEvent::Notice(self.notice_event(owner, notice).await),
            });
        }
        Ok(events)
    }

    /// Project one durable notice into its wire event, joining the bounded
    /// per-thread unread count.
    pub async fn notice_event(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
    ) -> RunCompletionNoticeEvent {
        let unread_count = self
            .notices
            .unread_for_thread(owner, &notice.thread_id, 99)
            .await
            .map(|notices| notices.len())
            .unwrap_or(usize::from(!notice.is_read()));
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
            CompletionSurface::WebPush => return,
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
