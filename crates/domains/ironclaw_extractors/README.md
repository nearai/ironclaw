# ironclaw_extractors

Pure, format-aware text extraction from file bytes — PDF, Office Open XML
(word/slide/sheet), legacy Office, RTF, and UTF-8 text/code — with
decompression-bomb caps on every ZIP-based format. A leaf with no I/O, no
async, and no knowledge of where the bytes came from; it exists specifically
to keep the heavy parser dependencies (`pdf-extract`, `zip`) out of every
consumer's build.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_extractors` · **Manifest:** `crates/domains/ironclaw_extractors/Cargo.toml`
- **Use this when:** you have bytes and a MIME type (or filename) and need
  plain text plus a classified outcome.
- **Don't use this when:** landing or storing attachments →
  `ironclaw_attachments`; anything needing I/O, network, or source awareness —
  by charter this crate cannot grow those.

## Public surface

Five items, deliberately (everything else is private, including the format
extractors and the ZIP bomb-guard):

- `extract_document(data, mime, filename)` — MIME-driven entry point.
- `extract_document_text_by_filename(...)` — extension-driven, for callers
  with no trustworthy MIME type.
- `truncate_to_chars` — the canonical char-boundary-safe cap.
- `DocumentExtraction` — the `Text` / `Empty` / `Failed` classification.
- `ExtractionError` — the failure type; **its `Display` renders the
  classification and nothing else**, so interpolating it into model-facing
  text is safe by construction. `Debug` carries parser structure and belongs
  in operator logs only.

Adding a `pub` item here needs a caller in the same PR (see
[`AGENTS.md`](./AGENTS.md) for the history of the two that shipped without
one).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common` — nothing else internal.
- **Consumed by (2, and there are no others):** `ironclaw_attachments`
  (inbound landing), `ironclaw_host_runtime` (capability download output). All
  call through the full path — no crate writes `use ironclaw_extractors::…`.
  (The third consumer, `ironclaw_extension_support`'s `read_file` coding tool,
  was retired in the coding-tool cutover; the `read` tool that replaced it
  does not use this crate.)

## Invariants

- **Failure detail is never model-facing:**
  `every_extraction_failure_display_is_content_free` (in `src/lib.rs`) drives
  both boundary functions across every private extractor; a new variant whose
  `#[error(…)]` interpolates a field fails it. The call-site regression test
  for the historical leak was removed with the retired `read_file` coding tool
  in the coding-tool cutover (its `read` replacement does not use this crate),
  so this test is the guard.
- **Bomb caps fail closed:** every ZIP-based format goes through
  `bounded_read_zip_entry` — per-entry (50 MB) and cumulative (100 MB)
  decompressed bounds, actual-bytes-read tracking because headers can lie.
- Pure leaf: no async, no I/O — enforced by charter and by the layer matrix.

## Tests

```bash
cargo test -p ironclaw_extractors    # all tests live in src/lib.rs
```

If you change what an extractor returns, also check the three consumers'
rendering tests (`ironclaw_attachments::inbound`,
`ironclaw_host_runtime::document_output`,
`ironclaw_extension_support::coding::file`).

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance — read
  its redaction section before touching `ExtractionError`).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md`, PROPOSAL §6.4.10.
