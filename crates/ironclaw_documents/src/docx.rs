//! Word (`.docx`) structured read and revision-aware edit.
//!
//! Word stores tracked changes as `w:ins` / `w:del` elements wrapping runs
//! (`w:r`). A deleted run's text lives in `w:delText`, not `w:t`, which is why
//! flat tag-stripping (`ironclaw_extractors::extract_document`) shows deleted
//! text as if it were still part of the document — the redline is invisible to
//! the model, and that is what this module fixes on the read side.
//!
//! On the write side, "accept" and "reject" are the two operations that
//! actually resolve a redline:
//!
//! | | `w:ins` (proposed addition) | `w:del` (proposed deletion) |
//! |---|---|---|
//! | accept | unwrap: keep the runs, drop the marker | drop the runs entirely |
//! | reject | drop the runs entirely | unwrap, and turn `w:delText` back into `w:t` |

use serde::{Deserialize, Serialize};

use crate::error::DocumentError;
use crate::ooxml::{EventAction, OoxmlPackage, local_name, transform_xml};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

const DOCUMENT_PART: &str = "word/document.xml";

/// One paragraph of a Word document, addressed by a stable id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paragraph {
    /// `p1`, `p2`, … in document order. Stable for a given document version;
    /// a caller that edits and re-reads must use the new read's ids.
    pub id: String,
    /// The text as the document currently reads with revisions *unresolved*:
    /// insertions included, deletions excluded — i.e. what you would see with
    /// "show final" in Word.
    pub text: String,
    /// Tracked changes carried by this paragraph, in document order. Empty for
    /// a clean paragraph.
    pub revisions: Vec<Revision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub kind: RevisionKind,
    /// The text the revision covers — the proposed addition for `Inserted`,
    /// the text proposed for removal for `Deleted`.
    pub text: String,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionKind {
    Inserted,
    Deleted,
}

/// A typed edit against a Word document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DocxEdit {
    /// Resolve every tracked change in one paragraph.
    ResolveRevisions {
        paragraph: String,
        disposition: RevisionDisposition,
    },
    /// Resolve every tracked change in the whole document.
    ResolveAllRevisions { disposition: RevisionDisposition },
    /// Replace a paragraph's visible text, preserving the paragraph's
    /// properties (`w:pPr`) and the formatting of its first run.
    ReplaceParagraphText { paragraph: String, text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionDisposition {
    Accept,
    Reject,
}

/// Read every paragraph of a `.docx`, with its tracked changes surfaced.
pub fn read_docx(data: &[u8]) -> Result<Vec<Paragraph>, DocumentError> {
    let package = OoxmlPackage::read(data)?;
    let xml = package.part_str(DOCUMENT_PART)?;
    parse_paragraphs(&xml)
}

/// Apply `edits` in order, returning a new `.docx`.
///
/// Every part except `word/document.xml` is copied through byte-for-byte, and
/// within that part every element the edits do not target is re-emitted
/// exactly as read.
pub fn edit_docx(data: &[u8], edits: &[DocxEdit]) -> Result<Vec<u8>, DocumentError> {
    let mut package = OoxmlPackage::read(data)?;
    let mut xml = package.part_str(DOCUMENT_PART)?;
    for edit in edits {
        xml = apply_edit(&xml, edit)?;
    }
    package.replace_part(DOCUMENT_PART, xml.into_bytes())?;
    package.write()
}

// --- read ------------------------------------------------------------------

/// Where the walker currently is, so text events can be attributed correctly.
#[derive(Default)]
struct RevisionState {
    in_insert: Option<Option<String>>,
    in_delete: Option<Option<String>>,
}

fn parse_paragraphs(xml: &str) -> Result<Vec<Paragraph>, DocumentError> {
    // Word nests `w:p` inside `w:p` (a text box lives in a run of its parent
    // paragraph), so a single `current` slot loses the outer paragraph. Ids are
    // assigned in Start order and shared with the write path, which counts the
    // same way — that agreement is what makes an id safe to edit against.
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut open: Vec<Paragraph> = Vec::new();
    let mut paragraph_count = 0usize;
    let mut state = RevisionState::default();
    // Word splits one sentence across many runs (spellcheck state, rsid churn,
    // formatting). Consecutive events of the same revision status are merged so
    // a caller sees one revision per contiguous span, not one per run.
    let mut pending: Option<(RevisionKind, String, Option<String>)> = None;

    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| DocumentError::MalformedPart {
                part: DOCUMENT_PART.to_string(),
                detail: error.to_string(),
            })?;
        match &event {
            Event::Eof => break,
            Event::Start(tag) => match local_name(tag.name().as_ref()) {
                b"p" => {
                    if let Some(parent) = open.last_mut() {
                        flush_pending(&mut pending, parent);
                    }
                    paragraph_count += 1;
                    open.push(Paragraph {
                        id: format!("p{paragraph_count}"),
                        text: String::new(),
                        revisions: Vec::new(),
                    });
                }
                b"ins" => state.in_insert = Some(author_of(tag)),
                b"del" => state.in_delete = Some(author_of(tag)),
                _ => {}
            },
            Event::End(tag) => match local_name(tag.name().as_ref()) {
                b"p" => {
                    if let Some(mut paragraph) = open.pop() {
                        flush_pending(&mut pending, &mut paragraph);
                        paragraphs.push(paragraph);
                    }
                }
                b"ins" => {
                    if let Some(paragraph) = open.last_mut() {
                        flush_pending(&mut pending, paragraph);
                    }
                    state.in_insert = None;
                }
                b"del" => {
                    if let Some(paragraph) = open.last_mut() {
                        flush_pending(&mut pending, paragraph);
                    }
                    state.in_delete = None;
                }
                _ => {}
            },
            Event::Text(text) if !open.is_empty() => {
                let decoded = text
                    .decode()
                    .map_err(|error| DocumentError::MalformedPart {
                        part: DOCUMENT_PART.to_string(),
                        detail: error.to_string(),
                    })?;
                absorb_text(&decoded, &state, &mut pending, open.last_mut());
            }
            _ => {}
        }
    }
    // Completion order puts an inner paragraph before its parent; sort by the
    // Start-order id so callers see document order.
    paragraphs.sort_by_key(|paragraph| {
        paragraph
            .id
            .trim_start_matches('p')
            .parse::<usize>()
            .unwrap_or(usize::MAX)
    });
    Ok(paragraphs)
}

fn absorb_text(
    text: &str,
    state: &RevisionState,
    pending: &mut Option<(RevisionKind, String, Option<String>)>,
    paragraph: Option<&mut Paragraph>,
) {
    let Some(paragraph) = paragraph else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let classified = if let Some(author) = &state.in_delete {
        Some((RevisionKind::Deleted, author.clone()))
    } else {
        state
            .in_insert
            .as_ref()
            .map(|author| (RevisionKind::Inserted, author.clone()))
    };

    match classified {
        // Deleted text is NOT part of the visible "final" text; inserted text
        // is. That asymmetry is the whole point of showing revisions.
        Some((kind, author)) => {
            if kind == RevisionKind::Inserted {
                paragraph.text.push_str(text);
            }
            match pending {
                Some((existing_kind, buffer, _)) if *existing_kind == kind => {
                    buffer.push_str(text);
                }
                _ => {
                    flush_pending(pending, paragraph);
                    *pending = Some((kind, text.to_string(), author));
                }
            }
        }
        None => {
            flush_pending(pending, paragraph);
            paragraph.text.push_str(text);
        }
    }
}

fn flush_pending(
    pending: &mut Option<(RevisionKind, String, Option<String>)>,
    paragraph: &mut Paragraph,
) {
    if let Some((kind, text, author)) = pending.take() {
        paragraph.revisions.push(Revision { kind, text, author });
    }
}

fn author_of(tag: &BytesStart<'_>) -> Option<String> {
    tag.attributes().flatten().find_map(|attribute| {
        (local_name(attribute.key.as_ref()) == b"author")
            .then(|| String::from_utf8_lossy(&attribute.value).into_owned())
    })
}

// --- write -----------------------------------------------------------------

fn apply_edit(xml: &str, edit: &DocxEdit) -> Result<String, DocumentError> {
    match edit {
        DocxEdit::ResolveAllRevisions { disposition } => resolve_revisions(xml, None, *disposition),
        DocxEdit::ResolveRevisions {
            paragraph,
            disposition,
        } => resolve_revisions(xml, Some(paragraph.as_str()), *disposition),
        DocxEdit::ReplaceParagraphText { paragraph, text } => {
            replace_paragraph_text(xml, paragraph, text)
        }
    }
}

/// Accept or reject tracked changes, either in one paragraph or document-wide.
fn resolve_revisions(
    xml: &str,
    only: Option<&str>,
    disposition: RevisionDisposition,
) -> Result<String, DocumentError> {
    let mut paragraph_index = 0usize;
    // A stack, not a flag: a nested `w:p` closing must restore the ENCLOSING
    // paragraph's target state rather than clearing it.
    let mut target_stack: Vec<bool> = Vec::new();
    let mut in_target = only.is_none();
    // Depth of the run subtree being discarded, so nested elements inside a
    // dropped run go with it instead of leaking through.
    let mut dropping: usize = 0;
    let mut in_insert = false;
    let mut in_delete = false;
    let mut resolved_any = false;

    let out = transform_xml(xml, |event| {
        // A discarded subtree swallows everything until it closes.
        if dropping > 0 {
            match event {
                Event::Start(_) => dropping += 1,
                Event::End(_) => dropping -= 1,
                _ => {}
            }
            return Ok(EventAction::Drop);
        }

        match event {
            Event::Start(tag) => match local_name(tag.name().as_ref()) {
                b"p" => {
                    paragraph_index += 1;
                    target_stack.push(in_target);
                    if let Some(target) = only {
                        in_target = format!("p{paragraph_index}") == target;
                    }
                    Ok(EventAction::Keep)
                }
                b"ins" if in_target => {
                    resolved_any = true;
                    match disposition {
                        // Accept an insertion: unwrap it. The marker's End must
                        // also be dropped, so the flag records that we are
                        // inside an unwrapped element.
                        RevisionDisposition::Accept => {
                            in_insert = true;
                            Ok(EventAction::Drop)
                        }
                        // Reject an insertion: the proposed text never existed.
                        // The whole subtree goes, INCLUDING its End, which the
                        // dropping branch consumes — so the flag must stay
                        // false or it would survive to eat a later element's
                        // closing tag and emit unbalanced XML.
                        RevisionDisposition::Reject => {
                            dropping = 1;
                            Ok(EventAction::Drop)
                        }
                    }
                }
                b"del" if in_target => {
                    resolved_any = true;
                    match disposition {
                        // Accept a deletion: the text goes away with it. The
                        // dropping branch consumes the End, so no flag (see the
                        // `ins` reject arm above).
                        RevisionDisposition::Accept => {
                            dropping = 1;
                            Ok(EventAction::Drop)
                        }
                        // Reject a deletion: unwrap it, keeping the runs.
                        // `w:delText` must become `w:t` again or Word shows
                        // nothing — the trap this branch exists to avoid.
                        RevisionDisposition::Reject => {
                            in_delete = true;
                            Ok(EventAction::Drop)
                        }
                    }
                }
                b"delText" if in_delete && disposition == RevisionDisposition::Reject => Ok(
                    EventAction::Replace(vec![Event::Start(BytesStart::new("w:t"))]),
                ),
                _ => Ok(EventAction::Keep),
            },
            Event::End(tag) => match local_name(tag.name().as_ref()) {
                b"p" => {
                    in_target = target_stack.pop().unwrap_or(only.is_none());
                    Ok(EventAction::Keep)
                }
                b"ins" if in_insert => {
                    in_insert = false;
                    Ok(EventAction::Drop)
                }
                b"del" if in_delete => {
                    in_delete = false;
                    Ok(EventAction::Drop)
                }
                b"delText" if in_delete && disposition == RevisionDisposition::Reject => {
                    Ok(EventAction::Replace(vec![Event::End(BytesEnd::new("w:t"))]))
                }
                _ => Ok(EventAction::Keep),
            },
            _ => Ok(EventAction::Keep),
        }
    })?;

    if let Some(target) = only
        && !resolved_any
    {
        return Err(DocumentError::InapplicableEdit {
            address: target.to_string(),
            detail: "paragraph carries no tracked changes".to_string(),
        });
    }
    Ok(out)
}

/// Replace a paragraph's text, keeping `w:pPr` and the first run's `w:rPr`.
fn replace_paragraph_text(
    xml: &str,
    paragraph: &str,
    replacement: &str,
) -> Result<String, DocumentError> {
    let mut paragraph_index = 0usize;
    let mut target_stack: Vec<bool> = Vec::new();
    let mut in_target = false;
    let mut wrote_replacement = false;
    let mut found = false;
    // Depth tracking so the original runs are removed but `w:pPr` survives —
    // dropping the whole paragraph body would lose alignment, numbering and
    // style, which is exactly the silent formatting loss this crate exists to
    // prevent.
    let mut dropping: usize = 0;

    let out = transform_xml(xml, |event| {
        if dropping > 0 {
            match event {
                Event::Start(_) => dropping += 1,
                Event::End(_) => dropping -= 1,
                _ => {}
            }
            return Ok(EventAction::Drop);
        }
        match event {
            Event::Start(tag) => {
                let qname = tag.name();
                let name = local_name(qname.as_ref());
                if name == b"p" {
                    paragraph_index += 1;
                    target_stack.push(in_target);
                    in_target = format!("p{paragraph_index}") == paragraph;
                    if in_target {
                        found = true;
                        wrote_replacement = false;
                    }
                    return Ok(EventAction::Keep);
                }
                if in_target && name == b"r" {
                    dropping = 1;
                    if wrote_replacement {
                        return Ok(EventAction::Drop);
                    }
                    wrote_replacement = true;
                    // One fresh run carrying the replacement text. `xml:space`
                    // is preserved so leading/trailing spaces are not eaten.
                    let mut text_tag = BytesStart::new("w:t");
                    text_tag.push_attribute(("xml:space", "preserve"));
                    return Ok(EventAction::Replace(vec![
                        Event::Start(BytesStart::new("w:r")),
                        Event::Start(text_tag),
                        Event::Text(BytesText::new(replacement).into_owned()),
                        Event::End(BytesEnd::new("w:t")),
                        Event::End(BytesEnd::new("w:r")),
                    ]));
                }
                Ok(EventAction::Keep)
            }
            Event::End(tag) => {
                if local_name(tag.name().as_ref()) == b"p" {
                    in_target = target_stack.pop().unwrap_or(false);
                }
                Ok(EventAction::Keep)
            }
            _ => Ok(EventAction::Keep),
        }
    })?;

    if !found {
        return Err(DocumentError::UnknownAddress {
            address: paragraph.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{docx_with_body, redlined_docx};

    #[test]
    fn read_surfaces_insertions_and_deletions_with_authors() {
        let paragraphs = read_docx(&redlined_docx()).unwrap();
        let redlined = paragraphs
            .iter()
            .find(|paragraph| !paragraph.revisions.is_empty())
            .expect("fixture carries a tracked change");

        let kinds: Vec<_> = redlined.revisions.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&RevisionKind::Inserted));
        assert!(kinds.contains(&RevisionKind::Deleted));
        assert_eq!(
            redlined.revisions[0].author.as_deref(),
            Some("Reviewer"),
            "author attribution must survive the read"
        );
    }

    #[test]
    fn read_excludes_deleted_text_from_the_visible_paragraph_text() {
        // The bug flat extraction has: deleted text reads as if it were still
        // in the document, so a model "reviewing" it sees the wrong contract.
        let paragraphs = read_docx(&redlined_docx()).unwrap();
        let redlined = paragraphs
            .iter()
            .find(|paragraph| !paragraph.revisions.is_empty())
            .unwrap();
        assert!(
            redlined.text.contains("thirty"),
            "the inserted replacement is part of the final text"
        );
        assert!(
            !redlined.text.contains("sixty"),
            "deleted text must not appear in the final text, got {:?}",
            redlined.text
        );
    }

    #[test]
    fn accepting_revisions_keeps_insertions_and_removes_deletions() {
        let edited = edit_docx(
            &redlined_docx(),
            &[DocxEdit::ResolveAllRevisions {
                disposition: RevisionDisposition::Accept,
            }],
        )
        .unwrap();

        let paragraphs = read_docx(&edited).unwrap();
        assert!(
            paragraphs.iter().all(|p| p.revisions.is_empty()),
            "accepting must leave a clean document"
        );
        let text: String = paragraphs.iter().map(|p| p.text.as_str()).collect();
        assert!(text.contains("thirty"), "accepted insertion survives");
        assert!(!text.contains("sixty"), "accepted deletion is gone");
    }

    #[test]
    fn rejecting_revisions_restores_the_original_wording() {
        let edited = edit_docx(
            &redlined_docx(),
            &[DocxEdit::ResolveAllRevisions {
                disposition: RevisionDisposition::Reject,
            }],
        )
        .unwrap();

        let paragraphs = read_docx(&edited).unwrap();
        assert!(paragraphs.iter().all(|p| p.revisions.is_empty()));
        let text: String = paragraphs.iter().map(|p| p.text.as_str()).collect();
        // The `w:delText` -> `w:t` conversion is what makes this pass; without
        // it Word renders the restored run as empty.
        assert!(
            text.contains("sixty"),
            "rejecting a deletion must restore its text, got {text:?}"
        );
        assert!(!text.contains("thirty"), "rejected insertion is gone");
    }

    #[test]
    fn revisions_split_across_runs_merge_into_one_span() {
        // Word splits a sentence across runs arbitrarily; a caller must see one
        // revision per contiguous span, not one per run.
        let body = r#"<w:p><w:ins w:id="1" w:author="R"><w:r><w:t>thir</w:t></w:r><w:r><w:t>ty days</w:t></w:r></w:ins></w:p>"#;
        let paragraphs = read_docx(&docx_with_body(body)).unwrap();
        assert_eq!(paragraphs[0].revisions.len(), 1, "runs must coalesce");
        assert_eq!(paragraphs[0].revisions[0].text, "thirty days");
    }

    #[test]
    fn resolving_a_paragraph_leaves_other_paragraphs_untouched() {
        let body = concat!(
            r#"<w:p><w:r><w:t>clean one</w:t></w:r></w:p>"#,
            r#"<w:p><w:ins w:id="1" w:author="R"><w:r><w:t>added</w:t></w:r></w:ins></w:p>"#,
        );
        let edited = edit_docx(
            &docx_with_body(body),
            &[DocxEdit::ResolveRevisions {
                paragraph: "p2".to_string(),
                disposition: RevisionDisposition::Accept,
            }],
        )
        .unwrap();
        let paragraphs = read_docx(&edited).unwrap();
        assert_eq!(paragraphs[0].text, "clean one");
        assert_eq!(paragraphs[1].text, "added");
        assert!(paragraphs[1].revisions.is_empty());
    }

    #[test]
    fn resolving_a_paragraph_without_revisions_is_a_typed_error() {
        let body = r#"<w:p><w:r><w:t>clean</w:t></w:r></w:p>"#;
        let error = edit_docx(
            &docx_with_body(body),
            &[DocxEdit::ResolveRevisions {
                paragraph: "p1".to_string(),
                disposition: RevisionDisposition::Accept,
            }],
        )
        .unwrap_err();
        assert!(matches!(error, DocumentError::InapplicableEdit { .. }));
    }

    #[test]
    fn replacing_paragraph_text_preserves_paragraph_properties() {
        let body = concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr>"#,
            r#"<w:r><w:rPr><w:b/></w:rPr><w:t>old</w:t></w:r>"#,
            r#"<w:r><w:t> more</w:t></w:r></w:p>"#,
        );
        let edited = edit_docx(
            &docx_with_body(body),
            &[DocxEdit::ReplaceParagraphText {
                paragraph: "p1".to_string(),
                text: "new text".to_string(),
            }],
        )
        .unwrap();
        let package = OoxmlPackage::read(&edited).unwrap();
        let xml = package.part_str(DOCUMENT_PART).unwrap();
        assert!(
            xml.contains(r#"<w:pStyle w:val="Heading1"/>"#),
            "paragraph style must survive a text replacement: {xml}"
        );
        assert_eq!(read_docx(&edited).unwrap()[0].text, "new text");
    }

    #[test]
    fn replacing_an_unknown_paragraph_is_a_typed_error() {
        let error = edit_docx(
            &docx_with_body(r#"<w:p><w:r><w:t>a</w:t></w:r></w:p>"#),
            &[DocxEdit::ReplaceParagraphText {
                paragraph: "p99".to_string(),
                text: "x".to_string(),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, DocumentError::UnknownAddress { .. }));
    }

    #[test]
    fn editing_preserves_every_unrelated_part_bit_for_bit() {
        // The core promise of the crate: styles/media/numbering are copied,
        // never regenerated.
        let original = redlined_docx();
        let edited = edit_docx(
            &original,
            &[DocxEdit::ResolveAllRevisions {
                disposition: RevisionDisposition::Accept,
            }],
        )
        .unwrap();

        let before = OoxmlPackage::read(&original).unwrap();
        let after = OoxmlPackage::read(&edited).unwrap();
        for name in before.names() {
            if name == DOCUMENT_PART {
                continue;
            }
            assert_eq!(
                before.part(name).unwrap(),
                after.part(name).unwrap(),
                "part {name} must be copied through untouched"
            );
        }
    }

    #[test]
    fn an_edit_list_with_no_edits_round_trips_to_identical_bytes() {
        let original = redlined_docx();
        let untouched = edit_docx(&original, &[]).unwrap();
        assert_eq!(
            OoxmlPackage::read(&original).unwrap().write().unwrap(),
            untouched,
            "a no-op edit must be byte-stable"
        );
    }
}

#[cfg(test)]
mod review_regressions {
    use super::*;
    use crate::test_fixtures::docx_with_body;

    /// Word nests `w:p` inside table cells and text boxes. If the reader loses
    /// the outer paragraph while the writer counts every `w:p`, the ids the
    /// caller reads no longer address the paragraphs the writer edits — so an
    /// edit silently lands on the wrong paragraph.
    #[test]
    fn nested_paragraphs_keep_read_and_write_ids_in_agreement() {
        let body = concat!(
            r#"<w:p><w:r><w:t>first</w:t></w:r></w:p>"#,
            r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>in cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
            r#"<w:p><w:r><w:t>last</w:t></w:r></w:p>"#,
        );
        let docx = docx_with_body(body);
        let paragraphs = read_docx(&docx).unwrap();
        let texts: Vec<_> = paragraphs.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["first", "in cell", "last"],
            "every paragraph, nested or not, must be readable"
        );

        // Whatever ids the read hands out, editing one must change exactly that
        // paragraph's text and nothing else.
        for target in &paragraphs {
            let edited = edit_docx(
                &docx,
                &[DocxEdit::ReplaceParagraphText {
                    paragraph: target.id.clone(),
                    text: "REPLACED".to_string(),
                }],
            )
            .unwrap();
            let after = read_docx(&edited).unwrap();
            let changed: Vec<_> = after
                .iter()
                .filter(|p| p.text == "REPLACED")
                .map(|p| p.id.as_str())
                .collect();
            assert_eq!(
                changed,
                vec![target.id.as_str()],
                "editing {} must change only {}; got {:?}",
                target.id,
                target.id,
                after
            );
        }
    }

    /// A text box genuinely nests `w:p` INSIDE `w:p` (unlike a table, whose
    /// cell paragraphs are siblings in document order). The reader must not
    /// lose the outer paragraph, and the ids it hands out must address the same
    /// paragraphs the writer counts — otherwise an edit lands on the wrong one.
    #[test]
    fn a_text_box_nests_paragraphs_without_desynchronising_read_and_write_ids() {
        let body = concat!(
            r#"<w:p><w:r><w:t>outer start</w:t></w:r>"#,
            r#"<w:r><w:txbxContent><w:p><w:r><w:t>inside box</w:t></w:r></w:p></w:txbxContent></w:r>"#,
            r#"<w:r><w:t> outer end</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>after</w:t></w:r></w:p>"#,
        );
        let docx = docx_with_body(body);
        let paragraphs = read_docx(&docx).unwrap();
        assert!(
            paragraphs.iter().any(|p| p.text.contains("outer start")),
            "the outer paragraph must survive a nested one: {paragraphs:?}"
        );
        assert!(
            paragraphs.iter().any(|p| p.text.contains("after")),
            "the paragraph after the nest must still be read: {paragraphs:?}"
        );

        for target in &paragraphs {
            let edited = edit_docx(
                &docx,
                &[DocxEdit::ReplaceParagraphText {
                    paragraph: target.id.clone(),
                    text: "REPLACED".to_string(),
                }],
            )
            .unwrap();
            let changed: Vec<String> = read_docx(&edited)
                .unwrap()
                .into_iter()
                .filter(|p| p.text.contains("REPLACED"))
                .map(|p| p.id)
                .collect();
            assert_eq!(
                changed,
                vec![target.id.clone()],
                "editing {} must change only {}",
                target.id,
                target.id
            );
        }
    }

    /// Resolving revisions in ONE paragraph must not disturb another's markup.
    /// The dropped-subtree branch swallows the matching `End`, so an
    /// `in_insert`/`in_delete` flag can stay set for the rest of the document
    /// and delete a later paragraph's closing tag — emitting XML Word rejects.
    #[test]
    fn resolving_one_paragraph_leaves_later_revision_markup_balanced() {
        let body = concat!(
            r#"<w:p><w:ins w:id="1" w:author="R"><w:r><w:t>alpha</w:t></w:r></w:ins></w:p>"#,
            r#"<w:p><w:ins w:id="2" w:author="R"><w:r><w:t>beta</w:t></w:r></w:ins></w:p>"#,
        );
        let edited = edit_docx(
            &docx_with_body(body),
            &[DocxEdit::ResolveRevisions {
                paragraph: "p1".to_string(),
                disposition: RevisionDisposition::Reject,
            }],
        )
        .unwrap();

        let xml = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str(DOCUMENT_PART)
            .unwrap();
        assert_eq!(
            xml.matches("<w:ins").count(),
            xml.matches("</w:ins>").count(),
            "every remaining w:ins must still be closed: {xml}"
        );

        let after = read_docx(&edited).unwrap();
        assert_eq!(after[0].text, "", "p1's rejected insertion is gone");
        assert_eq!(
            after[1].text, "beta",
            "p2 must be untouched, got {:?}",
            after[1]
        );
        assert_eq!(
            after[1].revisions.len(),
            1,
            "p2 keeps its unresolved revision"
        );
    }
}
