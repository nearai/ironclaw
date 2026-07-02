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
