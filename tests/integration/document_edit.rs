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
//! Two things the first refusal journey deliberately does NOT assert.
//!
//! 1. A successfully edited .docx coming back. The later journeys cover the
//!    structured OOXML writer; this first journey isolates the permanent rule
//!    that raw text tools still refuse binary documents.
//! 2. The overwrite-the-uploaded-file path. A landed attachment's storage key
//!    is minted at landing time (UTC-date + message-id partitioned), so a
//!    static script cannot name it, and one conversation admits exactly one
//!    harness. That path is pinned at the crate tier instead, by
//!    `builtin_write_file_rejects_docx_after_extracted_read_without_changing_bytes`
//!    and `builtin_write_file_rejects_extracted_read_representation_at_unlisted_extension`
//!    in `crates/kernel/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`.

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

// --- #6898 item 3: the document round-trip journeys ------------------------
//
// Shape note: each journey uploads on one conversation and edits on another,
// both in the SAME group. The upload has to happen first because a landed
// attachment's storage key is minted at landing time (UTC-date + message-id
// partitioned), so only after that turn can a script name the path; and one
// conversation admits exactly one harness, whose script is fixed at build.
// Threads in a group share the workspace filesystem — same tenant/user/project
// scope — so the edit thread reads exactly the bytes the upload landed.

const REDLINED_DOCX: &[u8] = include_bytes!("../fixtures/redlined-contract.docx");
const EXPENSES_XLSX: &[u8] = include_bytes!("../fixtures/expenses.xlsx");
const QUARTERLY_PPTX: &[u8] = include_bytes!("../fixtures/quarterly.pptx");
const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX_MIME: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

/// Upload `bytes` and return its storage key, which is the addressable workspace path.
async fn upload_document(
    group: &RebornIntegrationGroup,
    conversation: &str,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> String {
    let h = group
        .thread(conversation)
        .script([RebornScriptedReply::text("received")])
        .build()
        .await
        .expect("upload thread builds");
    h.submit_turn_with_attachments("here is the file", vec![(filename, mime, bytes.to_vec())])
        .await
        .expect("upload turn completes");
    let history = h
        .thread_harness
        .history(h.binding.thread_id.clone())
        .await
        .expect("thread history readable");
    history
        .iter()
        .find_map(|message| message.attachments.first())
        .expect("the landed document is persisted as an attachment ref")
        .storage_key
        .clone()
        .expect("a landed attachment carries its storage key")
}

/// Journey 1 — redlines. A contract arrives with tracked changes; the model
/// reads it (seeing the redlines as redlines, not as flattened text), resolves
/// them, and saves a clean copy to a new document.
#[tokio::test]
async fn redlined_docx_is_read_with_revisions_and_saved_clean_to_a_new_document() {
    let group = RebornIntegrationGroup::document_edit_tools()
        .await
        .expect("document-edit group builds");
    let source = upload_document(
        &group,
        "conv-docx-upload",
        "contract.docx",
        DOCX_MIME,
        REDLINED_DOCX,
    )
    .await;
    let output = "/workspace/contract-final.docx";

    let h = group
        .thread("conv-docx-redline")
        .script([
            RebornScriptedReply::tool_call("builtin.read_file", json!({"path": source})),
            RebornScriptedReply::tool_call(
                "builtin.document_edit",
                json!({
                    "path": source,
                    "output_path": output,
                    "edits": [{"op": "resolve_all_revisions", "disposition": "accept"}],
                }),
            ),
            RebornScriptedReply::tool_call("builtin.read_file", json!({"path": output})),
            RebornScriptedReply::text(
                "Accepted both redlines. The clean contract is at contract-final.docx.",
            ),
        ])
        .build()
        .await
        .expect("edit thread builds");

    h.submit_turn("accept the tracked changes and save a clean copy")
        .await
        .expect("turn completes");

    h.assert_tool_invoked("builtin.document_edit")
        .await
        .expect("the edit ran through real capability dispatch");
    // The read must surface the redlines structurally — this is what flat
    // extraction could not do, and what the accept operation addresses.
    h.assert_tool_result_contains("deleted")
        .await
        .expect("the redlined read exposes tracked changes");
    h.assert_tool_result_contains("Reviewer")
        .await
        .expect("revision authorship survives the read");
    // The re-read of the OUTPUT proves the resolution actually applied.
    h.assert_tool_result_contains("thirty")
        .await
        .expect("the accepted insertion is in the clean copy");
    h.assert_tool_result_contains("New York")
        .await
        .expect("the second accepted insertion is in the clean copy too");
    h.assert_no_tool_error(
        reborn_support::assertions::ToolErrorClass::Failed,
        "document_edit",
    )
    .await
    .expect("no document edit failed");
}

/// Journey 2 — spreadsheets. The model finds the column by its heading and puts
/// a total formula in the cell beneath it, saving to a new workbook.
#[tokio::test]
async fn xlsx_formula_is_set_under_a_named_column_and_saved_to_a_new_workbook() {
    let group = RebornIntegrationGroup::document_edit_tools()
        .await
        .expect("document-edit group builds");
    let source = upload_document(
        &group,
        "conv-xlsx-upload",
        "expenses.xlsx",
        XLSX_MIME,
        EXPENSES_XLSX,
    )
    .await;
    let output = "/workspace/expenses-totalled.xlsx";

    let h = group
        .thread("conv-xlsx-formula")
        .script([
            RebornScriptedReply::tool_call("builtin.read_file", json!({"path": source})),
            RebornScriptedReply::tool_call(
                "builtin.document_edit",
                json!({
                    "path": source,
                    "output_path": output,
                    // C is the Amount column; row 5 is the first empty row.
                    "edits": [{
                        "op": "set_cell_formula",
                        "sheet": "Expenses",
                        "cell": "C5",
                        "formula": "SUM(C2:C4)",
                    }],
                }),
            ),
            RebornScriptedReply::tool_call("builtin.read_file", json!({"path": output})),
            RebornScriptedReply::text("Added the total under Amount in expenses-totalled.xlsx."),
        ])
        .build()
        .await
        .expect("edit thread builds");

    h.submit_turn("total the Amount column and save it as a new workbook")
        .await
        .expect("turn completes");

    // Shared strings resolved: without that the headers read as "0"/"1"/"2" and
    // the model could not have located the Amount column at all.
    h.assert_tool_result_contains("Amount")
        .await
        .expect("column headings read as text");
    h.assert_tool_result_contains("SUM(C2:C4)")
        .await
        .expect("the formula is present in the saved workbook");
    h.assert_tool_invoked("builtin.document_edit")
        .await
        .expect("the edit ran through real capability dispatch");
}

/// Journey 3 — decks. The model appends a slide that inherits the source
/// slide's layout, so the new slide is styled like the rest of the deck.
#[tokio::test]
async fn pptx_slide_is_cloned_with_the_source_style_and_saved_to_a_new_deck() {
    let group = RebornIntegrationGroup::document_edit_tools()
        .await
        .expect("document-edit group builds");
    let source = upload_document(
        &group,
        "conv-pptx-upload",
        "quarterly.pptx",
        PPTX_MIME,
        QUARTERLY_PPTX,
    )
    .await;
    let output = "/workspace/quarterly-q2.pptx";

    let h = group
        .thread("conv-pptx-slide")
        .script([
            RebornScriptedReply::tool_call("builtin.read_file", json!({"path": source})),
            RebornScriptedReply::tool_call(
                "builtin.document_edit",
                json!({
                    "path": source,
                    "output_path": output,
                    "edits": [{
                        "op": "clone_slide",
                        "source": 1,
                        "text": ["Q2 Results", "Revenue up 18%"],
                    }],
                }),
            ),
            RebornScriptedReply::tool_call("builtin.read_file", json!({"path": output})),
            RebornScriptedReply::text("Added a Q2 slide in the same style as Q1."),
        ])
        .build()
        .await
        .expect("edit thread builds");

    h.submit_turn("add a Q2 slide in the same style")
        .await
        .expect("turn completes");

    h.assert_tool_result_contains("Q1 Results")
        .await
        .expect("the original slide survives");
    h.assert_tool_result_contains("Q2 Results")
        .await
        .expect("the cloned slide carries the new text");
    h.assert_tool_result_contains("Revenue up 18%")
        .await
        .expect("the cloned slide body text is in the saved deck");
    h.assert_tool_invoked("builtin.document_edit")
        .await
        .expect("the clone ran through real capability dispatch");
}

/// Journey 4 — PDF. PDFs are never edited in place: the model authors HTML,
/// which stays the document of record, and renders it.
#[tokio::test]
async fn pdf_is_produced_by_authoring_html_and_rendering_it() {
    let group = RebornIntegrationGroup::document_edit_tools()
        .await
        .expect("document-edit group builds");
    let html = "<h1>Q1 Expense Report</h1><p>Total spend was <strong>$2,450</strong>.</p>\
                <ul><li>Hosting: $1,200</li><li>Travel: $800</li><li>Licenses: $450</li></ul>";

    let h = group
        .thread("conv-pdf-render")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/report.html", "content": html}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.html_to_pdf",
                json!({
                    "path": "/workspace/report.pdf",
                    "html": html,
                    "title": "Q1 Expense Report",
                }),
            ),
            RebornScriptedReply::text("Wrote report.html and rendered it to report.pdf."),
        ])
        .build()
        .await
        .expect("thread builds");

    h.submit_turn("write up the Q1 expenses and give me a PDF")
        .await
        .expect("turn completes");

    h.assert_tool_invoked("builtin.html_to_pdf")
        .await
        .expect("the render ran through real capability dispatch");
    h.assert_tool_result_contains("report.pdf")
        .await
        .expect("the rendered path is reported back to the model");
    h.assert_no_tool_error(
        reborn_support::assertions::ToolErrorClass::Failed,
        "html_to_pdf",
    )
    .await
    .expect("rendering succeeded");
    let pdf_path = group
        .capability_harness()
        .expect("document_edit_tools uses a host-runtime capability backend")
        .workspace_file_path("report.pdf");
    let pdf = std::fs::read(&pdf_path).expect("persisted PDF is readable");
    assert!(pdf.starts_with(b"%PDF-"), "persisted output must be a PDF");
    assert!(
        pdf.len() > 100,
        "persisted PDF must contain rendered content"
    );
}
