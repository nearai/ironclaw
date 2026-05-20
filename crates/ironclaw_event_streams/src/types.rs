use std::sync::Arc;

use ironclaw_event_projections::{
    ProjectionCursor, ProjectionReplay, ProjectionScope, ProjectionSnapshot,
};
use ironclaw_host_api::{InvocationId, MissionId, ProcessId, ThreadId};
use ironclaw_outbound::{OutboundPushKind, ProjectionUpdateRef};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{admission::ProjectionStreamAdmissionPermit, error::ProjectionStreamError};

const DEFAULT_SUBSCRIPTION_BUFFER: usize = 16;
const MIN_SUBSCRIPTION_BUFFER: usize = 1;
const MAX_SUBSCRIPTION_BUFFER: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionFetchRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionFetchResponse {
    pub snapshot: ProductProjectionEnvelope,
    pub cursor: ProjectionCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscribeRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
    pub after_cursor: Option<ProjectionCursor>,
    pub limit: usize,
    pub capabilities: SubscriberCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriberCapabilities {
    pub buffer_capacity: usize,
}

impl Default for SubscriberCapabilities {
    fn default() -> Self {
        Self {
            buffer_capacity: DEFAULT_SUBSCRIPTION_BUFFER,
        }
    }
}

impl SubscriberCapabilities {
    pub(crate) fn bounded_buffer_capacity(&self) -> Result<usize, ProjectionStreamError> {
        if self.buffer_capacity > MAX_SUBSCRIPTION_BUFFER {
            return Err(ProjectionStreamError::InvalidRequest {
                reason: "projection subscription buffer capacity exceeds host maximum",
            });
        }
        Ok(self.buffer_capacity.max(MIN_SUBSCRIPTION_BUFFER))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushCandidatesForUpdateRequest {
    pub actor: TurnActor,
    pub projection_scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
    pub scope: TurnScope,
    pub turn_run_id: Option<TurnRunId>,
    pub reply_target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionViewClass {
    ProductThread,
    ProductMission,
    ProductRun,
    DeliveryStatus,
    DebugSupport,
    AdminAudit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionTarget {
    Thread { thread_id: ThreadId },
    Mission { mission_id: MissionId },
    Run { invocation_id: InvocationId },
    Process { process_id: ProcessId },
    DeliveryStatus { thread_id: ThreadId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStreamItem {
    Snapshot(ProductProjectionEnvelope),
    Update(Arc<ProductProjectionEnvelope>),
    RebaseRequired {
        snapshot: Box<ProductProjectionEnvelope>,
        rebased_from: Option<ProjectionCursor>,
        snapshot_cursor: ProjectionCursor,
    },
    Lagged {
        reason: LagReason,
        snapshot_cursor: ProjectionCursor,
    },
    KeepAlive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LagReason {
    SourceLagged,
    SourceFailed,
    SubscriberBackpressure,
    RedactionBlocked,
    AccessBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductProjectionEnvelope {
    ThreadSnapshot(ProjectionSnapshot),
    ThreadUpdates(ProjectionReplay),
    DeliveryStatus(DeliveryStatusProjectionPayload),
    Debug(DebugProjectionPayload),
}

impl ProductProjectionEnvelope {
    pub fn cursor(&self) -> ProjectionCursor {
        match self {
            Self::ThreadSnapshot(snapshot) => snapshot.next_cursor.clone(),
            Self::ThreadUpdates(replay) => replay.next_cursor.clone(),
            Self::DeliveryStatus(payload) => payload.cursor.clone(),
            Self::Debug(payload) => payload.cursor.clone(),
        }
    }

    pub fn scope(&self) -> &ProjectionScope {
        match self {
            Self::ThreadSnapshot(snapshot) => &snapshot.next_cursor.scope,
            Self::ThreadUpdates(replay) => &replay.next_cursor.scope,
            Self::DeliveryStatus(payload) => &payload.cursor.scope,
            Self::Debug(payload) => &payload.cursor.scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryStatusProjectionPayload {
    pub cursor: ProjectionCursor,
    pub delivery_ref: ProjectionUpdateRef,
    pub status: DeliveryProjectionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryProjectionStatus {
    Pending,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugProjectionPayload {
    pub cursor: ProjectionCursor,
    pub redacted_summary: String,
}

pub struct ProjectionSubscription {
    receiver: mpsc::Receiver<ProjectionStreamItem>,
    terminal_receiver: mpsc::Receiver<ProjectionStreamItem>,
    terminated: bool,
    observed_cursor: Option<ProjectionCursor>,
    admission: Option<ProjectionStreamAdmissionPermit>,
}

impl std::fmt::Debug for ProjectionSubscription {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectionSubscription")
            .field("receiver", &"<bounded_projection_stream>")
            .field(
                "admission",
                &"<optional_projection_stream_admission_permit>",
            )
            .finish()
    }
}

impl ProjectionSubscription {
    pub(crate) fn new(
        receiver: mpsc::Receiver<ProjectionStreamItem>,
        terminal_receiver: mpsc::Receiver<ProjectionStreamItem>,
        admission: ProjectionStreamAdmissionPermit,
    ) -> Self {
        Self {
            receiver,
            terminal_receiver,
            terminated: false,
            observed_cursor: None,
            admission: Some(admission),
        }
    }

    pub async fn next(&mut self) -> Option<ProjectionStreamItem> {
        if self.terminated {
            return None;
        }
        match self.terminal_receiver.try_recv() {
            Ok(item) => {
                self.terminated = true;
                self.receiver.close();
                self.release_admission();
                return Some(with_observed_terminal_cursor(
                    item,
                    self.observed_cursor.as_ref(),
                ));
            }
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {}
        }

        tokio::select! {
            biased;
            terminal = self.terminal_receiver.recv() => {
                if let Some(item) = terminal {
                    self.terminated = true;
                    self.receiver.close();
                    self.release_admission();
                    Some(with_observed_terminal_cursor(item, self.observed_cursor.as_ref()))
                } else {
                    let item = self.receiver.recv().await;
                    self.observe_item(item)
                }
            }
            item = self.receiver.recv() => self.observe_item(item),
        }
    }

    fn observe_item(&mut self, item: Option<ProjectionStreamItem>) -> Option<ProjectionStreamItem> {
        if let Some(item) = item {
            if let Some(cursor) = observable_cursor(&item) {
                self.observed_cursor = Some(cursor);
            }
            if matches!(item, ProjectionStreamItem::Lagged { .. }) {
                self.terminated = true;
                self.receiver.close();
                self.release_admission();
            }
            Some(item)
        } else {
            self.terminated = true;
            self.release_admission();
            None
        }
    }

    fn release_admission(&mut self) {
        self.admission.take();
    }
}

fn observable_cursor(item: &ProjectionStreamItem) -> Option<ProjectionCursor> {
    match item {
        ProjectionStreamItem::Snapshot(envelope) => Some(envelope.cursor()),
        ProjectionStreamItem::Update(envelope) => Some(envelope.cursor()),
        ProjectionStreamItem::RebaseRequired {
            snapshot_cursor, ..
        }
        | ProjectionStreamItem::Lagged {
            snapshot_cursor, ..
        } => Some(snapshot_cursor.clone()),
        ProjectionStreamItem::KeepAlive => None,
    }
}

fn with_observed_terminal_cursor(
    item: ProjectionStreamItem,
    observed_cursor: Option<&ProjectionCursor>,
) -> ProjectionStreamItem {
    match (item, observed_cursor) {
        (
            ProjectionStreamItem::Lagged {
                reason,
                snapshot_cursor,
            },
            Some(observed),
        ) if observed.scope == snapshot_cursor.scope => ProjectionStreamItem::Lagged {
            reason,
            snapshot_cursor: observed.clone(),
        },
        (item, _) => item,
    }
}

pub fn keep_alive_item() -> ProjectionStreamItem {
    ProjectionStreamItem::KeepAlive
}
