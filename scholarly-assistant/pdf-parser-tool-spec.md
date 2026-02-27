# PDF Parser WASM Tool Specification

## Purpose

Extract text and structure from academic PDF papers for analysis and summarization.

## Directory Structure

```
tools-src/pdf-parser/
├── Cargo.toml
├── pdf-parser.capabilities.json
├── src/
│   └── lib.rs
└── README.md
```

## Cargo.toml

```toml
[package]
name = "pdf-parser-tool"
version = "0.1.0"
edition = "2021"
description = "PDF text extraction and structure detection for academic papers"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
wit-bindgen = "0.41.0"
# Note: pdf-extract doesn't compile to WASM32
# Alternative: lopdf (basic PDF reading)
# For production: Use host-side tool or MCP server with PyPDF2/pdfplumber

[lib]
crate-type = ["cdylib"]

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1

[workspace]
```

## Capabilities File (pdf-parser.capabilities.json)

```json
{
  "name": "pdf_parser",
  "version": "0.1.0",
  "description": "Extract text and structure from PDF files",
  "http_endpoints": [],
  "secrets": [],
  "tools": [
    {
      "name": "parse_pdf",
      "description": "Extract text from a PDF file path or URL",
      "parameters": {
        "type": "object",
        "properties": {
          "source": {
            "type": "string",
            "description": "File path or HTTP URL to PDF"
          },
          "extract_structure": {
            "type": "boolean",
            "description": "Whether to detect paper structure (Abstract, Methods, etc.)",
            "default": true
          },
          "extract_citations": {
            "type": "boolean",
            "description": "Whether to extract citations/references",
            "default": false
          }
        },
        "required": ["source"]
      }
    }
  ]
}
```

## Implementation Strategy

### Challenge: WASM PDF Parsing

Most PDF libraries (pdf-extract, PyPDF2, pdfplumber) don't compile to WASM or require system calls.

### Solution Options:

#### Option 1: Built-in Tool (Recommended)

Implement as a **built-in Rust tool** in `src/tools/builtin/pdf.rs`:

```rust
// src/tools/builtin/pdf.rs
use async_trait::async_trait;
use std::path::Path;
use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolOutput, ToolError};

pub struct PdfParserTool;

#[async_trait]
impl Tool for PdfParserTool {
    fn name(&self) -> &str {
        "parse_pdf"
    }

    fn description(&self) -> &str {
        "Extract text and structure from PDF files"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to PDF file in workspace"
                },
                "extract_structure": {
                    "type": "boolean",
                    "description": "Detect paper structure (Abstract, Methods, etc.)",
                    "default": true
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'path'".to_string()))?;

        // TODO: Implement PDF parsing with lopdf or pdf-extract
        // For now, return placeholder
        let result = serde_json::json!({
            "text": "PDF text would be extracted here",
            "pages": 10,
            "structure": {
                "abstract": "Abstract section...",
                "introduction": "Introduction section...",
                "methods": "Methods section...",
                "results": "Results section...",
                "conclusion": "Conclusion section..."
            }
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // PDF content could be malicious
    }
}
```

#### Option 2: MCP Server (Best for PhD Use)

Create a Python MCP server using pdfplumber (better text extraction):

```python
#!/usr/bin/env python3
"""PDF Parser MCP Server for IronClaw."""
import json
import sys
import re
from pathlib import Path

try:
    import pdfplumber
except ImportError:
    print("Error: pdfplumber not installed. Run: pip install pdfplumber", file=sys.stderr)
    sys.exit(1)


def extract_text(pdf_path, extract_structure=True):
    """Extract text from PDF and optionally detect structure."""
    with pdfplumber.open(pdf_path) as pdf:
        full_text = ""
        pages_text = []

        for page in pdf.pages:
            text = page.extract_text()
            if text:
                pages_text.append(text)
                full_text += text + "\n"

        result = {
            "text": full_text,
            "pages": len(pdf.pages),
            "page_texts": pages_text
        }

        if extract_structure:
            result["structure"] = detect_paper_structure(full_text)

        return result


def detect_paper_structure(text):
    """Detect academic paper structure using section headers."""
    structure = {}

    # Common section patterns
    sections = {
        "abstract": r"(?i)(abstract|summary)\s*\n",
        "introduction": r"(?i)(introduction|1\.?\s+introduction)\s*\n",
        "methods": r"(?i)(methods?|methodology|materials and methods|2\.?\s+methods?)\s*\n",
        "results": r"(?i)(results?|findings|3\.?\s+results?)\s*\n",
        "discussion": r"(?i)(discussion|4\.?\s+discussion)\s*\n",
        "conclusion": r"(?i)(conclusions?|summary|5\.?\s+conclusions?)\s*\n",
        "references": r"(?i)(references|bibliography|works cited)\s*\n",
    }

    for section_name, pattern in sections.items():
        match = re.search(pattern, text)
        if match:
            start = match.end()
            # Find next section or end
            next_match = None
            for other_pattern in sections.values():
                m = re.search(other_pattern, text[start:])
                if m and (next_match is None or m.start() < next_match):
                    next_match = m.start()

            if next_match:
                section_text = text[start:start + next_match].strip()
            else:
                section_text = text[start:start + 1000].strip()  # Max 1000 chars

            structure[section_name] = section_text[:500]  # Limit to 500 chars

    return structure


def handle_parse_pdf(params):
    """Handle parse_pdf tool request."""
    source = params.get("source")
    if not source:
        return {"error": "Missing 'source' parameter"}

    extract_structure = params.get("extract_structure", True)

    try:
        path = Path(source).expanduser()
        if not path.exists():
            return {"error": f"File not found: {source}"}

        result = extract_text(path, extract_structure)
        return result

    except Exception as e:
        return {"error": str(e)}


def main():
    """MCP server main loop."""
    print(json.dumps({
        "jsonrpc": "2.0",
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "pdf-parser",
                "version": "0.1.0"
            }
        }
    }))
    sys.stdout.flush()

    for line in sys.stdin:
        try:
            request = json.loads(line)
            method = request.get("method")

            if method == "tools/list":
                response = {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": {
                        "tools": [
                            {
                                "name": "parse_pdf",
                                "description": "Extract text and structure from PDF files",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "source": {
                                            "type": "string",
                                            "description": "Path to PDF file"
                                        },
                                        "extract_structure": {
                                            "type": "boolean",
                                            "description": "Detect paper structure",
                                            "default": True
                                        }
                                    },
                                    "required": ["source"]
                                }
                            }
                        ]
                    }
                }

            elif method == "tools/call":
                tool_name = request.get("params", {}).get("name")
                arguments = request.get("params", {}).get("arguments", {})

                if tool_name == "parse_pdf":
                    result = handle_parse_pdf(arguments)
                    response = {
                        "jsonrpc": "2.0",
                        "id": request.get("id"),
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": json.dumps(result, indent=2)
                                }
                            ]
                        }
                    }
                else:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request.get("id"),
                        "error": {
                            "code": -32601,
                            "message": f"Unknown tool: {tool_name}"
                        }
                    }
            else:
                response = {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "error": {
                        "code": -32601,
                        "message": f"Unknown method: {method}"
                        }
                }

            print(json.dumps(response))
            sys.stdout.flush()

        except Exception as e:
            error_response = {
                "jsonrpc": "2.0",
                "id": request.get("id") if 'request' in locals() else None,
                "error": {
                    "code": -32603,
                    "message": str(e)
                }
            }
            print(json.dumps(error_response), file=sys.stderr)
            sys.stdout.flush()


if __name__ == "__main__":
    main()
```

Save as `pdf_parser_server.py` and add to the HTTP wrapper.

## Testing

```bash
# Test with a sample PDF
python pdf_parser_server.py
# Send request:
{"jsonrpc":"2.0","method":"tools/list","id":1}

{"jsonrpc":"2.0","method":"tools/call","params":{"name":"parse_pdf","arguments":{"source":"~/Downloads/paper.pdf"}},"id":2}
```

## Usage in IronClaw

```
> Parse PDF /path/to/paper.pdf
> Extract abstract and methods from paper.pdf
> Summarize the PDF at ~/Downloads/research.pdf
```

## Future Enhancements

1. **Citation extraction** - Parse References section
2. **Figure/table extraction** - Extract images and tables
3. **Metadata extraction** - Authors, title, DOI from PDF metadata
4. **OCR support** - Handle scanned PDFs
5. **Batch processing** - Process multiple PDFs at once

## Integration with Literature Review

Once PDF text is extracted:
1. Store in workspace/papers/[filename].md
2. Run structure analysis
3. Extract and link citations
4. Add to bibliography automatically

---

*Specification by Andy*
*For Joaquín's PhD project*
