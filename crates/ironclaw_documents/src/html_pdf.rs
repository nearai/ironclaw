//! Render a documented HTML subset to PDF.
//!
//! PDF is the one format this crate does not edit. There is no reliable
//! structured edit for an arbitrary PDF — its content stream is positioned
//! glyphs, not text with structure — so rather than pretend, the supported
//! workflow is: **author or revise HTML, then render it.** Revising means
//! editing the HTML source and re-rendering, which is lossless because the
//! HTML, not the PDF, is the document of record.
//!
//! ## Supported subset
//!
//! Deliberately small, and deliberately *documented* — a renderer that
//! silently ignores most of what it is given produces confusing output.
//!
//! | Category | Supported |
//! |---|---|
//! | Headings | `<h1>` `<h2>` `<h3>` |
//! | Blocks | `<p>`, `<ul>`/`<ol>` with `<li>`, `<hr>`, `<blockquote>` |
//! | Inline | `<strong>`/`<b>`, `<em>`/`<i>`, `<code>`, `<br>` |
//! | Entities | `&amp; &lt; &gt; &quot; &#39; &nbsp;` and numeric `&#NN;` |
//!
//! Any other tag is transparent: it is ignored but its text still renders, so
//! wrapping markup (`<div>`, `<span>`, `<body>`) never swallows content.
//! CSS is not supported at all — styling comes from the element vocabulary.
//!
//! ## Why not a real CSS engine
//!
//! `printpdf`'s optional `html` feature embeds one, but it resolves fonts
//! through `rust-fontconfig`, which scans **system** fonts. That makes output
//! depend on which fonts the host happens to have installed — different
//! pagination on CI than on a laptop, and a test that cannot assert anything
//! stable. This renderer uses only the PDF standard-14 fonts, whose metrics
//! `printpdf` embeds, so the same HTML produces the same PDF everywhere.

use std::collections::BTreeMap;

use printpdf::{
    BuiltinFont, Color, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, Rgb, TextItem,
};

use crate::error::DocumentError;

/// Internal marker for an explicit `<br>`.
///
/// A control character that cannot occur in meaningful HTML text, so it can
/// never collide with author content — unlike `\n`, which appears in every
/// hand-formatted source document and must collapse to a space.
const HARD_BREAK: char = '\u{0}';

/// Page geometry and typography. Defaults are A4 with 20 mm margins at 11 pt.
#[derive(Debug, Clone)]
pub struct PdfOptions {
    pub title: String,
    pub page_width: Mm,
    pub page_height: Mm,
    pub margin: Mm,
    pub body_size: Pt,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            title: "Document".to_string(),
            page_width: Mm(210.0),
            page_height: Mm(297.0),
            margin: Mm(20.0),
            body_size: Pt(11.0),
        }
    }
}

/// Render `html` to PDF bytes.
pub fn html_to_pdf(html: &str, options: &PdfOptions) -> Result<Vec<u8>, DocumentError> {
    let blocks = parse_blocks(html);
    if blocks.is_empty() {
        return Err(DocumentError::Html(
            "no renderable content: the supported subset is headings, paragraphs, lists, rules \
             and inline emphasis"
                .to_string(),
        ));
    }
    let mut layout = Layout::new(options)?;
    for block in &blocks {
        layout.place(block);
    }
    let pages = layout.finish();

    let mut document = PdfDocument::new(&options.title);
    Ok(document
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut Vec::new()))
}

// --- document model --------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Block {
    Heading { level: u8, spans: Vec<Span> },
    Paragraph { spans: Vec<Span> },
    ListItem { marker: String, spans: Vec<Span> },
    Quote { spans: Vec<Span> },
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Span {
    text: String,
    style: Style,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
}

impl Style {
    fn font(self) -> BuiltinFont {
        match (self.code, self.bold, self.italic) {
            (true, true, _) => BuiltinFont::CourierBold,
            (true, false, _) => BuiltinFont::Courier,
            (false, true, true) => BuiltinFont::HelveticaBoldOblique,
            (false, true, false) => BuiltinFont::HelveticaBold,
            (false, false, true) => BuiltinFont::HelveticaOblique,
            (false, false, false) => BuiltinFont::Helvetica,
        }
    }
}

// --- parsing ---------------------------------------------------------------

/// A tolerant tag/text tokenizer.
///
/// Hand-written rather than reusing the XML reader because model-authored HTML
/// routinely has unclosed `<li>` and `<p>` and bare `<br>`, all of which a
/// strict XML parser rejects outright. Rejecting the whole document over a
/// missing close tag would make the tool fail on its most common input.
fn parse_blocks(html: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut style = Style::default();
    let mut spans: Vec<Span> = Vec::new();
    // Innermost list kind and its running item number, so `<ol>` numbering
    // restarts per list and nested lists do not share a counter.
    let mut lists: Vec<(bool, usize)> = Vec::new();
    let mut pending: Option<Block> = None;
    let mut text = String::new();

    let bytes = html.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'<' {
            let Some(close) = html[index..].find('>').map(|at| index + at) else {
                break;
            };
            let raw = &html[index + 1..close];
            index = close + 1;

            // Comments and declarations carry no content.
            if raw.starts_with('!') || raw.starts_with('?') {
                text.clear();
                continue;
            }
            if !text.is_empty() {
                spans.push(Span {
                    text: decode_entities(&text),
                    style,
                });
                text.clear();
            }

            let closing = raw.starts_with('/');
            let name: String = raw
                .trim_start_matches('/')
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();

            match name.as_str() {
                "strong" | "b" => style.bold = !closing,
                "em" | "i" => style.italic = !closing,
                "code" => style.code = !closing,
                "br" => spans.push(Span {
                    text: HARD_BREAK.to_string(),
                    style,
                }),
                "hr" => {
                    flush(&mut blocks, &mut spans, &mut pending);
                    blocks.push(Block::Rule);
                }
                "h1" | "h2" | "h3" => {
                    flush(&mut blocks, &mut spans, &mut pending);
                    if !closing {
                        pending = Some(Block::Heading {
                            level: name.as_bytes()[1] - b'0',
                            spans: Vec::new(),
                        });
                    }
                }
                "p" => flush(&mut blocks, &mut spans, &mut pending),
                "blockquote" => {
                    flush(&mut blocks, &mut spans, &mut pending);
                    if !closing {
                        pending = Some(Block::Quote { spans: Vec::new() });
                    }
                }
                "ul" | "ol" => {
                    flush(&mut blocks, &mut spans, &mut pending);
                    if closing {
                        lists.pop();
                    } else {
                        lists.push((name == "ol", 0));
                    }
                }
                "li" => {
                    // An unclosed `<li>` is closed implicitly by the next one.
                    flush(&mut blocks, &mut spans, &mut pending);
                    if !closing {
                        let marker = match lists.last_mut() {
                            Some((true, counter)) => {
                                *counter += 1;
                                format!("{counter}.")
                            }
                            _ => "\u{2022}".to_string(),
                        };
                        pending = Some(Block::ListItem {
                            marker,
                            spans: Vec::new(),
                        });
                    }
                }
                // Unknown tags are transparent: ignored, but their text stays.
                _ => {}
            }
            continue;
        }

        let next = html[index..]
            .find('<')
            .map(|at| index + at)
            .unwrap_or(bytes.len());
        text.push_str(&html[index..next]);
        index = next;
    }

    if !text.is_empty() {
        spans.push(Span {
            text: decode_entities(&text),
            style,
        });
    }
    flush(&mut blocks, &mut spans, &mut pending);
    blocks
}

fn flush(blocks: &mut Vec<Block>, spans: &mut Vec<Span>, pending: &mut Option<Block>) {
    let trimmed = trim_spans(std::mem::take(spans));
    if trimmed.is_empty() {
        *pending = None;
        return;
    }
    match pending.take() {
        Some(Block::Heading { level, .. }) => blocks.push(Block::Heading {
            level,
            spans: trimmed,
        }),
        Some(Block::ListItem { marker, .. }) => blocks.push(Block::ListItem {
            marker,
            spans: trimmed,
        }),
        Some(Block::Quote { .. }) => blocks.push(Block::Quote { spans: trimmed }),
        _ => blocks.push(Block::Paragraph { spans: trimmed }),
    }
}

fn trim_spans(spans: Vec<Span>) -> Vec<Span> {
    let mut collapsed: Vec<Span> = Vec::new();
    for span in spans {
        // HTML collapses runs of whitespace; without this every source newline
        // becomes a visible gap.
        let text = collapse_whitespace(&span.text);
        if text.is_empty() {
            continue;
        }
        match collapsed.last_mut() {
            Some(last) if last.style == span.style => last.text.push_str(&text),
            _ => collapsed.push(Span {
                text,
                style: span.style,
            }),
        }
    }
    if let Some(first) = collapsed.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = collapsed.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    collapsed.retain(|span| !span.text.is_empty());
    collapsed
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for character in text.chars() {
        // Only an explicit `<br>` breaks a line. Source newlines are ordinary
        // whitespace in HTML, so they collapse like any other run — using `\n`
        // itself as the break marker would make every hand-formatted source
        // line render as a break.
        if character == HARD_BREAK {
            out.push(HARD_BREAK);
            last_space = true;
            continue;
        }
        if character.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(character);
            last_space = false;
        }
    }
    out
}

fn decode_entities(text: &str) -> String {
    // Drop any literal control character that would collide with HARD_BREAK.
    let sanitized = text.replace(HARD_BREAK, "");
    let mut out = String::with_capacity(sanitized.len());
    // Character-wise rather than byte-index slicing: entity bodies are ASCII
    // but the surrounding text is arbitrary UTF-8, and an index computed on one
    // and applied to the other is how multi-byte characters get split.
    let mut characters = sanitized.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '&' {
            out.push(character);
            continue;
        }
        // Entity names are short; anything longer is a stray ampersand.
        const MAX_ENTITY_LEN: usize = 10;
        let mut entity = String::new();
        let mut terminated = false;
        while entity.chars().count() < MAX_ENTITY_LEN {
            match characters.peek() {
                Some(';') => {
                    characters.next();
                    terminated = true;
                    break;
                }
                Some(next) => {
                    entity.push(*next);
                    characters.next();
                }
                None => break,
            }
        }

        let decoded = terminated
            .then(|| match entity.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                other => other
                    .strip_prefix('#')
                    .and_then(|digits| digits.parse::<u32>().ok())
                    .and_then(char::from_u32),
            })
            .flatten();

        match decoded {
            Some(character) => out.push(character),
            // Not an entity we know: emit it back verbatim.
            None => {
                out.push('&');
                out.push_str(&entity);
                if terminated {
                    out.push(';');
                }
            }
        }
    }
    out
}

// --- layout ----------------------------------------------------------------

/// Glyph metrics for the standard-14 fonts, parsed from the copies `printpdf`
/// embeds. No system font is consulted, so wrapping is identical everywhere.
struct Metrics {
    fonts: BTreeMap<String, Option<ParsedFont>>,
}

impl Metrics {
    fn new() -> Self {
        Self {
            fonts: BTreeMap::new(),
        }
    }

    /// Width of `text` in points at `size`.
    fn width(&mut self, text: &str, font: BuiltinFont, size: Pt) -> f32 {
        let key = format!("{font:?}");
        let parsed = self.fonts.entry(key).or_insert_with(|| {
            let subset = font.get_subset_font();
            ParsedFont::from_bytes(&subset.bytes, 0, &mut Vec::new())
        });
        let Some(parsed) = parsed.as_ref() else {
            // Half an em per character is the conventional fallback; only
            // reachable if a standard font fails to parse.
            return text.chars().count() as f32 * size.0 * 0.5;
        };
        let units = f32::from(parsed.units_per_em.max(1));
        text.chars()
            .map(|character| {
                parsed
                    .lookup_glyph_index(character as u32)
                    .and_then(|glyph| parsed.get_glyph_width(glyph))
                    .map(|width| f32::from(width) / units)
                    // Codepoints outside the Win-1252 subset have no glyph;
                    // charge them half an em so wrapping stays sane.
                    .unwrap_or(0.5)
                    * size.0
            })
            .sum()
    }
}

struct Layout {
    options: PdfOptions,
    metrics: Metrics,
    pages: Vec<PdfPage>,
    ops: Vec<Op>,
    cursor_y: f32,
    content_width: f32,
}

/// Points per millimetre.
const PT_PER_MM: f32 = 72.0 / 25.4;

impl Layout {
    fn new(options: &PdfOptions) -> Result<Self, DocumentError> {
        if options.margin.0 * 2.0 >= options.page_width.0
            || options.margin.0 * 2.0 >= options.page_height.0
        {
            return Err(DocumentError::Html(
                "margins leave no room for content".to_string(),
            ));
        }
        Ok(Self {
            options: options.clone(),
            metrics: Metrics::new(),
            pages: Vec::new(),
            ops: vec![Op::StartTextSection],
            cursor_y: (options.page_height.0 - options.margin.0) * PT_PER_MM,
            content_width: (options.page_width.0 - options.margin.0 * 2.0) * PT_PER_MM,
        })
    }

    fn bottom(&self) -> f32 {
        self.options.margin.0 * PT_PER_MM
    }

    fn break_page(&mut self) {
        let mut ops = std::mem::replace(&mut self.ops, vec![Op::StartTextSection]);
        ops.push(Op::EndTextSection);
        self.pages.push(PdfPage::new(
            self.options.page_width,
            self.options.page_height,
            ops,
        ));
        self.cursor_y = (self.options.page_height.0 - self.options.margin.0) * PT_PER_MM;
    }

    fn place(&mut self, block: &Block) {
        let body = self.options.body_size.0;
        match block {
            Block::Rule => self.cursor_y -= body * 0.8,
            Block::Heading { level, spans } => {
                let size = Pt(match level {
                    1 => body * 2.0,
                    2 => body * 1.5,
                    _ => body * 1.2,
                });
                self.cursor_y -= size.0 * 0.6;
                self.write_wrapped(spans, size, 0.0, true, None);
                self.cursor_y -= size.0 * 0.3;
            }
            Block::Paragraph { spans } => {
                self.write_wrapped(spans, self.options.body_size, 0.0, false, None);
                self.cursor_y -= body * 0.5;
            }
            Block::Quote { spans } => {
                self.write_wrapped(spans, self.options.body_size, body * 2.0, false, None);
                self.cursor_y -= body * 0.5;
            }
            Block::ListItem { marker, spans } => {
                self.write_wrapped(
                    spans,
                    self.options.body_size,
                    body * 1.6,
                    false,
                    Some(marker.as_str()),
                );
                self.cursor_y -= body * 0.25;
            }
        }
    }

    /// Wrap `spans` to the content width and emit them, breaking pages as
    /// needed. `marker` is drawn once, in the indent gutter of the first line.
    fn write_wrapped(
        &mut self,
        spans: &[Span],
        size: Pt,
        indent: f32,
        bold_default: bool,
        marker: Option<&str>,
    ) {
        let line_height = size.0 * 1.35;
        let available = self.content_width - indent;
        let left = self.options.margin.0 * PT_PER_MM + indent;

        // Split into styled words, keeping explicit `<br>` breaks.
        let mut words: Vec<(String, Style)> = Vec::new();
        for span in spans {
            let mut style = span.style;
            if bold_default {
                style.bold = true;
            }
            for (index, chunk) in span.text.split(HARD_BREAK).enumerate() {
                if index > 0 {
                    words.push((HARD_BREAK.to_string(), style));
                }
                for word in chunk.split(' ').filter(|word| !word.is_empty()) {
                    words.push((word.to_string(), style));
                }
            }
        }

        let mut line: Vec<(String, Style)> = Vec::new();
        let mut line_width = 0.0f32;
        let mut first_line = true;

        for (word, style) in words {
            if word == HARD_BREAK.to_string() {
                self.emit_line(&line, size, left, line_height, first_line, marker);
                first_line = false;
                line.clear();
                line_width = 0.0;
                continue;
            }
            let space = if line.is_empty() {
                0.0
            } else {
                self.metrics.width(" ", style.font(), size)
            };
            let width = self.metrics.width(&word, style.font(), size);
            if !line.is_empty() && line_width + space + width > available {
                self.emit_line(&line, size, left, line_height, first_line, marker);
                first_line = false;
                line.clear();
                line.push((word, style));
                line_width = width;
                continue;
            }
            line_width += space + width;
            line.push((word, style));
        }
        if !line.is_empty() {
            self.emit_line(&line, size, left, line_height, first_line, marker);
        }
    }

    fn emit_line(
        &mut self,
        line: &[(String, Style)],
        size: Pt,
        left: f32,
        line_height: f32,
        first_line: bool,
        marker: Option<&str>,
    ) {
        if line.is_empty() {
            return;
        }
        if self.cursor_y - line_height < self.bottom() {
            self.break_page();
        }
        self.cursor_y -= line_height;
        let baseline = self.cursor_y;

        if first_line && let Some(marker) = marker {
            let marker_width = self.metrics.width(marker, BuiltinFont::Helvetica, size);
            self.ops.extend([
                Op::SetTextCursor {
                    pos: Point::new(
                        Mm((left - marker_width - size.0 * 0.35) / PT_PER_MM),
                        Mm(baseline / PT_PER_MM),
                    ),
                },
                Op::SetFont {
                    font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
                    size,
                },
                Op::SetFillColor {
                    col: Color::Rgb(Rgb {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        icc_profile: None,
                    }),
                },
                Op::ShowText {
                    items: vec![TextItem::Text(marker.to_string())],
                },
            ]);
        }

        let mut x = left;
        for (index, (word, style)) in line.iter().enumerate() {
            let text = if index == 0 {
                word.clone()
            } else {
                format!(" {word}")
            };
            // Each styled word is positioned explicitly: one ShowText cannot
            // switch fonts mid-string, and emitting a bold run without
            // repositioning drifts the rest of the line.
            self.ops.extend([
                Op::SetTextCursor {
                    pos: Point::new(Mm(x / PT_PER_MM), Mm(baseline / PT_PER_MM)),
                },
                Op::SetFont {
                    font: PdfFontHandle::Builtin(style.font()),
                    size,
                },
                Op::ShowText {
                    items: vec![TextItem::Text(text.clone())],
                },
            ]);
            x += self.metrics.width(&text, style.font(), size);
        }
    }

    fn finish(mut self) -> Vec<PdfPage> {
        let mut ops = std::mem::take(&mut self.ops);
        ops.push(Op::EndTextSection);
        self.pages.push(PdfPage::new(
            self.options.page_width,
            self.options.page_height,
            ops,
        ));
        self.pages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_and_lists_become_distinct_blocks() {
        let parsed =
            parse_blocks("<h1>Title</h1><p>Body text</p><ul><li>one</li><li>two</li></ul>");
        assert_eq!(parsed.len(), 4);
        assert!(matches!(parsed[0], Block::Heading { level: 1, .. }));
        assert!(matches!(parsed[1], Block::Paragraph { .. }));
        match &parsed[2] {
            Block::ListItem { marker, spans } => {
                assert_eq!(marker, "\u{2022}");
                assert_eq!(spans[0].text, "one");
            }
            other => panic!("expected a list item, got {other:?}"),
        }
    }

    #[test]
    fn ordered_lists_number_their_items_and_restart_per_list() {
        let parsed = parse_blocks("<ol><li>a</li><li>b</li></ol><ol><li>c</li></ol>");
        let markers: Vec<_> = parsed
            .iter()
            .filter_map(|block| match block {
                Block::ListItem { marker, .. } => Some(marker.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(markers, vec!["1.", "2.", "1."]);
    }

    #[test]
    fn unclosed_list_items_still_separate() {
        // Model-authored HTML routinely omits these; a strict XML parse would
        // reject the whole document.
        let parsed = parse_blocks("<ul><li>one<li>two</ul>");
        assert_eq!(parsed.len(), 2);
        assert!(matches!(&parsed[0], Block::ListItem { spans, .. } if spans[0].text == "one"));
        assert!(matches!(&parsed[1], Block::ListItem { spans, .. } if spans[0].text == "two"));
    }

    #[test]
    fn inline_emphasis_selects_the_matching_builtin_font() {
        let parsed =
            parse_blocks("<p>plain <strong>bold</strong> <em>italic</em> <code>mono</code></p>");
        let Block::Paragraph { spans } = &parsed[0] else {
            panic!("expected a paragraph");
        };
        let fonts: Vec<_> = spans.iter().map(|span| span.style.font()).collect();
        assert!(fonts.contains(&BuiltinFont::HelveticaBold), "{fonts:?}");
        assert!(fonts.contains(&BuiltinFont::HelveticaOblique), "{fonts:?}");
        assert!(fonts.contains(&BuiltinFont::Courier), "{fonts:?}");
    }

    #[test]
    fn unknown_tags_are_transparent_rather_than_swallowing_their_text() {
        // A `<div>`/`<span>` wrapper must not make content disappear.
        let parsed = parse_blocks("<div><span>kept</span></div>");
        assert_eq!(parsed.len(), 1);
        assert!(matches!(&parsed[0], Block::Paragraph { spans } if spans[0].text == "kept"));
    }

    #[test]
    fn entities_decode_including_numeric_forms() {
        let parsed = parse_blocks("<p>a &amp; b &lt;c&gt; &#65; &quot;q&quot;</p>");
        let Block::Paragraph { spans } = &parsed[0] else {
            panic!("expected a paragraph");
        };
        let joined: String = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(joined, r#"a & b <c> A "q""#);
    }

    #[test]
    fn source_whitespace_collapses_but_explicit_breaks_survive() {
        let parsed = parse_blocks("<p>a\n   b<br>c</p>");
        let Block::Paragraph { spans } = &parsed[0] else {
            panic!("expected a paragraph");
        };
        let joined: String = spans.iter().map(|span| span.text.as_str()).collect();
        // The source newline collapsed to a space; only the `<br>` broke.
        assert_eq!(joined, format!("a b{HARD_BREAK}c"));
    }

    #[test]
    fn rendering_produces_a_pdf_with_a_header_and_trailer() {
        let pdf = html_to_pdf("<h1>Report</h1><p>Body</p>", &PdfOptions::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF-"), "must be a PDF");
        assert!(
            pdf.windows(5).any(|window| window == b"%%EOF"),
            "must be terminated"
        );
    }

    /// The page content streams — the positioned text — with the trailer
    /// stripped. `printpdf` mints a random `/ID` per save, so whole-file
    /// equality is not achievable; the laid-out content is what determinism
    /// actually means here.
    fn content_streams(pdf: &[u8]) -> Vec<u8> {
        let text = String::from_utf8_lossy(pdf).into_owned();
        let mut out = Vec::new();
        let mut rest = text.as_str();
        while let Some(at) = rest.find("stream") {
            // "endstream" also contains "stream"; skip those or the trailer
            // (which carries the random /ID) leaks into the comparison.
            if at >= 3 && &rest[at - 3..at] == "end" {
                rest = &rest[at + "stream".len()..];
                continue;
            }
            let body = at + "stream".len();
            let Some(end) = rest[body..].find("endstream") else {
                break;
            };
            out.extend_from_slice(&rest.as_bytes()[body..body + end]);
            rest = &rest[body + end + "endstream".len()..];
        }
        out
    }

    #[test]
    fn layout_is_deterministic_for_the_same_input() {
        // The whole reason for avoiding a system-font-scanning layout engine:
        // identical input must lay out identically on any machine.
        let options = PdfOptions::default();
        let first = html_to_pdf("<h1>T</h1><p>hello world</p>", &options).unwrap();
        let second = html_to_pdf("<h1>T</h1><p>hello world</p>", &options).unwrap();
        assert_eq!(content_streams(&first), content_streams(&second));
        assert!(!content_streams(&first).is_empty(), "streams must be found");
    }

    #[test]
    fn long_content_paginates_instead_of_overflowing_one_page() {
        let html = format!(
            "<p>{}</p>",
            "The quick brown fox jumps over the lazy dog. ".repeat(400)
        );
        let mut layout = Layout::new(&PdfOptions::default()).unwrap();
        for block in &parse_blocks(&html) {
            layout.place(block);
        }
        assert!(
            layout.finish().len() > 1,
            "long content must span more than one page"
        );
    }

    #[test]
    fn empty_or_markup_only_html_is_a_typed_error_not_a_blank_pdf() {
        // Silently emitting an empty PDF would look like success.
        for html in ["", "   ", "<div></div>", "<!-- just a comment -->"] {
            assert!(
                matches!(
                    html_to_pdf(html, &PdfOptions::default()),
                    Err(DocumentError::Html(_))
                ),
                "{html:?} must be rejected"
            );
        }
    }

    #[test]
    fn word_wrapping_uses_real_glyph_metrics() {
        // A proportional font must measure "iiii" narrower than "WWWW"; a
        // fixed per-character estimate would call them equal and wrap wrongly.
        let mut metrics = Metrics::new();
        let narrow = metrics.width("iiii", BuiltinFont::Helvetica, Pt(12.0));
        let wide = metrics.width("WWWW", BuiltinFont::Helvetica, Pt(12.0));
        assert!(narrow > 0.0, "metrics must resolve");
        assert!(wide > narrow * 2.0, "{narrow} vs {wide}");
    }

    #[test]
    fn margins_wider_than_the_page_are_rejected() {
        let options = PdfOptions {
            margin: Mm(120.0),
            ..PdfOptions::default()
        };
        assert!(matches!(
            html_to_pdf("<p>x</p>", &options),
            Err(DocumentError::Html(_))
        ));
    }
}
