//! Issue #6898: a text write must never corrupt a binary document.
//!
//! The user-reported symptom was a whole journey, not a single tool call:
//! upload a .docx, ask for an edit, ask for the document back — and get back a
//! file Word reports as corrupt. Two defects fed it. `read_file` returns
//! *extracted text* for a docx but records its read-proof fingerprint over the
//! *raw bytes*, so read-before-edit was satisfied by a representation the model
//! never saw as bytes; and `write_file` took a `&str` and wrote
//! `content.as_bytes()` with no target-extension or binary-content guard.
//!
//! This drives the journey through the real path — inbound attachment landing,
//! real capability dispatch, and the production `InboundAttachmentReader` the
//! WebUI download route serves bytes through — and pins the post-fix contract:
//! the write is REFUSED with an actionable reason, and the document the user
//! gets back is byte-identical to the one they uploaded.
//!
//! Two things this test deliberately does NOT assert.
//!
//! 1. A successfully edited .docx coming back. There is no OOXML writer and no
//!    binary/base64 write channel in the workspace (#6898 "Details"), so
//!    refusing is the correct terminal behavior today. A real document
//!    round-trip is that issue's deferred item 3; when it lands, extend here.
//! 2. The overwrite-the-uploaded-file path. A landed attachment's storage key
//!    is minted at landing time (UTC-date + message-id partitioned), so a
//!    static script cannot name it, and one conversation admits exactly one
//!    harness. That path is pinned at the crate tier instead, by
//!    `builtin_write_file_rejects_docx_after_extracted_read_without_changing_bytes`
//!    and `builtin_write_file_rejects_extracted_read_representation_at_unlisted_extension`
//!    in `crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const CONTRACT_DOCX: &[u8] = include_bytes!("../fixtures/contract.docx");
const DOCX_MIME: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
/// The typo the user asks to have fixed, verbatim from the fixture's
/// `word/document.xml`. Its presence in the captured model request proves the
/// upload was extracted to text rather than dropped or passed through as bytes.
const TYPO: &str = "reveiw";
/// Where the model tries to put the "corrected" document. A path it chooses
/// itself, so the script can name it — and precisely the move that used to
/// hand the user a .docx containing raw UTF-8 that Word calls corrupt.
const CORRECTED_DOCX: &str = "/workspace/contract-corrected.docx";

#[tokio::test]
async fn uploaded_docx_edit_request_is_refused_and_the_document_comes_back_byte_identical() {
    let group = RebornIntegrationGroup::document_edit_tools()
        .await
        .expect("document-edit group builds");
    let h = group
        .thread("conv-docx-edit")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({
                    "path": CORRECTED_DOCX,
                    "content": "Clause 4: the review period is thirty days.",
                }),
            ),
            RebornScriptedReply::text(
                "I can't hand back a .docx — writing the correction as text would \
                 produce a corrupt document. Clause 4 should read: the review \
                 period is thirty days.",
            ),
        ])
        .build()
        .await
        .expect("thread builds");

    h.submit_turn_with_attachments(
        "fix the typo in clause 4 and send me the corrected .docx back",
        vec![("contract.docx", DOCX_MIME, CONTRACT_DOCX.to_vec())],
    )
    .await
    .expect("turn completes");

    h.assert_model_request_contains(TYPO)
        .await
        .expect("the uploaded docx reached the model as extracted text");
    h.assert_tool_invoked("builtin.write_file")
        .await
        .expect("the write-back was really attempted through capability dispatch");
    h.assert_tool_error_summary_contains("binary documents cannot be edited with text tools")
        .await
        .expect("the refusal carries an actionable reason, not an opaque failure");
    h.assert_reply_contains("corrupt")
        .await
        .expect("the user is told why no new .docx is coming back");

    // The payload: what the user downloads is what they uploaded. Read back
    // through the SAME production reader the WebUI attachment route uses, so
    // this proves the bytes on the user's download path — not merely that one
    // tool call returned an error.
    let history = h
        .thread_harness
        .history(h.binding.thread_id.clone())
        .await
        .expect("thread history readable");
    let storage_key = history
        .iter()
        .find_map(|message| message.attachments.first())
        .expect("the landed docx is persisted as an attachment ref")
        .storage_key
        .clone()
        .expect("a landed attachment carries the storage key its bytes went to");
    let reader = group
        .capability_harness()
        .expect("document_edit_tools uses a host-runtime capability backend")
        .inbound_attachment_reader_for_test()
        .expect("local-dev inbound attachment reader wired");
    let thread_scope = reborn_support::builder::thread_scope_from_binding(&h.binding)
        .expect("thread scope resolves from the binding");
    let served = reader
        .read(&thread_scope, &storage_key)
        .await
        .expect("the uploaded docx is still readable");
    assert_eq!(
        served, CONTRACT_DOCX,
        "the document the user gets back must be byte-identical to the one they uploaded"
    );
}
