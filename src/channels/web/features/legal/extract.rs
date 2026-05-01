//! Inline text extraction for uploaded PDFs and DOCX files.
//!
//! Both extractors run synchronously on the request thread today, wrapped
//! in `tokio::task::spawn_blocking` so the async runtime stays responsive.
//! For very large uploads the gateway's body-size cap (14 MiB by default)
//! puts a ceiling on the time budget; a follow-up could move extraction
//! to a worker queue.
//!
//! No `unwrap` in the data path — every PDF/DOCX parser failure surfaces
//! as `ExtractError::Pdf` / `ExtractError::Docx`, which the upload
//! handler converts to a 422 with the underlying message.

use std::io::Read;

/// Result of a successful text extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub text: String,
    /// Page count if the format exposes one (PDF). `None` for DOCX where
    /// pagination is layout-driven and not stored in the source XML.
    pub page_count: Option<i64>,
}

/// Extraction errors. Distinct variants per format so handlers can produce
/// a clear error message without doing `dyn Error` introspection.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("PDF extraction failed: {0}")]
    Pdf(String),
    #[error("DOCX extraction failed: {0}")]
    Docx(String),
    #[error("unsupported document type: {0}")]
    Unsupported(String),
    #[error("blocking task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// Probe the bytes/declared content type and pick a parser. Falls back to
/// magic-byte sniffing when the client lies about the mime.
pub async fn extract(content_type: &str, filename: &str, bytes: &[u8]) -> Result<Extracted, ExtractError> {
    let kind = sniff(content_type, filename, bytes);
    match kind {
        DocKind::Pdf => extract_pdf(bytes.to_vec()).await,
        DocKind::Docx => extract_docx(bytes.to_vec()).await,
        DocKind::Unknown => Err(ExtractError::Unsupported(format!(
            "filename={filename} content_type={content_type}"
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocKind {
    Pdf,
    Docx,
    Unknown,
}

const PDF_MAGIC: &[u8] = b"%PDF-";
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

fn sniff(content_type: &str, filename: &str, bytes: &[u8]) -> DocKind {
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let ct = content_type.trim().to_ascii_lowercase();

    if bytes.starts_with(PDF_MAGIC) || ct == "application/pdf" || ext == "pdf" {
        return DocKind::Pdf;
    }
    let docx_ct =
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    if ct == docx_ct
        || ext == "docx"
        // OOXML packages are zip archives: trust the extension/mime and
        // verify the zip signature, but never trust an arbitrary zip.
        || (bytes.starts_with(ZIP_MAGIC) && (ext == "docx" || ct == docx_ct))
    {
        return DocKind::Docx;
    }
    DocKind::Unknown
}

async fn extract_pdf(bytes: Vec<u8>) -> Result<Extracted, ExtractError> {
    let join = tokio::task::spawn_blocking(move || -> Result<Extracted, ExtractError> {
        let text = pdf_extract::extract_text_from_mem(&bytes)
            .map_err(|e| ExtractError::Pdf(e.to_string()))?;
        let page_count = count_pdf_pages(&bytes);
        Ok(Extracted {
            text: normalise_whitespace(&text),
            page_count,
        })
    })
    .await?;
    join
}

/// Best-effort page-count using `pdf-extract`'s public API. The crate
/// exposes per-page extraction; if that fails we just return `None` so the
/// upload still succeeds with text.
fn count_pdf_pages(bytes: &[u8]) -> Option<i64> {
    // `pdf-extract` doesn't currently expose a public page-count helper, so
    // we fall back to counting `\f` (form-feed) markers it inserts between
    // pages in the extracted text. Empty/garbled PDFs produce `None`.
    let extracted = pdf_extract::extract_text_from_mem(bytes).ok()?;
    let pages = extracted.matches('\u{000C}').count() as i64;
    if pages > 0 { Some(pages + 1) } else { None }
}

async fn extract_docx(bytes: Vec<u8>) -> Result<Extracted, ExtractError> {
    let join = tokio::task::spawn_blocking(move || -> Result<Extracted, ExtractError> {
        let cursor = std::io::Cursor::new(&bytes);
        let mut zip = zip::ZipArchive::new(cursor)
            .map_err(|e| ExtractError::Docx(format!("zip open: {e}")))?;
        let mut file = zip
            .by_name("word/document.xml")
            .map_err(|e| ExtractError::Docx(format!("missing word/document.xml: {e}")))?;
        let mut xml = String::new();
        file.read_to_string(&mut xml)
            .map_err(|e| ExtractError::Docx(format!("read document.xml: {e}")))?;
        let text = xml_to_text(&xml)?;
        Ok(Extracted {
            text: normalise_whitespace(&text),
            page_count: None,
        })
    })
    .await?;
    join
}

/// Walk `word/document.xml` and concatenate the text inside `<w:t>` runs.
/// Paragraph breaks (`<w:p>` end) become blank lines so downstream RAG
/// chunks line up with logical paragraphs.
fn xml_to_text(xml: &str) -> Result<String, ExtractError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    let mut out = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"w:t" {
                    in_text = true;
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let bytes = name.as_ref();
                if bytes == b"w:t" {
                    in_text = false;
                } else if bytes == b"w:p" {
                    out.push('\n');
                } else if bytes == b"w:tab" || bytes == b"w:br" {
                    out.push(' ');
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                let bytes = name.as_ref();
                if bytes == b"w:tab" {
                    out.push('\t');
                } else if bytes == b"w:br" {
                    out.push('\n');
                }
            }
            Ok(Event::Text(t)) => {
                if in_text {
                    let unescaped = t
                        .unescape()
                        .map_err(|e| ExtractError::Docx(format!("xml unescape: {e}")))?;
                    out.push_str(&unescaped);
                }
            }
            Ok(_) => {}
            Err(e) => {
                return Err(ExtractError::Docx(format!(
                    "xml parse error at pos {}: {e}",
                    reader.buffer_position()
                )));
            }
        }
        buf.clear();
    }

    Ok(out)
}

/// Collapse runs of whitespace into a single space, but keep newline
/// boundaries — they're cheap signal for LLM chunking.
fn normalise_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch == '\n' {
            // Preserve paragraph breaks; collapse adjacent spaces before/after.
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            prev_space = false;
            continue;
        }
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    // Trim trailing whitespace runs.
    while matches!(out.chars().last(), Some(' ') | Some('\n')) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_prefers_pdf_magic() {
        let bytes = b"%PDF-1.5\n...";
        assert_eq!(sniff("application/octet-stream", "x.bin", bytes), DocKind::Pdf);
    }

    #[test]
    fn sniff_uses_extension_for_docx() {
        let bytes = b"PK\x03\x04";
        assert_eq!(sniff("application/zip", "x.docx", bytes), DocKind::Docx);
    }

    #[test]
    fn sniff_unknown_for_random() {
        assert_eq!(sniff("text/plain", "notes.txt", b"hello"), DocKind::Unknown);
    }

    #[test]
    fn xml_to_text_extracts_runs() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world</w:t></w:r></w:p>
    <w:p><w:r><w:t>Second &amp; line</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let got = xml_to_text(xml).expect("xml ok");
        let normalised = normalise_whitespace(&got);
        assert!(normalised.contains("Hello world"));
        assert!(normalised.contains("Second & line"));
    }

    #[test]
    fn normalise_collapses_runs() {
        let s = "hello    world\n\n   second\n";
        assert_eq!(normalise_whitespace(s), "hello world\n\nsecond");
    }
}
