use async_trait::async_trait;
use ironclaw_event_projections::ProjectionScope;
use ironclaw_turns::TurnActor;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::broadcast;

use crate::{
    error::ProjectionStreamError,
    keys::{ProjectionScopeKey, projection_scope_key},
    types::{ProductProjectionEnvelope, ProjectionTarget, ProjectionViewClass},
};

#[async_trait]
pub trait ProjectionUpdateSource: Send + Sync {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<broadcast::Receiver<Arc<ProductProjectionEnvelope>>, ProjectionStreamError>;
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
    senders: Mutex<HashMap<ProjectionScopeKey, broadcast::Sender<Arc<ProductProjectionEnvelope>>>>,
}

impl InMemoryProjectionUpdateSource {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            senders: Mutex::new(HashMap::new()),
        }
    }

    pub fn publish(
        &self,
        envelope: ProductProjectionEnvelope,
    ) -> Result<usize, ProjectionStreamError> {
        let key = projection_scope_key(envelope.scope());
        let sender = self
            .senders()
            .get(&key)
            .cloned()
            .ok_or(ProjectionStreamError::Source)?;
        sender
            .send(Arc::new(envelope))
            .map_err(|_| ProjectionStreamError::Source)
    }

    fn senders(
        &self,
    ) -> MutexGuard<
        '_,
        HashMap<ProjectionScopeKey, broadcast::Sender<Arc<ProductProjectionEnvelope>>>,
    > {
        self.senders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ProjectionUpdateSource for InMemoryProjectionUpdateSource {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<broadcast::Receiver<Arc<ProductProjectionEnvelope>>, ProjectionStreamError> {
        let key = projection_scope_key(&request.scope);
        let mut senders = self.senders();
        let sender = senders.entry(key).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.capacity);
            sender
        });
        Ok(sender.subscribe())
    }
}
