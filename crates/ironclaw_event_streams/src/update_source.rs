use std::{collections::HashMap, sync::Arc};

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
        self.publish_shared(Arc::new(envelope))
    }

    pub fn publish_shared(
        &self,
        envelope: Arc<ProductProjectionEnvelope>,
    ) -> Result<usize, ProjectionStreamError> {
        let key = live_update_key_for_envelope(envelope.as_ref())?;
        let senders = self.senders.lock();
        let sender = senders.get(&key).ok_or(ProjectionStreamError::Source)?;
        sender
            .send(envelope)
            .map_err(|_| ProjectionStreamError::Source)
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
        let sender = senders.entry(key).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.capacity);
            sender
        });
        Ok(sender.subscribe())
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
        | ProductProjectionEnvelope::Debug(_) => ProjectionTarget::Thread { thread_id },
    };
    Ok(scope_key(scope, &target))
}
