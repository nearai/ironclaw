# `ironclaw_extractors` — pure bytes → text

Target-architecture entry: PROPOSAL §6.4.10, `families/domains.md`.

Turn a file's bytes into plain text by format. One `src/lib.rs`, no I/O, no
async, no knowledge of where the bytes came from — the same three functions
serve chat attachments (`ironclaw_attachments`), capability download output
(`ironclaw_host_runtime`), and the `read_file` coding tool
(`ironclaw_extension_support`).

## The public surface, and why it is this small

| Item | What it is for |
|---|---|
| `extract_document` | MIME-driven entry point; classifies the outcome |
| `extract_document_text_by_filename` | extension-driven, for callers with no trustworthy MIME type |
| `truncate_to_chars` | the canonical char-boundary-safe cap |
| `DocumentExtraction` | the `Text` / `Empty` / `Failed` classification |
| `ExtractionError` | the failure type — read the next section before touching it |

Everything else is private, including the format extractors, the ZIP
bomb-guard, `extract_text`, and `TRUNCATION_MARKER`. **Adding a `pub` item
here needs a caller in the same PR.** Two public items reached WS6 with zero
callers outside the crate (`extract_text`, `TRUNCATION_MARKER`); both are now
private, and `TRUNCATION_MARKER` was actively confusing because
`ironclaw_agent_loop` and `ironclaw_mcp` each declare their own constant of
that name with a *different value*.

## The rule that matters: failure detail is not model-facing

`ExtractionError` carries parser diagnostics — a `pdf-extract` message, a ZIP
entry name, an offset — any of which can echo the document's own content.

- **`Display` renders the classification and nothing else.** Interpolating an
  `ExtractionError` into model-facing text with `{error}` is safe by
  construction. That is the entire reason the type exists.
- **`Debug` renders the payload**, so it belongs in an **operator log** and
  nowhere else — never a model result, capability output, projected event,
  snapshot, or user-visible error. What it carries is container/parser
  *structure* (`lopdf` object ids, byte offsets, dictionary keys; `zip`
  archive diagnostics; the fixed OOXML entry paths this crate reads), not
  document text — but a consumer under a stricter redaction charter, notably
  `ironclaw_host_runtime` (see its `AGENTS.md`), owns that ceiling and should
  re-check it before widening where the payload goes.

A new variant must keep that property: its `#[error("…")]` string may not
interpolate a field. `every_extraction_failure_display_is_content_free` drives
both public boundary functions across every private extractor and fails if it
does.

This was a doc comment before it was a type, and the doc comment only sat on
one of the two boundary sites — the other one leaked the raw string into a
model-facing safe summary in `ironclaw_extension_support`'s `read_file`. The
regression test for that lives at the call site
(`coding/file.rs::tests::read_file_extraction_failure_summary_carries_no_parser_detail`),
not here: `Display` alone is the helper, and the wrapper that composes the
summary is what actually leaked.

## Never contains

Network fetches, storage, async, attachment landing, channel logic, or
anything that knows where the bytes came from. Heavy format dependencies
(`pdf-extract`, `zip`) are isolated here precisely so consumers do not carry
them.

## Decompression-bomb caps

Every ZIP-based format goes through `bounded_read_zip_entry`, which bounds
per-entry (50 MB) and cumulative (100 MB) decompressed size, pre-checks the
declared header size as a cheap reject, and then tracks **actual bytes read**
because ZIP headers can lie. Reading exactly the per-entry cap fails closed —
a truncated entry means the real size exceeds the limit. Do not add a
ZIP-reading path that bypasses it.

## Tests

All in `src/lib.rs`. `cargo test -p ironclaw_extractors`. The consumers own
their own rendering tests; if you change what an extractor returns, check
`ironclaw_attachments::inbound`, `ironclaw_host_runtime::document_output`, and
`ironclaw_extension_support::coding::file` — those are the three callers, and
there are no others.
