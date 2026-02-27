# Word Tool for IronClaw

Secure Microsoft Word (.docx) integration for IronClaw using WASM sandbox.

## Features

- ✅ Read .docx files and extract text
- ✅ Create new .docx documents with paragraphs
- ✅ Parse document structure (paragraphs, runs, formatting)
- ✅ WASM-sandboxed execution with fuel metering
- ✅ No network access (pure file operations)

## Security

Runs in IronClaw's WASM sandbox with:
- **Fuel metering** - Prevents infinite loops
- **Memory limits** - Max 4MB heap (64 pages)
- **File access control** - Only allowed workspace paths
- **No network** - Pure local file operations

## Architecture

The Word tool returns .docx file contents as base64-encoded bytes. IronClaw's built-in `write_file` tool can then save these bytes to disk.

**Workflow:**
1. Word tool creates .docx in memory
2. Returns base64-encoded bytes in JSON output
3. LLM uses `write_file` tool to save bytes to disk

This design keeps the WASM tool simple and secure, while leveraging IronClaw's existing file I/O capabilities.

## Building

Requires:
- Rust toolchain with `wasm32-wasip2` target
- WIT bindings from IronClaw repo

```bash
# Install target
rustup target add wasm32-wasip2

# Copy WIT files from IronClaw
cp -r /path/to/ironclaw/wit/ ./

# Build
cargo build --target wasm32-wasip2 --release

# Output: target/wasm32-wasip2/release/word_tool.wasm
```

## Installation

```bash
ironclaw tool install target/wasm32-wasip2/release/word_tool.wasm
```

## Usage

### Read a .docx file

```json
{
  "action": "read_docx",
  "path": "documents/paper.docx",
  "include_formatting": false
}
```

Returns:
```json
{
  "paragraphs": ["Paragraph 1 text", "Paragraph 2 text"],
  "paragraph_count": 2,
  "character_count": 156,
  "word_count": 28
}
```

### Create a new .docx file

```json
{
  "action": "create_docx",
  "path": "output/report.docx",
  "title": "Research Report",
  "paragraphs": [
    "Introduction",
    "This report presents findings from our study...",
    "Methods",
    "We conducted a systematic review..."
  ]
}
```

Returns:
```json
{
  "success": true,
  "path": "output/report.docx",
  "paragraph_count": 4,
  "docx_bytes_base64": "UEsDBBQACAgIAH1o..."
}
```

The LLM will then call:
```json
{
  "tool": "write_file",
  "params": {
    "path": "output/report.docx",
    "content_base64": "UEsDBBQACAgIAH1o..."
  }
}
```

### Get document metadata

```json
{
  "action": "get_metadata",
  "path": "documents/thesis.docx"
}
```

Returns:
```json
{
  "title": "PhD Thesis",
  "subject": "Machine Learning",
  "creator": "John Doe",
  "created": "2026-01-15T10:30:00Z"
}
```

## Limitations

- Max file size: 10MB
- Max paragraphs: 10,000
- Max text length: 1MB per paragraph
- Text-only (no images, complex tables yet)

## Dependencies

- `docx-rs` - WASM-compatible .docx library
- `serde` / `serde_json` - Serialization
- `wit-bindgen` - WASM Component Model bindings

## License

MIT OR Apache-2.0
