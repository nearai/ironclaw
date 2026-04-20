//! Format-specific text extraction routines.

use std::io::Read;

/// Maximum decompressed size for a single ZIP entry (50 MB).
const MAX_DECOMPRESSED_ENTRY: u64 = 50 * 1024 * 1024;
/// Maximum total decompressed size across all ZIP entries (100 MB).
const MAX_DECOMPRESSED_TOTAL: u64 = 100 * 1024 * 1024;

/// Typed errors for ZIP decompression safety checks.
#[derive(Debug, thiserror::Error)]
enum ExtractionError {
    #[error("entry '{name}' decompressed size {size} exceeds per-entry limit {max}")]
    EntryTooLarge { name: String, size: u64, max: u64 },

    #[error("total decompressed size {current} exceeds limit {limit}")]
    TotalSizeLimitExceeded { limit: u64, current: u64 },

    #[error("failed to read zip entry '{name}': {source}")]
    EntryReadFailed {
        name: String,
        source: std::io::Error,
    },
}

/// Extract text from document bytes based on MIME type and optional filename.
pub fn extract_text(data: &[u8], mime: &str, filename: Option<&str>) -> Result<String, String> {
    let base_mime = mime.split(';').next().unwrap_or(mime).trim();

    match base_mime {
        // PDF
        "application/pdf" => extract_pdf(data),

        // Office XML formats
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            extract_docx(data)
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            extract_pptx(data)
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => extract_xlsx(data),

        // Legacy Office (best-effort: treat as binary, try text extraction)
        "application/msword" | "application/vnd.ms-powerpoint" | "application/vnd.ms-excel" => {
            // Legacy binary formats — try to extract any text strings
            extract_binary_strings(data)
        }

        // Plain text family
        "text/plain"
        | "text/csv"
        | "text/tab-separated-values"
        | "text/markdown"
        | "text/html"
        | "text/xml"
        | "text/x-python"
        | "text/x-java"
        | "text/x-c"
        | "text/x-c++"
        | "text/x-rust"
        | "text/x-go"
        | "text/x-ruby"
        | "text/x-shellscript"
        | "text/javascript"
        | "text/css"
        | "text/x-toml"
        | "text/x-yaml"
        | "text/x-log" => extract_utf8(data),

        // JSON / XML / YAML application types
        "application/json" | "application/xml" | "application/x-yaml" | "application/yaml"
        | "application/toml" | "application/x-sh" => extract_utf8(data),

        // RTF
        "application/rtf" | "text/rtf" => extract_rtf(data),

        // Fallback: try to infer from filename extension
        _ => {
            if let Some(text) = try_extract_by_extension(data, filename) {
                Ok(text)
            } else {
                Err(format!("unsupported document type: {base_mime}"))
            }
        }
    }
}

/// Read a zip entry into a string with configurable decompressed size limits.
fn bounded_read_zip_entry_with_limits(
    file: &mut zip::read::ZipFile<'_>,
    total_decompressed: &mut u64,
    max_entry: u64,
    max_total: u64,
) -> Result<String, ExtractionError> {
    let entry_size = file.size();
    let entry_name = file.name().to_string();

    // Fast pre-check using declared header size (untrusted, but cheap reject)
    // against per-entry limit.
    if entry_size > max_entry {
        return Err(ExtractionError::EntryTooLarge {
            name: entry_name,
            size: entry_size,
            max: max_entry,
        });
    }

    // Pre-check: reject early if the declared size would blow the cumulative
    // budget. The header value is untrusted, but it lets us reject obviously
    // oversized archives without decompressing.
    if *total_decompressed + entry_size > max_total {
        return Err(ExtractionError::TotalSizeLimitExceeded {
            limit: max_total,
            current: *total_decompressed + entry_size,
        });
    }

    let mut bounded = file.take(max_entry);
    let mut xml = String::new();
    bounded
        .read_to_string(&mut xml)
        .map_err(|e| ExtractionError::EntryReadFailed {
            name: entry_name.clone(),
            source: e,
        })?;

    let actual_size = xml.len() as u64;

    // Fail closed: if we read exactly the cap, the entry was truncated and
    // the real decompressed size exceeds the per-entry limit.
    if actual_size >= max_entry {
        return Err(ExtractionError::EntryTooLarge {
            name: entry_name,
            size: actual_size,
            max: max_entry,
        });
    }

    // Track cumulative budget using actual bytes, not header metadata.
    *total_decompressed += actual_size;
    if *total_decompressed > max_total {
        return Err(ExtractionError::TotalSizeLimitExceeded {
            limit: max_total,
            current: *total_decompressed,
        });
    }

    Ok(xml)
}

/// Read a zip entry into a string with default decompressed size limits.
///
/// Uses the declared header size as a fast pre-check for both per-entry and
/// cumulative budgets, then tracks **actual bytes read** for the cumulative
/// budget (ZIP headers can lie about sizes). The `take()` reader caps any
/// single entry at `MAX_DECOMPRESSED_ENTRY`. If the reader hits that cap
/// exactly we fail closed — the entry was truncated, meaning the real size
/// exceeds the limit.
fn bounded_read_zip_entry(
    file: &mut zip::read::ZipFile<'_>,
    total_decompressed: &mut u64,
) -> Result<String, ExtractionError> {
    bounded_read_zip_entry_with_limits(
        file,
        total_decompressed,
        MAX_DECOMPRESSED_ENTRY,
        MAX_DECOMPRESSED_TOTAL,
    )
}

fn extract_pdf(data: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(data)
        .map(|t| t.trim().to_string())
        .map_err(|e| format!("PDF extraction failed: {e}"))
}

fn extract_docx(data: &[u8]) -> Result<String, String> {
    extract_office_xml(data, "word/document.xml")
}

fn extract_pptx(data: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("invalid PPTX archive: {e}"))?;

    // Collect slide filenames (ppt/slides/slide1.xml, slide2.xml, ...)
    let mut slide_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            let name = file.name().to_string();
            if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                slide_names.push(name);
            }
        }
    }
    slide_names.sort();

    let mut all_text = Vec::new();
    let mut total_decompressed: u64 = 0;
    for name in &slide_names {
        let Ok(mut file) = archive.by_name(name) else {
            continue;
        };
        let Ok(xml) =
            bounded_read_zip_entry(&mut file, &mut total_decompressed).map_err(|e| e.to_string())
        else {
            continue;
        };
        let text = strip_xml_tags(&xml);
        if !text.is_empty() {
            all_text.push(text);
        }
    }

    if all_text.is_empty() {
        return Err("no text found in PPTX slides".to_string());
    }
    Ok(all_text.join("\n\n---\n\n"))
}

fn extract_xlsx(data: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("invalid XLSX archive: {e}"))?;

    let mut total_decompressed: u64 = 0;

    // Read shared strings (xl/sharedStrings.xml)
    let shared_strings = if let Ok(mut file) = archive.by_name("xl/sharedStrings.xml") {
        let xml = bounded_read_zip_entry(&mut file, &mut total_decompressed)
            .map_err(|e| format!("failed to read shared strings: {e}"))?;
        parse_xlsx_shared_strings(&xml)
    } else {
        Vec::new()
    };

    // Read sheet data
    let mut sheet_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            let name = file.name().to_string();
            if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
                sheet_names.push(name);
            }
        }
    }
    sheet_names.sort();

    let mut all_text = Vec::new();
    for name in &sheet_names {
        let Ok(mut file) = archive.by_name(name) else {
            continue;
        };
        let Ok(xml) =
            bounded_read_zip_entry(&mut file, &mut total_decompressed).map_err(|e| e.to_string())
        else {
            continue;
        };
        let text = parse_xlsx_sheet(&xml, &shared_strings);
        if !text.is_empty() {
            all_text.push(text);
        }
    }

    if all_text.is_empty() && !shared_strings.is_empty() {
        // Fallback: just return shared strings
        return Ok(shared_strings.join("\n"));
    }

    if all_text.is_empty() {
        return Err("no text found in XLSX".to_string());
    }
    Ok(all_text.join("\n\n"))
}

fn extract_office_xml(data: &[u8], content_path: &str) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("invalid Office XML archive: {e}"))?;

    let mut file = archive
        .by_name(content_path)
        .map_err(|e| format!("content file not found in archive: {e}"))?;

    let mut total_decompressed: u64 = 0;
    let xml = bounded_read_zip_entry(&mut file, &mut total_decompressed)
        .map_err(|e| format!("failed to read content: {e}"))?;

    let text = strip_xml_tags(&xml);
    if text.is_empty() {
        return Err("no text content found".to_string());
    }
    Ok(text)
}

fn extract_utf8(data: &[u8]) -> Result<String, String> {
    // Try UTF-8 first, fall back to lossy decoding
    match std::str::from_utf8(data) {
        Ok(s) => Ok(s.to_string()),
        Err(_) => Ok(String::from_utf8_lossy(data).to_string()),
    }
}

fn extract_rtf(data: &[u8]) -> Result<String, String> {
    // Basic RTF text extraction: strip control words and groups
    let text = String::from_utf8_lossy(data);
    let mut result = String::new();
    let mut depth = 0i32;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' => depth += 1,
            '}' => depth = (depth - 1).max(0),
            '\\' => {
                // Skip control word
                let mut word = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphabetic() {
                        chars.next();
                        word.push(next);
                    } else {
                        break;
                    }
                }
                // Skip optional numeric parameter
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() || next == '-' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Consume trailing space
                if let Some(&' ') = chars.peek() {
                    chars.next();
                }
                // Convert common control words to text
                match word.as_str() {
                    "par" | "line" => result.push('\n'),
                    "tab" => result.push('\t'),
                    _ => {}
                }
            }
            _ => {
                if depth <= 1 {
                    result.push(ch);
                }
            }
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        return Err("no text found in RTF".to_string());
    }
    Ok(trimmed)
}

fn extract_binary_strings(data: &[u8]) -> Result<String, String> {
    // Extract printable ASCII/UTF-8 runs from binary data (last resort)
    let mut strings = Vec::new();
    let mut current = String::new();

    for &byte in data {
        if (0x20..0x7F).contains(&byte) {
            current.push(byte as char);
        } else {
            if current.len() >= 4 {
                strings.push(std::mem::take(&mut current));
            }
            current.clear();
        }
    }
    if current.len() >= 4 {
        strings.push(current);
    }

    if strings.is_empty() {
        return Err("no readable text in binary document".to_string());
    }
    Ok(strings.join(" "))
}

/// Strip XML tags and return just the text content.
fn strip_xml_tags(xml: &str) -> String {
    let mut result = String::with_capacity(xml.len() / 2);
    let mut in_tag = false;
    let mut last_was_space = true;

    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
            }
            '>' => {
                in_tag = false;
                // Add space between tag-delimited text runs
                if !last_was_space && !result.is_empty() {
                    result.push(' ');
                    last_was_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_was_space {
                        result.push(' ');
                        last_was_space = true;
                    }
                } else {
                    result.push(ch);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }

    // Decode common XML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

/// Parse XLSX shared strings XML into a Vec of strings.
fn parse_xlsx_shared_strings(xml: &str) -> Vec<String> {
    // Shared strings are in <si><t>text</t></si> elements
    let mut strings = Vec::new();
    let mut in_t = false;
    let mut current = String::new();
    let mut in_tag = false;
    let mut tag_name = String::new();

    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_name.clear();
            }
            '>' => {
                in_tag = false;
                let tag = tag_name.trim().to_string();
                if tag == "t" || tag.starts_with("t ") {
                    in_t = true;
                    current.clear();
                } else if tag == "/t" {
                    in_t = false;
                    strings.push(std::mem::take(&mut current));
                } else if tag == "/si" {
                    in_t = false;
                }
            }
            _ if in_tag => {
                tag_name.push(ch);
            }
            _ if in_t => {
                current.push(ch);
            }
            _ => {}
        }
    }

    strings
}

/// Parse XLSX sheet XML into tab-separated rows.
fn parse_xlsx_sheet(xml: &str, shared_strings: &[String]) -> String {
    // Simple extraction: find <v> values in <c> cells, resolve shared string refs
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut in_v = false;
    let mut in_row = false;
    let mut current_val = String::new();
    let mut cell_type = String::new();
    let mut in_tag = false;
    let mut tag_buf = String::new();

    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' => {
                in_tag = false;
                let tag = tag_buf.trim().to_string();
                if tag == "row" || tag.starts_with("row ") {
                    in_row = true;
                    current_row.clear();
                } else if tag == "/row" {
                    in_row = false;
                    if !current_row.is_empty() {
                        rows.push(std::mem::take(&mut current_row));
                    }
                } else if in_row && (tag.starts_with("c ") || tag == "c") {
                    // Extract type attribute: t="s" means shared string
                    cell_type.clear();
                    if let Some(t_pos) = tag.find("t=\"") {
                        let rest = &tag[t_pos + 3..];
                        if let Some(end) = rest.find('"') {
                            cell_type = rest[..end].to_string();
                        }
                    }
                } else if tag == "v" || tag.starts_with("v ") {
                    in_v = true;
                    current_val.clear();
                } else if tag == "/v" {
                    in_v = false;
                    let val = if cell_type == "s" {
                        // Shared string reference
                        current_val
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|idx| shared_strings.get(idx))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        current_val.clone()
                    };
                    current_row.push(val);
                } else if tag == "/c" {
                    cell_type.clear();
                }
            }
            _ if in_tag => {
                tag_buf.push(ch);
            }
            _ if in_v => {
                current_val.push(ch);
            }
            _ => {}
        }
    }

    rows.iter()
        .map(|row| row.join("\t"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Try to extract text based on filename extension when MIME type is generic.
fn try_extract_by_extension(data: &[u8], filename: Option<&str>) -> Option<String> {
    let ext = filename?.rsplit('.').next()?.to_lowercase();

    match ext.as_str() {
        "pdf" => extract_pdf(data).ok(),
        "docx" => extract_docx(data).ok(),
        "pptx" => extract_pptx(data).ok(),
        "xlsx" => extract_xlsx(data).ok(),
        "doc" | "ppt" | "xls" => extract_binary_strings(data).ok(),
        "rtf" => extract_rtf(data).ok(),
        "txt" | "csv" | "tsv" | "json" | "xml" | "yaml" | "yml" | "toml" | "md" | "markdown"
        | "py" | "js" | "ts" | "rs" | "go" | "java" | "c" | "cpp" | "h" | "hpp" | "rb" | "sh"
        | "bash" | "zsh" | "fish" | "css" | "html" | "htm" | "sql" | "log" | "ini" | "cfg"
        | "conf" | "env" | "gitignore" | "dockerfile" => extract_utf8(data).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_xml_basic() {
        let xml = "<root><p>Hello</p><p>World</p></root>";
        assert_eq!(strip_xml_tags(xml), "Hello World");
    }

    #[test]
    fn strip_xml_entities() {
        let xml = "<t>A &amp; B &lt; C</t>";
        assert_eq!(strip_xml_tags(xml), "A & B < C");
    }

    #[test]
    fn extract_utf8_valid() {
        assert_eq!(extract_utf8(b"hello").unwrap(), "hello");
    }

    #[test]
    fn extract_utf8_lossy() {
        let data = b"hello \xff world";
        let result = extract_utf8(data).unwrap();
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
    }

    #[test]
    fn extract_by_extension_txt() {
        let result = try_extract_by_extension(b"content", Some("notes.txt"));
        assert_eq!(result, Some("content".to_string()));
    }

    #[test]
    fn extract_by_extension_unknown() {
        let result = try_extract_by_extension(b"data", Some("file.xyz"));
        assert!(result.is_none());
    }

    #[test]
    fn extract_by_extension_no_filename() {
        let result = try_extract_by_extension(b"data", None);
        assert!(result.is_none());
    }

    #[test]
    fn rtf_basic_extraction() {
        let rtf = br"{\rtf1\ansi Hello World\par Second line}";
        let result = extract_rtf(rtf).unwrap();
        assert!(result.contains("Hello World"));
        assert!(result.contains("Second line"));
    }

    #[test]
    fn xlsx_shared_strings_parsing() {
        let xml = r#"<sst><si><t>Name</t></si><si><t>Age</t></si></sst>"#;
        let strings = parse_xlsx_shared_strings(xml);
        assert_eq!(strings, vec!["Name", "Age"]);
    }

    /// Regression: bounded_read_zip_entry tracks actual bytes read (not header
    /// metadata) and a small entry should succeed with correct accounting.
    #[test]
    fn bounded_read_tracks_actual_bytes() {
        use std::io::{Cursor, Write};
        let content = b"<root>hello</root>";
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("test.xml", options).unwrap();
        writer.write_all(content).unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        let mut total: u64 = 0;
        let mut file = archive.by_index(0).unwrap();
        let result = bounded_read_zip_entry(&mut file, &mut total);
        assert!(result.is_ok(), "small entry should be readable");
        // Total must reflect actual content length, not header-declared size.
        assert_eq!(total, content.len() as u64);
    }

    /// Regression: total decompressed tracking must accumulate actual bytes
    /// across entries and equal the sum of real content sizes.
    #[test]
    fn bounded_read_accumulates_actual_bytes_across_entries() {
        use std::io::{Cursor, Write};
        let content_a = b"<a>data</a>";
        let content_b = b"<b>more data here</b>";
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("a.xml", options).unwrap();
        writer.write_all(content_a).unwrap();
        writer.start_file("b.xml", options).unwrap();
        writer.write_all(content_b).unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        let mut total: u64 = 0;
        let mut f0 = archive.by_index(0).unwrap();
        bounded_read_zip_entry(&mut f0, &mut total).unwrap();
        drop(f0);
        let mut f1 = archive.by_index(1).unwrap();
        bounded_read_zip_entry(&mut f1, &mut total).unwrap();
        let expected = (content_a.len() + content_b.len()) as u64;
        assert_eq!(
            total, expected,
            "total must equal sum of actual content sizes"
        );
    }

    /// Regression: bounded_read_zip_entry must reject when cumulative
    /// decompressed bytes exceed MAX_DECOMPRESSED_TOTAL, even for small entries.
    #[test]
    fn bounded_read_rejects_when_total_budget_exhausted() {
        use std::io::{Cursor, Write};
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("small.xml", options).unwrap();
        writer.write_all(b"<x/>").unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        // Pre-fill the budget to just below the limit so even a tiny entry
        // pushes it over.
        let mut total: u64 = MAX_DECOMPRESSED_TOTAL - 1;
        let mut file = archive.by_index(0).unwrap();
        let result = bounded_read_zip_entry(&mut file, &mut total);
        assert!(
            result.is_err(),
            "should reject when total budget is exceeded"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, ExtractionError::TotalSizeLimitExceeded { .. }),
            "error should be TotalSizeLimitExceeded, got: {err}"
        );
    }

    /// Regression: the pre-check must reject based on header-declared size
    /// against the cumulative budget before any decompression occurs.
    #[test]
    fn bounded_read_precheck_rejects_declared_size_over_total_budget() {
        use std::io::{Cursor, Write};
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("entry.xml", options).unwrap();
        writer.write_all(b"<ok/>").unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        // Set total so that the declared entry size pushes past the total limit.
        let mut file = archive.by_index(0).unwrap();
        let declared = file.size();
        let mut total: u64 = MAX_DECOMPRESSED_TOTAL - declared + 1;
        let result = bounded_read_zip_entry(&mut file, &mut total);
        assert!(
            result.is_err(),
            "pre-check should reject when declared size would exceed total budget"
        );
        assert!(
            matches!(
                result.unwrap_err(),
                ExtractionError::TotalSizeLimitExceeded { .. }
            ),
            "error should be TotalSizeLimitExceeded"
        );
    }

    /// Regression: per-entry truncation path must reject when actual decompressed
    /// bytes hit the per-entry cap (fail-closed). This is the path that stops a
    /// real zip bomb where the header lies about the size.
    #[test]
    fn bounded_read_rejects_entry_exceeding_per_entry_limit() {
        use std::io::{Cursor, Write};
        let content = b"<root>this content is longer than the limit</root>";
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("big.xml", options).unwrap();
        writer.write_all(content).unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        let mut total: u64 = 0;
        let mut file = archive.by_index(0).unwrap();
        // Use a small per-entry limit so the entry triggers truncation.
        let result =
            bounded_read_zip_entry_with_limits(&mut file, &mut total, 10, MAX_DECOMPRESSED_TOTAL);
        assert!(result.is_err(), "should reject entry exceeding per-entry limit");
        assert!(
            matches!(result.unwrap_err(), ExtractionError::EntryTooLarge { .. }),
            "error should be EntryTooLarge"
        );
    }

    /// Regression: per-entry pre-check must reject when the declared header size
    /// exceeds the per-entry limit before any decompression occurs.
    #[test]
    fn bounded_read_precheck_rejects_declared_entry_too_large() {
        use std::io::{Cursor, Write};
        let content = b"<root>this is bigger than 10 bytes</root>";
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("declared-big.xml", options).unwrap();
        writer.write_all(content).unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        let mut total: u64 = 0;
        let mut file = archive.by_index(0).unwrap();
        let result =
            bounded_read_zip_entry_with_limits(&mut file, &mut total, 10, MAX_DECOMPRESSED_TOTAL);
        assert!(result.is_err(), "pre-check should reject based on declared size");
        assert!(
            matches!(result.unwrap_err(), ExtractionError::EntryTooLarge { .. }),
            "error should be EntryTooLarge"
        );
    }

    /// Regression: cumulative total limit must reject when multiple small entries
    /// collectively exceed the total budget.
    #[test]
    fn bounded_read_rejects_cumulative_over_small_total_limit() {
        use std::io::{Cursor, Write};
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("a.xml", options).unwrap();
        writer.write_all(b"<a>aaaa</a>").unwrap();
        writer.start_file("b.xml", options).unwrap();
        writer.write_all(b"<b>bbbb</b>").unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let read_cursor = Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(read_cursor).unwrap();
        let mut total: u64 = 0;
        // Per-entry limit is generous, but total budget is very small.
        let max_total = 15;
        let mut f0 = archive.by_index(0).unwrap();
        let r0 = bounded_read_zip_entry_with_limits(&mut f0, &mut total, 1024, max_total);
        assert!(r0.is_ok(), "first entry should fit within total budget");
        drop(f0);

        let mut f1 = archive.by_index(1).unwrap();
        let r1 = bounded_read_zip_entry_with_limits(&mut f1, &mut total, 1024, max_total);
        assert!(r1.is_err(), "second entry should exceed total budget");
        assert!(
            matches!(r1.unwrap_err(), ExtractionError::TotalSizeLimitExceeded { .. }),
            "error should be TotalSizeLimitExceeded"
        );
    }

    /// Caller-level: extract_office_xml (DOCX path) must reject an oversized entry.
    #[test]
    fn extract_docx_rejects_oversized_entry() {
        use std::io::{Cursor, Write};
        let big_content = "x".repeat(60 * 1024 * 1024);
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("word/document.xml", options).unwrap();
        writer.write_all(big_content.as_bytes()).unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let result = extract_office_xml(&data, "word/document.xml");
        assert!(result.is_err(), "extract_office_xml must reject oversized entry");
    }

    /// Caller-level: extract_pptx must reject when a slide exceeds per-entry limit.
    #[test]
    fn extract_pptx_rejects_oversized_slide() {
        use std::io::{Cursor, Write};
        let big_slide = "<a:t>".to_string() + &"x".repeat(60 * 1024 * 1024) + "</a:t>";
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("ppt/slides/slide1.xml", options).unwrap();
        writer.write_all(big_slide.as_bytes()).unwrap();
        let cursor = writer.finish().unwrap();
        let data = cursor.into_inner();

        let result = extract_pptx(&data);
        // extract_pptx swallows per-entry errors (continues to next slide),
        // so with one oversized slide and no valid slides, it returns an error.
        assert!(result.is_err(), "extract_pptx must fail when only slide is oversized");
    }
}
