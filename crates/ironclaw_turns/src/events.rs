use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::{TurnError, TurnRunId, TurnScope, TurnStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct EventCursor(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnEventKind {
    Submitted,
    Resumed,
    RunnerClaimed,
    RunnerHeartbeat,
    Blocked,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnLifecycleEvent {
    pub cursor: EventCursor,
    pub scope: TurnScope,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub kind: TurnEventKind,
    pub sanitized_reason: Option<String>,
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError>;
}

#[derive(Default)]
pub struct InMemoryTurnEventSink {
    events: Mutex<Vec<TurnLifecycleEvent>>,
}

impl InMemoryTurnEventSink {
    pub fn events(&self) -> Vec<TurnLifecycleEvent> {
        match self.events.lock() {
            Ok(events) => events.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl TurnEventSink for InMemoryTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        let mut events = self.events.lock().map_err(|_| TurnError::Backend {
            reason: "turn event sink mutex poisoned".to_string(),
        })?;
        events.push(event);
        Ok(())
    }
}
