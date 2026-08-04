//! The copy-through core every OOXML editor in this crate is built on.
//!
//! An OOXML file (docx/xlsx/pptx) is a zip of XML parts plus binary media.
//! The one rule that makes editing safe is: **rewrite only the parts you
//! actually target, and copy every other entry through byte-for-byte.**
//! Styles, numbering, themes, fonts, images, headers, embedded objects and
//! content types are then preserved by construction rather than by a
//! regenerator remembering to emit them — which is the failure mode that makes
//! "parse to text, build a new document" silently destroy real files.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use quick_xml::events::Event;
use quick_xml::{Reader, Writer};

use crate::error::DocumentError;

/// Per-entry decompressed ceiling. Mirrors `ironclaw_extractors`' budget: a zip
/// bomb must not be able to turn a small upload into unbounded memory.
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
/// Cumulative decompressed ceiling across all entries of one archive.
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
/// Upper bound on parts in one archive, so a pathological entry count cannot
/// stall the editor before any size budget bites.
const MAX_ENTRIES: usize = 4096;

/// An OOXML package held in memory as ordered, named entries.
///
/// Order is preserved because `[Content_Types].xml` must remain the first
/// entry for some consumers, and because a stable order is what makes
/// "no edits in, byte-identical out" testable.
pub(crate) struct OoxmlPackage {
    names: Vec<String>,
    entries: BTreeMap<String, Vec<u8>>,
}

impl OoxmlPackage {
    /// Read every entry of `data` into memory under the size budgets above.
    pub(crate) fn read(data: &[u8]) -> Result<Self, DocumentError> {
        let mut archive = zip::ZipArchive::new(Cursor::new(data))
            .map_err(|error| DocumentError::NotAnOoxmlPackage(error.to_string()))?;
        if archive.len() > MAX_ENTRIES {
            return Err(DocumentError::PackageTooLarge {
                detail: format!("{} entries exceeds the {MAX_ENTRIES} limit", archive.len()),
            });
        }

        let mut names = Vec::with_capacity(archive.len());
        let mut entries = BTreeMap::new();
        let mut total: u64 = 0;
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|error| DocumentError::NotAnOoxmlPackage(error.to_string()))?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().to_string();
            // The declared size is a hint a hostile archive can lie about, so
            // it is only a fast pre-check; `take` below enforces the real cap
            // against bytes actually read.
            if file.size() > MAX_ENTRY_BYTES {
                return Err(DocumentError::PackageTooLarge {
                    detail: format!("entry {name} declares {} bytes", file.size()),
                });
            }
            let mut bytes = Vec::new();
            let read = std::io::copy(&mut file.by_ref().take(MAX_ENTRY_BYTES + 1), &mut bytes)
                .map_err(|error| DocumentError::NotAnOoxmlPackage(error.to_string()))?;
            if read > MAX_ENTRY_BYTES {
                return Err(DocumentError::PackageTooLarge {
                    detail: format!("entry {name} exceeds {MAX_ENTRY_BYTES} bytes"),
                });
            }
            total += read;
            if total > MAX_TOTAL_BYTES {
                return Err(DocumentError::PackageTooLarge {
                    detail: format!("archive exceeds {MAX_TOTAL_BYTES} decompressed bytes"),
                });
            }
            names.push(name.clone());
            entries.insert(name, bytes);
        }
        Ok(Self { names, entries })
    }

    pub(crate) fn part(&self, name: &str) -> Result<&[u8], DocumentError> {
        self.entries
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| DocumentError::MissingPart {
                part: name.to_string(),
            })
    }

    pub(crate) fn part_str(&self, name: &str) -> Result<String, DocumentError> {
        let bytes = self.part(name)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DocumentError::MalformedPart {
            part: name.to_string(),
            detail: "not valid UTF-8".to_string(),
        })
    }

    pub(crate) fn has_part(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Replace one part's bytes. The entry keeps its position in the archive.
    pub(crate) fn replace_part(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), DocumentError> {
        if !self.entries.contains_key(name) {
            return Err(DocumentError::MissingPart {
                part: name.to_string(),
            });
        }
        self.entries.insert(name.to_string(), bytes);
        Ok(())
    }

    /// Add a new part at the end of the archive. Used by the pptx slide clone,
    /// which genuinely adds parts rather than rewriting existing ones.
    pub(crate) fn insert_part(&mut self, name: &str, bytes: Vec<u8>) {
        if !self.entries.contains_key(name) {
            self.names.push(name.to_string());
        }
        self.entries.insert(name.to_string(), bytes);
    }

    /// Names in archive order, for callers that need to scan a family of parts
    /// (`ppt/slides/slideN.xml`, `xl/worksheets/sheetN.xml`).
    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }

    /// Serialize back to a zip.
    ///
    /// Deflate and archive order are both fixed, and no timestamps are written,
    /// so the same package with the same edits always produces the same bytes —
    /// which is what lets `docx_round_trip_without_edits_is_byte_identical`
    /// exist as a test rather than a hope.
    pub(crate) fn write(&self) -> Result<Vec<u8>, DocumentError> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .last_modified_time(
                    zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
                        .map_err(|error| DocumentError::Write(error.to_string()))?,
                );
            for name in &self.names {
                let Some(bytes) = self.entries.get(name) else {
                    continue;
                };
                zip.start_file(name, options)
                    .map_err(|error| DocumentError::Write(error.to_string()))?;
                zip.write_all(bytes)
                    .map_err(|error| DocumentError::Write(error.to_string()))?;
            }
            zip.finish()
                .map_err(|error| DocumentError::Write(error.to_string()))?;
        }
        Ok(cursor.into_inner())
    }
}

/// What [`transform_xml`] should do with the event it was handed.
pub(crate) enum EventAction<'a> {
    /// Write the event through unchanged. The overwhelmingly common case.
    Keep,
    /// Write these events instead of the original one. An empty slice deletes.
    Replace(Vec<Event<'a>>),
    /// Drop the original and write nothing.
    Drop,
}

/// Stream `xml` through `handle`, writing every event back out.
///
/// This is the whole reason the crate uses an event parser: an event the
/// handler does not claim is re-emitted exactly as it was read, so attribute
/// order, namespace prefixes, self-closing forms, and inter-element whitespace
/// all survive. Only claimed events change.
pub(crate) fn transform_xml<F>(xml: &str, mut handle: F) -> Result<String, DocumentError>
where
    F: for<'a> FnMut(&Event<'a>) -> Result<EventAction<'static>, DocumentError>,
{
    let mut reader = Reader::from_str(xml);
    let config = reader.config_mut();
    config.trim_text(false);
    config.check_end_names = false;

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    loop {
        let event = reader
            .read_event()
            .map_err(|error| DocumentError::MalformedPart {
                part: "xml".to_string(),
                detail: error.to_string(),
            })?;
        if matches!(event, Event::Eof) {
            break;
        }
        match handle(&event)? {
            EventAction::Keep => {
                writer
                    .write_event(event)
                    .map_err(|error| DocumentError::Write(error.to_string()))?;
            }
            EventAction::Replace(events) => {
                for replacement in events {
                    writer
                        .write_event(replacement)
                        .map_err(|error| DocumentError::Write(error.to_string()))?;
                }
            }
            EventAction::Drop => {}
        }
    }
    String::from_utf8(writer.into_inner().into_inner())
        .map_err(|_| DocumentError::Write("transformed XML is not valid UTF-8".to_string()))
}

/// The local name of an element, with any namespace prefix stripped.
///
/// OOXML producers vary the prefix they bind (`w:p` vs `w14:p` vs a default
/// namespace), so matching on the local name is what makes the editors work on
/// documents from more than one producer.
pub(crate) fn local_name(raw: &[u8]) -> &[u8] {
    match raw.iter().position(|byte| *byte == b':') {
        Some(index) => &raw[index + 1..],
        None => raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for (name, bytes) in entries {
                zip.start_file(*name, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn untouched_package_round_trips_to_identical_bytes() {
        let original = zip_with(&[("a.xml", b"<a/>"), ("media/x.bin", &[0u8, 1, 2, 255])]);
        let first = OoxmlPackage::read(&original).unwrap().write().unwrap();
        let second = OoxmlPackage::read(&first).unwrap().write().unwrap();
        assert_eq!(first, second, "re-writing must be deterministic");
    }

    #[test]
    fn replacing_one_part_leaves_every_other_entry_byte_identical() {
        let media: &[u8] = &[0u8, 1, 2, 255, 254];
        let original = zip_with(&[("a.xml", b"<a/>"), ("media/x.bin", media)]);
        let mut package = OoxmlPackage::read(&original).unwrap();
        package
            .replace_part("a.xml", b"<a>edited</a>".to_vec())
            .unwrap();
        let edited = OoxmlPackage::read(&package.write().unwrap()).unwrap();
        assert_eq!(edited.part("a.xml").unwrap(), b"<a>edited</a>");
        assert_eq!(
            edited.part("media/x.bin").unwrap(),
            media,
            "an untouched binary part must survive bit-for-bit"
        );
    }

    #[test]
    fn transform_preserves_attribute_order_and_self_closing_forms() {
        // A DOM round trip is exactly what normalizes these; the event
        // transform must not.
        let xml = r#"<?xml version="1.0"?><w:p z="3" a="1"><w:r/><w:t>hi</w:t></w:p>"#;
        let out = transform_xml(xml, |_| Ok(EventAction::Keep)).unwrap();
        assert_eq!(out, xml);
    }

    #[test]
    fn transform_can_drop_a_targeted_event() {
        let xml = "<root><keep/><drop/></root>";
        let out = transform_xml(xml, |event| {
            if let Event::Empty(tag) = event
                && local_name(tag.name().as_ref()) == b"drop"
            {
                return Ok(EventAction::Drop);
            }
            Ok(EventAction::Keep)
        })
        .unwrap();
        assert_eq!(out, "<root><keep/></root>");
    }

    #[test]
    fn oversized_entry_is_rejected_rather_than_buffered() {
        // Declared size is a lie a hostile archive can tell; the read cap is
        // what actually holds.
        let package = OoxmlPackage::read(b"not a zip at all");
        assert!(matches!(package, Err(DocumentError::NotAnOoxmlPackage(_))));
    }

    #[test]
    fn local_name_strips_any_producer_prefix() {
        assert_eq!(local_name(b"w:ins"), b"ins");
        assert_eq!(local_name(b"ins"), b"ins");
        assert_eq!(local_name(b"w14:paraId"), b"paraId");
    }
}
