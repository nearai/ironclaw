//! Microsoft Word (.docx) WASM Tool for IronClaw.
//!
//! Provides secure Word document operations: reading, writing, and modifying .docx files.
//!
//! # Security
//!
//! Runs in IronClaw's WASM sandbox with:
//! - Fuel metering to prevent infinite loops
//! - Memory limits
//! - File access restricted to workspace paths in capabilities
//! - No network access (pure file operations)

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};
use exports::near::agent::tool::{Guest, Request, Response};

const MAX_TEXT_LENGTH: usize = 1_000_000; // 1MB text limit
const MAX_PARAGRAPHS: usize = 10_000;

/// Validate input length to prevent oversized payloads.
fn validate_input_length(s: &str, field_name: &str) -> Result<(), String> {
    if s.len() > MAX_TEXT_LENGTH {
        return Err(format!(
            "Input '{}' exceeds maximum length of {} characters",
            field_name, MAX_TEXT_LENGTH
        ));
    }
    Ok(())
}

struct WordTool;

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum WordAction {
    /// Read a .docx file and extract text content
    #[serde(rename = "read_docx")]
    ReadDocx {
        /// Path to .docx file (relative to workspace)
        path: String,
        /// Include formatting metadata (default: false)
        include_formatting: Option<bool>,
    },

    /// Create a new .docx document
    #[serde(rename = "create_docx")]
    CreateDocx {
        /// Output path for new .docx file
        path: String,
        /// Document title
        title: Option<String>,
        /// Initial paragraphs to add
        paragraphs: Vec<String>,
    },

    /// Add paragraphs to an existing .docx document
    #[serde(rename = "append_docx")]
    AppendDocx {
        /// Path to existing .docx file
        path: String,
        /// Paragraphs to append
        paragraphs: Vec<String>,
    },

    /// Extract document metadata
    #[serde(rename = "get_metadata")]
    GetMetadata {
        /// Path to .docx file
        path: String,
    },
}

#[derive(Debug, Serialize)]
struct ReadDocxResult {
    paragraphs: Vec<String>,
    paragraph_count: usize,
    character_count: usize,
    word_count: usize,
}

#[derive(Debug, Serialize)]
struct CreateDocxResult {
    success: bool,
    path: String,
    paragraph_count: usize,
    docx_bytes_base64: String,
}

#[derive(Debug, Serialize)]
struct MetadataResult {
    title: Option<String>,
    subject: Option<String>,
    creator: Option<String>,
    created: Option<String>,
}

impl Guest for WordTool {
    fn execute(req: Request) -> Response {
        let action: WordAction = match serde_json::from_str(&req.params) {
            Ok(a) => a,
            Err(e) => {
                return Response {
                    output: None,
                    error: Some(format!("Invalid parameters: {}", e)),
                }
            }
        };

        match action {
            WordAction::ReadDocx { path, include_formatting } => {
                execute_read_docx(&path, include_formatting.unwrap_or(false))
            }
            WordAction::CreateDocx { path, title, paragraphs } => {
                execute_create_docx(&path, title, paragraphs)
            }
            WordAction::AppendDocx { path, paragraphs } => {
                execute_append_docx(&path, paragraphs)
            }
            WordAction::GetMetadata { path } => {
                execute_get_metadata(&path)
            }
        }
    }

    fn schema() -> String {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read_docx", "create_docx", "append_docx", "get_metadata"],
                    "description": "Operation to perform on Word document"
                },
                "path": {
                    "type": "string",
                    "description": "Path to .docx file (relative to workspace)"
                },
                "include_formatting": {
                    "type": "boolean",
                    "description": "Include formatting metadata when reading (default: false)"
                },
                "title": {
                    "type": "string",
                    "description": "Document title for new documents"
                },
                "paragraphs": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Text paragraphs to add to document"
                }
            },
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "read_docx" },
                        "path": {},
                        "include_formatting": {}
                    },
                    "required": ["action", "path"]
                },
                {
                    "properties": {
                        "action": { "const": "create_docx" },
                        "path": {},
                        "title": {},
                        "paragraphs": {}
                    },
                    "required": ["action", "path", "paragraphs"]
                },
                {
                    "properties": {
                        "action": { "const": "append_docx" },
                        "path": {},
                        "paragraphs": {}
                    },
                    "required": ["action", "path", "paragraphs"]
                },
                {
                    "properties": {
                        "action": { "const": "get_metadata" },
                        "path": {}
                    },
                    "required": ["action", "path"]
                }
            ]
        }).to_string()
    }

    fn description() -> String {
        "Read, write, and modify Microsoft Word (.docx) documents. \
         Supports extracting text, creating new documents, appending content, \
         and reading metadata. All file operations are sandboxed to the workspace."
            .to_string()
    }
}

fn execute_read_docx(path: &str, _include_formatting: bool) -> Response {
    use near::agent::host::workspace_read;

    // Read file from workspace
    let content_bytes = match workspace_read(path) {
        Some(data) => data.into_bytes(),
        None => {
            return Response {
                output: None,
                error: Some(format!("File not found or access denied: {}", path)),
            }
        }
    };

    // Parse .docx file using docx-rs
    // read_docx() expects a byte slice and returns the parsed document
    let docx = match docx_rs::read_docx(&content_bytes) {
        Ok(doc) => doc,
        Err(e) => {
            return Response {
                output: None,
                error: Some(format!("Failed to parse .docx file: {:?}", e)),
            }
        }
    };

    // Extract text from paragraphs
    let json_value = docx.json();

    // Parse the JSON to extract paragraph text
    let parsed: serde_json::Value = match serde_json::from_str(&json_value) {
        Ok(v) => v,
        Err(e) => {
            return Response {
                output: None,
                error: Some(format!("Failed to parse document JSON: {}", e)),
            }
        }
    };

    // Extract paragraphs from the JSON structure
    let mut paragraphs = Vec::new();
    let mut total_chars = 0;
    let mut total_words = 0;

    if let Some(children) = parsed["document"]["children"].as_array() {
        for child in children {
            if let Some(data) = child["data"]["Paragraph"].as_object() {
                let mut para_text = String::new();

                if let Some(children) = data["children"].as_array() {
                    for run_child in children {
                        if let Some(run_data) = run_child["data"]["Run"].as_object() {
                            if let Some(run_children) = run_data["children"].as_array() {
                                for text_child in run_children {
                                    if let Some(text_data) = text_child["data"]["Text"].as_object() {
                                        if let Some(text) = text_data["text"].as_str() {
                                            para_text.push_str(text);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !para_text.is_empty() {
                    total_chars += para_text.len();
                    total_words += para_text.split_whitespace().count();
                    paragraphs.push(para_text);
                }
            }
        }
    }

    let result = ReadDocxResult {
        paragraphs,
        paragraph_count: paragraphs.len(),
        character_count: total_chars,
        word_count: total_words,
    };

    Response {
        output: Some(serde_json::to_string(&result).unwrap()),
        error: None,
    }
}

fn execute_create_docx(path: &str, _title: Option<String>, paragraphs: Vec<String>) -> Response {
    use docx_rs::{Docx, Paragraph, Run};

    // Validate inputs
    if paragraphs.len() > MAX_PARAGRAPHS {
        return Response {
            output: None,
            error: Some(format!("Too many paragraphs: {} (max: {})", paragraphs.len(), MAX_PARAGRAPHS)),
        };
    }

    for (i, p) in paragraphs.iter().enumerate() {
        if let Err(e) = validate_input_length(p, &format!("paragraph[{}]", i)) {
            return Response {
                output: None,
                error: Some(e),
            };
        }
    }

    // Build document
    let mut docx = Docx::new();

    for paragraph_text in paragraphs.iter() {
        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new().add_text(paragraph_text)
            )
        );
    }

    // Build and pack to bytes
    let mut buffer = Vec::new();
    match docx.build().pack(&mut buffer) {
        Ok(_) => {},
        Err(e) => {
            return Response {
                output: None,
                error: Some(format!("Failed to build .docx: {:?}", e)),
            }
        }
    }

    // Encode bytes as base64 for JSON transport
    let base64_bytes = base64_encode(&buffer);

    let result = CreateDocxResult {
        success: true,
        path: path.to_string(),
        paragraph_count: paragraphs.len(),
        docx_bytes_base64: base64_bytes,
    };

    Response {
        output: Some(serde_json::to_string(&result).unwrap()),
        error: None,
    }
}

/// Simple base64 encoding without external dependencies
fn base64_encode(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();

    let mut i = 0;
    while i < bytes.len() {
        let b1 = bytes[i];
        let b2 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b3 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };

        result.push(CHARS[(b1 >> 2) as usize] as char);
        result.push(CHARS[(((b1 & 0x03) << 4) | (b2 >> 4)) as usize] as char);
        result.push(if i + 1 < bytes.len() { CHARS[(((b2 & 0x0f) << 2) | (b3 >> 6)) as usize] as char } else { '=' });
        result.push(if i + 2 < bytes.len() { CHARS[(b3 & 0x3f) as usize] as char } else { '=' });

        i += 3;
    }

    result
}

fn execute_append_docx(_path: &str, _paragraphs: Vec<String>) -> Response {
    Response {
        output: None,
        error: Some("append_docx implementation in progress".to_string()),
    }
}

fn execute_get_metadata(_path: &str) -> Response {
    Response {
        output: None,
        error: Some("get_metadata implementation in progress".to_string()),
    }
}
