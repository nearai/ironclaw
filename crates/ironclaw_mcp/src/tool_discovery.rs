use ironclaw_extensions::{
    HostedMcpDiscoveredTool, HostedMcpDiscoveredToolAnnotations, HostedMcpToolCandidate,
    HostedMcpToolPublicationDisposition, HostedMcpToolRejection, HostedMcpToolRejectionReason,
};
use serde_json::Value;

const MAX_DISCOVERED_TOOLS: usize = 128;
const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 2048;
const MAX_SCHEMA_DEPTH: u8 = 8;
const MAX_SCHEMA_NODES: usize = 512;
const MAX_SCHEMA_STRING_BYTES: usize = 1024;
const INVALID_TOOL_LIST_REASON: &str = "mcp_invalid_tool_list";

#[derive(Debug)]
pub(crate) struct ParsedMcpToolList {
    pub(crate) tools: Vec<HostedMcpToolCandidate>,
    pub(crate) rejections: Vec<HostedMcpToolRejection>,
}

pub(crate) fn parse_tools_list_result(value: &Value) -> Result<ParsedMcpToolList, String> {
    let invalid_tool_list = || INVALID_TOOL_LIST_REASON.to_string();
    let raw_tools = value
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(invalid_tool_list)?;
    if raw_tools.len() > MAX_DISCOVERED_TOOLS {
        return Err(invalid_tool_list());
    }

    let mut tools = Vec::with_capacity(raw_tools.len());
    let mut rejections = Vec::new();
    for (source_index, raw_tool) in raw_tools.iter().enumerate() {
        let Some(name) = raw_tool.get("name").and_then(Value::as_str) else {
            return Err(invalid_tool_list());
        };
        if !is_supported_mcp_tool_name(name, MAX_TOOL_NAME_BYTES) {
            rejections.push(HostedMcpToolRejection {
                tool_index: source_index,
                reason: HostedMcpToolRejectionReason::UnsupportedName,
            });
            continue;
        }
        let description = raw_tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        if description.len() > MAX_TOOL_DESCRIPTION_BYTES
            || description.chars().any(is_unsupported_description_char)
        {
            return Err(invalid_tool_list());
        }
        let input_schema = raw_tool
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .cloned()
            .ok_or_else(invalid_tool_list)?;
        if !is_supported_mcp_input_schema(
            &input_schema,
            MAX_SCHEMA_DEPTH,
            MAX_SCHEMA_NODES,
            MAX_SCHEMA_STRING_BYTES,
        ) {
            return Err(invalid_tool_list());
        }
        let annotations = parse_tool_annotations(raw_tool.get("annotations"))?;
        tools.push(HostedMcpToolCandidate {
            source_index,
            tool: HostedMcpDiscoveredTool {
                name: name.to_string(),
                description: description.to_string(),
                input_schema,
                annotations,
            },
            disposition: HostedMcpToolPublicationDisposition::ModelVisible,
        });
    }

    Ok(ParsedMcpToolList { tools, rejections })
}

fn is_supported_mcp_input_schema(
    schema: &Value,
    max_depth: u8,
    max_nodes: usize,
    max_string_bytes: usize,
) -> bool {
    let mut nodes = 0usize;
    validate_mcp_schema_value(
        schema,
        0,
        max_depth,
        max_nodes,
        max_string_bytes,
        &mut nodes,
    )
}

fn validate_mcp_schema_value(
    value: &Value,
    depth: u8,
    max_depth: u8,
    max_nodes: usize,
    max_string_bytes: usize,
    nodes: &mut usize,
) -> bool {
    if depth > max_depth {
        return false;
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > max_nodes {
        return false;
    }
    match value {
        Value::String(value) => {
            value.len() <= max_string_bytes && !value.chars().any(is_unsupported_description_char)
        }
        Value::Array(values) => values.iter().all(|value| {
            validate_mcp_schema_value(
                value,
                depth + 1,
                max_depth,
                max_nodes,
                max_string_bytes,
                nodes,
            )
        }),
        Value::Object(values) => values.values().all(|value| {
            validate_mcp_schema_value(
                value,
                depth + 1,
                max_depth,
                max_nodes,
                max_string_bytes,
                nodes,
            )
        }),
        _ => true,
    }
}

fn is_unsupported_description_char(value: char) -> bool {
    value.is_control() && !matches!(value, '\n' | '\r' | '\t')
}

fn parse_tool_annotations(
    value: Option<&Value>,
) -> Result<HostedMcpDiscoveredToolAnnotations, String> {
    let Some(value) = value else {
        return Ok(HostedMcpDiscoveredToolAnnotations::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| INVALID_TOOL_LIST_REASON.to_string())?;
    Ok(HostedMcpDiscoveredToolAnnotations {
        destructive_hint: object
            .get("destructiveHint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        side_effects_hint: object
            .get("sideEffectsHint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        read_only_hint: object
            .get("readOnlyHint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub(crate) fn is_supported_mcp_tool_name(value: &str, max_bytes: usize) -> bool {
    if value.is_empty() || value.len() > max_bytes || value.contains("..") {
        return false;
    }
    value.split('.').all(is_supported_mcp_tool_name_segment)
}

fn is_supported_mcp_tool_name_segment(segment: &str) -> bool {
    let Some(first) = segment.as_bytes().first().copied() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    segment.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    })
}
