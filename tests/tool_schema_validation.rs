//! Validates that all built-in tool schemas conform to OpenAI strict-mode rules.
//!
//! This catches the class of bugs where `required` keys aren't in `properties`,
//! properties are missing `type` (intentional freeform is allowed), or nested
//! objects/arrays are malformed.
//!
//! See: <https://github.com/nearai/ironclaw/issues/352> (QA plan, item 1.1)

use ironclaw::tools::validate_tool_schema;
use ironclaw::tools::{Tool, ToolRegistry};

/// Validate schemas of all tools registered via `register_builtin_tools()` and
/// `register_dev_tools()` (echo, time, json, http, shell, file tools).
///
/// These tools can be constructed without external dependencies (no DB, no
/// workspace, no extension manager). Tools requiring dependencies (memory, job,
/// skill, extension, routine) are validated individually below where test
/// construction helpers exist.
#[tokio::test]
async fn all_core_builtin_tool_schemas_are_valid() {
    let registry = ToolRegistry::new();
    registry.register_builtin_tools();
    registry.register_dev_tools();

    let tools = registry.all().await;
    assert!(
        !tools.is_empty(),
        "registry should have tools after registration"
    );

    let mut all_errors = Vec::new();
    for tool in &tools {
        let schema = tool.parameters_schema();
        let errors = validate_tool_schema(&schema, tool.name());
        if !errors.is_empty() {
            all_errors.push(format!(
                "Tool '{}' has schema errors:\n  {}",
                tool.name(),
                errors.join("\n  ")
            ));
        }
    }

    assert!(
        all_errors.is_empty(),
        "Tool schema validation failures:\n{}",
        all_errors.join("\n\n")
    );
}

/// Verify the exact set of tools registered by the core registration methods.
/// This guards against a new tool being added without schema validation coverage.
#[tokio::test]
async fn core_registration_covers_expected_tools() {
    let registry = ToolRegistry::new();
    registry.register_builtin_tools();
    registry.register_dev_tools();

    let mut names = registry.list().await;
    names.sort();

    let expected = &[
        "apply_patch",
        "echo",
        "http",
        "json",
        "list_dir",
        "read_file",
        "shell",
        "time",
        "web_fetch",
        "write_file",
    ];

    assert_eq!(
        names, expected,
        "Core tool set changed. Update this test and ensure new tools have valid schemas."
    );
}

/// Validate individual tool schemas that are known to use non-trivial patterns.
/// These are regression tests for specific bugs.
#[test]
fn json_tool_freeform_data_field_is_valid() {
    // Regression: json tool's "data" field intentionally has no "type" for
    // OpenAI compatibility (union types with arrays require "items").
    let tool = ironclaw::tools::builtin::JsonTool;
    let schema = tool.parameters_schema();
    let errors = validate_tool_schema(&schema, "json");
    assert!(errors.is_empty(), "json tool schema errors: {errors:?}");

    // Verify the freeform pattern is still in place
    let data = schema
        .get("properties")
        .and_then(|p| p.get("data"))
        .expect("json tool should have 'data' property");
    assert!(
        data.get("type").is_none(),
        "json.data should be freeform (no type) for OpenAI compatibility"
    );
}

#[test]
fn http_tool_headers_array_is_valid() {
    // Regression: http tool's "headers" is an array of {name, value} objects.
    let tool = ironclaw::tools::builtin::HttpTool::new();
    let schema = tool.parameters_schema();
    let errors = validate_tool_schema(&schema, "http");
    assert!(errors.is_empty(), "http tool schema errors: {errors:?}");

    // Verify array structure
    let headers = schema
        .get("properties")
        .and_then(|p| p.get("headers"))
        .expect("http tool should have 'headers' property");
    assert_eq!(
        headers.get("type").and_then(|t| t.as_str()),
        Some("array"),
        "headers should be an array"
    );
    assert!(
        headers.get("items").is_some(),
        "headers array should have items defined"
    );
}

#[test]
fn time_tool_schema_is_valid() {
    let tool = ironclaw::tools::builtin::TimeTool;
    let schema = tool.parameters_schema();
    let errors = validate_tool_schema(&schema, "time");
    assert!(errors.is_empty(), "time tool schema errors: {errors:?}");
}

#[test]
fn shell_tool_schema_is_valid() {
    let tool = ironclaw::tools::builtin::ShellTool::new();
    let schema = tool.parameters_schema();
    let errors = validate_tool_schema(&schema, "shell");
    assert!(errors.is_empty(), "shell tool schema errors: {errors:?}");
}
