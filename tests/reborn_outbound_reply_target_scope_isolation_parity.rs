#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_product_adapters::{
    DeliveryStatus, ExternalConversationRef, FakeProtocolHttpEgress, FinalReplyView,
    ProductAdapter, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductRenderOutcome, ProjectionCursor,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use reborn_support::{
    delivery::RecordingOutboundDeliverySink, test_adapter::RebornTestProductAdapter,
};

#[tokio::test]
async fn reborn_outbound_reply_target_scope_isolation_parity() {
    let adapter_a =
        RebornTestProductAdapter::new("reborn-test", "install-alpha").expect("adapter alpha");
    let adapter_b =
        RebornTestProductAdapter::new("reborn-test", "install-beta").expect("adapter beta");
    let target_a =
        ReplyTargetBindingRef::new("reply:install-alpha:room-shared").expect("target alpha");
    let target_b =
        ReplyTargetBindingRef::new("reply:install-beta:room-shared").expect("target beta");
    let sink_a = RecordingOutboundDeliverySink::new();
    let sink_b = RecordingOutboundDeliverySink::new();
    let egress = FakeProtocolHttpEgress::new(["api.example.test".to_string()]);

    let envelope_a = outbound_envelope(&adapter_a, target_a.clone(), "alpha reply");
    let envelope_b = outbound_envelope(&adapter_b, target_b.clone(), "beta reply");

    let outcome_a = adapter_a
        .render_outbound(envelope_a, &egress, &sink_a)
        .await
        .expect("render alpha outbound");
    let outcome_b = adapter_b
        .render_outbound(envelope_b, &egress, &sink_b)
        .await
        .expect("render beta outbound");

    assert_eq!(outcome_a, ProductRenderOutcome::DeliveryRecorded);
    assert_eq!(outcome_b, ProductRenderOutcome::DeliveryRecorded);

    assert_delivered_only_to(&sink_a.statuses(), &target_a, &target_b, "alpha");
    assert_delivered_only_to(&sink_b.statuses(), &target_b, &target_a, "beta");
}

fn outbound_envelope(
    adapter: &RebornTestProductAdapter,
    target_ref: ReplyTargetBindingRef,
    text: &str,
) -> ProductOutboundEnvelope {
    ProductOutboundEnvelope::new(
        adapter.adapter_id().clone(),
        adapter.installation_id().clone(),
        ProductOutboundTarget::new(
            target_ref,
            ExternalConversationRef::new(None, "room-shared", None, None)
                .expect("conversation ref"),
            None,
        ),
        ProjectionCursor::new("cursor:reply-target-scope").expect("projection cursor"),
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.to_string(),
            generated_at: chrono::Utc::now(),
        }),
    )
}

fn assert_delivered_only_to(
    statuses: &[DeliveryStatus],
    expected: &ReplyTargetBindingRef,
    forbidden: &ReplyTargetBindingRef,
    label: &str,
) {
    assert_eq!(statuses.len(), 1, "{label} should record one delivery");
    assert!(
        matches!(
            &statuses[0],
            DeliveryStatus::Delivered { target, .. } if target == expected && target != forbidden
        ),
        "{label} delivery should target only matching reply binding; statuses={statuses:?}"
    );
}
