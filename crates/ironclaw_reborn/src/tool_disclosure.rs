//! Pure progressive tool-disclosure catalog and selector.
//!
use std::{
    collections::{BTreeSet, HashSet},
    sync::LazyLock,
};

use ironclaw_host_api::{CapabilityId, RuntimeKind};
use ironclaw_turns::run_profile::{
    CapabilityDescriptorView, ConcurrencyHint, ProviderToolDefinition,
};
use serde_json::{Map, Value, json};

/// Canonical core tool names from the progressive-disclosure policy.
///
/// Builtin provider names may be encoded from capability ids by the host
/// runtime (for example `builtin.read_file` can be exposed as
/// `builtin__read_file`). Core matching also checks the canonical builtin
/// suffix so this list stays stable across provider-name encoding changes.
pub(crate) const CORE_TOOL_NAMES: &[&str] = &[
    // bridges + result hydration
    "tool_search",
    "tool_describe",
    "tool_call",
    "result_read",
    // file / code / exec (everyday)
    "read_file",
    "write_file",
    "list_dir",
    "glob",
    "grep",
    "apply_patch",
    "shell",
    // memory
    "memory_search",
    "memory_read",
    "memory_write",
    // web
    "http",
    "web_search",
    // onboarding entry points — the full extension lifecycle is core so a weak
    // model can run search -> install -> activate -> remove directly, without
    // routing the install/remove steps through tool_search.
    "extension_search",
    "extension_install",
    "extension_activate",
    "extension_remove",
    // skills + time
    "skill_list",
    "time",
];

const BRIDGE_CAPABILITY_PREFIX: &str = "ironclaw";
pub(crate) const TOOL_SEARCH_NAME: &str = "tool_search";
pub(crate) const TOOL_DESCRIBE_NAME: &str = "tool_describe";
pub(crate) const TOOL_CALL_NAME: &str = "tool_call";
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
pub(crate) struct CatalogEntry {
    definition: ProviderToolDefinition,
    est_schema_tokens: u32,
    search_blob: String,
    search_terms: HashSet<String>,
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
            .filter(|definition| {
                !is_bridge_name(&definition.name)
                    && !is_bridge_capability_id(&definition.capability_id)
            })
            .map(|definition| {
                let est_schema_tokens = estimate_definition_tokens(definition);
                let search_blob =
                    format!("{} {}", definition.name, definition.description).to_lowercase();
                let search_terms = search_terms(&search_blob);
                let tier = if is_core_tool_definition(definition)
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
                    search_terms,
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

    pub(crate) fn definition_by_name(&self, name: &str) -> Option<&ProviderToolDefinition> {
        self.entry_by_name(name).map(|entry| &entry.definition)
    }

    pub(crate) fn definitions(&self) -> impl Iterator<Item = &ProviderToolDefinition> {
        self.entries.iter().map(|entry| &entry.definition)
    }

    pub(crate) fn search_result(&self, name: &str) -> Option<CatalogSearchResult> {
        self.entry_by_name(name)
            .map(CatalogSearchResult::from_entry)
    }

    pub(crate) fn active_or_disclosed_descriptors(
        &self,
        active: &ActiveSet,
        disclosed_names: &BTreeSet<String>,
    ) -> Vec<CapabilityDescriptorView> {
        let mut included = BTreeSet::new();
        let mut descriptors = Vec::new();
        for definition in &active.definitions {
            if is_bridge_name(&definition.name) {
                descriptors.push(bridge_descriptor(definition));
            } else if let Some(entry) = self.entry_by_name(&definition.name)
                && included.insert(definition.name.clone())
            {
                descriptors.push(catalog_descriptor(entry));
            }
        }
        for name in disclosed_names {
            if included.contains(name) {
                continue;
            }
            if let Some(entry) = self.entry_by_name(name) {
                included.insert(name.clone());
                descriptors.push(catalog_descriptor(entry));
            }
        }
        descriptors.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        descriptors
    }
}

fn is_core_tool_definition(definition: &ProviderToolDefinition) -> bool {
    CORE_TOOL_NAMES
        .iter()
        .any(|core_name| definition_matches_core_name(definition, core_name))
}

pub(crate) fn definition_matches_provider_name(
    definition: &ProviderToolDefinition,
    provider_name: &str,
) -> bool {
    if definition.name == provider_name {
        return true;
    }
    if let Some(builtin_name) = provider_name
        .strip_prefix("builtin__")
        .or_else(|| provider_name.strip_prefix("builtin."))
    {
        return definition_matches_core_name(definition, builtin_name);
    }
    let capability_id = definition.capability_id.as_str();
    capability_id
        .strip_prefix("builtin.")
        .is_some_and(|name| name == provider_name)
}

fn definition_matches_core_name(definition: &ProviderToolDefinition, core_name: &str) -> bool {
    if definition.name == core_name {
        return true;
    }
    let capability_id = definition.capability_id.as_str();
    if capability_id
        .strip_prefix("builtin.")
        .is_some_and(|name| name == core_name)
    {
        return true;
    }
    matches!(
        (capability_id, core_name),
        ("web-access.search", "web_search") | ("web-access.get_content", "web_fetch")
    ) || capability_id.ends_with(&format!(".{core_name}"))
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

    #[cfg(test)]
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

static BRIDGE_TOOL_DEFINITIONS: LazyLock<Vec<(ProviderToolDefinition, u32)>> =
    LazyLock::new(|| {
        let definitions = vec![
            bridge_tool_definition(
                TOOL_SEARCH_NAME,
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
                TOOL_DESCRIBE_NAME,
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
                TOOL_CALL_NAME,
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
        ];
        definitions
            .into_iter()
            .map(|definition| {
                let est_schema_tokens = estimate_definition_tokens(&definition);
                (definition, est_schema_tokens)
            })
            .collect()
    });

type BridgeDefinitionWithTokens = (&'static ProviderToolDefinition, u32);

pub(crate) fn bridge_tool_definitions() -> Vec<ProviderToolDefinition> {
    bridge_tool_definitions_with_tokens()
        .map(|(definition, _)| definition.clone())
        .collect()
}

fn bridge_tool_definitions_with_tokens() -> impl Iterator<Item = BridgeDefinitionWithTokens> {
    BRIDGE_TOOL_DEFINITIONS
        .iter()
        .map(|(definition, est_schema_tokens)| (definition, *est_schema_tokens))
}

fn advertised_bridge_tool_definitions(deferred_count: usize) -> Vec<(ProviderToolDefinition, u32)> {
    bridge_tool_definitions_with_tokens()
        .map(|(definition, est_schema_tokens)| {
            let mut advertised = definition.clone();
            if advertised.name == TOOL_SEARCH_NAME {
                advertised.description = count_aware_tool_search_description(deferred_count);
                let est_schema_tokens = estimate_definition_tokens(&advertised);
                return (advertised, est_schema_tokens);
            }
            if advertised.name == TOOL_CALL_NAME {
                advertised.description = tool_call_safety_description();
                let est_schema_tokens = estimate_definition_tokens(&advertised);
                return (advertised, est_schema_tokens);
            }
            (advertised, est_schema_tokens)
        })
        .collect()
}

fn count_aware_tool_search_description(deferred_count: usize) -> String {
    format!(
        "Search {deferred_count} additional tools that are loaded on demand. Returns up to `limit` matches with name and description. Follow with tool_describe to load a tool's full parameter schema, then tool_call to invoke it. Tools already listed are available and do not need to be searched."
    )
}

fn tool_call_safety_description() -> String {
    "Invoke one named tool through the normal dispatcher path. Approvals, policy, and hooks run exactly as for a directly-listed tool."
        .to_string()
}

pub(crate) fn is_bridge_name(name: &str) -> bool {
    matches!(name, TOOL_SEARCH_NAME | TOOL_DESCRIBE_NAME | TOOL_CALL_NAME)
}

pub(crate) fn is_bridge_capability_id(capability_id: &CapabilityId) -> bool {
    bridge_tool_definitions_with_tokens()
        .any(|(definition, _)| &definition.capability_id == capability_id)
}

/// Selects the active wire surface for a turn.
///
/// TODO(next pass): if promoted tools are truncated by caps, start a deliberate
/// prompt-surface epoch reset rather than silently carrying old prompt context.
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

    let mut core_definitions = Vec::new();
    let mut core_names: HashSet<String> = HashSet::new();

    for entry in catalog
        .entries
        .iter()
        .filter(|entry| entry.tier == ToolTier::Core)
    {
        if core_names.insert(entry.definition.name.clone()) {
            core_definitions.push((entry.definition.clone(), entry.est_schema_tokens));
        }
    }

    let threshold_tokens = caps.defer_threshold_tokens();
    let core_tokens = sum_definition_tokens(&core_definitions);
    let mut advertised_non_bridge_count = core_definitions.len();

    loop {
        let deferred_count = catalog.len().saturating_sub(advertised_non_bridge_count);
        let bridge_definitions = advertised_bridge_tool_definitions(deferred_count);
        let bridge_tokens = sum_definition_tokens(&bridge_definitions);
        let promoted_definitions = select_promoted_definitions(
            catalog,
            promoted,
            &core_names,
            core_tokens.saturating_add(bridge_tokens),
            core_definitions
                .len()
                .saturating_add(bridge_definitions.len()),
            threshold_tokens,
            caps.max_tools,
        );
        let next_advertised_non_bridge_count = core_definitions
            .len()
            .saturating_add(promoted_definitions.len());
        if next_advertised_non_bridge_count == advertised_non_bridge_count {
            let mut definitions = Vec::new();
            let mut advertised_tokens = 0_u32;
            let mut included_names: HashSet<String> = HashSet::new();

            for (definition, est_schema_tokens) in core_definitions
                .into_iter()
                .chain(bridge_definitions)
                .chain(promoted_definitions)
            {
                append_definition(
                    &mut definitions,
                    &mut advertised_tokens,
                    &mut included_names,
                    definition,
                    est_schema_tokens,
                );
            }

            return ActiveSet {
                definitions,
                deferred: true,
                advertised_tokens,
            };
        }
        advertised_non_bridge_count = next_advertised_non_bridge_count;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogSearchResult {
    pub(crate) name: String,
    pub(crate) capability_id: CapabilityId,
    pub(crate) description: String,
    pub(crate) required_params: Vec<String>,
    pub(crate) parameters: Value,
}

impl CatalogSearchResult {
    fn from_entry(entry: &CatalogEntry) -> Self {
        Self {
            name: entry.definition.name.clone(),
            capability_id: entry.definition.capability_id.clone(),
            description: entry.definition.description.clone(),
            required_params: required_params(&entry.definition.parameters),
            parameters: canonicalize_json(&entry.definition.parameters),
        }
    }
}

pub(crate) fn required_params(parameters: &Value) -> Vec<String> {
    let mut names = BTreeSet::new();
    collect_required_params(parameters, true, &mut names);
    names.into_iter().collect()
}

fn collect_required_params(
    value: &Value,
    contributes_required: bool,
    names: &mut BTreeSet<String>,
) {
    if contributes_required && let Some(required) = value.get("required").and_then(Value::as_array)
    {
        names.extend(
            required
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
    }
    if let Some(variants) = value.get("allOf").and_then(Value::as_array) {
        for variant in variants {
            collect_required_params(variant, contributes_required, names);
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = value.get(key).and_then(Value::as_array) {
            for variant in variants {
                collect_required_params(variant, false, names);
            }
        }
    }
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

fn select_promoted_definitions(
    catalog: &CapabilityCatalog,
    promoted: &PromotedSet,
    core_names: &HashSet<String>,
    mut advertised_tokens: u32,
    mut advertised_count: usize,
    threshold_tokens: u32,
    max_tools: usize,
) -> Vec<(ProviderToolDefinition, u32)> {
    let mut selected = Vec::new();
    let mut included_names = core_names.clone();
    for name in promoted.iter() {
        if let Some(entry) = catalog.entry_by_name(name) {
            if included_names.contains(name) {
                continue;
            }
            if advertised_count >= max_tools {
                break;
            }
            if advertised_tokens.saturating_add(entry.est_schema_tokens) > threshold_tokens {
                break;
            }
            included_names.insert(entry.definition.name.clone());
            selected.push((entry.definition.clone(), entry.est_schema_tokens));
            advertised_tokens = advertised_tokens.saturating_add(entry.est_schema_tokens);
            advertised_count = advertised_count.saturating_add(1);
        }
    }
    selected
}

fn sum_definition_tokens(definitions: &[(ProviderToolDefinition, u32)]) -> u32 {
    definitions
        .iter()
        .fold(0_u32, |total, (_definition, est_schema_tokens)| {
            total.saturating_add(*est_schema_tokens)
        })
}

fn score_tool_entry(entry: &CatalogEntry, query_terms: &[&str]) -> u32 {
    let mut keyword_score = 0_u32;
    for term in query_terms {
        if entry.definition.name.eq_ignore_ascii_case(term) || entry.search_terms.contains(*term) {
            keyword_score = keyword_score.saturating_add(10);
        } else if entry.search_blob.contains(term) {
            keyword_score = keyword_score.saturating_add(5);
        }
    }
    keyword_score.min(MAX_KEYWORD_SCORE)
}

fn search_terms(search_blob: &str) -> HashSet<String> {
    search_blob
        .split_whitespace()
        .map(|term| term.trim_matches(|c: char| !c.is_alphanumeric() && c != '_'))
        .filter(|term| !term.is_empty())
        .map(str::to_string)
        .collect()
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

fn bridge_descriptor(definition: &ProviderToolDefinition) -> CapabilityDescriptorView {
    CapabilityDescriptorView {
        capability_id: definition.capability_id.clone(),
        provider: None,
        runtime: RuntimeKind::FirstParty,
        safe_name: definition.name.clone(),
        safe_description: definition.description.clone(),
        concurrency_hint: ConcurrencyHint::Exclusive,
        parameters_schema: definition.parameters.clone(),
    }
}

fn catalog_descriptor(entry: &CatalogEntry) -> CapabilityDescriptorView {
    CapabilityDescriptorView {
        capability_id: entry.definition.capability_id.clone(),
        provider: None,
        runtime: RuntimeKind::FirstParty,
        safe_name: entry.definition.name.clone(),
        safe_description: entry.definition.description.clone(),
        concurrency_hint: ConcurrencyHint::Exclusive,
        parameters_schema: entry.definition.parameters.clone(),
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

pub(crate) fn canonicalize_json(value: &Value) -> Value {
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
    fn core_builtin_names_are_backed_by_known_capability_ids() {
        let known_builtin_core_names = [
            (
                "memory_search",
                ironclaw_host_runtime::MEMORY_SEARCH_CAPABILITY_ID,
            ),
            (
                "memory_read",
                ironclaw_host_runtime::MEMORY_READ_CAPABILITY_ID,
            ),
            (
                "memory_write",
                ironclaw_host_runtime::MEMORY_WRITE_CAPABILITY_ID,
            ),
            (
                "skill_list",
                ironclaw_host_runtime::SKILL_LIST_CAPABILITY_ID,
            ),
            ("read_file", ironclaw_host_runtime::READ_FILE_CAPABILITY_ID),
            (
                "write_file",
                ironclaw_host_runtime::WRITE_FILE_CAPABILITY_ID,
            ),
            ("list_dir", ironclaw_host_runtime::LIST_DIR_CAPABILITY_ID),
            ("glob", ironclaw_host_runtime::GLOB_CAPABILITY_ID),
            ("grep", ironclaw_host_runtime::GREP_CAPABILITY_ID),
            (
                "apply_patch",
                ironclaw_host_runtime::APPLY_PATCH_CAPABILITY_ID,
            ),
            ("shell", ironclaw_host_runtime::SHELL_CAPABILITY_ID),
            ("http", ironclaw_host_runtime::HTTP_CAPABILITY_ID),
            ("extension_search", "builtin.extension_search"),
            ("extension_install", "builtin.extension_install"),
            ("extension_activate", "builtin.extension_activate"),
            ("extension_remove", "builtin.extension_remove"),
            ("time", ironclaw_host_runtime::TIME_CAPABILITY_ID),
        ];
        let synthetic_or_extension_core_names = [
            TOOL_SEARCH_NAME,
            TOOL_DESCRIBE_NAME,
            TOOL_CALL_NAME,
            "result_read",
            "web_search",
        ];
        let mut covered_names = BTreeSet::new();

        for (name, capability_id) in known_builtin_core_names {
            assert!(
                CORE_TOOL_NAMES.contains(&name),
                "known builtin core tool {name} is missing from CORE_TOOL_NAMES"
            );
            assert_eq!(
                capability_id.strip_prefix("builtin."),
                Some(name),
                "builtin core tool {name} must map to builtin.{name}"
            );
            assert!(
                covered_names.insert(name),
                "core tool {name} is covered more than once"
            );
        }
        for name in synthetic_or_extension_core_names {
            assert!(
                CORE_TOOL_NAMES.contains(&name),
                "synthetic/extension core tool {name} is missing from CORE_TOOL_NAMES"
            );
            assert!(
                covered_names.insert(name),
                "core tool {name} is covered more than once"
            );
        }
        for name in CORE_TOOL_NAMES {
            assert!(
                covered_names.contains(name),
                "core tool {name} is neither a known builtin nor an intentional synthetic/extension entry"
            );
        }
        assert_eq!(
            covered_names.len(),
            CORE_TOOL_NAMES.len(),
            "every CORE_TOOL_NAME must be covered by exactly one regression list"
        );
    }

    #[test]
    fn catalog_marks_provider_encoded_builtin_names_core_by_capability_id() {
        let definitions = vec![ProviderToolDefinition {
            capability_id: CapabilityId::new(ironclaw_host_runtime::READ_FILE_CAPABILITY_ID)
                .expect("valid capability id"),
            name: "builtin__read_file".to_string(),
            description: "Read files from the workspace.".to_string(),
            parameters: medium_schema(0),
        }];

        let catalog = CapabilityCatalog::new(&definitions, &[]);

        assert_eq!(
            catalog
                .entry_by_name("builtin__read_file")
                .map(|entry| entry.tier),
            Some(ToolTier::Core)
        );
    }

    #[test]
    fn catalog_sorts_entries_and_marks_core_and_pins() {
        let definitions = vec![
            fixture_tool("zeta_tool", "Zeta tool", small_no_arg_schema()),
            fixture_tool(
                "read_file",
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
        assert_eq!(names, vec!["alpha_tool", "read_file", "zeta_tool"]);
        assert_eq!(
            catalog.entry_by_name("read_file").map(|entry| entry.tier),
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
    fn catalog_reserves_bridge_names_for_synthetic_definitions() {
        let definitions = vec![
            fixture_tool(
                TOOL_SEARCH_NAME,
                "Conflicting real tool",
                small_no_arg_schema(),
            ),
            fixture_tool(
                "read_file",
                "Read files from the workspace.",
                medium_schema(0),
            ),
        ];
        let catalog = CapabilityCatalog::new(&definitions, &[]);

        assert_eq!(catalog.len(), 1);
        assert!(catalog.definition_by_name(TOOL_SEARCH_NAME).is_none());

        let active = select_active_set(
            &catalog,
            &PromotedSet::default(),
            DisclosureCaps {
                max_tokens: 1,
                max_tools: 0,
                ctx_limit: None,
            },
        );
        let bridge = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name == TOOL_SEARCH_NAME)
            .expect("tool_search bridge definition");
        let advertised = active
            .definitions
            .iter()
            .find(|definition| definition.name == TOOL_SEARCH_NAME)
            .expect("tool_search advertised");
        assert_eq!(advertised.capability_id, bridge.capability_id);
    }

    #[test]
    fn catalog_reserves_bridge_capability_ids_for_synthetic_definitions() {
        let bridge = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name == TOOL_SEARCH_NAME)
            .expect("tool_search bridge definition");
        let definitions = vec![
            ProviderToolDefinition {
                capability_id: bridge.capability_id.clone(),
                name: "ordinary_tool_name".to_string(),
                description: "Conflicting real tool with a reserved bridge id".to_string(),
                parameters: small_no_arg_schema(),
            },
            fixture_tool(
                "read_file",
                "Read files from the workspace.",
                medium_schema(0),
            ),
        ];
        let catalog = CapabilityCatalog::new(&definitions, &[]);

        assert_eq!(catalog.len(), 1);
        assert!(catalog.definition_by_name("ordinary_tool_name").is_none());
        assert_eq!(
            catalog
                .definitions()
                .map(|definition| definition.name.as_str())
                .collect::<Vec<_>>(),
            vec!["read_file"]
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
        assert_eq!(
            bridges[0].description,
            "Search the deferred tool catalog by name and description."
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
                "read_file",
                "Read files from the workspace.",
                medium_schema(2),
            ),
            fixture_tool("memory_search", "Search memory.", medium_schema(3)),
            fixture_tool("aaa_promoted", "Promoted", medium_schema(4)),
            fixture_tool("other_tool", "Other", medium_schema(5)),
        ];
        let mut definitions = definitions;
        for index in 0..8 {
            definitions.push(fixture_tool(
                format!("large_unpromoted_{index}"),
                "Large unpromoted",
                large_nested_schema(index + 6),
            ));
        }
        let catalog = CapabilityCatalog::new(&definitions, &[]);
        let mut promoted = PromotedSet::default();
        promoted.push("zzz_promoted");
        promoted.push("aaa_promoted");
        promoted.push("read_file");
        let advertised_non_bridge_count = 4;
        let bridge_tokens = advertised_bridge_tool_definitions(
            catalog.len().saturating_sub(advertised_non_bridge_count),
        )
        .iter()
        .fold(0_u32, |total, (_definition, est_schema_tokens)| {
            total.saturating_add(*est_schema_tokens)
        });
        let active_budget = ["read_file", "memory_search", "zzz_promoted", "aaa_promoted"]
            .into_iter()
            .filter_map(|name| catalog.entry_by_name(name))
            .fold(bridge_tokens, |total, entry| {
                total.saturating_add(entry.est_schema_tokens)
            });

        let active = select_active_set(
            &catalog,
            &promoted,
            DisclosureCaps {
                max_tokens: active_budget,
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
                "memory_search",
                "read_file",
                "tool_search",
                "tool_describe",
                "tool_call",
                "zzz_promoted",
                "aaa_promoted"
            ]
        );
    }

    #[test]
    fn select_active_set_caps_promoted_suffix_without_dropping_core_or_bridges() {
        let mut definitions = vec![fixture_tool(
            "read_file",
            "Read files from the workspace.",
            medium_schema(0),
        )];
        for index in 0..12 {
            definitions.push(fixture_tool(
                format!("promoted_{index:02}"),
                format!("Promoted operation {index}"),
                large_nested_schema(index),
            ));
        }
        let catalog = CapabilityCatalog::new(&definitions, &[]);
        let mut promoted = PromotedSet::default();
        for index in 0..12 {
            promoted.push(format!("promoted_{index:02}"));
        }

        let base_count = bridge_tool_definitions().len() + 1;
        let by_count = select_active_set(
            &catalog,
            &promoted,
            DisclosureCaps {
                max_tokens: u32::MAX,
                max_tools: base_count + 1,
                ctx_limit: None,
            },
        );
        let by_count_names: Vec<&str> = by_count
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert!(by_count.deferred);
        assert!(by_count.definitions.len() <= base_count + 1);
        assert!(by_count_names.contains(&"read_file"));
        assert!(by_count_names.contains(&TOOL_SEARCH_NAME));
        assert!(by_count_names.contains(&TOOL_DESCRIBE_NAME));
        assert!(by_count_names.contains(&TOOL_CALL_NAME));
        assert!(by_count_names.contains(&"promoted_00"));
        assert!(!by_count_names.contains(&"promoted_01"));

        let advertised_non_bridge_count = 2;
        let bridge_tokens = advertised_bridge_tool_definitions(
            catalog.len().saturating_sub(advertised_non_bridge_count),
        )
        .iter()
        .fold(0_u32, |total, (_definition, est_schema_tokens)| {
            total.saturating_add(*est_schema_tokens)
        });
        let token_threshold = bridge_tokens
            .saturating_add(
                catalog
                    .entry_by_name("read_file")
                    .expect("read_file entry")
                    .est_schema_tokens,
            )
            .saturating_add(
                catalog
                    .entry_by_name("promoted_00")
                    .expect("promoted entry")
                    .est_schema_tokens,
            );
        assert!(
            catalog.total_schema_tokens() > token_threshold,
            "fixture must force deferred mode by token budget"
        );
        let by_tokens = select_active_set(
            &catalog,
            &promoted,
            DisclosureCaps {
                max_tokens: token_threshold,
                max_tools: 32,
                ctx_limit: None,
            },
        );
        let by_token_names: Vec<&str> = by_tokens
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert!(by_tokens.deferred);
        assert!(by_tokens.definitions.len() <= 32);
        assert!(by_tokens.advertised_tokens <= token_threshold);
        assert!(by_token_names.contains(&"read_file"));
        assert!(by_token_names.contains(&TOOL_SEARCH_NAME));
        assert!(by_token_names.contains(&TOOL_DESCRIBE_NAME));
        assert!(by_token_names.contains(&TOOL_CALL_NAME));
        assert!(by_token_names.contains(&"promoted_00"));
        assert!(!by_token_names.contains(&"promoted_01"));
    }

    #[test]
    fn select_active_set_advertises_count_aware_bridge_descriptions_and_tokens() {
        let definitions = vec![
            fixture_tool(
                "read_file",
                "Read files from the workspace.",
                medium_schema(0),
            ),
            fixture_tool("alpha_tool", "Alpha", medium_schema(1)),
            fixture_tool("beta_tool", "Beta", medium_schema(2)),
            fixture_tool("gamma_tool", "Gamma", medium_schema(3)),
        ];
        let catalog = CapabilityCatalog::new(&definitions, &[]);

        let active = select_active_set(
            &catalog,
            &PromotedSet::default(),
            DisclosureCaps {
                max_tokens: 1,
                max_tools: 0,
                ctx_limit: None,
            },
        );

        assert!(active.deferred);
        let deferred_count = catalog.len().saturating_sub(1);
        let tool_search = active
            .definitions
            .iter()
            .find(|definition| definition.name == TOOL_SEARCH_NAME)
            .expect("tool_search advertised");
        assert_eq!(
            tool_search.description,
            count_aware_tool_search_description(deferred_count)
        );
        let tool_call = active
            .definitions
            .iter()
            .find(|definition| definition.name == TOOL_CALL_NAME)
            .expect("tool_call advertised");
        assert_eq!(tool_call.description, tool_call_safety_description());

        let actual_tokens = active.definitions.iter().fold(0_u32, |total, definition| {
            total.saturating_add(estimate_definition_tokens(definition))
        });
        assert_eq!(active.advertised_tokens, actual_tokens);
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
                "read_file",
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
        assert_eq!(tool_search_rank(&catalog, "read", 2), vec!["read_file"]);
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
            "read_file",
            "write_file",
            "list_dir",
            "glob",
            "grep",
            "apply_patch",
            "shell",
            "memory_search",
            "memory_read",
            "memory_write",
            "http",
            "web_search",
            "extension_search",
            "extension_activate",
            "skill_list",
            "time",
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

        for index in 0..33 {
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
