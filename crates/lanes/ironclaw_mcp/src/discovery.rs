//! `tools/list` catalog admission.
//!
//! A hosted MCP server advertises tools; this module decides which of them the
//! host will publish. It owns the ceilings (tool count, page count, aggregate
//! bytes), the per-tool classification that separates a shape-only defect from
//! a security/bounds violation, and the grammar a discovered tool name must
//! satisfy before it becomes a Reborn capability suffix. It never sends or
//! receives a request — `client` runs the loop and reads these same constants,
//! so the two enforcement points cannot drift apart.

use ironclaw_extension_contracts::hosted_mcp::{
    HostedMcpDiscoveredTool, HostedMcpDiscoveredToolAnnotations,
};
use serde_json::Value;

use crate::diagnostics::{McpInvalidToolListCause, invalid_tool_list};

/// Maximum number of tools accepted from a hosted MCP `tools/list` discovery
/// pass, across all pages. Shared by the discovery loop's running-total check
/// and [`parse_tools_list_result`]'s per-page cap so the two enforcement
/// points cannot drift apart.
///
/// Sized for a real integration catalog rather than a single vendor's server.
/// At the previous 1,024 a large catalog did not degrade -- it FAILED: exceeding
/// the cap aborted the whole discovery pass and the extension published zero
/// tools. Measured against a 47,337-tool MCP endpoint, discovery pulled six
/// pages, tripped the cap, and every `tool_search` afterwards ran against an
/// empty index; the agent reported "the extension is registered and installed
/// -- but the MCP server's tools are not publishing" and answered from nothing.
/// The tools are deferred (definitions never enter the model's context), so the
/// cost of a larger ceiling is host-side index memory, not prompt tokens.
pub(crate) const MAX_DISCOVERED_MCP_TOOLS: usize = 65_536;

/// Maximum number of `tools/list` pagination pages followed during a single
/// discovery pass.
///
/// Sized against [`MAX_DISCOVERED_MCP_TOOLS`], not chosen independently: a server free to
/// pick its own page size needs enough pages to deliver the tool ceiling. At 50 pages this
/// was the effective ceiling regardless of the tool limit -- a 47,337-tool catalog served
/// 200 per page stopped at exactly 10,000 tools (21%), because 50 pages ran out first.
pub(crate) const MAX_MCP_TOOLS_LIST_PAGES: usize = 512;

/// Maximum aggregate serialized bytes accepted across all `tools/list` pages
/// during a single discovery pass.
///
/// Raised alongside [`MAX_DISCOVERED_MCP_TOOLS`]: a catalog large enough to need
/// the higher tool ceiling also carries more schema bytes, and tripping this
/// limit had the same all-or-nothing consequence. A 47,337-tool catalog
/// serializes to roughly 44 MB.
pub(crate) const MAX_MCP_TOOLS_CATALOG_BYTES: usize = 96 * 1024 * 1024;

pub(crate) fn parse_tools_list_result(
    value: &Value,
    manifest_max_tools: u32,
) -> Result<Vec<HostedMcpDiscoveredTool>, String> {
    const MAX_TOOL_NAME_BYTES: usize = 128;
    const MAX_TOOL_DESCRIPTION_BYTES: usize = 2048;
    const MAX_SCHEMA_DEPTH: u8 = 32;
    const MAX_SCHEMA_NODES: usize = 8192;
    const MAX_SCHEMA_STRING_BYTES: usize = 16 * 1024;

    let tools = value
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_tool_list(McpInvalidToolListCause::MissingToolsArray))?;
    let manifest_max_tools = usize::try_from(manifest_max_tools)
        .unwrap_or(MAX_DISCOVERED_MCP_TOOLS)
        .min(MAX_DISCOVERED_MCP_TOOLS);
    if manifest_max_tools == 0 || tools.len() > manifest_max_tools {
        return Err(invalid_tool_list(McpInvalidToolListCause::TooManyTools));
    }

    // Catalog acceptance distinguishes shape-only defects from security/bounds
    // violations. A single tool with a shape-only defect (an unsupported name,
    // an invalid description, or malformed annotations) is dropped from this
    // generation and recorded, so one malformed entry cannot brick an otherwise
    // valid integration that has no prior generation to fall back to. A
    // security/bounds violation (missing or unsafe input schema — checked first
    // per tool so a co-occurring cosmetic defect cannot downgrade it — or a
    // catalog that overflows the host cap) still rejects the whole generation
    // with a stable safe subcause; the previous published generation, if any,
    // remains authoritative until a complete bounded catalog is discovered.
    let mut published = Vec::with_capacity(tools.len());
    let mut first_skipped_cause: Option<McpInvalidToolListCause> = None;
    for (index, tool) in tools.iter().enumerate() {
        match classify_discovered_tool(
            tool,
            MAX_TOOL_NAME_BYTES,
            MAX_TOOL_DESCRIPTION_BYTES,
            MAX_SCHEMA_DEPTH,
            MAX_SCHEMA_NODES,
            MAX_SCHEMA_STRING_BYTES,
        )
        .map_err(invalid_tool_list)?
        {
            DiscoveredToolClassification::Published(discovered) => published.push(discovered),
            DiscoveredToolClassification::SkippedShapeViolation(cause) => {
                first_skipped_cause.get_or_insert(cause);
                // Bounded, provider-neutral record: the tool index and stable
                // cause token only — never the raw provider-supplied content.
                tracing::debug!(
                    tool_index = index,
                    skip_cause = cause.stable_token(),
                    "skipping shape-nonconforming hosted MCP tool from discovery catalog"
                );
            }
        }
    }
    if published.is_empty()
        && let Some(cause) = first_skipped_cause
    {
        // Every advertised tool was shape-nonconforming: there is nothing to
        // publish, so fail this generation non-retryably with a stable subcause
        // rather than activating on an empty catalog. An empty provider list
        // (no tools advertised, nothing skipped) is left as an empty result the
        // caller treats as "no tools discovered yet".
        return Err(invalid_tool_list(cause));
    }
    Ok(published)
}

pub(crate) fn parse_tools_list_page(
    value: &Value,
) -> Result<(Vec<HostedMcpDiscoveredTool>, Option<String>), String> {
    let tools = parse_tools_list_result(value, MAX_DISCOVERED_MCP_TOOLS as u32)?;
    let next_cursor = match value.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor))
            if !cursor.is_empty()
                && cursor.len() <= 4_096
                && !cursor.chars().any(|character| character.is_control()) =>
        {
            Some(cursor.clone())
        }
        Some(_) => return Err(invalid_tool_list(McpInvalidToolListCause::InvalidCursor)),
    };
    Ok((tools, next_cursor))
}

/// Result of classifying one advertised MCP tool during discovery.
enum DiscoveredToolClassification {
    /// The tool conforms to the host contract and is published.
    Published(HostedMcpDiscoveredTool),
    /// The tool violates a shape-only, non-security rule and is dropped from
    /// this generation while the rest of a bounded catalog still publishes.
    SkippedShapeViolation(McpInvalidToolListCause),
}

/// Classify a single advertised tool. Security/bounds violations (missing or
/// unsafe input schema) return `Err(cause)` and reject the whole generation;
/// they are evaluated first so a co-occurring cosmetic defect cannot downgrade
/// them to a per-tool skip. Shape-only defects return
/// `Ok(SkippedShapeViolation(cause))`.
fn classify_discovered_tool(
    tool: &Value,
    max_name_bytes: usize,
    max_description_bytes: usize,
    max_schema_depth: u8,
    max_schema_nodes: usize,
    max_schema_string_bytes: usize,
) -> Result<DiscoveredToolClassification, McpInvalidToolListCause> {
    let input_schema = tool
        .get("inputSchema")
        .filter(|schema| schema.is_object())
        .cloned()
        .ok_or(McpInvalidToolListCause::MissingInputSchema)?;
    // A schema can fail for two unrelated reasons, and they deserve different
    // blast radii. An UNSAFE construct (control characters smuggled into a key or
    // string) is a trust violation: the provider is not behaving, so the whole
    // generation is rejected. Merely OVERSIZE (too deep, too many nodes, one long
    // string) is a resource limit, exactly like the page/byte/tool ceilings -- and
    // resource limits already truncate rather than discard. Collapsing both into
    // one bool meant a single tool with a long parameter description destroyed an
    // otherwise valid catalog: measured against a 47,337-tool endpoint, three tools
    // carried a >16 KiB parameter description, the first at index 9,325, and their
    // presence published ZERO of the other 47,334. The agent then ran 91 fruitless
    // `tool_search` calls against an empty index.
    match classify_mcp_input_schema(
        &input_schema,
        max_schema_depth,
        max_schema_nodes,
        max_schema_string_bytes,
    ) {
        SchemaVerdict::Ok => {}
        SchemaVerdict::Unsafe => return Err(McpInvalidToolListCause::UnsafeInputSchema),
        SchemaVerdict::Oversize => {
            return Ok(DiscoveredToolClassification::SkippedShapeViolation(
                McpInvalidToolListCause::OversizeInputSchema,
            ));
        }
    }
    // Discovered tool names become Reborn capability suffixes, so discovery
    // skips unsupported names instead of normalizing them into potentially
    // colliding capability IDs.
    let Some(name) = tool
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| is_supported_mcp_tool_name(name, max_name_bytes))
    else {
        return Ok(DiscoveredToolClassification::SkippedShapeViolation(
            McpInvalidToolListCause::InvalidToolName,
        ));
    };
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(description) = bound_mcp_tool_description(description, max_description_bytes) else {
        return Ok(DiscoveredToolClassification::SkippedShapeViolation(
            McpInvalidToolListCause::InvalidDescription,
        ));
    };
    let annotations = match parse_tool_annotations(tool.get("annotations")) {
        Ok(annotations) => annotations,
        Err(cause) => return Ok(DiscoveredToolClassification::SkippedShapeViolation(cause)),
    };
    Ok(DiscoveredToolClassification::Published(
        HostedMcpDiscoveredTool {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            annotations,
        },
    ))
}

/// Why a schema was rejected. `Oversize` is a resource limit (drop this tool);
/// `Unsafe` is a trust violation (reject the generation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemaVerdict {
    Ok,
    Oversize,
    Unsafe,
}

fn classify_mcp_input_schema(
    schema: &Value,
    max_depth: u8,
    max_nodes: usize,
    max_string_bytes: usize,
) -> SchemaVerdict {
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
) -> SchemaVerdict {
    if depth > max_depth {
        return SchemaVerdict::Oversize;
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > max_nodes {
        return SchemaVerdict::Oversize;
    }
    match value {
        // Unsafe is checked before size so a string that is both oversized and
        // carries control characters still classifies as the trust violation --
        // preserving the "security evaluated first" property of the original.
        Value::String(value) => {
            if value.chars().any(is_unsupported_description_char) {
                SchemaVerdict::Unsafe
            } else if value.len() > max_string_bytes {
                SchemaVerdict::Oversize
            } else {
                SchemaVerdict::Ok
            }
        }
        Value::Array(values) => {
            for value in values {
                let verdict = validate_mcp_schema_value(
                    value,
                    depth + 1,
                    max_depth,
                    max_nodes,
                    max_string_bytes,
                    nodes,
                );
                if verdict != SchemaVerdict::Ok {
                    return verdict;
                }
            }
            SchemaVerdict::Ok
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.chars().any(is_unsupported_description_char) {
                    return SchemaVerdict::Unsafe;
                }
                if key.len() > max_string_bytes {
                    return SchemaVerdict::Oversize;
                }
                let verdict = validate_mcp_schema_value(
                    value,
                    depth + 1,
                    max_depth,
                    max_nodes,
                    max_string_bytes,
                    nodes,
                );
                if verdict != SchemaVerdict::Ok {
                    return verdict;
                }
            }
            SchemaVerdict::Ok
        }
        _ => SchemaVerdict::Ok,
    }
}

fn is_unsupported_description_char(value: char) -> bool {
    value.is_control() && !matches!(value, '\n' | '\r' | '\t')
}

/// Preserve a provider's otherwise-valid tool catalog when only descriptive
/// prose exceeds the host display/prompt budget. Names and schemas remain
/// fail-closed because truncating either could change capability semantics;
/// descriptions are presentation metadata and can be safely bounded.
fn bound_mcp_tool_description(value: &str, max_bytes: usize) -> Option<String> {
    if value.chars().any(is_unsupported_description_char) {
        return None;
    }
    if value.len() <= max_bytes {
        return Some(value.to_string());
    }

    const TRUNCATION_MARKER: &str = "...";
    if max_bytes <= TRUNCATION_MARKER.len() {
        return Some(".".repeat(max_bytes));
    }

    let mut end = max_bytes - TRUNCATION_MARKER.len();
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = value.get(..end)?;
    let mut bounded = String::with_capacity(max_bytes);
    bounded.push_str(prefix);
    bounded.push_str(TRUNCATION_MARKER);
    Some(bounded)
}

fn parse_tool_annotations(
    value: Option<&Value>,
) -> Result<HostedMcpDiscoveredToolAnnotations, McpInvalidToolListCause> {
    let Some(value) = value else {
        return Ok(HostedMcpDiscoveredToolAnnotations::default());
    };
    let object = value
        .as_object()
        .ok_or(McpInvalidToolListCause::InvalidAnnotations)?;
    let title = object
        .get("title")
        .map(|value| {
            value
                .as_str()
                .and_then(|title| bound_mcp_tool_description(title, 2_048))
                .ok_or(McpInvalidToolListCause::InvalidAnnotations)
        })
        .transpose()?;
    Ok(HostedMcpDiscoveredToolAnnotations {
        title,
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
        idempotent_hint: object.get("idempotentHint").and_then(Value::as_bool),
        open_world_hint: object.get("openWorldHint").and_then(Value::as_bool),
    })
}

fn is_supported_mcp_tool_name(value: &str, max_bytes: usize) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_tools_list_result_rejects_oversized_tool_list() {
        let tools = (0..129)
            .map(|index| valid_tool(&format!("tool-{index}"), json!({"type": "object"})))
            .collect::<Vec<_>>();

        let error = parse_tools_list_result(&json!({ "tools": tools }), 128)
            .expect_err("tool discovery must cap returned tools");

        assert_eq!(error, "mcp_invalid_tool_list: too_many_tools");
    }

    #[test]
    fn parse_tools_list_result_honors_manifest_budget_under_host_cap() {
        let tools = (0..129)
            .map(|index| valid_tool(&format!("tool-{index}"), json!({"type": "object"})))
            .collect::<Vec<_>>();

        let discovered = parse_tools_list_result(&json!({ "tools": tools }), 256)
            .expect("the manifest may declare a catalog larger than the old hidden limit");

        assert_eq!(discovered.len(), 129);
    }

    #[test]
    fn parse_tools_list_result_caps_manifest_budget_at_host_maximum() {
        // Derived from the constant, not a literal: this assertion hardcoded 1025 and broke
        // the moment the ceiling moved, even though what it checks -- that a provider's
        // declared budget is clamped to the host's -- was unaffected.
        let tools = (0..=MAX_DISCOVERED_MCP_TOOLS)
            .map(|index| valid_tool(&format!("tool-{index}"), json!({"type": "object"})))
            .collect::<Vec<_>>();

        let error = parse_tools_list_result(&json!({ "tools": tools }), u32::MAX)
            .expect_err("provider-declared budgets cannot exceed the host ceiling");

        assert_eq!(error, "mcp_invalid_tool_list: too_many_tools");
    }

    #[test]
    fn parse_tools_list_result_rejects_unsupported_description_control_char() {
        let mut tool = valid_tool("search", json!({"type": "object"}));
        tool["description"] = json!("bad\u{0000}description");

        let error = parse_tools_list_result(&json!({ "tools": [tool] }), 128)
            .expect_err("unsupported description control characters must fail");

        assert_eq!(error, "mcp_invalid_tool_list: invalid_description");
    }

    #[test]
    fn parse_tools_list_result_bounds_utf8_description_at_character_boundary() {
        let mut tool = valid_tool("search", json!({"type": "object"}));
        tool["description"] = json!("🔧".repeat(600));

        let tools = parse_tools_list_result(&json!({ "tools": [tool] }), 128)
            .expect("descriptive prose must not invalidate the catalog");
        let description = &tools[0].description;

        assert!(description.len() <= 2_048);
        assert!(description.ends_with("..."));
        assert!(description.is_char_boundary(description.len()));
    }

    #[test]
    fn parse_tools_list_result_accepts_bounded_real_world_openapi_schema_shape() {
        // OpenAPI-derived MCP catalogs legitimately exceed the old depth-8 /
        // 512-node parser constants. The response body remains independently
        // bounded by the host egress plan, so a safe, finite schema within the
        // catalog budget must not make the whole extension unactivatable.
        let tool = valid_tool(
            "update-resource",
            json!({
                "type": "object",
                "properties": {
                    "nested": nested_schema(5),
                    "wide": wide_schema(600)
                }
            }),
        );

        let tools = parse_tools_list_result(&json!({ "tools": [tool] }), 128)
            .expect("bounded OpenAPI-derived schemas must remain discoverable");

        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn parse_tools_list_result_rejects_missing_or_non_object_schema() {
        let mut missing_schema = valid_tool("missing-schema", json!({"type": "object"}));
        missing_schema
            .as_object_mut()
            .expect("test tool object")
            .remove("inputSchema");
        let non_object_schema = valid_tool("bad-schema", json!("object please"));

        for tool in [missing_schema, non_object_schema] {
            let error = parse_tools_list_result(&json!({ "tools": [tool] }), 128)
                .expect_err("schema must be present and object-shaped");

            assert_eq!(error, "mcp_invalid_tool_list: missing_input_schema");
        }
    }

    #[test]
    fn parse_tools_list_result_rejects_schemas_carrying_control_characters() {
        // A control character smuggled into a schema string or key is a TRUST
        // violation, so it still rejects the whole generation.
        let cases = [
            valid_tool(
                "control",
                json!({"type": "object", "description": "bad\u{0008}schema"}),
            ),
            valid_tool("control-key", json!({"type": "object", "ba\u{0008}d": "x"})),
        ];

        for tool in cases {
            let error = parse_tools_list_result(&json!({ "tools": [tool] }), 128)
                .expect_err("control characters in a schema must fail");

            assert_eq!(error, "mcp_invalid_tool_list: unsafe_input_schema");
        }
    }

    #[test]
    #[tracing_test::traced_test]
    fn parse_tools_list_result_skips_oversize_schemas_and_publishes_the_rest() {
        // Size bounds are RESOURCE limits, not trust violations: an oversized
        // schema drops that one tool and the rest of the catalog still
        // publishes. Conflating these two was catastrophic at scale -- a
        // 47,337-tool endpoint carried three tools with a >16 KiB parameter
        // description (first at index 9,325) and published none of the other
        // 47,334.
        let oversize = [
            valid_tool(
                "long-string",
                json!({"type": "object", "description": "a".repeat(16 * 1024 + 1)}),
            ),
            valid_tool("too-deep", nested_schema(17)),
            valid_tool("too-many-nodes", wide_schema(8193)),
        ];

        for tool in oversize {
            let name = tool["name"].as_str().expect("test tool name").to_string();
            let tools = vec![
                valid_tool("alpha", json!({"type": "object"})),
                tool,
                valid_tool("beta", json!({"type": "object"})),
            ];

            let published = parse_tools_list_result(&json!({ "tools": tools }), 128)
                .expect("an oversized schema must not destroy its valid neighbors");

            assert_eq!(published.len(), 2, "only the oversized tool is dropped");
            assert!(
                published.iter().all(|tool| tool.name != name),
                "the oversized tool must not publish"
            );
            assert!(published.iter().any(|tool| tool.name == "alpha"));
            assert!(published.iter().any(|tool| tool.name == "beta"));
        }
        assert!(logs_contain("oversize_input_schema"));
    }

    #[test]
    fn parse_tools_list_result_treats_oversize_and_unsafe_together_as_unsafe() {
        // Fail closed when both apply: a string that is oversized AND carries a
        // control character is a trust violation, not a resource limit, so the
        // cosmetic size defect cannot downgrade it to a per-tool skip.
        let mut payload = "a".repeat(16 * 1024 + 1);
        payload.push('\u{0008}');
        let tool = valid_tool("both", json!({"type": "object", "description": payload}));

        let error = parse_tools_list_result(&json!({ "tools": [tool] }), 128)
            .expect_err("a control character must win over the size bound");

        assert_eq!(error, "mcp_invalid_tool_list: unsafe_input_schema");
    }

    #[test]
    #[tracing_test::traced_test]
    fn parse_tools_list_result_skips_shape_invalid_tools_and_publishes_bounded_remainder() {
        // A real MCP server can advertise a mostly-valid catalog alongside a
        // few shape-nonconforming entries (an uppercase tool name, a
        // control-char description). Those individual tools are dropped and
        // recorded, but the remaining valid tools must still publish so one
        // malformed entry cannot brick the whole integration on first install.
        let mut tools = (0..24)
            .map(|index| valid_tool(&format!("tool-{index}"), json!({"type": "object"})))
            .collect::<Vec<_>>();
        tools[5]["name"] = json!("UppercaseName");
        tools[10]["description"] = json!("bad\u{0000}description");

        let published = parse_tools_list_result(&json!({ "tools": tools }), 128)
            .expect("a bounded catalog must survive a few shape-nonconforming tools");

        assert_eq!(published.len(), 22);
        assert!(
            published.iter().all(|tool| tool.name != "UppercaseName"),
            "the uppercase-named tool must not be published"
        );
        assert!(
            published.iter().any(|tool| tool.name == "tool-0"),
            "valid tools before the skipped entries must still publish"
        );
        assert!(
            published.iter().any(|tool| tool.name == "tool-23"),
            "valid tools after the skipped entries must still publish"
        );
        assert!(logs_contain("skipping shape-nonconforming hosted MCP tool"));
        assert!(logs_contain("invalid_tool_name"));
        assert!(logs_contain("invalid_description"));
    }

    #[test]
    fn parse_tools_list_result_fails_whole_catalog_when_unsafe_schema_amid_valid_tools() {
        // Trust violations are never downgraded to a per-tool skip: a schema
        // carrying a control character fails the entire generation even when
        // surrounded by otherwise-valid tools, so a hostile entry cannot
        // smuggle itself in by riding a valid catalog. (Size violations are a
        // resource limit and DO drop per-tool -- covered separately above.)
        let mut tools = vec![
            valid_tool("alpha", json!({"type": "object"})),
            valid_tool("beta", json!({"type": "object"})),
        ];
        tools.insert(
            1,
            valid_tool(
                "control",
                json!({"type": "object", "description": "bad\u{0008}schema"}),
            ),
        );

        let error = parse_tools_list_result(&json!({ "tools": tools }), 128)
            .expect_err("an unsafe schema must fail the whole catalog even with valid neighbors");

        assert_eq!(error, "mcp_invalid_tool_list: unsafe_input_schema");
    }

    #[test]
    fn parse_tools_list_result_fails_when_every_tool_is_shape_invalid() {
        // When nothing survives the shape filter there is nothing to publish,
        // so discovery still fails non-retryably with a stable subcause rather
        // than activating on an empty catalog.
        let tools = vec![
            valid_tool("Uppercase-A", json!({"type": "object"})),
            valid_tool("Uppercase-B", json!({"type": "object"})),
        ]
        .into_iter()
        .map(|mut tool| {
            let bad = tool["name"].as_str().unwrap().to_string();
            tool["name"] = json!(bad);
            tool
        })
        .collect::<Vec<_>>();

        let error = parse_tools_list_result(&json!({ "tools": tools }), 128)
            .expect_err("a catalog with no shape-valid tools must not activate");

        assert_eq!(error, "mcp_invalid_tool_list: invalid_tool_name");
    }

    #[test]
    fn parse_tools_list_result_preserves_empty_provider_catalog_as_empty() {
        // An empty provider list (no advertised tools, nothing skipped) is not
        // a shape failure: it stays an empty result the caller treats as "no
        // tools discovered yet", distinct from the all-skipped failure above.
        let published = parse_tools_list_result(&json!({ "tools": [] }), 128)
            .expect("an empty provider catalog is not a shape failure");

        assert!(published.is_empty());
    }

    #[test]
    fn is_supported_mcp_tool_name_boundary_cases() {
        let exactly_128 = "a".repeat(128);
        let too_long = "a".repeat(129);

        assert!(!is_supported_mcp_tool_name("", 128));
        assert!(is_supported_mcp_tool_name(&exactly_128, 128));
        assert!(!is_supported_mcp_tool_name(&too_long, 128));
        assert!(!is_supported_mcp_tool_name("search..issues", 128));
        assert!(!is_supported_mcp_tool_name("Search", 128));
        assert!(!is_supported_mcp_tool_name("search._private", 128));
    }

    #[test]
    fn tools_list_page_preserves_accepted_catalog_fields_exactly() {
        let schema = json!({"type": "object", "properties": {"q": {"type": "string"}}});
        let value = json!({
            "tools": [{
                "name": "search.docs",
                "description": "Find docs\nwithout rewriting provider text.",
                "inputSchema": schema,
                "annotations": {"readOnlyHint": true}
            }],
            "nextCursor": "second-page"
        });

        let (tools, cursor) = parse_tools_list_page(&value).expect("valid page");
        assert_eq!(cursor.as_deref(), Some("second-page"));
        assert_eq!(tools[0].name, "search.docs");
        assert_eq!(
            tools[0].description,
            "Find docs\nwithout rewriting provider text."
        );
        assert_eq!(tools[0].input_schema, schema);
        assert!(tools[0].annotations.read_only_hint);
    }

    #[test]
    fn tools_list_page_rejects_non_string_cursor() {
        let error = parse_tools_list_page(&json!({
            "tools": [valid_tool("search", json!({"type": "object"}))],
            "nextCursor": 12
        }))
        .expect_err("cursor is protocol data, not a value to normalize");
        assert_eq!(error, "mcp_invalid_tool_list: invalid_cursor");
    }

    fn valid_tool(name: &str, input_schema: Value) -> Value {
        json!({
            "name": name,
            "description": "Search hosted data",
            "inputSchema": input_schema
        })
    }

    fn nested_schema(depth: usize) -> Value {
        let mut value = json!({"type": "string"});
        for _ in 0..depth {
            value = json!({"type": "object", "properties": {"next": value}});
        }
        value
    }

    fn wide_schema(nodes: usize) -> Value {
        let properties = (0..nodes)
            .map(|index| (format!("field_{index}"), json!({"type": "string"})))
            .collect::<serde_json::Map<_, _>>();
        json!({"type": "object", "properties": properties})
    }
}
