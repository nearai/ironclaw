# ironclaw_documents

Structure-preserving document transforms for DOCX, XLSX, and PPTX packages,
plus deterministic HTML-subset PDF generation. The crate exposes addressable
document views and typed edits while copying every untargeted OOXML package
part through unchanged.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_documents` · **Manifest:**
  `crates/substrates/ironclaw_documents/Cargo.toml`
- **Use this when:** a caller needs to inspect or transform OOXML without
  rebuilding the document from extracted text, or render the supported HTML
  subset to PDF.
- **Don't use this when:** deciding whether a caller may edit a file, selecting
  workspace paths, or performing filesystem I/O. Those checks remain with the
  mediated caller; this crate transforms bounded byte inputs into byte outputs.

## Public surface

- `read_document` / `edit_document` dispatch DOCX, XLSX, and PPTX operations.
- `docx`, `xlsx`, and `pptx` expose typed views and edit operations owned by
  each format.
- `html_to_pdf` renders the documented HTML subset using deterministic
  standard-font metrics.
- `DocumentError` reports unsupported formats, malformed package parts,
  unknown addresses, and bounded-input violations.

## Invariants

- Untargeted ZIP entries and XML events are copied through unchanged.
- Read-side addresses and write-side targeting use the same ordering rules.
- Malformed, ambiguous, duplicate, or oversized package data fails loudly.
- The crate performs no filesystem, authorization, approval, or product
  orchestration work.

## Tests

```bash
cargo test -p ironclaw_documents
```

Caller-path coverage for the mediated capabilities lives in
`ironclaw_host_runtime` and `tests/integration/document_edit.rs`.
