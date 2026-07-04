//! C-ATTACH: `attachment_read_port` int-tier coverage (rev-3 Tier-2, A1 audit).
//!
//! Production wires `attachment_read_port` from the local-dev workspace
//! filesystem (`ProjectScopedAttachmentReader` over `local_runtime.workspace_filesystem`,
//! `crates/ironclaw_reborn_composition/src/runtime.rs:3328-3334`) so the loop
//! model port can read landed attachment bytes back and the gateway
//! (`crates/ironclaw_reborn/src/model_gateway.rs::convert_messages`) builds a
//! `ContentPart::ImageUrl` multimodal part for a vision-capable model. That seam
//! was never exercised by any Reborn test: `DefaultPlannedRuntimeParts.attachment_read_port`
//! was `None` everywhere, so every image attachment silently degraded to the
//! textual `<attachments>` pointer regardless of the model's vision capability.
//!
//! This test wires the read port + the real `InboundAttachmentLander` (same
//! production `ProjectScopedAttachmentLander`) via `RebornIntegrationGroup::attachment_tools()`,
//! lands an image through the real `submit_inbound_with_attachments` production
//! entry point, routes the thread through a vision-pattern model id
//! (`.with_model_override`), and asserts the model-visible request actually
//! carried the image as a `data:` URL content part.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

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

/// Negative control: without `.with_model_override(<vision id>)` the thread
/// keeps the harness's default scripted model id, which is not a vision-pattern
/// match. `convert_messages` then drops the image part (text-only fallback), so
/// no multimodal content part should reach the model even though the
/// attachment landed and the read port is wired. Proves the model-override knob
/// (not just the read port) is load-bearing for this assertion.
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

/// Negative control: a plain harness (not built via
/// `RebornIntegrationGroup::attachment_tools()`) has no `InboundAttachmentLander`
/// wired, so `submit_turn_with_image_attachment`'s explicit no-lander guard must
/// fire before any turn is submitted. Proves that fail-fast path stays load-bearing
/// — without this, a regression that removed or weakened the guard would not be
/// caught, since every other test in this file only exercises harnesses with a
/// lander wired.
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

/// W4-ATTACH-VARIANTS: a `text/plain` attachment (`AttachmentKind::Document`,
/// per `ironclaw_common::AttachmentKind::from_mime_type`) is landed through the
/// real `InboundAttachmentLander`, its text extracted by
/// `land_inbound_attachments` -> `extract_document_text`, and rendered into the
/// `<attachments>` block `ironclaw_threads::attachment_context::augment_model_content`
/// appends to the user message — the textual (non-multimodal) attachment path
/// no `RebornIntegrationHarness` test exercised before this file only covered
/// images. Reads the captured model request (not just tool output), proving
/// the extracted text reached the model.
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
    // `assert_model_request_contains` matches against the JSON-serialized
    // captured request, so a literal `"` inside the rendered `<attachments>`
    // XML is escaped to `\"` in that serialization — the needle must match the
    // escaped form.
    harness
        .assert_model_request_contains("type=\\\"document\\\"")
        .await
        .expect("the attachment block tags the attachment as a document, not an image");

    // Non-vacuity guard: a marker that was never written must be ABSENT, so
    // `assert_model_request_contains` is proven to discriminate rather than
    // pass unconditionally (e.g. on a captured-request stringification bug
    // that always matches).
    if harness
        .assert_model_request_contains("UNWRITTEN-MARKER-999")
        .await
        .is_ok()
    {
        panic!("negative guard failed: model request must not contain an unwritten marker");
    }
}

/// W4-ATTACH-VARIANTS: two attachments landed on a SINGLE turn both reach the
/// model in one captured request — proves `submit_turn_with_attachments`
/// carries N attachments through the real
/// `DefaultProductWorkflow::submit_inbound_with_attachments` entry point (every
/// prior attachment test in this file submitted exactly one), and that
/// `land_inbound_attachments`/`augment_model_content` render every landed
/// attachment into the `<attachments>` block, not just the first.
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
    // Both attachments are indexed distinctly in the rendered block, so both
    // ordinal markers must be present — proves two DISTINCT attachment blocks
    // were rendered, not one attachment whose body happens to contain both
    // marker strings. (See the escaping note above: needles match the
    // JSON-serialized, quote-escaped form.)
    harness
        .assert_model_request_contains("index=\\\"1\\\"")
        .await
        .expect("first attachment rendered at index 1");
    harness
        .assert_model_request_contains("index=\\\"2\\\"")
        .await
        .expect("second attachment rendered at index 2");
}
