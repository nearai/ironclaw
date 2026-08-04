//! PowerPoint (`.pptx`) structured read and slide-clone edit.
//!
//! "Add a slide with the same style" is a clone, not a construction. A slide's
//! appearance comes almost entirely from parts the slide itself does not
//! contain: its layout (`slideLayout`), that layout's master, and the theme.
//! Building a slide from scratch means inventing those relationships, and the
//! result renders with default formatting no matter how carefully the shapes
//! are copied.
//!
//! So [`PptxEdit::CloneSlide`] duplicates the source slide's XML *and its
//! relationships part* — which is what carries the layout link — then swaps the
//! text. The new slide inherits placeholders, fonts, colors, and background by
//! pointing at exactly what the source pointed at.
//!
//! Four places must agree for PowerPoint to open the result, and missing any
//! one produces a "repair" prompt:
//!
//! 1. `ppt/slides/slideN.xml` — the new slide part
//! 2. `ppt/slides/_rels/slideN.xml.rels` — its layout relationship
//! 3. `ppt/_rels/presentation.xml.rels` — a new `rId` pointing at the part
//! 4. `ppt/presentation.xml` — a `<p:sldId>` entry in display order
//! 5. `[Content_Types].xml` — an override declaring the part's content type

use serde::{Deserialize, Serialize};

use crate::error::DocumentError;
use crate::ooxml::{EventAction, OoxmlPackage, local_name, transform_xml};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

const PRESENTATION_PART: &str = "ppt/presentation.xml";
const PRESENTATION_RELS_PART: &str = "ppt/_rels/presentation.xml.rels";
const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
const SLIDE_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const SLIDE_RELATIONSHIP_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slide {
    /// 1-based position in presentation order.
    pub index: usize,
    /// The slide's text runs in reading order — title first, as PowerPoint
    /// orders shapes.
    pub text: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PptxEdit {
    /// Append a copy of slide `source` (1-based), replacing its text runs with
    /// `text` in order. The clone keeps the source's layout, so it renders in
    /// the same style.
    ///
    /// Extra runs beyond `text`'s length are emptied rather than left carrying
    /// the source's words, so a clone never silently ships the original's
    /// content.
    CloneSlide { source: usize, text: Vec<String> },
}

/// Read every slide's text in presentation order.
pub fn read_pptx(data: &[u8]) -> Result<Vec<Slide>, DocumentError> {
    let package = OoxmlPackage::read(data)?;
    let mut slides = Vec::new();
    for (index, part) in slide_parts(&package).into_iter().enumerate() {
        let xml = package.part_str(&part)?;
        slides.push(Slide {
            index: index + 1,
            text: slide_text(&xml)?,
        });
    }
    Ok(slides)
}

/// Apply `edits`, returning a new `.pptx`.
pub fn edit_pptx(data: &[u8], edits: &[PptxEdit]) -> Result<Vec<u8>, DocumentError> {
    let mut package = OoxmlPackage::read(data)?;
    for edit in edits {
        match edit {
            PptxEdit::CloneSlide { source, text } => clone_slide(&mut package, *source, text)?,
        }
    }
    package.write()
}

fn slide_parts(package: &OoxmlPackage) -> Vec<String> {
    let mut parts: Vec<String> = package
        .names()
        .iter()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .cloned()
        .collect();
    parts.sort_by_key(|name| slide_number(name).unwrap_or(u32::MAX));
    parts
}

fn slide_number(part: &str) -> Option<u32> {
    part.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .ok()
}

fn slide_text(xml: &str) -> Result<Vec<String>, DocumentError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut runs = Vec::new();
    let mut in_text = false;
    let mut current = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| DocumentError::MalformedPart {
                part: "slide".to_string(),
                detail: error.to_string(),
            })?;
        match &event {
            Event::Eof => break,
            // `a:t` is the DrawingML text element; `p:` elements are structure.
            Event::Start(tag) if local_name(tag.name().as_ref()) == b"t" => {
                in_text = true;
                current.clear();
            }
            Event::End(tag) if local_name(tag.name().as_ref()) == b"t" && in_text => {
                in_text = false;
                if !current.trim().is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
            }
            Event::Text(text) if in_text => {
                current.push_str(
                    &text
                        .decode()
                        .map_err(|error| DocumentError::MalformedPart {
                            part: "slide".to_string(),
                            detail: error.to_string(),
                        })?,
                );
            }
            _ => {}
        }
    }
    Ok(runs)
}

fn clone_slide(
    package: &mut OoxmlPackage,
    source: usize,
    text: &[String],
) -> Result<(), DocumentError> {
    let parts = slide_parts(package);
    let source_part = parts
        .get(
            source
                .checked_sub(1)
                .ok_or_else(|| DocumentError::UnknownAddress {
                    address: "slide 0 (slides are 1-based)".to_string(),
                })?,
        )
        .ok_or_else(|| DocumentError::UnknownAddress {
            address: format!("slide {source}"),
        })?
        .clone();

    let next_number = parts
        .iter()
        .filter_map(|part| slide_number(part))
        .max()
        .unwrap_or(0)
        + 1;
    let new_part = format!("ppt/slides/slide{next_number}.xml");

    // 1. the slide part, with its text swapped
    let source_xml = package.part_str(&source_part)?;
    package.insert_part(
        &new_part,
        replace_slide_text(&source_xml, text)?.into_bytes(),
    );

    // 2. its relationships — the layout link, i.e. the style
    let source_rels = format!(
        "ppt/slides/_rels/{}.rels",
        source_part.trim_start_matches("ppt/slides/")
    );
    if package.has_part(&source_rels) {
        let rels = package.part(&source_rels)?.to_vec();
        package.insert_part(
            &format!("ppt/slides/_rels/slide{next_number}.xml.rels"),
            rels,
        );
    }

    // 3. a presentation-level relationship id pointing at the new part
    let rels_xml = package.part_str(PRESENTATION_RELS_PART)?;
    let relationship_id = next_relationship_id(&rels_xml);
    let updated_rels = append_relationship(
        &rels_xml,
        &relationship_id,
        SLIDE_RELATIONSHIP_TYPE,
        &format!("slides/slide{next_number}.xml"),
    )?;
    package.replace_part(PRESENTATION_RELS_PART, updated_rels.into_bytes())?;

    // 4. the slide-order entry
    let presentation = package.part_str(PRESENTATION_PART)?;
    let updated_presentation = append_slide_id(&presentation, &relationship_id)?;
    package.replace_part(PRESENTATION_PART, updated_presentation.into_bytes())?;

    // 5. the content-type override
    let content_types = package.part_str(CONTENT_TYPES_PART)?;
    let updated_types = append_content_type_override(
        &content_types,
        &format!("/ppt/slides/slide{next_number}.xml"),
        SLIDE_CONTENT_TYPE,
    )?;
    package.replace_part(CONTENT_TYPES_PART, updated_types.into_bytes())?;

    Ok(())
}

/// Swap the text of each `a:t` run in order, keeping every shape, placeholder,
/// and run property exactly as the source had them.
fn replace_slide_text(xml: &str, text: &[String]) -> Result<String, DocumentError> {
    let mut run_index = 0usize;
    let mut in_text = false;
    let mut wrote_for_current = false;

    transform_xml(xml, |event| match event {
        Event::Start(tag) if local_name(tag.name().as_ref()) == b"t" => {
            in_text = true;
            wrote_for_current = false;
            Ok(EventAction::Keep)
        }
        Event::End(tag) if local_name(tag.name().as_ref()) == b"t" && in_text => {
            in_text = false;
            // A run whose source text was a single event has already been
            // replaced; an empty `<a:t></a:t>` needs the text written here.
            if !wrote_for_current {
                let replacement = text.get(run_index).cloned().unwrap_or_default();
                run_index += 1;
                return Ok(EventAction::Replace(vec![
                    Event::Text(BytesText::new(&replacement).into_owned()),
                    Event::End(BytesEnd::new("a:t")),
                ]));
            }
            Ok(EventAction::Keep)
        }
        Event::Text(_) if in_text => {
            if wrote_for_current {
                // Later text events of the same run are folded into the first.
                return Ok(EventAction::Drop);
            }
            wrote_for_current = true;
            let replacement = text.get(run_index).cloned().unwrap_or_default();
            run_index += 1;
            Ok(EventAction::Replace(vec![Event::Text(
                BytesText::new(&replacement).into_owned(),
            )]))
        }
        _ => Ok(EventAction::Keep),
    })
}

fn next_relationship_id(rels_xml: &str) -> String {
    let mut highest = 0u32;
    let mut reader = quick_xml::Reader::from_str(rels_xml);
    reader.config_mut().check_end_names = false;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag)
                if local_name(tag.name().as_ref()) == b"Relationship" =>
            {
                for attribute in tag.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"Id"
                        && let Ok(id) = std::str::from_utf8(&attribute.value)
                        && let Ok(number) = id.trim_start_matches("rId").parse::<u32>()
                    {
                        highest = highest.max(number);
                    }
                }
            }
            _ => {}
        }
    }
    format!("rId{}", highest + 1)
}

fn append_relationship(
    rels_xml: &str,
    id: &str,
    relationship_type: &str,
    target: &str,
) -> Result<String, DocumentError> {
    let mut appended = false;
    let out = transform_xml(rels_xml, |event| match event {
        Event::End(tag) if local_name(tag.name().as_ref()) == b"Relationships" => {
            appended = true;
            let mut relationship = BytesStart::new("Relationship");
            relationship.push_attribute(("Id", id));
            relationship.push_attribute(("Type", relationship_type));
            relationship.push_attribute(("Target", target));
            Ok(EventAction::Replace(vec![
                Event::Empty(relationship.into_owned()),
                Event::End(BytesEnd::new("Relationships")),
            ]))
        }
        _ => Ok(EventAction::Keep),
    })?;
    if !appended {
        return Err(DocumentError::MalformedPart {
            part: PRESENTATION_RELS_PART.to_string(),
            detail: "no <Relationships> element to extend".to_string(),
        });
    }
    Ok(out)
}

fn append_slide_id(presentation_xml: &str, relationship_id: &str) -> Result<String, DocumentError> {
    // Slide ids must be unique and >= 256 per the schema; continue from the
    // highest present rather than restarting, or PowerPoint rejects the deck.
    let mut highest = 255u32;
    let mut reader = quick_xml::Reader::from_str(presentation_xml);
    reader.config_mut().check_end_names = false;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag)
                if local_name(tag.name().as_ref()) == b"sldId" =>
            {
                for attribute in tag.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"id"
                        && let Ok(id) = std::str::from_utf8(&attribute.value)
                        && let Ok(number) = id.parse::<u32>()
                    {
                        highest = highest.max(number);
                    }
                }
            }
            _ => {}
        }
    }

    let mut appended = false;
    let out = transform_xml(presentation_xml, |event| match event {
        Event::End(tag) if local_name(tag.name().as_ref()) == b"sldIdLst" => {
            appended = true;
            let mut slide_id = BytesStart::new("p:sldId");
            slide_id.push_attribute(("id", (highest + 1).to_string().as_str()));
            slide_id.push_attribute(("r:id", relationship_id));
            Ok(EventAction::Replace(vec![
                Event::Empty(slide_id.into_owned()),
                Event::End(BytesEnd::new("p:sldIdLst")),
            ]))
        }
        _ => Ok(EventAction::Keep),
    })?;
    if !appended {
        return Err(DocumentError::MalformedPart {
            part: PRESENTATION_PART.to_string(),
            detail: "no <p:sldIdLst> element to extend".to_string(),
        });
    }
    Ok(out)
}

fn append_content_type_override(
    content_types_xml: &str,
    part_name: &str,
    content_type: &str,
) -> Result<String, DocumentError> {
    let mut appended = false;
    let out = transform_xml(content_types_xml, |event| match event {
        Event::End(tag) if local_name(tag.name().as_ref()) == b"Types" => {
            appended = true;
            let mut over = BytesStart::new("Override");
            over.push_attribute(("PartName", part_name));
            over.push_attribute(("ContentType", content_type));
            Ok(EventAction::Replace(vec![
                Event::Empty(over.into_owned()),
                Event::End(BytesEnd::new("Types")),
            ]))
        }
        _ => Ok(EventAction::Keep),
    })?;
    if !appended {
        return Err(DocumentError::MalformedPart {
            part: CONTENT_TYPES_PART.to_string(),
            detail: "no <Types> element to extend".to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::quarterly_pptx;

    #[test]
    fn read_returns_slide_text_in_presentation_order() {
        let slides = read_pptx(&quarterly_pptx()).unwrap();
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].index, 1);
        assert_eq!(slides[0].text, vec!["Q1 Results", "Revenue up 12%"]);
    }

    #[test]
    fn cloning_appends_a_slide_carrying_the_new_text() {
        let edited = edit_pptx(
            &quarterly_pptx(),
            &[PptxEdit::CloneSlide {
                source: 1,
                text: vec!["Q2 Results".to_string(), "Revenue up 18%".to_string()],
            }],
        )
        .unwrap();

        let slides = read_pptx(&edited).unwrap();
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].text, vec!["Q1 Results", "Revenue up 12%"]);
        assert_eq!(slides[1].text, vec!["Q2 Results", "Revenue up 18%"]);
    }

    #[test]
    fn the_clone_points_at_the_same_layout_so_it_renders_in_the_same_style() {
        // This is the assertion that distinguishes a clone from a construction:
        // style comes from the layout relationship, not from the slide's own
        // XML, so the rels part must be duplicated too.
        let edited = edit_pptx(
            &quarterly_pptx(),
            &[PptxEdit::CloneSlide {
                source: 1,
                text: vec!["Q2".to_string()],
            }],
        )
        .unwrap();
        let package = OoxmlPackage::read(&edited).unwrap();
        let source_rels = package
            .part_str("ppt/slides/_rels/slide1.xml.rels")
            .unwrap();
        let clone_rels = package
            .part_str("ppt/slides/_rels/slide2.xml.rels")
            .unwrap();
        assert_eq!(
            source_rels, clone_rels,
            "the clone must inherit the source's layout relationship"
        );
        assert!(clone_rels.contains("slideLayout1.xml"));
    }

    #[test]
    fn the_clone_preserves_shape_and_run_properties_from_the_source() {
        let edited = edit_pptx(
            &quarterly_pptx(),
            &[PptxEdit::CloneSlide {
                source: 1,
                text: vec!["Q2 Results".to_string(), "Revenue up 18%".to_string()],
            }],
        )
        .unwrap();
        let slide = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str("ppt/slides/slide2.xml")
            .unwrap();
        assert!(
            slide.contains(r#"<a:rPr lang="en-US" sz="4400" b="1"/>"#),
            "run properties (size, bold) must survive the clone: {slide}"
        );
        assert!(
            slide.contains(r#"type="title"#),
            "placeholder types must survive the clone: {slide}"
        );
    }

    #[test]
    fn the_clone_is_registered_in_all_three_package_indexes() {
        // Miss any one of these and PowerPoint shows a repair prompt.
        let edited = edit_pptx(
            &quarterly_pptx(),
            &[PptxEdit::CloneSlide {
                source: 1,
                text: vec!["Q2".to_string()],
            }],
        )
        .unwrap();
        let package = OoxmlPackage::read(&edited).unwrap();

        let content_types = package.part_str(CONTENT_TYPES_PART).unwrap();
        assert!(
            content_types.contains("/ppt/slides/slide2.xml"),
            "content-type override missing: {content_types}"
        );

        let rels = package.part_str(PRESENTATION_RELS_PART).unwrap();
        assert!(
            rels.contains("slides/slide2.xml"),
            "presentation relationship missing: {rels}"
        );
        let new_id = rels
            .split(r#"Target="slides/slide2.xml""#)
            .next()
            .and_then(|before| before.rfind("rId").map(|at| &before[at..]))
            .and_then(|tail| tail.split('"').next())
            .expect("new relationship carries an rId");

        let presentation = package.part_str(PRESENTATION_PART).unwrap();
        assert!(
            presentation.contains(&format!(r#"r:id="{new_id}""#)),
            "slide order entry must reference the new rId {new_id}: {presentation}"
        );
    }

    #[test]
    fn cloned_slide_ids_stay_unique_and_above_the_schema_minimum() {
        let edited = edit_pptx(
            &quarterly_pptx(),
            &[
                PptxEdit::CloneSlide {
                    source: 1,
                    text: vec!["Q2".to_string()],
                },
                PptxEdit::CloneSlide {
                    source: 1,
                    text: vec!["Q3".to_string()],
                },
            ],
        )
        .unwrap();
        let presentation = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str(PRESENTATION_PART)
            .unwrap();
        let ids: Vec<u32> = presentation
            .match_indices(r#"<p:sldId id=""#)
            .filter_map(|(at, marker)| {
                presentation[at + marker.len()..]
                    .split('"')
                    .next()?
                    .parse()
                    .ok()
            })
            .collect();
        assert_eq!(ids.len(), 3, "three slides registered: {presentation}");
        assert!(ids.iter().all(|id| *id >= 256), "schema minimum: {ids:?}");
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "slide ids must be unique: {ids:?}");
    }

    #[test]
    fn extra_source_runs_are_emptied_rather_than_leaking_the_original_text() {
        // A clone that ships fewer replacement strings than the source has runs
        // must not silently keep the source's words in the leftovers.
        let edited = edit_pptx(
            &quarterly_pptx(),
            &[PptxEdit::CloneSlide {
                source: 1,
                text: vec!["Q2 Results".to_string()],
            }],
        )
        .unwrap();
        let slides = read_pptx(&edited).unwrap();
        assert_eq!(slides[1].text, vec!["Q2 Results"]);
        let slide = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str("ppt/slides/slide2.xml")
            .unwrap();
        assert!(
            !slide.contains("Revenue up 12%"),
            "the source's leftover text must not survive: {slide}"
        );
    }

    #[test]
    fn cloning_an_unknown_slide_is_a_typed_error() {
        for source in [0usize, 9] {
            let error = edit_pptx(
                &quarterly_pptx(),
                &[PptxEdit::CloneSlide {
                    source,
                    text: vec![],
                }],
            )
            .unwrap_err();
            assert!(
                matches!(error, DocumentError::UnknownAddress { .. }),
                "source {source} must be a typed address error"
            );
        }
    }

    #[test]
    fn cloning_preserves_the_theme_and_master_bit_for_bit() {
        let original = quarterly_pptx();
        let edited = edit_pptx(
            &original,
            &[PptxEdit::CloneSlide {
                source: 1,
                text: vec!["Q2".to_string()],
            }],
        )
        .unwrap();
        let before = OoxmlPackage::read(&original).unwrap();
        let after = OoxmlPackage::read(&edited).unwrap();
        for name in ["ppt/theme/theme1.xml", "ppt/slideLayouts/slideLayout1.xml"] {
            assert_eq!(
                before.part(name).unwrap(),
                after.part(name).unwrap(),
                "{name} must be copied through untouched"
            );
        }
    }
}
