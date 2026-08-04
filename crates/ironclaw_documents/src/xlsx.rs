//! Excel (`.xlsx`) structured read and cell/formula edit.
//!
//! Three details make spreadsheet editing different from documents, and each
//! one is a silent-corruption trap if skipped:
//!
//! 1. **Text lives out of line.** A cell with `t="s"` stores an *index* into
//!    `xl/sharedStrings.xml`, not text. Reading without resolving that gives
//!    you `"3"` where the sheet says `"Total"` — so column headers, the thing
//!    a caller navigates by, are unreadable.
//! 2. **Cached values go stale.** `<c><f>SUM(A1:A3)</f><v>6</v></c>` — the
//!    `<v>` is the last value Excel computed. Writing a new `<f>` without
//!    dropping `<v>` leaves the old number on screen until a manual recalc.
//!    Setting `fullCalcOnLoad` makes Excel recompute on open.
//! 3. **Cells are order-sensitive.** Within a `<row>`, `<c>` elements must be
//!    in ascending column order. Appending a new cell blindly produces a file
//!    Excel repairs (and silently drops content from).

use serde::{Deserialize, Serialize};

use crate::error::DocumentError;
use crate::ooxml::{EventAction, OoxmlPackage, local_name, transform_xml};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

const WORKBOOK_PART: &str = "xl/workbook.xml";
const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sheet {
    pub name: String,
    /// Non-empty cells in document order.
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// `A1`-style reference.
    pub reference: String,
    /// The cell's text as displayed — shared strings already resolved.
    pub value: Option<String>,
    /// The formula source without its leading `=`, when the cell has one.
    pub formula: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum XlsxEdit {
    /// Set (or replace) a cell's formula. Accepts the formula with or without
    /// a leading `=`; the cached value is dropped so Excel recomputes.
    SetCellFormula {
        sheet: String,
        cell: String,
        formula: String,
    },
}

impl Sheet {
    /// Resolve "the cell in `column_header`'s column, on `row`".
    ///
    /// This is the navigation a caller actually performs — "put a total under
    /// the Amount column" — and doing it here means the caller never has to
    /// compute a spreadsheet reference by hand and get it subtly wrong.
    pub fn cell_under_header(&self, column_header: &str, row: u32) -> Option<String> {
        let header = self
            .cells
            .iter()
            .find(|cell| cell.value.as_deref() == Some(column_header))?;
        let column = column_of(&header.reference)?;
        Some(format!("{column}{row}"))
    }

    /// The 1-based index of the first row after the last non-empty one — where
    /// a totals row naturally goes.
    pub fn first_empty_row(&self) -> u32 {
        self.cells
            .iter()
            .filter_map(|cell| row_of(&cell.reference))
            .max()
            .unwrap_or(0)
            + 1
    }
}

/// Read every worksheet, resolving shared strings so text reads as text.
pub fn read_xlsx(data: &[u8]) -> Result<Vec<Sheet>, DocumentError> {
    let package = OoxmlPackage::read(data)?;
    let shared = read_shared_strings(&package)?;
    let mut sheets = Vec::new();
    for (name, part) in sheet_parts(&package)? {
        let xml = package.part_str(&part)?;
        sheets.push(Sheet {
            name,
            cells: parse_cells(&xml, &shared)?,
        });
    }
    Ok(sheets)
}

/// Apply `edits`, returning a new `.xlsx` with every other part copied through.
pub fn edit_xlsx(data: &[u8], edits: &[XlsxEdit]) -> Result<Vec<u8>, DocumentError> {
    let mut package = OoxmlPackage::read(data)?;
    let parts = sheet_parts(&package)?;
    let mut recalc_needed = false;

    for edit in edits {
        match edit {
            XlsxEdit::SetCellFormula {
                sheet,
                cell,
                formula,
            } => {
                let part = parts
                    .iter()
                    .find(|(name, _)| name == sheet)
                    .map(|(_, part)| part.clone())
                    .ok_or_else(|| DocumentError::UnknownAddress {
                        address: sheet.clone(),
                    })?;
                let xml = package.part_str(&part)?;
                let updated = set_cell_formula(&xml, cell, formula.trim_start_matches('='))?;
                package.replace_part(&part, updated.into_bytes())?;
                recalc_needed = true;
            }
        }
    }

    if recalc_needed {
        let workbook = package.part_str(WORKBOOK_PART)?;
        let updated = force_full_recalc(&workbook)?;
        package.replace_part(WORKBOOK_PART, updated.into_bytes())?;
    }
    package.write()
}

// --- read ------------------------------------------------------------------

fn read_shared_strings(package: &OoxmlPackage) -> Result<Vec<String>, DocumentError> {
    if !package.has_part(SHARED_STRINGS_PART) {
        return Ok(Vec::new());
    }
    let xml = package.part_str(SHARED_STRINGS_PART)?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut strings = Vec::new();
    let mut in_item = false;
    let mut current = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| DocumentError::MalformedPart {
                part: SHARED_STRINGS_PART.to_string(),
                detail: error.to_string(),
            })?;
        match &event {
            Event::Eof => break,
            Event::Start(tag) => {
                if local_name(tag.name().as_ref()) == b"si" {
                    in_item = true;
                    current.clear();
                }
            }
            Event::End(tag) => {
                if local_name(tag.name().as_ref()) == b"si" && in_item {
                    in_item = false;
                    strings.push(std::mem::take(&mut current));
                }
            }
            // A shared string can be split across several `<r>` rich-text runs;
            // concatenating every text event inside `<si>` handles both shapes.
            Event::Text(text) if in_item => {
                current.push_str(
                    &text
                        .decode()
                        .map_err(|error| DocumentError::MalformedPart {
                            part: SHARED_STRINGS_PART.to_string(),
                            detail: error.to_string(),
                        })?,
                );
            }
            _ => {}
        }
    }
    Ok(strings)
}

/// Sheet display names paired with their worksheet part paths, in workbook order.
fn sheet_parts(package: &OoxmlPackage) -> Result<Vec<(String, String)>, DocumentError> {
    let workbook = package.part_str(WORKBOOK_PART)?;
    let mut reader = quick_xml::Reader::from_str(&workbook);
    reader.config_mut().check_end_names = false;

    let mut names = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| DocumentError::MalformedPart {
                part: WORKBOOK_PART.to_string(),
                detail: error.to_string(),
            })?;
        match &event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag) => {
                if local_name(tag.name().as_ref()) == b"sheet"
                    && let Some(name) = attribute(tag, b"name")
                {
                    names.push(name);
                }
            }
            _ => {}
        }
    }

    // Worksheet parts are conventionally `xl/worksheets/sheetN.xml` in workbook
    // order. Paired positionally rather than resolved through r:id in the rels
    // part: the mapping is order-preserving for every producer observed, and
    // this keeps the crate off a second relationship parser.
    let mut parts: Vec<String> = package
        .names()
        .iter()
        .filter(|name| name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml"))
        .cloned()
        .collect();
    parts.sort_by_key(|name| {
        name.trim_start_matches("xl/worksheets/sheet")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });

    if parts.is_empty() {
        return Err(DocumentError::MissingPart {
            part: "xl/worksheets/sheet1.xml".to_string(),
        });
    }
    Ok(names.into_iter().zip(parts).collect())
}

fn parse_cells(xml: &str, shared: &[String]) -> Result<Vec<Cell>, DocumentError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut cells: Vec<Cell> = Vec::new();
    let mut reference = String::new();
    let mut is_shared = false;
    let mut in_value = false;
    let mut in_formula = false;
    let mut value = String::new();
    let mut formula = String::new();
    let mut open = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| DocumentError::MalformedPart {
                part: "worksheet".to_string(),
                detail: error.to_string(),
            })?;
        match &event {
            Event::Eof => break,
            Event::Start(tag) | Event::Empty(tag) => match local_name(tag.name().as_ref()) {
                b"c" => {
                    reference = attribute(tag, b"r").unwrap_or_default();
                    is_shared = attribute(tag, b"t").as_deref() == Some("s");
                    value.clear();
                    formula.clear();
                    open = true;
                }
                b"v" => in_value = true,
                b"f" => in_formula = true,
                _ => {}
            },
            Event::End(tag) => match local_name(tag.name().as_ref()) {
                b"c" if open => {
                    open = false;
                    let resolved = if is_shared {
                        value
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared.get(index).cloned())
                    } else if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                    if resolved.is_some() || !formula.is_empty() {
                        cells.push(Cell {
                            reference: std::mem::take(&mut reference),
                            value: resolved,
                            formula: (!formula.is_empty()).then(|| formula.clone()),
                        });
                    }
                }
                b"v" => in_value = false,
                b"f" => in_formula = false,
                _ => {}
            },
            Event::Text(text) => {
                let decoded = text
                    .decode()
                    .map_err(|error| DocumentError::MalformedPart {
                        part: "worksheet".to_string(),
                        detail: error.to_string(),
                    })?;
                if in_value {
                    value.push_str(&decoded);
                } else if in_formula {
                    formula.push_str(&decoded);
                }
            }
            _ => {}
        }
    }
    Ok(cells)
}

// --- write -----------------------------------------------------------------

fn set_cell_formula(xml: &str, target: &str, formula: &str) -> Result<String, DocumentError> {
    let target_row = row_of(target).ok_or_else(|| DocumentError::UnknownAddress {
        address: target.to_string(),
    })?;
    let target_column = column_of(target).ok_or_else(|| DocumentError::UnknownAddress {
        address: target.to_string(),
    })?;
    let target_column_index = column_index(&target_column);

    let mut in_target_row = false;
    let mut wrote = false;
    // Whether the target row exists at all. A totals row is normally the first
    // row BELOW the data, so "the row is not there yet" is the common case, not
    // an edge case — without this the most ordinary spreadsheet edit fails.
    let mut row_seen = false;
    // Depth of the existing `<c>` subtree being replaced, so its stale `<v>`
    // goes with it (trap 2 in the module docs).
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
            Event::Start(tag) | Event::Empty(tag) => {
                let qname = tag.name();
                match local_name(qname.as_ref()) {
                    b"row" => {
                        let row_number = attribute(tag, b"r").and_then(|r| r.parse::<u32>().ok());
                        in_target_row = row_number == Some(target_row);
                        if in_target_row {
                            row_seen = true;
                        }
                        // Rows, like cells, must stay in ascending order: insert
                        // the new row before the first row that sorts after it.
                        if !row_seen
                            && !wrote
                            && row_number.is_some_and(|number| number > target_row)
                        {
                            row_seen = true;
                            wrote = true;
                            let mut events = new_row(target_row, target, formula);
                            events.push(clone_event(event));
                            return Ok(EventAction::Replace(events));
                        }
                        Ok(EventAction::Keep)
                    }
                    b"c" if in_target_row && !wrote => {
                        let reference = attribute(tag, b"r").unwrap_or_default();
                        let Some(column) = column_of(&reference) else {
                            return Ok(EventAction::Keep);
                        };
                        let index = column_index(&column);
                        if index == target_column_index {
                            // Replace this cell wholesale.
                            wrote = true;
                            if matches!(event, Event::Start(_)) {
                                dropping = 1;
                            }
                            return Ok(EventAction::Replace(formula_cell(target, formula)));
                        }
                        if index > target_column_index {
                            // Cells must stay in ascending column order, so the
                            // new one goes in *before* the first later column.
                            wrote = true;
                            let mut events = formula_cell(target, formula);
                            events.push(clone_event(event));
                            return Ok(EventAction::Replace(events));
                        }
                        Ok(EventAction::Keep)
                    }
                    _ => Ok(EventAction::Keep),
                }
            }
            Event::End(tag) => {
                // Target row sorts after every existing row: append it at the
                // end of the sheet data.
                if local_name(tag.name().as_ref()) == b"sheetData" && !wrote {
                    wrote = true;
                    let mut events = new_row(target_row, target, formula);
                    events.push(Event::End(BytesEnd::new("sheetData")));
                    return Ok(EventAction::Replace(events));
                }
                if local_name(tag.name().as_ref()) == b"row" && in_target_row {
                    in_target_row = false;
                    if !wrote {
                        // Target column is past every existing cell in the row.
                        wrote = true;
                        let mut events = formula_cell(target, formula);
                        events.push(Event::End(BytesEnd::new("row")));
                        return Ok(EventAction::Replace(events));
                    }
                }
                Ok(EventAction::Keep)
            }
            _ => Ok(EventAction::Keep),
        }
    })?;

    if !wrote {
        return Err(DocumentError::UnknownAddress {
            address: format!("row {target_row} (for cell {target})"),
        });
    }
    Ok(out)
}

/// A whole `<row>` containing just the formula cell, for a row that does not
/// exist yet.
fn new_row(row: u32, reference: &str, formula: &str) -> Vec<Event<'static>> {
    let mut row_tag = BytesStart::new("row");
    row_tag.push_attribute(("r", row.to_string().as_str()));
    let mut events = vec![Event::Start(row_tag.into_owned())];
    events.extend(formula_cell(reference, formula));
    events.push(Event::End(BytesEnd::new("row")));
    events
}

/// A `<c>` element carrying only a formula — no `t` (a formula result is not a
/// shared string) and no `<v>` (Excel recomputes it).
fn formula_cell(reference: &str, formula: &str) -> Vec<Event<'static>> {
    let mut cell = BytesStart::new("c");
    cell.push_attribute(("r", reference));
    vec![
        Event::Start(cell),
        Event::Start(BytesStart::new("f")),
        Event::Text(BytesText::new(formula).into_owned()),
        Event::End(BytesEnd::new("f")),
        Event::End(BytesEnd::new("c")),
    ]
}

/// Ask Excel to recompute on open, so no cell shows a stale cached value.
fn force_full_recalc(workbook_xml: &str) -> Result<String, DocumentError> {
    let mut saw_calc_pr = false;
    transform_xml(workbook_xml, |event| match event {
        Event::Start(tag) | Event::Empty(tag) => {
            if local_name(tag.name().as_ref()) == b"calcPr" {
                saw_calc_pr = true;
                let mut replacement = BytesStart::new("calcPr");
                for attribute in tag.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) != b"fullCalcOnLoad" {
                        replacement.push_attribute(attribute);
                    }
                }
                replacement.push_attribute(("fullCalcOnLoad", "1"));
                return Ok(EventAction::Replace(vec![Event::Empty(
                    replacement.into_owned(),
                )]));
            }
            Ok(EventAction::Keep)
        }
        Event::End(tag) if local_name(tag.name().as_ref()) == b"workbook" && !saw_calc_pr => {
            let mut calc_pr = BytesStart::new("calcPr");
            calc_pr.push_attribute(("fullCalcOnLoad", "1"));
            Ok(EventAction::Replace(vec![
                Event::Empty(calc_pr),
                Event::End(BytesEnd::new("workbook")),
            ]))
        }
        _ => Ok(EventAction::Keep),
    })
}

// --- reference helpers -----------------------------------------------------

fn column_of(reference: &str) -> Option<String> {
    let column: String = reference
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    (!column.is_empty()).then(|| column.to_ascii_uppercase())
}

fn row_of(reference: &str) -> Option<u32> {
    reference
        .chars()
        .skip_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .parse()
        .ok()
}

/// Base-26 column index (`A`=1, `Z`=26, `AA`=27) so ordering comparisons are
/// numeric — a lexicographic compare would sort `AA` before `B`.
fn column_index(column: &str) -> u32 {
    column
        .bytes()
        .filter(|byte| byte.is_ascii_alphabetic())
        .fold(0u32, |acc, byte| {
            acc * 26 + u32::from(byte.to_ascii_uppercase() - b'A' + 1)
        })
}

fn attribute(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes().flatten().find_map(|attribute| {
        (local_name(attribute.key.as_ref()) == name)
            .then(|| String::from_utf8_lossy(&attribute.value).into_owned())
    })
}

fn clone_event(event: &Event<'_>) -> Event<'static> {
    match event {
        Event::Start(tag) => Event::Start(tag.clone().into_owned()),
        Event::Empty(tag) => Event::Empty(tag.clone().into_owned()),
        Event::End(tag) => Event::End(tag.clone().into_owned()),
        Event::Text(text) => Event::Text(text.clone().into_owned()),
        other => other.clone().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::expenses_xlsx;

    #[test]
    fn read_resolves_shared_strings_so_headers_are_readable() {
        let sheets = read_xlsx(&expenses_xlsx()).unwrap();
        let headers: Vec<_> = sheets[0]
            .cells
            .iter()
            .filter(|cell| row_of(&cell.reference) == Some(1))
            .filter_map(|cell| cell.value.clone())
            .collect();
        // Without shared-string resolution these read as "0", "1", "2".
        assert_eq!(headers, vec!["Item", "Quarter", "Amount"]);
    }

    #[test]
    fn read_surfaces_existing_formulas_separately_from_values() {
        let sheets = read_xlsx(&expenses_xlsx()).unwrap();
        let formula_cell = sheets[0]
            .cells
            .iter()
            .find(|cell| cell.formula.is_some())
            .expect("fixture has a formula");
        assert_eq!(formula_cell.reference, "C5");
        assert_eq!(formula_cell.formula.as_deref(), Some("SUM(C2:C4)"));
    }

    #[test]
    fn first_empty_row_lands_just_past_the_last_populated_row() {
        // Where a totals row goes, without the caller counting rows by hand.
        let sheets = read_xlsx(&expenses_xlsx()).unwrap();
        assert_eq!(sheets[0].first_empty_row(), 6);
    }

    #[test]
    fn cell_under_header_resolves_the_column_by_its_heading() {
        let sheets = read_xlsx(&expenses_xlsx()).unwrap();
        assert_eq!(
            sheets[0].cell_under_header("Amount", 5).as_deref(),
            Some("C5")
        );
        assert_eq!(sheets[0].cell_under_header("Nope", 5), None);
    }

    #[test]
    fn setting_a_formula_replaces_the_existing_one_and_drops_the_cached_value() {
        let edited = edit_xlsx(
            &expenses_xlsx(),
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "C5".to_string(),
                formula: "=SUM(C2:C4)*1.2".to_string(),
            }],
        )
        .unwrap();

        let sheets = read_xlsx(&edited).unwrap();
        let cell = sheets[0]
            .cells
            .iter()
            .find(|cell| cell.reference == "C5")
            .unwrap();
        assert_eq!(cell.formula.as_deref(), Some("SUM(C2:C4)*1.2"));
        assert_eq!(
            cell.value, None,
            "the stale cached value must be dropped, or Excel shows the old total"
        );
    }

    #[test]
    fn setting_a_formula_marks_the_workbook_for_recalculation() {
        let edited = edit_xlsx(
            &expenses_xlsx(),
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "C5".to_string(),
                formula: "SUM(C2:C4)".to_string(),
            }],
        )
        .unwrap();
        let workbook = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str(WORKBOOK_PART)
            .unwrap();
        assert!(
            workbook.contains("fullCalcOnLoad=\"1\""),
            "workbook must request recalculation: {workbook}"
        );
    }

    #[test]
    fn a_new_cell_is_inserted_in_ascending_column_order() {
        // Excel repairs (and drops content from) a row whose cells are out of
        // order, so position is correctness, not tidiness.
        let edited = edit_xlsx(
            &expenses_xlsx(),
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "B5".to_string(),
                formula: "COUNTA(B2:B4)".to_string(),
            }],
        )
        .unwrap();
        let sheet_xml = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        let row5 = sheet_xml
            .split("<row r=\"5\"")
            .nth(1)
            .and_then(|rest| rest.split("</row>").next())
            .expect("row 5 present");
        let b_at = row5.find("r=\"B5\"").expect("B5 written");
        let c_at = row5.find("r=\"C5\"").expect("C5 still present");
        assert!(b_at < c_at, "B5 must precede C5 in the row: {row5}");
    }

    #[test]
    fn a_cell_past_every_existing_column_is_appended_inside_the_row() {
        let edited = edit_xlsx(
            &expenses_xlsx(),
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "E5".to_string(),
                formula: "C5*2".to_string(),
            }],
        )
        .unwrap();
        let sheets = read_xlsx(&edited).unwrap();
        assert!(
            sheets[0]
                .cells
                .iter()
                .any(|cell| cell.reference == "E5" && cell.formula.as_deref() == Some("C5*2"))
        );
    }

    #[test]
    fn a_formula_in_a_row_that_does_not_exist_yet_creates_that_row() {
        // A totals row sits just below the data, so the row is normally absent.
        // The integration journey caught this: without row creation the most
        // ordinary spreadsheet edit there is fails outright.
        let edited = edit_xlsx(
            &expenses_xlsx(),
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "C9".to_string(),
                formula: "SUM(C2:C4)".to_string(),
            }],
        )
        .unwrap();
        let sheets = read_xlsx(&edited).unwrap();
        assert!(
            sheets[0]
                .cells
                .iter()
                .any(|cell| cell.reference == "C9" && cell.formula.is_some()),
            "the new row must carry the formula: {:?}",
            sheets[0].cells
        );
    }

    #[test]
    fn a_created_row_is_inserted_in_ascending_row_order() {
        // Excel repairs a sheet whose rows are out of order, exactly as it does
        // for out-of-order cells.
        let mut with_gap = expenses_xlsx();
        with_gap = edit_xlsx(
            &with_gap,
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "C9".to_string(),
                formula: "1".to_string(),
            }],
        )
        .unwrap();
        // Row 6 must land BEFORE the row 9 created above.
        let edited = edit_xlsx(
            &with_gap,
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "C6".to_string(),
                formula: "2".to_string(),
            }],
        )
        .unwrap();
        let xml = OoxmlPackage::read(&edited)
            .unwrap()
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        let six = xml.find(r#"<row r="6""#).expect("row 6 written");
        let nine = xml.find(r#"<row r="9""#).expect("row 9 still present");
        assert!(six < nine, "rows must stay ordered: {xml}");
    }

    #[test]
    fn an_unknown_sheet_name_is_a_typed_error() {
        let error = edit_xlsx(
            &expenses_xlsx(),
            &[XlsxEdit::SetCellFormula {
                sheet: "Nope".to_string(),
                cell: "A1".to_string(),
                formula: "1".to_string(),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, DocumentError::UnknownAddress { .. }));
    }

    #[test]
    fn editing_preserves_shared_strings_and_styles_bit_for_bit() {
        let original = expenses_xlsx();
        let edited = edit_xlsx(
            &original,
            &[XlsxEdit::SetCellFormula {
                sheet: "Expenses".to_string(),
                cell: "C5".to_string(),
                formula: "SUM(C2:C4)".to_string(),
            }],
        )
        .unwrap();
        let before = OoxmlPackage::read(&original).unwrap();
        let after = OoxmlPackage::read(&edited).unwrap();
        for name in before.names() {
            if name == "xl/worksheets/sheet1.xml" || name == WORKBOOK_PART {
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
    fn column_index_orders_multi_letter_columns_numerically() {
        // A lexicographic compare would put AA before B and corrupt row order.
        assert!(column_index("B") < column_index("AA"));
        assert_eq!(column_index("A"), 1);
        assert_eq!(column_index("Z"), 26);
        assert_eq!(column_index("AA"), 27);
    }
}
