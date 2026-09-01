//! The product projection reply sink: the WebUI edge of progressive reply
//! publication. A reconciled revision becomes the live projection items the
//! browser already renders; the checkpoint makes repeats idempotent; an
//! unbound sink fails closed instead of dropping revisions.

use super::*;
use crate::projection::reply_sink::ProjectionReplySink;
use ironclaw_extension_contracts::reply::{
    ReplyActivityState, ReplyAudience, ReplyDisplayPreview, ReplyDisplayText, ReplyDocument,
    ReplyItemId, ReplyPhase, ReplyReasoningText, ReplyReconcilePoint, ReplyReconcileRequest,
    ReplyRevision, ReplySink, ReplySinkCheckpoint, ReplySinkOutcome, ReplyTarget,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_product_contracts::outbound::ProjectionCursor;
use std::sync::Arc;

struct NoEgress;

#[async_trait]
impl RestrictedEgress for NoEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

struct SinkFixture {
    services: RebornProjectionServices,
    sink: ProjectionReplySink,
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    invocation_id: InvocationId,
}

fn sink_fixture(label: &str) -> SinkFixture {
    let tenant_id = TenantId::new(format!("{label}-tenant")).unwrap();
    let user_id = UserId::new(format!("{label}-user")).unwrap();
    let agent_id = AgentId::new(format!("{label}-agent")).unwrap();
    let thread_id = ThreadId::new(format!("{label}-thread")).unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new(format!("{label}-reply")).unwrap(),
    );
    let sink = ProjectionReplySink::new();
    assert!(sink.bind_publisher(services.live_projection_publisher(user_id.clone())));
    SinkFixture {
        services,
        sink,
        scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
        actor: TurnActor::new(user_id),
        run_id: TurnRunId::new(),
        invocation_id: InvocationId::new(),
    }
}

impl SinkFixture {
    fn target(&self) -> ReplyTarget {
        ReplyTarget {
            scope: self.scope.clone(),
            actor: self.actor.clone(),
            run_id: self.run_id,
            conversation: None,
            thread_anchor: None,
            audience: ReplyAudience::Private,
        }
    }

    fn request(
        &self,
        revision: u64,
        document: ReplyDocument,
        checkpoint: Option<ReplySinkCheckpoint>,
    ) -> ReplyReconcileRequest {
        let point = if document.is_terminal() {
            ReplyReconcilePoint::Terminal
        } else if revision == 1 {
            ReplyReconcilePoint::Opened
        } else {
            ReplyReconcilePoint::Progress
        };
        ReplyReconcileRequest {
            revision: ReplyRevision { revision, document },
            point,
            target: self.target(),
            reply_context: None,
            checkpoint,
            extension_generation: 0,
            materialized_attachments: Vec::new(),
        }
    }

    async fn drain_items(
        &self,
        after: Option<ProjectionCursor>,
    ) -> (Vec<ProductProjectionItem>, Option<ProjectionCursor>) {
        let envelopes = self
            .services
            .product_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor: self.actor.clone(),
                scope: self.scope.clone(),
                after_cursor: after,
            })
            .await
            .unwrap();
        let mut items = Vec::new();
        let mut last = None;
        for envelope in envelopes {
            if let ProductOutboundPayload::ProjectionUpdate { state }
            | ProductOutboundPayload::ProjectionSnapshot { state } = envelope.payload()
            {
                items.extend(state.items.iter().cloned());
            }
            last = Some(envelope.projection_cursor().clone());
        }
        (items, last)
    }
}

fn text(value: &str) -> ReplyDisplayText {
    ReplyDisplayText::new(value).unwrap()
}

#[tokio::test]
async fn projection_reply_sink_publishes_each_facet_as_the_live_items_the_browser_renders() {
    let fixture = sink_fixture("reply-sink-facets");
    let mut document = ReplyDocument::default();
    document.note_phase(ReplyPhase::Thinking);
    document.close_reasoning(ReplyReasoningText::new("Checking the workspace first.").unwrap());
    document.append_answer("Here is ");
    document.activity_started(
        ReplyItemId::new(fixture.invocation_id.to_string()).unwrap(),
        text("acme.search"),
        Some(ReplyDisplayPreview::new("query: runbook").unwrap()),
    );
    document.set_status(text("Searching the runbook"), None);

    let report = fixture
        .sink
        .reconcile(fixture.request(1, document.clone(), None), &NoEgress)
        .await
        .unwrap();
    assert!(report.outcome.is_applied());
    let checkpoint = report
        .checkpoint
        .clone()
        .expect("a checkpoint after the first apply");

    let (items, _) = fixture.drain_items(None).await;
    assert!(
        items.iter().any(|item| matches!(
            item,
            ProductProjectionItem::Thinking { run_id: Some(run), body, .. }
                if *run == fixture.run_id && body == "Checking the workspace first."
        )),
        "reasoning summary becomes a thinking item: {items:?}"
    );
    let first_thinking_ids: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            ProductProjectionItem::Thinking { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(first_thinking_ids.len(), 1);
    assert!(
        items.iter().any(|item| matches!(
            item,
            ProductProjectionItem::Text { id, run_id: Some(run), body, finalized: false }
                if *run == fixture.run_id && body == "Here is " && id == &format!("text:{}", fixture.run_id)
        )),
        "the cumulative answer becomes the run's live text item: {items:?}"
    );
    assert!(
        items.iter().any(|item| matches!(
            item,
            ProductProjectionItem::CapabilityActivity(view)
                if view.invocation_id == fixture.invocation_id
                    && view.status == CapabilityActivityStatusView::Started
                    && view.capability_id.as_str() == "acme.search"
                    && view.input_summary.as_deref() == Some("query: runbook")
        )),
        "an activity row becomes a capability activity card keyed by invocation: {items:?}"
    );
    assert!(
        items.iter().any(|item| matches!(
            item,
            ProductProjectionItem::WorkSummary { run_id, body, .. }
                if *run_id == fixture.run_id && body == "Searching the runbook"
        )),
        "the status summary becomes a work summary: {items:?}"
    );

    // Revision 2: the activity finishes and the answer grows. Only the
    // changed facets are re-published; the reasoning segment is not repeated.
    document.activity_finished(
        ReplyItemId::new(fixture.invocation_id.to_string()).unwrap(),
        ReplyActivityState::Completed,
        Some(ReplyDisplayPreview::new("3 matches").unwrap()),
        None,
    );
    document.append_answer("what I found.");
    let report = fixture
        .sink
        .reconcile(fixture.request(2, document, Some(checkpoint)), &NoEgress)
        .await
        .unwrap();
    assert!(report.outcome.is_applied());

    // The product stream retains the latest value per stable identity, so a
    // drain shows the current state of every facet: the activity card now
    // reads Completed, the answer is the cumulative text, and the reasoning
    // segment still carries its ORIGINAL id — a re-publish would have minted
    // a new one.
    let (items, _) = fixture.drain_items(None).await;
    assert!(
        items.iter().any(|item| matches!(
            item,
            ProductProjectionItem::CapabilityActivity(view)
                if view.invocation_id == fixture.invocation_id
                    && view.status == CapabilityActivityStatusView::Completed
        )),
        "the finished activity is re-published: {items:?}"
    );
    assert!(
        items.iter().any(|item| matches!(
            item,
            ProductProjectionItem::Text { body, finalized: false, .. } if body == "Here is what I found."
        )),
        "the answer is re-published cumulatively: {items:?}"
    );
    let thinking_ids: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            ProductProjectionItem::Thinking { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        thinking_ids, first_thinking_ids,
        "an already-published reasoning segment is not repeated under a new id: {items:?}"
    );
}

#[tokio::test]
async fn projection_reply_sink_repeats_nothing_for_a_repeated_revision() {
    let fixture = sink_fixture("reply-sink-idempotent");
    let mut document = ReplyDocument::default();
    document.append_answer("stable");
    let report = fixture
        .sink
        .reconcile(fixture.request(1, document.clone(), None), &NoEgress)
        .await
        .unwrap();
    let checkpoint = report.checkpoint.expect("checkpoint");
    let (first, first_cursor) = fixture.drain_items(None).await;
    assert!(
        first
            .iter()
            .any(|item| matches!(item, ProductProjectionItem::Text { .. }))
    );

    let repeat = fixture
        .sink
        .reconcile(fixture.request(1, document, Some(checkpoint)), &NoEgress)
        .await
        .unwrap();
    assert!(repeat.outcome.is_applied());
    // Nothing new was published: the stream head has not moved past the
    // cursor the first drain ended on.
    let (again, again_cursor) = fixture.drain_items(None).await;
    assert_eq!(again, first, "a repeated revision publishes nothing new");
    assert_eq!(again_cursor, first_cursor);
}

#[tokio::test]
async fn projection_reply_sink_fails_closed_until_composition_binds_the_publisher() {
    let fixture = sink_fixture("reply-sink-unbound");
    let unbound = ProjectionReplySink::new();
    let mut document = ReplyDocument::default();
    document.append_answer("nobody sees this yet");
    let report = unbound
        .reconcile(fixture.request(1, document, None), &NoEgress)
        .await
        .unwrap();
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Retryable { .. }),
        "an unbound sink must ask for a retry, not claim Applied: {:?}",
        report.outcome
    );
    assert!(report.checkpoint.is_none());
    let (items, _) = fixture.drain_items(None).await;
    assert!(items.is_empty());
}

/// `append_reasoning` grows the OPEN tail segment in place — the segment
/// count never moves — so the checkpoint must track the tail's content, not
/// only how many segments exist. Every delta republishes the same stable
/// item id (the browser upserts one thinking card), the closing replacement
/// publishes the final text under that id, and an unchanged document
/// publishes nothing.
#[tokio::test]
async fn open_reasoning_deltas_republish_one_stable_thinking_item() {
    let fixture = sink_fixture("reply-sink-open-reasoning");
    let mut document = ReplyDocument::default();
    document.append_reasoning(&ReplyReasoningText::new("Reading the config").unwrap());

    let report = fixture
        .sink
        .reconcile(fixture.request(1, document.clone(), None), &NoEgress)
        .await
        .unwrap();
    assert!(report.outcome.is_applied());
    let checkpoint = report.checkpoint.clone().expect("checkpoint");
    let (items, cursor) = fixture.drain_items(None).await;
    let thinking: Vec<(String, String)> = items
        .iter()
        .filter_map(|item| match item {
            ProductProjectionItem::Thinking { id, body, .. } => Some((id.clone(), body.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(thinking.len(), 1, "first fragment publishes: {items:?}");
    let (stable_id, body) = thinking[0].clone();
    assert_eq!(body, "Reading the config");

    // A later fragment grows the SAME open segment in place.
    document.append_reasoning(&ReplyReasoningText::new(", then the mounts").unwrap());
    let report = fixture
        .sink
        .reconcile(
            fixture.request(2, document.clone(), Some(checkpoint)),
            &NoEgress,
        )
        .await
        .unwrap();
    assert!(report.outcome.is_applied());
    let checkpoint = report.checkpoint.clone().expect("checkpoint");
    let (items, cursor) = fixture.drain_items(cursor).await;
    let thinking: Vec<(String, String)> = items
        .iter()
        .filter_map(|item| match item {
            ProductProjectionItem::Thinking { id, body, .. } => Some((id.clone(), body.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        thinking,
        vec![(
            stable_id.clone(),
            "Reading the config, then the mounts".to_string()
        )],
        "the grown open segment republishes under its stable id"
    );

    // Nothing changed: nothing republishes.
    let report = fixture
        .sink
        .reconcile(
            fixture.request(3, document.clone(), Some(checkpoint)),
            &NoEgress,
        )
        .await
        .unwrap();
    assert!(report.outcome.is_applied());
    let checkpoint = report.checkpoint.clone().expect("checkpoint");
    let (items, cursor) = fixture.drain_items(cursor).await;
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, ProductProjectionItem::Thinking { .. })),
        "an unchanged open segment is not republished: {items:?}"
    );

    // The closing replacement lands as the final text of the same item.
    document.close_reasoning(ReplyReasoningText::new("Checked config and mounts.").unwrap());
    document.append_reasoning(&ReplyReasoningText::new("Next: the manifest").unwrap());
    let report = fixture
        .sink
        .reconcile(
            fixture.request(4, document.clone(), Some(checkpoint)),
            &NoEgress,
        )
        .await
        .unwrap();
    assert!(report.outcome.is_applied());
    let (items, _) = fixture.drain_items(cursor).await;
    let thinking: Vec<(String, String)> = items
        .iter()
        .filter_map(|item| match item {
            ProductProjectionItem::Thinking { id, body, .. } => Some((id.clone(), body.clone())),
            _ => None,
        })
        .collect();
    assert!(
        thinking.contains(&(stable_id.clone(), "Checked config and mounts.".to_string())),
        "closure replaces the first item's text: {thinking:?}"
    );
    let second: Vec<_> = thinking.iter().filter(|(id, _)| id != &stable_id).collect();
    assert_eq!(
        second.len(),
        1,
        "the next open segment is its own item: {thinking:?}"
    );
    assert_eq!(second[0].1, "Next: the manifest");
}

/// The same status republished after the checkpoint was lost (an unreadable
/// checkpoint, a publication resumed on another node) must keep ONE stable
/// work-summary id, so the browser upserts the status line instead of
/// appending a second identical bubble.
#[tokio::test(flavor = "multi_thread")]
async fn a_republished_status_keeps_one_stable_work_summary_id() {
    let fixture = sink_fixture("stable-status");
    let mut document = ReplyDocument::default();
    document.set_status(
        ironclaw_extension_contracts::reply::ReplyDisplayText::new("Searching the runbook")
            .unwrap(),
        None,
    );

    let report = fixture
        .sink
        .reconcile(fixture.request(1, document.clone(), None), &NoEgress)
        .await
        .unwrap();
    let (items, cursor) = fixture.drain_items(None).await;
    let first_id = items
        .iter()
        .find_map(|item| match item {
            ProductProjectionItem::WorkSummary { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("the status publishes a work summary");

    // The checkpoint is LOST: the republish starts from a fresh checkpoint,
    // exactly like a resumed publisher on another node.
    let _ = report;
    fixture
        .sink
        .reconcile(fixture.request(2, document, None), &NoEgress)
        .await
        .unwrap();
    let (items, _) = fixture.drain_items(cursor).await;
    let republished_ids: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            ProductProjectionItem::WorkSummary { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !republished_ids.is_empty(),
        "the republished status publishes a work summary"
    );
    assert!(
        republished_ids.iter().all(|id| id == &first_id),
        "a republished status upserts under one stable id; got {republished_ids:?} vs {first_id:?}"
    );
}
