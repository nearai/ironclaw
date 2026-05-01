//! Pure DOCX renderer for [`crate::legal::ChatExport`].
//!
//! This module is intentionally small and synchronous: it takes a fully
//! materialized [`ChatExport`] and returns the bytes of an OOXML
//! (`.docx`) file. The HTTP layer reads the chat from the database and
//! then hands the snapshot here, so this code stays I/O-free and easy
//! to fuzz.
//!
//! # Layout
//!
//! 1. A title paragraph: the chat title, or `Chat <id>` when the title
//!    is unset. Visually emphasised (bold + size 36 = 18pt).
//! 2. A subtitle paragraph with the chat-creation timestamp in ISO-8601
//!    UTC.
//! 3. For each message in chronological order:
//!    - A heading paragraph: `[<ISO timestamp UTC>] <Role label>`,
//!      bold, sized 28 (14pt).
//!    - One paragraph per `\n\n`-separated block of `content`. Each
//!      paragraph is run-split on `\n` so single newlines become DOCX
//!      line breaks rather than paragraph splits — most chat content
//!      assumes the markdown convention that double-newline = new
//!      paragraph. We deliberately do not interpret markdown beyond
//!      this; v1 keeps the rendering unambiguous and reversible.
//!    - When `document_refs` is non-empty, an italic "Documents
//!      referenced:" line followed by one italic bullet-prefixed line
//!      per filename.
//!
//! # Hardening notes
//!
//! - `docx-rs` escapes XML on output (it builds a model and serialises
//!   it), so embedded `<`, `>`, `&`, etc. in user content cannot break
//!   out of the document. The renderer itself never concatenates
//!   user-provided strings into XML — it only feeds them through the
//!   builder API.
//! - Control characters that are illegal in OOXML body text (anything
//!   below `0x20` except tab/newline/carriage return) are stripped via
//!   [`sanitize_body_text`] so `docx-rs` does not emit a payload that
//!   Word and LibreOffice will refuse to open.
//! - The renderer caps total output paragraphs and per-message
//!   paragraph count to defend against a pathological chat that would
//!   otherwise materialise hundreds of megabytes of XML in memory. The
//!   limits ([`MAX_RENDERED_PARAGRAPHS`], [`MAX_PARAGRAPHS_PER_MESSAGE`])
//!   are deliberately generous; real chats will not hit them.

use std::io::Cursor;

use docx_rs::{Docx, Paragraph, Run};

use crate::legal::{ChatExport, ChatMessage, LegalError};

/// Hard cap on the total number of paragraphs the renderer will emit.
///
/// Matches roughly a 1 GB OOXML zip in the worst case (each paragraph
/// is in the order of a few hundred bytes of XML). A pathological chat
/// that would exceed this is truncated and a final "(... export
/// truncated)" paragraph is appended so the recipient knows the file is
/// incomplete rather than mysteriously short. v2 can stream paragraphs
/// directly to the response writer if a real use-case ever hits it.
const MAX_RENDERED_PARAGRAPHS: usize = 250_000;

/// Per-message paragraph cap. A single message's content is split on
/// `\n\n`; if the user pasted a megabyte of newlines we cap so the
/// renderer doesn't blow up converting one row.
const MAX_PARAGRAPHS_PER_MESSAGE: usize = 5_000;

/// Cap on `document_refs` filenames rendered per message. Anything past
/// this is shown as a single "(... and N more)" line.
const MAX_DOC_REFS_PER_MESSAGE: usize = 100;

/// Cap on body-text bytes per paragraph after sanitisation. A single
/// pasted log file would otherwise turn into one gigantic `<w:t>` run
/// that Word will silently reject. Excess bytes are emitted as
/// additional paragraphs, preserving the message but in chunks the
/// reader can scroll.
const MAX_BYTES_PER_PARAGRAPH: usize = 64 * 1024;

/// Build a `.docx` byte vector from a chat snapshot.
///
/// Returns [`LegalError::Render`] when `docx-rs` fails to produce or
/// pack a valid OOXML zip, and [`LegalError::ChatEmpty`] when the
/// snapshot has no messages (the HTTP layer should already reject this,
/// but enforcing it here keeps the renderer safe to call directly).
pub fn render_chat_to_docx(chat: &ChatExport) -> Result<Vec<u8>, LegalError> {
    if chat.messages.is_empty() {
        return Err(LegalError::ChatEmpty(chat.id.clone()));
    }

    let mut docx = Docx::new();
    let mut paragraph_budget = MAX_RENDERED_PARAGRAPHS;

    let title_text = chat
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Chat {}", chat.id));
    docx = docx.add_paragraph(title_paragraph(&title_text));
    paragraph_budget = paragraph_budget.saturating_sub(1);

    docx = docx.add_paragraph(subtitle_paragraph(&format!(
        "Created {}",
        chat.created_at
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    )));
    paragraph_budget = paragraph_budget.saturating_sub(1);

    for msg in &chat.messages {
        if paragraph_budget == 0 {
            docx = docx.add_paragraph(truncation_marker());
            break;
        }
        let paras = render_message_paragraphs(msg, paragraph_budget);
        let consumed = paras.len();
        for p in paras {
            docx = docx.add_paragraph(p);
        }
        paragraph_budget = paragraph_budget.saturating_sub(consumed);
    }

    let xml = docx.build();
    let mut buf = Cursor::new(Vec::with_capacity(64 * 1024));
    xml.pack(&mut buf)
        .map_err(|e| LegalError::Render(format!("zip pack failed: {e}")))?;
    Ok(buf.into_inner())
}

/// Render the paragraphs that represent a single message. Caps total
/// paragraph emission at `budget` so the caller's overall cap is
/// honoured.
fn render_message_paragraphs(msg: &ChatMessage, budget: usize) -> Vec<Paragraph> {
    let mut out = Vec::new();
    if budget == 0 {
        return out;
    }

    // Heading: `[<ts>] <Role>`
    let heading_text = format!(
        "[{}] {}",
        msg.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
        msg.role.label()
    );
    out.push(heading_paragraph(&heading_text));
    if out.len() >= budget {
        return out;
    }

    // Body: split on \n\n, sanitise, chunk if huge.
    let sanitised = sanitize_body_text(&msg.content);
    // Strip trailing \r on every block so single-CRLF Windows pastes
    // don't leave dangling carriage returns inside a run's text. The
    // trim is per-block rather than whole-string because the split
    // boundary may have eaten a `\r\n\r\n` already.
    let blocks: Vec<&str> = sanitised
        .split("\n\n")
        .map(|line| line.trim_end_matches('\r'))
        .collect();

    let mut emitted = 0usize;
    for block in blocks {
        if emitted >= MAX_PARAGRAPHS_PER_MESSAGE {
            out.push(truncation_marker());
            return out;
        }
        if out.len() >= budget {
            return out;
        }
        // A single block may itself be longer than MAX_BYTES_PER_PARAGRAPH
        // (e.g. a pasted log without blank lines). Chunk it on byte
        // boundaries that align with `\n` where possible so the reader
        // sees natural breaks.
        for chunk in chunk_paragraph_bytes(block) {
            if emitted >= MAX_PARAGRAPHS_PER_MESSAGE || out.len() >= budget {
                if emitted >= MAX_PARAGRAPHS_PER_MESSAGE {
                    out.push(truncation_marker());
                }
                return out;
            }
            out.push(body_paragraph(chunk));
            emitted += 1;
        }
    }

    if !msg.document_refs.is_empty() && out.len() < budget {
        out.push(documents_callout_paragraph());
        let count = msg.document_refs.len();
        let visible = count.min(MAX_DOC_REFS_PER_MESSAGE);
        for filename in msg.document_refs.iter().take(visible) {
            if out.len() >= budget {
                return out;
            }
            out.push(document_ref_paragraph(filename));
        }
        if count > visible && out.len() < budget {
            out.push(document_ref_paragraph(&format!(
                "(\u{2026} and {} more)",
                count - visible
            )));
        }
    }

    out
}

/// Strip ASCII control characters that are illegal inside OOXML `<w:t>`
/// text (anything below 0x20 except `\t`, `\n`, `\r`). The Open XML
/// spec calls these "discouraged" and Word/LibreOffice silently refuse
/// to open files containing them.
///
/// Returns an owned String so the caller can split on `\n\n`. Allocates
/// only when the input contains something to strip; identity-clones the
/// borrow otherwise.
fn sanitize_body_text(input: &str) -> String {
    if input
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
    {
        return input.to_owned();
    }
    input
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

/// Chunk `block` into UTF-8-safe pieces no longer than
/// [`MAX_BYTES_PER_PARAGRAPH`]. Splits on `\n` boundaries when one is
/// available; otherwise falls back to a char-boundary split so we
/// never emit invalid UTF-8.
fn chunk_paragraph_bytes(block: &str) -> Vec<&str> {
    if block.len() <= MAX_BYTES_PER_PARAGRAPH {
        return vec![block];
    }
    let mut chunks = Vec::new();
    let mut remaining = block;
    while remaining.len() > MAX_BYTES_PER_PARAGRAPH {
        let head_limit = MAX_BYTES_PER_PARAGRAPH;
        // Prefer to break on the last newline within the budget.
        let split_at = match remaining[..head_limit].rfind('\n') {
            Some(i) if i + 1 > 0 => i + 1,
            _ => floor_char_boundary(remaining, head_limit),
        };
        let (head, tail) = remaining.split_at(split_at);
        chunks.push(head);
        remaining = tail;
    }
    if !remaining.is_empty() {
        chunks.push(remaining);
    }
    chunks
}

/// Round `idx` down to the nearest UTF-8 char boundary. Mirrors the
/// nightly-only `str::floor_char_boundary` so we work on stable Rust.
fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn title_paragraph(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text).bold().size(36))
}

fn subtitle_paragraph(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text).italic().size(20))
}

fn heading_paragraph(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text).bold().size(28))
}

fn body_paragraph(text: &str) -> Paragraph {
    // Single-newlines within a logical paragraph become explicit DOCX
    // line breaks ("soft returns"). docx-rs' Run::add_break creates a
    // <w:br/> element for that.
    let mut run = Run::new();
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            run = run.add_break(docx_rs::BreakType::TextWrapping);
        }
        first = false;
        run = run.add_text(line);
    }
    Paragraph::new().add_run(run)
}

fn documents_callout_paragraph() -> Paragraph {
    Paragraph::new().add_run(
        Run::new()
            .add_text("Documents referenced:")
            .italic()
            .bold(),
    )
}

fn document_ref_paragraph(filename: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(format!("\u{2022} {filename}")).italic())
}

fn truncation_marker() -> Paragraph {
    Paragraph::new().add_run(
        Run::new()
            .add_text("(\u{2026} export truncated)")
            .italic(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legal::{ChatMessage, ChatRole};
    use chrono::{TimeZone, Utc};
    use std::io::Read;

    fn ts(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid ts")
    }

    fn sample_chat() -> ChatExport {
        ChatExport {
            id: "01HZ-test-chat".to_string(),
            title: Some("NDA review thread".to_string()),
            created_at: ts(1_700_000_000),
            messages: vec![
                ChatMessage {
                    id: "m1".to_string(),
                    role: ChatRole::User,
                    content: "What does Section 3 of the NDA say?".to_string(),
                    document_refs: vec!["nda.pdf".to_string()],
                    created_at: ts(1_700_000_010),
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: ChatRole::Assistant,
                    content: "Section 3 covers confidentiality obligations.\n\nIt requires both parties to keep information secret for 5 years.".to_string(),
                    document_refs: vec!["nda.pdf".to_string(), "exhibit-a.pdf".to_string()],
                    created_at: ts(1_700_000_020),
                },
                ChatMessage {
                    id: "m3".to_string(),
                    role: ChatRole::User,
                    content: "Thanks.".to_string(),
                    document_refs: vec![],
                    created_at: ts(1_700_000_030),
                },
            ],
        }
    }

    /// Open the produced bytes as a zip archive and return the contents
    /// of `word/document.xml` as a UTF-8 string. Asserts the zip is
    /// well-formed in the process — a corrupt OOXML payload makes this
    /// helper fail loudly with a clear message.
    fn extract_document_xml(bytes: &[u8]) -> String {
        let reader = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader).expect("valid zip");
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).expect("entry").name().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n == "[Content_Types].xml"),
            "OOXML must include [Content_Types].xml; archive had {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "word/document.xml"),
            "OOXML must include word/document.xml; archive had {names:?}"
        );
        let mut entry = archive
            .by_name("word/document.xml")
            .expect("word/document.xml present");
        let mut out = String::new();
        entry.read_to_string(&mut out).expect("read document.xml");
        out
    }

    #[test]
    fn render_emits_valid_ooxml_with_expected_paragraphs() {
        let chat = sample_chat();
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        // OOXML files start with the zip local-file-header magic 'PK\x03\x04'.
        assert_eq!(&bytes[0..4], b"PK\x03\x04", "zip magic");
        let xml = extract_document_xml(&bytes);
        assert!(xml.contains("NDA review thread"), "title present");
        assert!(
            xml.contains("Created 2023-11-14T22:13:20Z"),
            "subtitle present: {}",
            xml.chars().take(400).collect::<String>()
        );
        assert!(xml.contains("] User"), "user heading present");
        assert!(xml.contains("] Assistant"), "assistant heading present");
        assert!(
            xml.contains("Section 3 covers confidentiality obligations."),
            "first body block present"
        );
        assert!(
            xml.contains("It requires both parties to keep information secret for 5 years."),
            "second body block present"
        );
        assert!(
            xml.contains("Documents referenced:"),
            "documents callout present"
        );
        assert!(xml.contains("nda.pdf"), "doc ref filename present");
        assert!(xml.contains("exhibit-a.pdf"), "doc ref filename 2 present");
    }

    #[test]
    fn render_falls_back_to_chat_id_when_title_missing() {
        let mut chat = sample_chat();
        chat.title = None;
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        let xml = extract_document_xml(&bytes);
        assert!(
            xml.contains("Chat 01HZ-test-chat"),
            "fallback title present"
        );
    }

    #[test]
    fn render_blank_title_falls_back_to_chat_id() {
        let mut chat = sample_chat();
        chat.title = Some("   ".to_string());
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        let xml = extract_document_xml(&bytes);
        assert!(
            xml.contains("Chat 01HZ-test-chat"),
            "blank title falls back"
        );
    }

    #[test]
    fn render_empty_chat_returns_chat_empty() {
        let mut chat = sample_chat();
        chat.messages.clear();
        let err = render_chat_to_docx(&chat).expect_err("must reject");
        assert!(matches!(err, LegalError::ChatEmpty(_)));
    }

    #[test]
    fn render_strips_xml_break_attempts_in_content() {
        // A user pasting raw XML should not be able to break out of the
        // document — docx-rs escapes via the builder API, but we also
        // want to assert the angle brackets show up encoded in the XML
        // rather than as literal markup.
        let mut chat = sample_chat();
        chat.messages = vec![ChatMessage {
            id: "m1".to_string(),
            role: ChatRole::User,
            content: "</w:t><w:body><w:p>broken</w:p>".to_string(),
            document_refs: vec![],
            created_at: ts(1_700_000_000),
        }];
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        let xml = extract_document_xml(&bytes);
        // The literal `</w:t>` from user content must appear as
        // `&lt;/w:t&gt;` after escaping. Confirm the escaped form is
        // present and the unescaped substring shows up only as part of
        // legitimate doc structure (not as literal user content). A
        // simple safety check is that `&lt;` exists at all and the
        // sentinel "broken" word is escaped in context, not adjacent to
        // a real `<w:p>` we control.
        assert!(
            xml.contains("&lt;/w:t&gt;") || xml.contains("&lt;w:t&gt;"),
            "user-supplied angle brackets must be XML-escaped: {}",
            xml.chars().take(2000).collect::<String>()
        );
    }

    #[test]
    fn render_strips_control_characters() {
        let mut chat = sample_chat();
        chat.messages = vec![ChatMessage {
            id: "m1".to_string(),
            role: ChatRole::User,
            content: "before\u{0001}after\nfine".to_string(),
            document_refs: vec![],
            created_at: ts(1_700_000_000),
        }];
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        let xml = extract_document_xml(&bytes);
        // `\u{0001}` would otherwise serialise as `&#x1;` — confirm it
        // does not appear in the output and the surrounding text is
        // glued together cleanly.
        assert!(
            !xml.contains("&#x1;") && !xml.contains('\u{0001}'),
            "control character must be stripped"
        );
        assert!(
            xml.contains("beforeafter"),
            "surrounding text glued together"
        );
    }

    #[test]
    fn render_handles_missing_document_refs_section_when_empty() {
        let mut chat = sample_chat();
        for m in &mut chat.messages {
            m.document_refs.clear();
        }
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        let xml = extract_document_xml(&bytes);
        assert!(
            !xml.contains("Documents referenced:"),
            "callout absent when no refs"
        );
    }

    #[test]
    fn render_truncates_oversized_document_refs_list() {
        let mut chat = sample_chat();
        chat.messages = vec![ChatMessage {
            id: "m1".to_string(),
            role: ChatRole::User,
            content: "test".to_string(),
            document_refs: (0..(MAX_DOC_REFS_PER_MESSAGE + 5))
                .map(|i| format!("file-{i}.pdf"))
                .collect(),
            created_at: ts(1_700_000_000),
        }];
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        let xml = extract_document_xml(&bytes);
        assert!(xml.contains("and 5 more"), "truncation summary present");
    }

    #[test]
    fn render_chunks_oversized_paragraph() {
        // A single block above the byte cap should split into multiple
        // body paragraphs without the renderer panicking or producing
        // invalid UTF-8.
        let huge = "x".repeat(MAX_BYTES_PER_PARAGRAPH * 3 + 17);
        let mut chat = sample_chat();
        chat.messages = vec![ChatMessage {
            id: "m1".to_string(),
            role: ChatRole::User,
            content: huge,
            document_refs: vec![],
            created_at: ts(1_700_000_000),
        }];
        let bytes = render_chat_to_docx(&chat).expect("render ok");
        // Just assert it produced a valid zip — reading the document
        // back is the structural integrity check.
        let _ = extract_document_xml(&bytes);
    }

    #[test]
    fn floor_char_boundary_handles_multibyte() {
        let s = "héllo"; // 'é' is two bytes
        // Index 2 is mid-char; should round down to 1.
        assert_eq!(floor_char_boundary(s, 2), 1);
        // Index 0 is always a boundary.
        assert_eq!(floor_char_boundary(s, 0), 0);
        // Index past end clamps to len.
        assert_eq!(floor_char_boundary(s, 99), s.len());
    }

    #[test]
    fn sanitize_body_text_keeps_tab_newline_cr() {
        let s = "a\tb\nc\rd".to_string();
        let cleaned = sanitize_body_text(&s);
        assert_eq!(cleaned, s);
    }
}
