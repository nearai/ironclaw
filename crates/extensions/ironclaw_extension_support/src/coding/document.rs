//! Document capabilities: structured edits for OOXML, and HTML-to-PDF.
//!
//! These are the write half of issue #6898 item 3. The read half is folded
//! into `read_file` (see [`super::file::read_file`]) rather than exposed as a
//! separate tool: a model reaches for `read_file` on any path it is given, so
//! a `document_read` tool it has to know to prefer would mostly go unused
//! while `read_file` kept returning tag-stripped text with deleted redline
//! content silently included.
//!
//! The write half cannot be folded the same way, and that asymmetry is
//! deliberate. `write_file`'s contract is `(path, content: &str)` — "replace
//! this file with this text" — which cannot express "accept the revision in
//! p3" or "set C5's formula". Overloading it would mean either regenerating
//! the document from text (the corruption #6898 banned) or making `content`
//! sometimes-JSON, which is the stringly-typed shape `.claude/rules/types.md`
//! exists to prevent. `apply_patch` fails for the same reason plus one more:
//! its anchors match *extracted* text, and mapping a match back to specific
//! runs is ambiguous when a string spans a revision boundary.
//!
//! So: reads unify, writes stay typed.

use ironclaw_documents::{
    DocumentError, DocumentFormat, PdfOptions, docx, html_to_pdf, pptx, xlsx,
};
use ironclaw_host_api::dispatch::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use super::{
    CodingCapabilityError, CodingCapabilityOutput, CodingCapabilityRequest,
    file::{read_before_edit_error, stale_read_error},
    input_error,
    inputs::required_str,
    operation_error_with_summary,
    paths::{
        filesystem_error_with_summary, is_workspace_path, operation_allowed, resolve_required_path,
        safe_summary_path,
    },
    state::{
        ReadRepresentation, SharedCodingEditLocks, SharedCodingReadStates, content_fingerprint,
        read_scope_key,
    },
};
use ironclaw_filesystem::{FileType, FilesystemOperation};

/// Ceiling on a document read/edit, matching the OOXML package budget in
/// `ironclaw_documents`.
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

fn document_error(
    operation: &str,
    scoped_path: &str,
    error: DocumentError,
) -> CodingCapabilityError {
    // `DocumentError`'s Display is already caller-safe: it names addresses and
    // structural problems, never file content.
    operation_error_with_summary(format!(
        "{operation} failed for {}: {error}",
        safe_summary_path(scoped_path)
    ))
}

/// Render a document's structure as the JSON `read_file` returns for OOXML.
///
/// Deliberately the same shape the edit capability addresses: paragraph ids,
/// cell references, and slide indexes a caller reads here are exactly what it
/// passes back to `document_edit`.
pub(super) fn structured_document_view(
    format: DocumentFormat,
    bytes: &[u8],
) -> Result<Value, DocumentError> {
    Ok(match format {
        DocumentFormat::Docx => json!({
            "format": "docx",
            "paragraphs": docx::read_docx(bytes)?,
        }),
        DocumentFormat::Xlsx => json!({
            "format": "xlsx",
            "sheets": xlsx::read_xlsx(bytes)?,
        }),
        DocumentFormat::Pptx => json!({
            "format": "pptx",
            "slides": pptx::read_pptx(bytes)?,
        }),
    })
}

/// Apply typed structural edits to a document, writing the result to a NEW path.
///
/// Never edits in place: the source stays byte-identical, so a failed or
/// unwanted edit can never cost the user their original. That is also what
/// makes the operation safe to retry.
pub(super) async fn document_edit(
    request: &CodingCapabilityRequest<'_>,
    edit_locks: &SharedCodingEditLocks,
    read_states: &SharedCodingReadStates,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    let source_input = required_str(request.input, "path")?;
    if is_workspace_path(source_input) {
        return Err(input_error());
    }
    let source = resolve_required_path(request, "path", FilesystemOperation::ReadFile)?;
    let target = resolve_required_path(request, "output_path", FilesystemOperation::WriteFile)?;
    if source.virtual_path == target.virtual_path {
        return Err(operation_error_with_summary(
            "document_edit writes to a new file: output_path must differ from path".to_string(),
        ));
    }
    let Some(format) = DocumentFormat::from_path(source.scoped_path.as_str()) else {
        return Err(operation_error_with_summary(format!(
            "document_edit supports .docx, .xlsx and .pptx; {} is not one of them",
            safe_summary_path(source.scoped_path.as_str())
        )));
    };
    if DocumentFormat::from_path(target.scoped_path.as_str()) != Some(format) {
        return Err(operation_error_with_summary(
            "output_path must have the same document extension as path".to_string(),
        ));
    }
    let edits = request
        .input
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(input_error)?;
    if edits.is_empty() {
        return Err(operation_error_with_summary(
            "document_edit needs at least one edit".to_string(),
        ));
    }

    let scope = read_scope_key(request);
    let _guard = edit_locks
        .lock_edit(scope.edit_lock_key(), source.virtual_path.as_str())
        .await;

    // Read-before-edit, on the STRUCTURED representation. The fingerprint is
    // recorded over raw bytes either way, so this keeps the same mid-air
    // collision guarantee `write_file` has while requiring that what the model
    // saw was the addressable view whose ids these edits name.
    let Some(recorded) = read_states.recorded(&scope, source.virtual_path.as_str()) else {
        return Err(read_before_edit_error(
            "document_edit",
            source.scoped_path.as_str(),
        ));
    };
    if recorded.representation != ReadRepresentation::Structured {
        return Err(operation_error_with_summary(format!(
            "document_edit failed for {}: read it with read_file first so its paragraph, cell \
                 and slide addresses are known",
            safe_summary_path(source.scoped_path.as_str())
        )));
    }

    let bytes = request
        .filesystem
        .read_file(&source.virtual_path)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("document_edit", source.scoped_path.as_str(), error)
        })?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    if content_fingerprint(&bytes) != recorded.fingerprint {
        return Err(stale_read_error(
            "document_edit",
            source.scoped_path.as_str(),
        ));
    }

    let edited = apply_typed_edits(format, &bytes, edits)
        .map_err(|error| document_error("document_edit", source.scoped_path.as_str(), error))?;

    request
        .filesystem
        .write_file(&target.virtual_path, &edited)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("document_edit", target.scoped_path.as_str(), error)
        })?;
    // Record the NEW file's structured fingerprint so a follow-up edit can
    // chain off it without a redundant read, exactly as write_file does.
    read_states.record(
        &scope,
        target.virtual_path.as_str(),
        content_fingerprint(&edited),
        ReadRepresentation::Structured,
    );

    Ok(CodingCapabilityOutput::new(json!({
        "path": target.scoped_path.as_str(),
        "source_path": source.scoped_path.as_str(),
        "bytes_written": edited.len(),
        "edits_applied": edits.len(),
        "success": true,
    })))
}

fn apply_typed_edits(
    format: DocumentFormat,
    bytes: &[u8],
    edits: &[Value],
) -> Result<Vec<u8>, DocumentError> {
    let raw = Value::Array(edits.to_vec());
    match format {
        DocumentFormat::Docx => {
            let typed: Vec<docx::DocxEdit> = serde_json::from_value(raw)
                .map_err(|error| DocumentError::UnsupportedFormat(error.to_string()))?;
            docx::edit_docx(bytes, &typed)
        }
        DocumentFormat::Xlsx => {
            let typed: Vec<xlsx::XlsxEdit> = serde_json::from_value(raw)
                .map_err(|error| DocumentError::UnsupportedFormat(error.to_string()))?;
            xlsx::edit_xlsx(bytes, &typed)
        }
        DocumentFormat::Pptx => {
            let typed: Vec<pptx::PptxEdit> = serde_json::from_value(raw)
                .map_err(|error| DocumentError::UnsupportedFormat(error.to_string()))?;
            pptx::edit_pptx(bytes, &typed)
        }
    }
}

/// Render HTML to a new PDF file.
///
/// The counterpart to the OOXML editors for a format that cannot be edited in
/// place: the caller keeps the HTML as the document of record, revises *that*
/// with the ordinary text tools, and re-renders.
pub(super) async fn html_to_pdf_capability(
    request: &CodingCapabilityRequest<'_>,
    edit_locks: &SharedCodingEditLocks,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    let path_input = required_str(request.input, "path")?;
    if is_workspace_path(path_input) {
        return Err(input_error());
    }
    let html = required_str(request.input, "html")?;
    let target = resolve_required_path(request, "path", FilesystemOperation::WriteFile)?;
    if !target
        .scoped_path
        .as_str()
        .to_ascii_lowercase()
        .ends_with(".pdf")
    {
        return Err(operation_error_with_summary(
            "html_to_pdf writes a .pdf file".to_string(),
        ));
    }
    if !operation_allowed(&target.grant.permissions, FilesystemOperation::WriteFile) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }

    let scope = read_scope_key(request);
    let _guard = edit_locks
        .lock_edit(scope.edit_lock_key(), target.virtual_path.as_str())
        .await;

    // Fail closed on an existing target rather than overwrite. A rendered PDF
    // is a derived artifact; silently replacing a PDF the user uploaded is the
    // same class of loss the binary-write guard exists to prevent, and the
    // caller can always name a fresh path.
    if let Ok(stat) = request.filesystem.stat(&target.virtual_path).await
        && stat.file_type == FileType::File
    {
        return Err(operation_error_with_summary(format!(
            "html_to_pdf will not overwrite {}: choose a path that does not exist yet",
            safe_summary_path(target.scoped_path.as_str())
        )));
    }

    let title = request
        .input
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Document");
    let pdf = html_to_pdf(
        html,
        &PdfOptions {
            title: title.to_string(),
            ..PdfOptions::default()
        },
    )
    .map_err(|error| document_error("html_to_pdf", target.scoped_path.as_str(), error))?;

    request
        .filesystem
        .write_file(&target.virtual_path, &pdf)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("html_to_pdf", target.scoped_path.as_str(), error)
        })?;

    Ok(CodingCapabilityOutput::new(json!({
        "path": target.scoped_path.as_str(),
        "bytes_written": pdf.len(),
        "success": true,
    })))
}
