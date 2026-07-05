//! C-ATTACH: `attachment_read_port` int-tier coverage.
//!
//! Production wires `attachment_read_port` from the local-dev workspace
//! filesystem (`ProjectScopedAttachmentReader`,
//! `crates/ironclaw_reborn_composition/src/runtime.rs:3328-3334`) so the loop
//! model port reads landed attachment bytes back for the gateway
//! (`convert_messages`) to build a `ContentPart::ImageUrl` for vision-capable
//! models. Regression: this port was `None` everywhere pre-fix, so images
//! silently degraded to the textual `<attachments>` pointer.
//!
//! Wires the read port + real `InboundAttachmentLander` via
//! `RebornIntegrationGroup::attachment_tools()`, lands an image, routes
//! through a vision-pattern model id, and asserts the model request carried
//! the image as a `data:` URL.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use axum::http::StatusCode;
use ironclaw_product_workflow::RebornServices;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::webui_mount::{get_raw, mount_webui_v2_router, webui_caller_for};
use std::sync::Arc;

/// A vision-capable model id per `ironclaw_llm::vision_models::VISION_PATTERNS`.
const VISION_MODEL: &str = "claude-3-5-sonnet-20241022";
const PNG_MIME: &str = "image/png";
const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3, 4];

#[tokio::test]
async fn landed_image_attachment_reaches_the_model_as_a_multimodal_part() {
    let group = RebornIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let harness = group
        .thread("conv-attach")
        .with_model_override(VISION_MODEL)
        .script([RebornScriptedReply::text("I see a diagram")])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn_with_image_attachment(
            "what's in this image?",
            "diagram.png",
            PNG_MIME,
            PNG_BYTES.to_vec(),
        )
        .await
        .expect("turn completes");

    harness
        .assert_model_saw_image_attachment(PNG_MIME, PNG_BYTES)
        .await
        .expect("landed image bytes reached the model intact as a multimodal content part");
}

/// Negative control: without a vision-model override, `convert_messages`
/// drops the image part (text-only fallback) — proves the override knob, not
/// just the read port, is load-bearing.
#[tokio::test]
async fn non_vision_model_does_not_receive_a_multimodal_image_part() {
    let group = RebornIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let harness = group
        .thread("conv-attach-no-vision")
        .script([RebornScriptedReply::text("got it")])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn_with_image_attachment(
            "what's in this image?",
            "diagram.png",
            PNG_MIME,
            PNG_BYTES.to_vec(),
        )
        .await
        .expect("turn completes");

    let err = harness
        .assert_model_saw_image_attachment(PNG_MIME, PNG_BYTES)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("no captured image data: URL matching"),
        "expected error about missing image data URL, got: {err}"
    );
}

/// Negative control: a plain harness with no `InboundAttachmentLander` wired
/// must fail fast via the no-lander guard before any turn is submitted —
/// every other test in this file wires a lander.
#[tokio::test]
async fn submit_with_image_attachment_fails_fast_without_a_lander() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("harness builds");

    let err = harness
        .submit_turn_with_image_attachment(
            "what's in this image?",
            "diagram.png",
            PNG_MIME,
            PNG_BYTES.to_vec(),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no attachment lander wired"),
        "expected the no-lander fail-fast error, got: {err}"
    );
}

/// W4-ATTACH-VARIANTS: a `text/plain` attachment (`AttachmentKind::Document`)
/// is landed and text-extracted via `land_inbound_attachments`, and its
/// content is verified in the captured model request (not just tool output)
/// — the textual (non-multimodal) attachment path, uncovered by the
/// image-only tests above.
#[tokio::test]
async fn doc_attachment_reaches_the_model_with_extracted_text() {
    const MARKER: &str = "ZAFFRE-DOCUMENT-MARKER-771";
    let group = RebornIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let harness = group
        .thread("conv-attach-doc")
        .script([RebornScriptedReply::text("read it")])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn_with_attachments(
            "summarize this note",
            vec![(
                "note.txt",
                "text/plain",
                format!("Reminder: the launch codeword is {MARKER}.").into_bytes(),
            )],
        )
        .await
        .expect("turn completes");

    harness
        .assert_model_request_contains(MARKER)
        .await
        .expect("extracted document text reached the model");
    // Serialized capture escapes `"` to `\"`; the needle must match the
    // escaped form.
    harness
        .assert_model_request_contains("type=\\\"document\\\"")
        .await
        .expect("the attachment block tags the attachment as a document, not an image");

    // Non-vacuity guard: an unwritten marker must be ABSENT, proving the
    // assertion discriminates rather than passing unconditionally.
    if harness
        .assert_model_request_contains("UNWRITTEN-MARKER-999")
        .await
        .is_ok()
    {
        panic!("negative guard failed: model request must not contain an unwritten marker");
    }
}

/// W4-ATTACH-VARIANTS: two attachments landed in one turn both reach the
/// model in one captured request — proves N-attachment support through
/// `submit_inbound_with_attachments`, not just the single-attachment path
/// exercised above.
#[tokio::test]
async fn multiple_attachments_in_one_turn_all_reach_the_model() {
    const MARKER_ONE: &str = "TOPAZ-MARKER-ALPHA";
    const MARKER_TWO: &str = "TOPAZ-MARKER-BETA";
    let group = RebornIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let harness = group
        .thread("conv-attach-multi")
        .script([RebornScriptedReply::text("read both")])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn_with_attachments(
            "summarize both notes",
            vec![
                (
                    "one.txt",
                    "text/plain",
                    format!("First note: {MARKER_ONE}.").into_bytes(),
                ),
                (
                    "two.txt",
                    "text/plain",
                    format!("Second note: {MARKER_TWO}.").into_bytes(),
                ),
            ],
        )
        .await
        .expect("turn completes");

    harness
        .assert_model_request_contains(MARKER_ONE)
        .await
        .expect("first attachment's extracted text reached the model");
    harness
        .assert_model_request_contains(MARKER_TWO)
        .await
        .expect("second attachment's extracted text reached the model");
    // Both ordinal markers prove two DISTINCT attachment blocks were
    // rendered, not one block whose body contains both strings.
    harness
        .assert_model_request_contains("index=\\\"1\\\"")
        .await
        .expect("first attachment rendered at index 1");
    harness
        .assert_model_request_contains("index=\\\"2\\\"")
        .await
        .expect("second attachment rendered at index 2");
}

/// W5-WEBUI-API-1: attachment bytes served via the real `webui_v2_router`
/// GET-attachment route, over `RebornServices` wired with the production
/// `InboundAttachmentReader` (Enabler C) — not the model-injection
/// `LoopAttachmentReadPort` path above.
#[tokio::test]
async fn landed_attachment_reaches_webui_get_attachment_after_refresh() {
    let group = RebornIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let h = group
        .thread("conv-attach-webui-get")
        .with_model_override(VISION_MODEL)
        .script([RebornScriptedReply::text("I see a diagram")])
        .build()
        .await
        .expect("thread builds");

    h.submit_turn_with_image_attachment(
        "what's in this image?",
        "diagram.png",
        PNG_MIME,
        PNG_BYTES.to_vec(),
    )
    .await
    .expect("turn completes");

    let history = h
        .thread_harness
        .history(h.binding.thread_id.clone())
        .await
        .expect("thread history readable");
    let message = history
        .iter()
        .find(|message| !message.attachments.is_empty())
        .expect("a persisted message carries the landed attachment ref");
    let attachment = message
        .attachments
        .first()
        .expect("message has at least one attachment ref");

    let capability_harness = group
        .capability_harness()
        .expect("attachment_tools group uses a host-runtime capability backend");
    let reader = capability_harness
        .inbound_attachment_reader_for_test()
        .expect("local-dev inbound attachment reader wired");

    let services = RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
        .with_inbound_attachment_reader(reader);
    let caller = webui_caller_for(&h.binding);
    let router = mount_webui_v2_router(Arc::new(services), caller);

    let (status, headers, bytes) = get_raw(
        router,
        &format!(
            "/api/webchat/v2/threads/{}/messages/{}/attachments/{}",
            h.binding.thread_id.as_str(),
            message.message_id.as_uuid(),
            attachment.id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "attachment GET status");
    assert_eq!(
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some(PNG_MIME),
        "Content-Type must reflect the stored ref's mime type"
    );
    assert_eq!(
        bytes, PNG_BYTES,
        "served bytes must byte-match the landed image"
    );
}
