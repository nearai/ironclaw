//! Pure progressive tool-disclosure catalog and selector.
//!
//! This module is intentionally not wired into the live model path yet. The
//! next disclosure pass will connect it behind the rollout flag and add bridge
//! execution.

#![allow(dead_code)]

use std::collections::HashSet;

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::ProviderToolDefinition;
use serde_json::{Map, Value, json};

/// Candidate core names from the design doc. Exact membership is
/// telemetry-tunable and may become profile-specific as production traces land.
pub(crate) const CORE_TOOL_NAMES: &[&str] = &[
    "tool_search",
    "tool_describe",
    "tool_call",
    "result_read",
    "memory_search",
    "memory_read",
    "memory_write",
    "skill_search",
    "file_read",
    "list_dir",
];

const BRIDGE_CAPABILITY_PREFIX: &str = "ironclaw";
const MAX_KEYWORD_SCORE: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolTier {
    Core,
    Discoverable,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityCatalog {
    entries: Vec<CatalogEntry>,
    total_schema_tokens: u32,
}

#[derive(Debug, Clone)]
struct CatalogEntry {
    definition: ProviderToolDefinition,
    est_schema_tokens: u32,
    search_blob: String,
    tier: ToolTier,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromotedSet {
    names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisclosureCaps {
    pub(crate) max_tokens: u32,
    pub(crate) max_tools: usize,
    pub(crate) ctx_limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveSet {
    pub(crate) definitions: Vec<ProviderToolDefinition>,
    pub(crate) deferred: bool,
    pub(crate) advertised_tokens: u32,
}

impl CapabilityCatalog {
    pub(crate) fn new(
        definitions: &[ProviderToolDefinition],
        profile_pins: &[String],
    ) -> CapabilityCatalog {
        let pinned_names: HashSet<&str> = profile_pins.iter().map(String::as_str).collect();
        let mut entries: Vec<CatalogEntry> = definitions
            .iter()
            .map(|definition| {
                let est_schema_tokens = estimate_definition_tokens(definition);
                let search_blob =
                    format!("{} {}", definition.name, definition.description).to_lowercase();
                let tier = if CORE_TOOL_NAMES.contains(&definition.name.as_str())
                    || pinned_names.contains(definition.name.as_str())
                {
                    ToolTier::Core
                } else {
                    ToolTier::Discoverable
                };
                CatalogEntry {
                    definition: definition.clone(),
                    est_schema_tokens,
                    search_blob,
                    tier,
                }
            })
            .collect();
        entries.sort_by(|left, right| left.definition.name.cmp(&right.definition.name));
        let total_schema_tokens = entries.iter().fold(0_u32, |total, entry| {
            total.saturating_add(entry.est_schema_tokens)
        });
        CapabilityCatalog {
            entries,
            total_schema_tokens,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn total_schema_tokens(&self) -> u32 {
        self.total_schema_tokens
    }

    fn entry_by_name(&self, name: &str) -> Option<&CatalogEntry> {
        self.entries
            .binary_search_by(|entry| entry.definition.name.as_str().cmp(name))
            .ok()
            .and_then(|index| self.entries.get(index))
    }
}

impl PromotedSet {
    pub(crate) fn push(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !self.contains(name.as_str()) {
            self.names.push(name);
        }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|candidate| candidate == name)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    pub(crate) fn len(&self) -> usize {
        self.names.len()
    }
}

impl Default for DisclosureCaps {
    fn default() -> Self {
        Self {
            max_tokens: 12_000,
            max_tools: 32,
            ctx_limit: None,
        }
    }
}

impl DisclosureCaps {
    pub(crate) fn defer_threshold_tokens(&self) -> u32 {
        self.ctx_limit
            .map(|ctx_limit| self.max_tokens.min(ctx_limit / 10))
            .unwrap_or(self.max_tokens)
    }
}

pub(crate) fn bridge_tool_definitions() -> Vec<ProviderToolDefinition> {
    vec![
        bridge_tool_definition(
            "tool_search",
            "Search the deferred tool catalog by name and description.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query for the deferred tool catalog."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of matching tool names to return.",
                        "default": 10,
                        "minimum": 1
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        bridge_tool_definition(
            "tool_describe",
            "Return the full schema for one named deferred tool.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Provider-facing tool name to describe."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        ),
        bridge_tool_definition(
            "tool_call",
            "Invoke one named tool through the normal dispatcher path.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Provider-facing tool name to invoke."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments for the named tool.",
                        "additionalProperties": true
                    }
                },
                "required": ["name", "arguments"],
                "additionalProperties": false
            }),
        ),
    ]
}

/// Selects the active wire surface for a turn.
///
/// TODO(next pass): enforce the 24-tool / 12k advertised cap once promotion is
/// live; if eviction is required, start a deliberate prompt-surface epoch reset
/// rather than silently reordering the promoted suffix.
pub(crate) fn select_active_set(
    catalog: &CapabilityCatalog,
    promoted: &PromotedSet,
    caps: DisclosureCaps,
) -> ActiveSet {
    if catalog.total_schema_tokens() <= caps.defer_threshold_tokens()
        && catalog.len() <= caps.max_tools
    {
        return ActiveSet {
            definitions: catalog
                .entries
                .iter()
                .map(|entry| entry.definition.clone())
                .collect(),
            deferred: false,
            advertised_tokens: catalog.total_schema_tokens(),
        };
    }

    let mut definitions = Vec::new();
    let mut advertised_tokens = 0_u32;
    let mut included_names: HashSet<String> = HashSet::new();

    for entry in catalog
        .entries
        .iter()
        .filter(|entry| entry.tier == ToolTier::Core)
    {
        append_definition(
            &mut definitions,
            &mut advertised_tokens,
            &mut included_names,
            entry.definition.clone(),
            entry.est_schema_tokens,
        );
    }

    for definition in bridge_tool_definitions() {
        let est_schema_tokens = estimate_definition_tokens(&definition);
        append_definition(
            &mut definitions,
            &mut advertised_tokens,
            &mut included_names,
            definition,
            est_schema_tokens,
        );
    }

    for name in promoted.iter() {
        if let Some(entry) = catalog.entry_by_name(name) {
            append_definition(
                &mut definitions,
                &mut advertised_tokens,
                &mut included_names,
                entry.definition.clone(),
                entry.est_schema_tokens,
            );
        }
    }

    ActiveSet {
        definitions,
        deferred: true,
        advertised_tokens,
    }
}

pub(crate) fn tool_search_rank(
    catalog: &CapabilityCatalog,
    query: &str,
    limit: usize,
) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower
        .split_whitespace()
        .map(|term| term.trim_matches(|c: char| !c.is_alphanumeric() && c != '_'))
        .filter(|term| !term.is_empty())
        .collect();

    if query_terms.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(String, u32)> = catalog
        .entries
        .iter()
        .filter_map(|entry| {
            let score = score_tool_entry(entry, &query_terms);
            if score > 0 {
                Some((entry.definition.name.clone(), score))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.as_str().cmp(right.0.as_str()))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(name, _score)| name)
        .collect()
}

fn append_definition(
    definitions: &mut Vec<ProviderToolDefinition>,
    advertised_tokens: &mut u32,
    included_names: &mut HashSet<String>,
    definition: ProviderToolDefinition,
    est_schema_tokens: u32,
) {
    if included_names.insert(definition.name.clone()) {
        definitions.push(definition);
        *advertised_tokens = advertised_tokens.saturating_add(est_schema_tokens);
    }
}

fn score_tool_entry(entry: &CatalogEntry, query_terms: &[&str]) -> u32 {
    let blob_terms: HashSet<&str> = entry
        .search_blob
        .split_whitespace()
        .map(|term| term.trim_matches(|c: char| !c.is_alphanumeric() && c != '_'))
        .filter(|term| !term.is_empty())
        .collect();
    let mut keyword_score = 0_u32;
    for term in query_terms {
        if entry.definition.name.eq_ignore_ascii_case(term) || blob_terms.contains(term) {
            keyword_score = keyword_score.saturating_add(10);
        } else if entry.search_blob.contains(term) {
            keyword_score = keyword_score.saturating_add(5);
        }
    }
    keyword_score.min(MAX_KEYWORD_SCORE)
}

fn bridge_tool_definition(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: bridge_capability_id(name),
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

fn bridge_capability_id(name: &'static str) -> CapabilityId {
    let raw = format!("{BRIDGE_CAPABILITY_PREFIX}.{name}");
    match CapabilityId::new(raw) {
        Ok(capability_id) => capability_id,
        Err(error) => {
            // Static bridge ids use validated literal segments. Reaching this
            // branch means this source file was edited to contain an invalid id.
            panic!("invalid static bridge capability id: {error}");
        }
    }
}

fn estimate_definition_tokens(definition: &ProviderToolDefinition) -> u32 {
    crate::context_shadow::estimate_tokens(&canonical_tool_schema_json(definition))
}

fn canonical_tool_schema_json(definition: &ProviderToolDefinition) -> String {
    canonical_tool_schema_value(definition).to_string()
}

fn canonical_tool_schema_value(definition: &ProviderToolDefinition) -> Value {
    let mut entries = vec![
        (
            "description".to_string(),
            Value::String(definition.description.clone()),
        ),
        ("name".to_string(), Value::String(definition.name.clone())),
        (
            "parameters".to_string(),
            canonicalize_json(&definition.parameters),
        ),
    ];
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut object = Map::new();
    for (key, value) in entries {
        object.insert(key, value);
    }
    Value::Object(object)
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        Value::Object(object) => {
            let mut entries: Vec<(&String, &Value)> = object.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key.clone(), canonicalize_json(value));
            }
            Value::Object(sorted)
        }
        scalar => scalar.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_sorts_entries_and_marks_core_and_pins() {
        let definitions = vec![
            fixture_tool("zeta_tool", "Zeta tool", small_no_arg_schema()),
            fixture_tool(
                "file_read",
                "Read files from the workspace.",
                medium_schema(0),
            ),
            fixture_tool("alpha_tool", "Alpha tool", small_no_arg_schema()),
        ];
        let profile_pins = vec!["zeta_tool".to_string()];

        let catalog = CapabilityCatalog::new(&definitions, &profile_pins);

        let names: Vec<&str> = catalog
            .entries
            .iter()
            .map(|entry| entry.definition.name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha_tool", "file_read", "zeta_tool"]);
        assert_eq!(
            catalog.entry_by_name("file_read").map(|entry| entry.tier),
            Some(ToolTier::Core)
        );
        assert_eq!(
            catalog.entry_by_name("zeta_tool").map(|entry| entry.tier),
            Some(ToolTier::Core)
        );
        assert_eq!(
            catalog.entry_by_name("alpha_tool").map(|entry| entry.tier),
            Some(ToolTier::Discoverable)
        );
    }

    #[test]
    fn canonical_schema_sorts_nested_json_keys() {
        let left = fixture_tool(
            "demo_tool",
            "Demo",
            json!({
                "type": "object",
                "properties": {
                    "z": { "type": "string", "description": "last" },
                    "a": { "description": "first", "type": "string" }
                }
            }),
        );
        let right = fixture_tool(
            "demo_tool",
            "Demo",
            json!({
                "properties": {
                    "a": { "type": "string", "description": "first" },
                    "z": { "description": "last", "type": "string" }
                },
                "type": "object"
            }),
        );

        assert_eq!(
            canonical_tool_schema_json(&left),
            canonical_tool_schema_json(&right)
        );
    }

    #[test]
    fn bridge_tool_definitions_are_fixed_order_and_schema_backed() {
        let bridges = bridge_tool_definitions();
        let names: Vec<&str> = bridges
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert_eq!(names, vec!["tool_search", "tool_describe", "tool_call"]);
        assert_eq!(
            bridges[0].parameters["required"],
            json!(["query"]),
            "tool_search requires query"
        );
        assert_eq!(
            bridges[2].parameters["required"],
            json!(["name", "arguments"]),
            "tool_call requires target name and argument object"
        );
    }

    #[test]
    fn promoted_set_is_append_only_and_unique() {
        let mut promoted = PromotedSet::default();
        promoted.push("workspace_search");
        promoted.push("http_fetch");
        promoted.push("workspace_search");

        assert!(promoted.contains("workspace_search"));
        assert_eq!(promoted.len(), 2);
        assert_eq!(
            promoted.iter().collect::<Vec<_>>(),
            vec!["workspace_search", "http_fetch"]
        );
    }

    #[test]
    fn select_active_set_returns_full_when_under_threshold() {
        let definitions = vec![
            fixture_tool("alpha_tool", "Alpha", small_no_arg_schema()),
            fixture_tool("beta_tool", "Beta", small_no_arg_schema()),
        ];
        let catalog = CapabilityCatalog::new(&definitions, &[]);

        let active =
            select_active_set(&catalog, &PromotedSet::default(), DisclosureCaps::default());

        assert!(!active.deferred);
        assert_eq!(active.definitions.len(), 2);
        assert_eq!(active.advertised_tokens, catalog.total_schema_tokens());
    }

    #[test]
    fn select_active_set_defers_to_core_bridges_then_promoted_order() {
        let definitions = vec![
            fixture_tool("zzz_promoted", "Promoted", medium_schema(1)),
            fixture_tool(
                "file_read",
                "Read files from the workspace.",
                medium_schema(2),
            ),
            fixture_tool("memory_search", "Search memory.", medium_schema(3)),
            fixture_tool("aaa_promoted", "Promoted", medium_schema(4)),
            fixture_tool("other_tool", "Other", medium_schema(5)),
        ];
        let catalog = CapabilityCatalog::new(&definitions, &[]);
        let mut promoted = PromotedSet::default();
        promoted.push("zzz_promoted");
        promoted.push("aaa_promoted");
        promoted.push("file_read");

        let active = select_active_set(
            &catalog,
            &promoted,
            DisclosureCaps {
                max_tokens: 1,
                max_tools: 32,
                ctx_limit: None,
            },
        );

        let names: Vec<&str> = active
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert!(active.deferred);
        assert_eq!(
            names,
            vec![
                "file_read",
                "memory_search",
                "tool_search",
                "tool_describe",
                "tool_call",
                "zzz_promoted",
                "aaa_promoted"
            ]
        );
    }

    #[test]
    fn tool_search_rank_scores_deterministically() {
        let definitions = vec![
            fixture_tool(
                "http_fetch",
                "Fetch an HTTP URL and return status and body.",
                medium_schema(1),
            ),
            fixture_tool(
                "file_read",
                "Read a workspace file by path.",
                medium_schema(2),
            ),
            fixture_tool(
                "github_issue_search",
                "Search GitHub issues and pull requests.",
                medium_schema(3),
            ),
        ];
        let catalog = CapabilityCatalog::new(&definitions, &[]);

        assert_eq!(
            tool_search_rank(&catalog, "search issue", 2),
            vec!["github_issue_search"]
        );
        assert_eq!(tool_search_rank(&catalog, "read", 2), vec!["file_read"]);
    }

    #[test]
    fn disclosure_caps_uses_context_limited_threshold_when_known() {
        assert_eq!(
            DisclosureCaps {
                max_tokens: 12_000,
                max_tools: 32,
                ctx_limit: Some(64_000),
            }
            .defer_threshold_tokens(),
            6_400
        );
        assert_eq!(
            DisclosureCaps {
                max_tokens: 12_000,
                max_tools: 32,
                ctx_limit: Some(200_000),
            }
            .defer_threshold_tokens(),
            12_000
        );
    }

    #[test]
    fn benchmark_tool_disclosure_token_reduction() {
        let definitions = representative_tool_fixture();
        let catalog = CapabilityCatalog::new(&definitions, &[]);
        let full_count = catalog.len();
        let full_tokens = catalog.total_schema_tokens();

        let disclosed =
            select_active_set(&catalog, &PromotedSet::default(), DisclosureCaps::default());
        let disclosed_count = disclosed.definitions.len();
        let disclosed_tokens = disclosed.advertised_tokens;
        let reduction_abs = full_tokens.saturating_sub(disclosed_tokens);
        let reduction_pct = if full_tokens == 0 {
            0.0
        } else {
            (reduction_abs as f64 / full_tokens as f64) * 100.0
        };

        println!(
            "\n| full_count | full_tokens | disclosed_count | disclosed_tokens | reduction_abs | reduction_pct |\n| ---: | ---: | ---: | ---: | ---: | ---: |\n| {full_count} | {full_tokens} | {disclosed_count} | {disclosed_tokens} | {reduction_abs} | {reduction_pct:.1}% |"
        );

        assert_eq!(full_count, 91);
        assert!(disclosed.deferred);
        assert!(
            disclosed_tokens as f64 <= full_tokens as f64 * 0.5,
            "disclosed={disclosed_tokens}, full={full_tokens}"
        );
    }

    // Representative benchmark fixture for today's broad provider tool
    // surface: 15 small no-arg tools, 50 medium 2-4 parameter tools, and
    // 26 larger nested-object tools. The real production number is emitted by
    // the Phase-0 shadow log (`est_tool_schema_tokens`) and this fixture should
    // be cross-checked against it as traces arrive.
    fn representative_tool_fixture() -> Vec<ProviderToolDefinition> {
        let core_names = [
            "result_read",
            "memory_search",
            "memory_read",
            "memory_write",
            "skill_search",
            "file_read",
            "list_dir",
        ];
        let mut definitions: Vec<ProviderToolDefinition> = core_names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                fixture_tool(
                    *name,
                    format!("Core loop primitive for deterministic {name} operations."),
                    medium_schema(index),
                )
            })
            .collect();

        for index in 0..15 {
            definitions.push(fixture_tool(
                format!("small_status_{index:02}"),
                format!("Read small status signal {index} without arguments."),
                small_no_arg_schema(),
            ));
        }

        for index in 0..43 {
            definitions.push(fixture_tool(
                format!("medium_workspace_{index:02}"),
                format!(
                    "Perform workspace, memory, issue, document, or process operation {index} with bounded arguments."
                ),
                medium_schema(index),
            ));
        }

        for index in 0..26 {
            definitions.push(fixture_tool(
                format!("large_integration_{index:02}"),
                format!(
                    "Execute integration workflow {index} with nested filters, pagination, safety metadata, and output controls."
                ),
                large_nested_schema(index),
            ));
        }

        assert_eq!(definitions.len(), 91);
        definitions
    }

    fn fixture_tool(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> ProviderToolDefinition {
        let name = name.into();
        ProviderToolDefinition {
            capability_id: CapabilityId::new(format!("fixture.{name}")).expect("fixture id"),
            name,
            description: description.into(),
            parameters,
        }
    }

    fn small_no_arg_schema() -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        })
    }

    fn medium_schema(index: usize) -> Value {
        let mode_default = if index.is_multiple_of(2) {
            "summary"
        } else {
            "full"
        };
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": format!("Primary query, path, identifier, or selector for medium operation {index}.")
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return.",
                    "default": 20,
                    "minimum": 1,
                    "maximum": 200
                },
                "mode": {
                    "type": "string",
                    "description": "Result detail mode.",
                    "enum": ["summary", "full", "metadata"],
                    "default": mode_default
                },
                "include_archived": {
                    "type": "boolean",
                    "description": "Whether archived or hidden records may be included.",
                    "default": false
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn large_nested_schema(index: usize) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request": {
                    "type": "object",
                    "description": format!("Nested request envelope for integration workflow {index}."),
                    "properties": {
                        "target": {
                            "type": "string",
                            "description": "Repository, workspace, service, or remote collection identifier."
                        },
                        "filters": {
                            "type": "object",
                            "description": "Structured filters applied before dispatch.",
                            "properties": {
                                "states": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Allowed lifecycle states."
                                },
                                "labels": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Labels, tags, or categories to include."
                                },
                                "updated_after": {
                                    "type": "string",
                                    "description": "Inclusive ISO-8601 update lower bound."
                                },
                                "owner": {
                                    "type": "string",
                                    "description": "Optional owner, assignee, or author filter."
                                }
                            },
                            "additionalProperties": false
                        },
                        "pagination": {
                            "type": "object",
                            "properties": {
                                "cursor": {
                                    "type": "string",
                                    "description": "Opaque page cursor returned by a previous call."
                                },
                                "page_size": {
                                    "type": "integer",
                                    "default": 50,
                                    "minimum": 1,
                                    "maximum": 500
                                }
                            },
                            "additionalProperties": false
                        }
                    },
                    "required": ["target"],
                    "additionalProperties": false
                },
                "output": {
                    "type": "object",
                    "description": "Output shaping and safety controls.",
                    "properties": {
                        "format": {
                            "type": "string",
                            "enum": ["json", "markdown", "compact"],
                            "default": "json"
                        },
                        "include_raw": {
                            "type": "boolean",
                            "description": "Include raw provider payload fields when available.",
                            "default": false
                        },
                        "redact_secrets": {
                            "type": "boolean",
                            "description": "Redact credentials, tokens, and sensitive headers.",
                            "default": true
                        }
                    },
                    "additionalProperties": false
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Validate the request without committing side effects.",
                    "default": true
                }
            },
            "required": ["request"],
            "additionalProperties": false
        })
    }
}
