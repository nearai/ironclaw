//! Code templates for common tool patterns.
//!
//! Templates provide scaffolding that the LLM fills in, reducing the chance
//! of structural errors and ensuring consistent patterns.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Type of template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateType {
    /// WASM tool with HTTP capability.
    WasmHttpTool,
    /// WASM tool for data transformation.
    WasmTransformTool,
    /// WASM tool for computation.
    WasmComputeTool,
    /// CLI application.
    CliBinary,
    /// Python script.
    PythonScript,
    /// Bash script.
    BashScript,
}

/// A code template with placeholders.
#[derive(Debug, Clone)]
pub struct Template {
    pub template_type: TemplateType,
    pub name: &'static str,
    pub description: &'static str,
    pub files: Vec<TemplateFile>,
}

/// A file within a template.
#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub path: &'static str,
    pub content: &'static str,
    pub is_required: bool,
}

/// Engine for rendering templates with variable substitution.
#[derive(Debug, Default)]
pub struct TemplateEngine {
    variables: HashMap<String, String>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a template variable.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.variables.insert(key.into(), value.into());
        self
    }

    /// Render a template string, replacing {{variable}} placeholders.
    pub fn render(&self, template: &str) -> String {
        let mut result = template.to_string();
        for (key, value) in &self.variables {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value);
        }
        result
    }

    /// Render all files in a template.
    pub fn render_template(&self, template: &Template) -> Vec<(String, String)> {
        template
            .files
            .iter()
            .map(|f| (self.render(f.path), self.render(f.content)))
            .collect()
    }
}

impl Template {
    /// Get template by type.
    pub fn get(template_type: TemplateType) -> Self {
        match template_type {
            TemplateType::WasmHttpTool => Self::wasm_http_tool(),
            TemplateType::WasmTransformTool => Self::wasm_transform_tool(),
            TemplateType::WasmComputeTool => Self::wasm_compute_tool(),
            TemplateType::CliBinary => Self::cli_binary(),
            TemplateType::PythonScript => Self::python_script(),
            TemplateType::BashScript => Self::bash_script(),
        }
    }

    fn wasm_http_tool() -> Self {
        Self {
            template_type: TemplateType::WasmHttpTool,
            name: "WASM HTTP Tool",
            description: "A WASM tool that makes HTTP requests to external APIs",
            files: vec![
                TemplateFile {
                    path: "Cargo.toml",
                    content: WASM_CARGO_TOML,
                    is_required: true,
                },
                TemplateFile {
                    path: "src/lib.rs",
                    content: WASM_HTTP_LIB_RS,
                    is_required: true,
                },
            ],
        }
    }

    fn wasm_transform_tool() -> Self {
        Self {
            template_type: TemplateType::WasmTransformTool,
            name: "WASM Transform Tool",
            description: "A WASM tool that transforms data (JSON, text, etc.)",
            files: vec![
                TemplateFile {
                    path: "Cargo.toml",
                    content: WASM_CARGO_TOML,
                    is_required: true,
                },
                TemplateFile {
                    path: "src/lib.rs",
                    content: WASM_TRANSFORM_LIB_RS,
                    is_required: true,
                },
            ],
        }
    }

    fn wasm_compute_tool() -> Self {
        Self {
            template_type: TemplateType::WasmComputeTool,
            name: "WASM Compute Tool",
            description: "A WASM tool for pure computation (no I/O)",
            files: vec![
                TemplateFile {
                    path: "Cargo.toml",
                    content: WASM_CARGO_TOML,
                    is_required: true,
                },
                TemplateFile {
                    path: "src/lib.rs",
                    content: WASM_COMPUTE_LIB_RS,
                    is_required: true,
                },
            ],
        }
    }

    fn cli_binary() -> Self {
        Self {
            template_type: TemplateType::CliBinary,
            name: "CLI Binary",
            description: "A command-line application with argument parsing",
            files: vec![
                TemplateFile {
                    path: "Cargo.toml",
                    content: CLI_CARGO_TOML,
                    is_required: true,
                },
                TemplateFile {
                    path: "src/main.rs",
                    content: CLI_MAIN_RS,
                    is_required: true,
                },
            ],
        }
    }

    fn python_script() -> Self {
        Self {
            template_type: TemplateType::PythonScript,
            name: "Python Script",
            description: "A Python script with argument parsing",
            files: vec![TemplateFile {
                path: "{{name}}.py",
                content: PYTHON_SCRIPT,
                is_required: true,
            }],
        }
    }

    fn bash_script() -> Self {
        Self {
            template_type: TemplateType::BashScript,
            name: "Bash Script",
            description: "A Bash script with argument handling",
            files: vec![TemplateFile {
                path: "{{name}}.sh",
                content: BASH_SCRIPT,
                is_required: true,
            }],
        }
    }
}

// =============================================================================
// WASM Templates
// =============================================================================

const WASM_CARGO_TOML: &str = r##"[package]
name = "{{name}}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
"##;

const WASM_HTTP_LIB_RS: &str = r##"//! {{description}}
//!
//! This WASM tool makes HTTP requests to external APIs.

use serde::{Deserialize, Serialize};

// Host function imports
#[link(wasm_import_module = "env")]
extern "C" {
    fn host_log(level: i32, ptr: *const u8, len: usize);
    fn host_http_request(
        method_ptr: *const u8, method_len: usize,
        url_ptr: *const u8, url_len: usize,
        headers_ptr: *const u8, headers_len: usize,
        body_ptr: *const u8, body_len: usize,
        response_ptr: *mut u8, response_max_len: usize,
    ) -> i32;
}

fn log_info(msg: &str) {
    unsafe { host_log(1, msg.as_ptr(), msg.len()); }
}

fn http_get(url: &str) -> Result<String, String> {
    let method = "GET";
    let mut response_buf = vec![0u8; 65536];
    let result = unsafe {
        host_http_request(
            method.as_ptr(), method.len(),
            url.as_ptr(), url.len(),
            std::ptr::null(), 0,
            std::ptr::null(), 0,
            response_buf.as_mut_ptr(), response_buf.len(),
        )
    };
    if result < 0 { return Err(format!("HTTP error: {}", result)); }
    response_buf.truncate(result as usize);
    String::from_utf8(response_buf).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct Input {
    {{input_fields}}
}

#[derive(Serialize)]
struct Output {
    {{output_fields}}
}

#[no_mangle]
pub extern "C" fn run(input_ptr: *const u8, input_len: usize) -> u64 {
    let result = run_inner(input_ptr, input_len);
    let json = match result {
        Ok(output) => serde_json::to_string(&output).unwrap_or_else(|e| {
            format!("{{\"error\":\"serialize: {}\"}}", e)
        }),
        Err(e) => format!("{{\"error\":\"{}\"}}", e.replace('"', "'")),
    };
    let bytes = json.into_bytes();
    let ptr = bytes.as_ptr() as u64;
    let len = bytes.len() as u64;
    std::mem::forget(bytes);
    (len << 32) | ptr
}

fn run_inner(input_ptr: *const u8, input_len: usize) -> Result<Output, String> {
    let input_bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let input: Input = serde_json::from_slice(input_bytes)
        .map_err(|e| format!("Invalid input: {}", e))?;

    log_info("Processing request...");

    {{implementation}}

    Ok(Output {
        {{output_construction}}
    })
}
"##;

const WASM_TRANSFORM_LIB_RS: &str = r##"//! {{description}}
//!
//! This WASM tool transforms input data.

use serde::{Deserialize, Serialize};

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_log(level: i32, ptr: *const u8, len: usize);
}

fn log_info(msg: &str) {
    unsafe { host_log(1, msg.as_ptr(), msg.len()); }
}

#[derive(Deserialize)]
struct Input {
    {{input_fields}}
}

#[derive(Serialize)]
struct Output {
    {{output_fields}}
}

#[no_mangle]
pub extern "C" fn run(input_ptr: *const u8, input_len: usize) -> u64 {
    let result = run_inner(input_ptr, input_len);
    let json = match result {
        Ok(output) => serde_json::to_string(&output).unwrap_or_else(|e| {
            format!("{{\"error\":\"serialize: {}\"}}", e)
        }),
        Err(e) => format!("{{\"error\":\"{}\"}}", e.replace('"', "'")),
    };
    let bytes = json.into_bytes();
    let ptr = bytes.as_ptr() as u64;
    let len = bytes.len() as u64;
    std::mem::forget(bytes);
    (len << 32) | ptr
}

fn run_inner(input_ptr: *const u8, input_len: usize) -> Result<Output, String> {
    let input_bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let input: Input = serde_json::from_slice(input_bytes)
        .map_err(|e| format!("Invalid input: {}", e))?;

    log_info("Transforming data...");

    {{implementation}}

    Ok(Output {
        {{output_construction}}
    })
}
"##;

const WASM_COMPUTE_LIB_RS: &str = r##"//! {{description}}
//!
//! This WASM tool performs pure computation.

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input {
    {{input_fields}}
}

#[derive(Serialize)]
struct Output {
    {{output_fields}}
}

#[no_mangle]
pub extern "C" fn run(input_ptr: *const u8, input_len: usize) -> u64 {
    let result = run_inner(input_ptr, input_len);
    let json = match result {
        Ok(output) => serde_json::to_string(&output).unwrap_or_else(|e| {
            format!("{{\"error\":\"serialize: {}\"}}", e)
        }),
        Err(e) => format!("{{\"error\":\"{}\"}}", e.replace('"', "'")),
    };
    let bytes = json.into_bytes();
    let ptr = bytes.as_ptr() as u64;
    let len = bytes.len() as u64;
    std::mem::forget(bytes);
    (len << 32) | ptr
}

fn run_inner(input_ptr: *const u8, input_len: usize) -> Result<Output, String> {
    let input_bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let input: Input = serde_json::from_slice(input_bytes)
        .map_err(|e| format!("Invalid input: {}", e))?;

    {{implementation}}

    Ok(Output {
        {{output_construction}}
    })
}
"##;

// =============================================================================
// CLI Templates
// =============================================================================

const CLI_CARGO_TOML: &str = r##"[package]
name = "{{name}}"
version = "0.1.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
"##;

const CLI_MAIN_RS: &str = r##"//! {{description}}

use clap::Parser;
use anyhow::Result;

#[derive(Parser, Debug)]
#[command(name = "{{name}}")]
#[command(about = "{{description}}")]
struct Args {
    {{cli_args}}
}

fn main() -> Result<()> {
    let args = Args::parse();

    {{implementation}}

    Ok(())
}
"##;

// =============================================================================
// Script Templates
// =============================================================================

const PYTHON_SCRIPT: &str = r##"#!/usr/bin/env python3
"""{{description}}"""

import argparse
import json
import sys


def main():
    parser = argparse.ArgumentParser(description="{{description}}")
    {{python_args}}
    args = parser.parse_args()

    {{implementation}}


if __name__ == "__main__":
    main()
"##;

const BASH_SCRIPT: &str = r##"#!/bin/bash
# {{description}}

set -euo pipefail

usage() {
    echo "Usage: $0 {{bash_usage}}"
    exit 1
}

{{bash_arg_parsing}}

{{implementation}}
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_engine() {
        let mut engine = TemplateEngine::new();
        engine.set("name", "my_tool");
        engine.set("description", "A cool tool");

        let result = engine.render("Name: {{name}}, Desc: {{description}}");
        assert_eq!(result, "Name: my_tool, Desc: A cool tool");
    }

    #[test]
    fn test_get_template() {
        let template = Template::get(TemplateType::WasmHttpTool);
        assert_eq!(template.name, "WASM HTTP Tool");
        assert!(!template.files.is_empty());
    }
}
