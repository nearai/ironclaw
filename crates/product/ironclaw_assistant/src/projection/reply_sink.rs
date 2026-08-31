//! The product projection reply sink: the WebUI/API edge of progressive reply
//! publication (`docs/internal/design/2026-08-31-progressive-reply-publication.md`
//! §9).
//!
//! The deployment's authenticated-session channel binds this sink as its
//! `[channel.reply] transport = "stream"` half, exactly as a vendor package
//! binds its own. Each reconciled revision is converted into product
//! projection items and published through the live update source the SSE and
//! WebSocket transports tail. That broadcast is a latency optimization only:
//! the durable reply journal is what an origin snapshot rebuilds from, so a
//! reconnect or restart never depends on this process's memory.
//!
//! The publisher is late-bound because channel bindings are assembled by the
//! binary before composition builds the projection graph; until composition
//! binds it the sink fails closed with a retryable outcome rather than
//! dropping revisions on the floor.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::ChannelError;
use ironclaw_extension_contracts::reply::{
    ReplyOutcomeReason, ReplyReconcileRequest, ReplySink, ReplySinkEvidence, ReplySinkOutcome,
    ReplySinkReport,
};
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;

use crate::projection::live_progress::LiveProjectionPublisher;

/// The WebUI edge: reconciles reply revisions into product projection items.
#[derive(Default)]
pub struct ProjectionReplySink {
    publisher: OnceLock<Arc<LiveProjectionPublisher>>,
}

impl ProjectionReplySink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the live projection publisher. First write wins so a runtime
    /// cannot swap the stream under in-flight replies; returns whether this
    /// call bound it.
    pub fn bind_publisher(&self, publisher: Arc<LiveProjectionPublisher>) -> bool {
        self.publisher.set(publisher).is_ok()
    }

    pub fn is_bound(&self) -> bool {
        self.publisher.get().is_some()
    }
}

impl std::fmt::Debug for ProjectionReplySink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectionReplySink")
            .field("bound", &self.is_bound())
            .finish()
    }
}

#[async_trait]
impl ReplySink for ProjectionReplySink {
    async fn reconcile(
        &self,
        request: ReplyReconcileRequest,
        _egress: &dyn RestrictedEgress,
    ) -> Result<ReplySinkReport, ChannelError> {
        let Some(publisher) = self.publisher.get() else {
            return Ok(ReplySinkReport {
                outcome: ReplySinkOutcome::Retryable {
                    reason: ReplyOutcomeReason::new(
                        "product projection publisher is not bound yet",
                    ),
                    retry_after: None,
                },
                checkpoint: None,
                evidence: ReplySinkEvidence::default(),
            });
        };
        let checkpoint = publisher.publish_reply_revision(&request)?;
        // The browser is not a provider: there is no message id to cite and
        // no read-back to perform, so the evidence stays honestly empty.
        Ok(ReplySinkReport::applied(
            Some(checkpoint),
            ReplySinkEvidence::default(),
        ))
    }
}
