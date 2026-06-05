use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionScope;
use ironclaw_turns::TurnActor;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    error::ProjectionStreamError,
    keys::{ScopeAdmissionKey, scope_key},
    types::{ProductProjectionEnvelope, ProjectionTarget, ProjectionViewClass},
};

#[async_trait]
pub trait ProjectionUpdateSource: Send + Sync {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<broadcast::Receiver<Arc<ProductProjectionEnvelope>>, ProjectionStreamError>;

    async fn replay_after(
        &self,
        _request: ProjectionLiveUpdateRequest,
        _after: Option<ironclaw_event_projections::ProjectionCursor>,
        _limit: usize,
    ) -> Result<Vec<Arc<ProductProjectionEnvelope>>, ProjectionStreamError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionLiveUpdateRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
}

pub struct InMemoryProjectionUpdateSource {
    capacity: usize,
    senders: Mutex<HashMap<ScopeAdmissionKey, broadcast::Sender<Arc<ProductProjectionEnvelope>>>>,
    replay: Mutex<HashMap<ScopeAdmissionKey, InMemoryProjectionReplayBuffer>>,
}

impl InMemoryProjectionUpdateSource {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            senders: Mutex::new(HashMap::new()),
            replay: Mutex::new(HashMap::new()),
        }
    }

    pub fn publish(
        &self,
        envelope: ProductProjectionEnvelope,
    ) -> Result<usize, ProjectionStreamError> {
        self.publish_shared(Arc::new(envelope))
    }

    pub fn publish_shared(
        &self,
        envelope: Arc<ProductProjectionEnvelope>,
    ) -> Result<usize, ProjectionStreamError> {
        let key = live_update_key_for_envelope(envelope.as_ref())?;
        self.remember(&key, Arc::clone(&envelope))?;
        let sender = {
            let mut senders = self.senders.lock();
            prune_inactive_senders(&mut senders);
            senders
                .get(&key)
                .cloned()
                .ok_or(ProjectionStreamError::Source)?
        };
        match sender.send(envelope) {
            Ok(count) => Ok(count),
            Err(_) => {
                self.remove_inactive_sender(&key);
                Err(ProjectionStreamError::Source)
            }
        }
    }

    fn remember(
        &self,
        key: &ScopeAdmissionKey,
        envelope: Arc<ProductProjectionEnvelope>,
    ) -> Result<(), ProjectionStreamError> {
        let mut replay = self.replay.lock();
        let buffer = replay
            .entry(key.clone())
            .or_insert_with(|| InMemoryProjectionReplayBuffer::new(self.capacity));
        buffer.push(envelope);
        Ok(())
    }

    fn remove_inactive_sender(&self, key: &ScopeAdmissionKey) {
        let mut senders = self.senders.lock();
        if senders
            .get(key)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            senders.remove(key);
        }
    }
}

#[async_trait]
impl ProjectionUpdateSource for InMemoryProjectionUpdateSource {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<broadcast::Receiver<Arc<ProductProjectionEnvelope>>, ProjectionStreamError> {
        let key = scope_key(&request.scope, &request.target);
        let mut senders = self.senders.lock();
        prune_inactive_senders(&mut senders);
        let sender = senders.entry(key).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.capacity);
            sender
        });
        Ok(sender.subscribe())
    }

    async fn replay_after(
        &self,
        request: ProjectionLiveUpdateRequest,
        after: Option<ironclaw_event_projections::ProjectionCursor>,
        limit: usize,
    ) -> Result<Vec<Arc<ProductProjectionEnvelope>>, ProjectionStreamError> {
        let key = scope_key(&request.scope, &request.target);
        let replay = self.replay.lock();
        let Some(buffer) = replay.get(&key) else {
            return Ok(Vec::new());
        };
        buffer.replay_after(after, limit)
    }
}

struct InMemoryProjectionReplayBuffer {
    capacity: usize,
    entries: VecDeque<Arc<ProductProjectionEnvelope>>,
}

impl InMemoryProjectionReplayBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    fn push(&mut self, envelope: Arc<ProductProjectionEnvelope>) {
        self.entries.push_back(envelope);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    fn replay_after(
        &self,
        after: Option<ironclaw_event_projections::ProjectionCursor>,
        limit: usize,
    ) -> Result<Vec<Arc<ProductProjectionEnvelope>>, ProjectionStreamError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                after.as_ref().is_none_or(|after| {
                    entry.scope() == &after.scope && entry.cursor().runtime > after.runtime
                })
            })
            .take(limit)
            .cloned()
            .collect())
    }
}

fn live_update_key_for_envelope(
    envelope: &ProductProjectionEnvelope,
) -> Result<ScopeAdmissionKey, ProjectionStreamError> {
    let scope = envelope.scope();
    let thread_id =
        scope
            .read_scope
            .thread_id
            .clone()
            .ok_or(ProjectionStreamError::InvalidRequest {
                reason: "projection live update is missing thread target",
            })?;
    let target = match envelope {
        ProductProjectionEnvelope::DeliveryStatus(_) => {
            ProjectionTarget::DeliveryStatus { thread_id }
        }
        ProductProjectionEnvelope::ThreadSnapshot(_)
        | ProductProjectionEnvelope::ThreadUpdates(_)
        | ProductProjectionEnvelope::ThreadLiveUpdate(_)
        | ProductProjectionEnvelope::Debug(_) => ProjectionTarget::Thread { thread_id },
    };
    Ok(scope_key(scope, &target))
}

fn prune_inactive_senders(
    senders: &mut HashMap<ScopeAdmissionKey, broadcast::Sender<Arc<ProductProjectionEnvelope>>>,
) {
    senders.retain(|_, sender| sender.receiver_count() > 0);
}
