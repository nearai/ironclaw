//! Minimal but *real* OOXML packages, built in code so the crate's own tests
//! need no binary fixtures checked in.
//!
//! These are deliberately small. They are correct enough to exercise the
//! editors' structural logic, and NOT a substitute for documents produced by
//! real Word/Excel/PowerPoint — the integration tier uses those (see
//! `tests/fixtures/`), because only a real producer exhibits the run-splitting
//! and rsid churn that breaks naive editors.

use std::io::{Cursor, Write};

/// Build a zip from ordered entries, deterministically.
pub(crate) fn package(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const WORD_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

/// A styles part, present so the "unrelated parts are copied through" tests
/// have something real to assert about.
const WORD_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style></w:styles>"#;

/// A `.docx` whose `word/document.xml` body is exactly `body`.
pub(crate) fn docx_with_body(body: &str) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    package(&[
        (
            "[Content_Types].xml",
            WORD_CONTENT_TYPES.as_bytes().to_vec(),
        ),
        ("_rels/.rels", RELS.as_bytes().to_vec()),
        ("word/styles.xml", WORD_STYLES.as_bytes().to_vec()),
        ("word/document.xml", document.into_bytes()),
    ])
}

const SHEET_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;

/// An expenses sheet with `Item`/`Quarter`/`Amount` headers stored as shared
/// strings (the shape Excel actually writes) and an existing total formula in
/// `C5` with a cached value — so the "stale `<v>`" trap is reachable.
pub(crate) fn expenses_xlsx() -> Vec<u8> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets><sheet name="Expenses" sheetId="1" r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></sheets></workbook>"#;
    let shared = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="6" uniqueCount="6"><si><t>Item</t></si><si><t>Quarter</t></si><si><t>Amount</t></si><si><t>Hosting</t></si><si><t>Travel</t></si><si><t>Licenses</t></si></sst>"#;
    let sheet = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c></row>"#,
        r#"<row r="2"><c r="A2" t="s"><v>3</v></c><c r="B2"><v>1</v></c><c r="C2"><v>1200</v></c></row>"#,
        r#"<row r="3"><c r="A3" t="s"><v>4</v></c><c r="B3"><v>1</v></c><c r="C3"><v>800</v></c></row>"#,
        r#"<row r="4"><c r="A4" t="s"><v>5</v></c><c r="B4"><v>1</v></c><c r="C4"><v>450</v></c></row>"#,
        // A cached value that is deliberately WRONG for the formula, so a test
        // can prove the editor drops it rather than leaving it on screen.
        r#"<row r="5"><c r="C5"><f>SUM(C2:C4)</f><v>9999</v></c></row>"#,
        r#"</sheetData></worksheet>"#,
    );
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#;

    package(&[
        (
            "[Content_Types].xml",
            SHEET_CONTENT_TYPES.as_bytes().to_vec(),
        ),
        ("xl/workbook.xml", workbook.as_bytes().to_vec()),
        ("xl/sharedStrings.xml", shared.as_bytes().to_vec()),
        ("xl/styles.xml", styles.as_bytes().to_vec()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes().to_vec()),
    ])
}

/// A one-slide deck with a real layout/theme chain, so a clone has something
/// meaningful to inherit and the "same style" assertions are not vacuous.
pub(crate) fn quarterly_pptx() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#;

    let presentation = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#;

    let presentation_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

    // Placeholder types and run properties are what "same style" means in
    // practice, so the fixture carries both.
    let slide = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>"#,
        r#"<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r>"#,
        r#"<a:rPr lang="en-US" sz="4400" b="1"/><a:t>Q1 Results</a:t></a:r></a:p></p:txBody></p:sp>"#,
        r#"<p:sp><p:nvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r>"#,
        r#"<a:rPr lang="en-US" sz="2000"/><a:t>Revenue up 12%</a:t></a:r></a:p></p:txBody></p:sp>"#,
        r#"</p:spTree></p:cSld></p:sld>"#,
    );

    let slide_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#;

    let layout = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="titleAndBody"><p:cSld name="Title and Content"/></p:sldLayout>"#;

    let theme = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="IronClaw"><a:themeElements><a:clrScheme name="IronClaw"/></a:themeElements></a:theme>"#;

    package(&[
        ("[Content_Types].xml", content_types.as_bytes().to_vec()),
        ("ppt/presentation.xml", presentation.as_bytes().to_vec()),
        (
            "ppt/_rels/presentation.xml.rels",
            presentation_rels.as_bytes().to_vec(),
        ),
        ("ppt/slides/slide1.xml", slide.as_bytes().to_vec()),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            slide_rels.as_bytes().to_vec(),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            layout.as_bytes().to_vec(),
        ),
        ("ppt/theme/theme1.xml", theme.as_bytes().to_vec()),
    ])
}

/// A contract with a real redline: "sixty" struck out, "thirty" proposed in its
/// place, both attributed to a reviewer — the shape a legal review actually
/// arrives in.
pub(crate) fn redlined_docx() -> Vec<u8> {
    let body = concat!(
        r#"<w:p><w:r><w:t>Master Services Agreement</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t xml:space="preserve">Clause 4: the review period is </w:t></w:r>"#,
        r#"<w:del w:id="1" w:author="Reviewer" w:date="2026-01-01T00:00:00Z"><w:r><w:delText>sixty</w:delText></w:r></w:del>"#,
        r#"<w:ins w:id="2" w:author="Reviewer" w:date="2026-01-01T00:00:00Z"><w:r><w:t>thirty</w:t></w:r></w:ins>"#,
        r#"<w:r><w:t xml:space="preserve"> days.</w:t></w:r></w:p>"#,
    );
    docx_with_body(body)
}
