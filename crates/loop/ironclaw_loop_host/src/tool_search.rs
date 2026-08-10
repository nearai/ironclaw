//! Bounded, authorization-fitted lexical retrieval for deferred tools.

use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
};

use ironclaw_host_api::{capability::CapabilityDescriptionTrust, ids::CapabilityId};
use ironclaw_loop_contracts::ProviderToolDefinition;
use serde_json::Value;

pub(crate) const MAX_SEARCH_QUERY_BYTES: usize = 1_024;
const MAX_QUERY_TERMS: usize = 32;
const MAX_SCHEMA_DEPTH: usize = 8;
const MAX_SCHEMA_NODES: usize = 256;
const MAX_SCHEMA_FIELDS: usize = 128;
const MAX_FIELD_BYTES: usize = 256;
const MAX_DOCUMENT_BYTES: usize = 8_192;
const MAX_UNIQUE_TERMS: usize = 512;
const RANKER_VERSION: &str = "bounded-bm25f-v1";
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

const NAME_WEIGHT: f64 = 8.0;
const PROVIDER_WEIGHT: f64 = 4.0;
const PARAMETER_WEIGHT: f64 = 5.0;
const DESCRIPTION_WEIGHT: f64 = 1.0;
const EXACT_IDENTIFIER_BONUS: f64 = 1_000_000.0;

#[derive(Debug, Clone)]
pub(crate) struct AuthorizedToolSearchIndex {
    documents: Vec<IndexedDocument>,
    document_frequencies: BTreeMap<String, usize>,
    average_length: f64,
}

#[derive(Debug, Clone)]
struct IndexedDocument {
    name: String,
    capability_id: CapabilityId,
    exact_identifiers: BTreeSet<String>,
    term_weights: BTreeMap<String, f64>,
    length: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchQueryClass {
    ExactIdentifier,
    Lexical,
    NoMatch,
}

impl SearchQueryClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExactIdentifier => "exact_identifier",
            Self::Lexical => "lexical",
            Self::NoMatch => "no_match",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchOutcome {
    pub(crate) names: Vec<String>,
    pub(crate) query_class: SearchQueryClass,
}

impl AuthorizedToolSearchIndex {
    /// Callers must pass only the effective authorized definitions. Keeping the
    /// constructor authorization-shaped prevents denied schemas from affecting
    /// document frequency, ordering, counts, or index-build cost.
    pub(crate) fn new<'a>(
        definitions: impl IntoIterator<Item = &'a ProviderToolDefinition>,
    ) -> Self {
        let mut documents: Vec<_> = definitions.into_iter().map(IndexedDocument::new).collect();
        documents.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        let total_length: f64 = documents.iter().map(|document| document.length).sum();
        let mut document_frequencies = BTreeMap::new();
        for document in &documents {
            for term in document.term_weights.keys() {
                document_frequencies
                    .entry(term.clone())
                    .and_modify(|count: &mut usize| *count = count.saturating_add(1))
                    .or_insert(1);
            }
        }
        let average_length = if documents.is_empty() {
            1.0
        } else {
            total_length / documents.len() as f64
        };
        Self {
            documents,
            document_frequencies,
            average_length,
        }
    }

    pub(crate) fn search(&self, query: &str, limit: usize) -> SearchOutcome {
        if limit == 0 {
            return SearchOutcome {
                names: Vec::new(),
                query_class: SearchQueryClass::NoMatch,
            };
        }
        let normalized_query = query.trim().to_lowercase();
        let query_terms: Vec<String> = tokenize(&normalized_query)
            .into_iter()
            .take(MAX_QUERY_TERMS)
            .collect();
        if query_terms.is_empty() {
            return SearchOutcome {
                names: Vec::new(),
                query_class: SearchQueryClass::NoMatch,
            };
        }

        let exact = self
            .documents
            .iter()
            .any(|document| document.exact_identifiers.contains(&normalized_query));
        let mut scored = Vec::new();
        for document in &self.documents {
            let mut score = if document.exact_identifiers.contains(&normalized_query) {
                EXACT_IDENTIFIER_BONUS
            } else {
                0.0
            };
            for term in &query_terms {
                let Some(term_weight) = document.term_weights.get(term) else {
                    continue;
                };
                let document_frequency = self
                    .document_frequencies
                    .get(term)
                    .copied()
                    .unwrap_or_default() as f64;
                let document_count = self.documents.len() as f64;
                let inverse_document_frequency = (1.0
                    + (document_count - document_frequency + 0.5) / (document_frequency + 0.5))
                    .ln();
                let length_normalization =
                    BM25_K1 * (1.0 - BM25_B + BM25_B * document.length / self.average_length);
                score += inverse_document_frequency * term_weight * (BM25_K1 + 1.0)
                    / (term_weight + length_normalization);
            }
            if score > 0.0 {
                scored.push((document.name.clone(), document.capability_id.clone(), score));
            }
        }
        scored.sort_by(|left, right| {
            right
                .2
                .total_cmp(&left.2)
                .then_with(|| left.1.cmp(&right.1))
        });
        let names: Vec<_> = scored
            .into_iter()
            .take(limit)
            .map(|(name, _capability_id, _score)| name)
            .collect();
        SearchOutcome {
            query_class: if names.is_empty() {
                SearchQueryClass::NoMatch
            } else if exact {
                SearchQueryClass::ExactIdentifier
            } else {
                SearchQueryClass::Lexical
            },
            names,
        }
    }
}

impl IndexedDocument {
    fn new(definition: &ProviderToolDefinition) -> Self {
        let mut builder = SearchDocumentBuilder::default();
        let capability_id = definition.capability_id.as_str();
        let provider = capability_id.split('.').next();
        if let Some(provider) = provider {
            builder.add_field(provider, PROVIDER_WEIGHT);
        }
        let capability_local_name = capability_id
            .split_once('.')
            .map_or(capability_id, |(_, local_name)| local_name);
        builder.add_field(capability_local_name, NAME_WEIGHT);
        let provider_name = definition.name.as_str();
        let provider_local_name = provider
            .and_then(|provider| provider_name.strip_prefix(provider))
            .and_then(|name| name.strip_prefix("__"))
            .unwrap_or(provider_name);
        builder.add_field(provider_local_name, NAME_WEIGHT);
        builder.add_field(&definition.description, DESCRIPTION_WEIGHT);
        builder.collect_schema(
            &definition.parameters,
            0,
            definition.description_trust == CapabilityDescriptionTrust::VerifiedCatalog,
        );

        let mut exact_identifiers = BTreeSet::from([
            capability_id.to_lowercase(),
            definition.name.as_str().to_lowercase(),
        ]);
        exact_identifiers.insert(capability_id.replace('.', "__").to_lowercase());
        let length = builder.term_weights.len().max(1) as f64;
        Self {
            name: definition.name.to_string(),
            capability_id: definition.capability_id.clone(),
            exact_identifiers,
            term_weights: builder.term_weights,
            length,
        }
    }
}

#[derive(Debug, Default)]
struct SearchDocumentBuilder {
    term_weights: BTreeMap<String, f64>,
    schema_nodes: usize,
    schema_fields: usize,
    admitted_bytes: usize,
}

impl SearchDocumentBuilder {
    fn add_field(&mut self, value: &str, weight: f64) {
        if self.admitted_bytes >= MAX_DOCUMENT_BYTES || self.term_weights.len() >= MAX_UNIQUE_TERMS
        {
            return;
        }
        let remaining = MAX_DOCUMENT_BYTES - self.admitted_bytes;
        let admitted: String = value
            .chars()
            .scan(0_usize, |bytes, character| {
                let next = bytes.saturating_add(character.len_utf8());
                if next > remaining.min(MAX_FIELD_BYTES) {
                    None
                } else {
                    *bytes = next;
                    Some(character)
                }
            })
            .collect();
        self.admitted_bytes = self.admitted_bytes.saturating_add(admitted.len());
        for term in tokenize(&admitted) {
            if self.term_weights.len() >= MAX_UNIQUE_TERMS && !self.term_weights.contains_key(&term)
            {
                break;
            }
            self.term_weights
                .entry(term)
                .and_modify(|current| *current = current.max(weight))
                .or_insert(weight);
        }
    }

    fn collect_schema(&mut self, value: &Value, depth: usize, trusted_descriptions: bool) {
        if depth > MAX_SCHEMA_DEPTH || self.schema_nodes >= MAX_SCHEMA_NODES {
            return;
        }
        let Some(object) = value.as_object() else {
            return;
        };
        self.schema_nodes = self.schema_nodes.saturating_add(1);

        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            for (name, schema) in properties {
                if self.schema_fields >= MAX_SCHEMA_FIELDS {
                    break;
                }
                self.schema_fields = self.schema_fields.saturating_add(1);
                self.add_field(name, PARAMETER_WEIGHT);
                if trusted_descriptions
                    && let Some(description) = schema.get("description").and_then(Value::as_str)
                {
                    self.add_field(description, DESCRIPTION_WEIGHT);
                }
                self.collect_schema_children(std::iter::once(schema), depth, trusted_descriptions);
            }
        }
        if let Some(items) = object.get("items") {
            match items {
                Value::Array(items) => {
                    self.collect_schema_children(items, depth, trusted_descriptions)
                }
                _ => self.collect_schema_children(
                    std::iter::once(items),
                    depth,
                    trusted_descriptions,
                ),
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf"] {
            if let Some(variants) = object.get(keyword).and_then(Value::as_array) {
                self.collect_schema_children(variants, depth, trusted_descriptions);
            }
        }
        if let Some(additional) = object.get("additionalProperties")
            && additional.is_object()
        {
            self.collect_schema_children(std::iter::once(additional), depth, trusted_descriptions);
        }
        for keyword in ["$defs", "definitions"] {
            if let Some(definitions) = object.get(keyword).and_then(Value::as_object) {
                self.collect_schema_children(definitions.values(), depth, trusted_descriptions);
            }
        }
    }

    fn collect_schema_children<'a>(
        &mut self,
        children: impl IntoIterator<Item = &'a Value>,
        parent_depth: usize,
        trusted_descriptions: bool,
    ) {
        for child in children.into_iter().take(MAX_SCHEMA_NODES) {
            if self.schema_nodes >= MAX_SCHEMA_NODES {
                break;
            }
            self.collect_schema(child, parent_depth.saturating_add(1), trusted_descriptions);
        }
    }
}

fn tokenize(value: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_lowercase_or_digit = false;
    let mut previous_uppercase = false;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_alphanumeric() {
            let starts_word = character.is_uppercase()
                && (previous_lowercase_or_digit
                    || (previous_uppercase
                        && characters.peek().is_some_and(|next| next.is_lowercase())));
            if starts_word {
                normalized.push(' ');
            }
            for lowercase in character.to_lowercase() {
                normalized.push(lowercase);
            }
            previous_lowercase_or_digit = character.is_lowercase() || character.is_numeric();
            previous_uppercase = character.is_uppercase();
        } else {
            normalized.push(' ');
            previous_lowercase_or_digit = false;
            previous_uppercase = false;
        }
    }
    let mut seen = BTreeSet::new();
    normalized
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .filter(|term| seen.insert((*term).to_string()))
        .map(str::to_string)
        .collect()
}

/// Stable for a fixed ranker version and effective authorized metadata. Object
/// keys are sorted recursively so semantically identical schemas share a key.
pub(crate) fn definitions_fingerprint(definitions: &[ProviderToolDefinition]) -> u64 {
    let mut definitions: Vec<_> = definitions.iter().collect();
    definitions.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    RANKER_VERSION.hash(&mut hasher);
    definitions.len().hash(&mut hasher);
    for definition in definitions {
        definition.capability_id.hash(&mut hasher);
        definition.name.hash(&mut hasher);
        definition.description.hash(&mut hasher);
        definition.description_trust.hash(&mut hasher);
        hash_json(&definition.parameters, &mut hasher);
    }
    hasher.finish()
}

fn hash_json(value: &Value, hasher: &mut impl Hasher) {
    std::mem::discriminant(value).hash(hasher);
    match value {
        Value::Null => {}
        Value::Bool(value) => value.hash(hasher),
        Value::Number(value) => value.to_string().hash(hasher),
        Value::String(value) => value.hash(hasher),
        Value::Array(values) => values.iter().for_each(|value| hash_json(value, hasher)),
        Value::Object(values) => {
            let mut values: Vec<_> = values.iter().collect();
            values.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in values {
                key.hash(hasher);
                hash_json(value, hasher);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::ProviderToolName;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use std::time::Instant;

    fn definition(
        capability_id: &str,
        name: &str,
        description: &str,
        parameters: Value,
        trust: CapabilityDescriptionTrust,
    ) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: CapabilityId::new(capability_id).expect("valid capability id"),
            name: ProviderToolName::new(name).expect("valid provider tool name"),
            description: description.to_string(),
            description_trust: trust,
            parameters,
        }
    }

    #[test]
    fn indexes_parameter_keys_through_objects_arrays_and_unions() {
        let tool = definition(
            "calendar.create_event",
            "calendar__create_event",
            "Create an event.",
            json!({
                "type": "object",
                "properties": {
                    "attendees": {"type": "array", "items": {
                        "type": "object", "properties": {"timezone": {"type": "string"}}
                    }},
                    "schedule": {"oneOf": [
                        {"type": "object", "properties": {"recurrence": {"type": "string"}}},
                        {"type": "object", "properties": {"start_at": {"type": "string"}}}
                    ]}
                }
            }),
            CapabilityDescriptionTrust::Untrusted,
        );
        let index = AuthorizedToolSearchIndex::new([&tool]);

        for query in ["timezone", "recurrence", "start at"] {
            assert_eq!(
                index.search(query, 5).names,
                vec!["calendar__create_event"],
                "nested parameter query {query:?} must discover the tool"
            );
        }
    }

    #[test]
    fn nested_descriptions_require_verified_catalog_provenance() {
        let schema = json!({
            "type": "object",
            "properties": {
                "opaque": {"type": "string", "description": "sensitive prose canary"}
            }
        });
        let untrusted = definition(
            "mcp.lookup",
            "mcp__lookup",
            "Lookup records.",
            schema.clone(),
            CapabilityDescriptionTrust::Untrusted,
        );
        let trusted = definition(
            "catalog.lookup",
            "catalog__lookup",
            "Lookup records.",
            schema,
            CapabilityDescriptionTrust::VerifiedCatalog,
        );

        assert!(
            AuthorizedToolSearchIndex::new([&untrusted])
                .search("canary", 5)
                .names
                .is_empty(),
            "untrusted nested schema prose must not enter retrieval metadata"
        );
        assert_eq!(
            AuthorizedToolSearchIndex::new([&trusted])
                .search("canary", 5)
                .names,
            vec!["catalog__lookup"]
        );
    }

    #[test]
    fn schema_walk_is_bounded_and_terminates_on_adversarial_depth_and_width() {
        let mut nested = json!({"type": "string"});
        // Deep enough to exceed the production walk cap without making
        // serde_json's recursive drop itself the subject of this test.
        for depth in (0..64).rev() {
            nested = json!({
                "type": "object",
                "properties": {format!("level_{depth}"): nested}
            });
        }
        let wide_properties: serde_json::Map<String, Value> = (0..1_000)
            .map(|field| (format!("wide_{field}"), json!({"type": "string"})))
            .collect();
        let tool = definition(
            "fixture.adversarial",
            "fixture__adversarial",
            "Bounded schema fixture.",
            json!({
                "type": "object",
                "properties": {
                    "nested": nested,
                    "wide": {"type": "object", "properties": wide_properties}
                }
            }),
            CapabilityDescriptionTrust::VerifiedCatalog,
        );

        let index = AuthorizedToolSearchIndex::new([&tool]);
        assert_eq!(
            index.search("nested", 1).names,
            vec!["fixture__adversarial"]
        );
        assert!(index.search("63", 1).names.is_empty());
        assert!(index.search("999", 1).names.is_empty());
    }

    #[test]
    fn field_and_document_term_budgets_are_enforced() {
        let mut builder = SearchDocumentBuilder::default();
        let long_field = format!("{} tail_canary", "prefix ".repeat(64));
        builder.add_field(&long_field, DESCRIPTION_WEIGHT);
        assert!(builder.term_weights.contains_key("prefix"));
        assert!(
            !builder.term_weights.contains_key("tail"),
            "terms beyond the per-field byte cap must not be admitted"
        );

        for field in 0..(MAX_DOCUMENT_BYTES / MAX_FIELD_BYTES + 4) {
            builder.add_field(
                &format!("budget{field} {}", "padding ".repeat(64)),
                DESCRIPTION_WEIGHT,
            );
        }
        assert_eq!(builder.admitted_bytes, MAX_DOCUMENT_BYTES);
        assert!(builder.term_weights.len() <= MAX_UNIQUE_TERMS);
        let terms_at_cap = builder.term_weights.len();
        builder.add_field("document_tail_canary", DESCRIPTION_WEIGHT);
        assert_eq!(builder.term_weights.len(), terms_at_cap);
        assert!(!builder.term_weights.contains_key("document"));

        let mut unique_term_builder = SearchDocumentBuilder::default();
        for term in 0..(MAX_UNIQUE_TERMS + 32) {
            unique_term_builder.add_field(&format!("term{term}"), DESCRIPTION_WEIGHT);
        }
        assert_eq!(unique_term_builder.term_weights.len(), MAX_UNIQUE_TERMS);
        assert!(
            !unique_term_builder
                .term_weights
                .contains_key(&format!("term{}", MAX_UNIQUE_TERMS))
        );
    }

    #[test]
    fn unauthorized_documents_cannot_change_authorized_order_or_counts() {
        let first = definition(
            "allowed.first",
            "allowed__first",
            "Find records.",
            json!({"type":"object","properties":{"assignee":{"type":"string"}}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let second = definition(
            "allowed.second",
            "allowed__second",
            "Find records.",
            json!({"type":"object","properties":{"assignee":{"type":"string"}}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let denied = definition(
            "denied.stuffed",
            "denied__stuffed",
            "assignee assignee assignee",
            json!({"type":"object","properties":{"assignee":{"type":"string"}}}),
            CapabilityDescriptionTrust::VerifiedCatalog,
        );
        let authorized = [&first, &second];
        let authorized_index = AuthorizedToolSearchIndex::new(authorized);
        let unfiltered_index = AuthorizedToolSearchIndex::new([&first, &second, &denied]);

        assert_eq!(authorized_index.document_frequencies["assignee"], 2);
        assert_eq!(unfiltered_index.document_frequencies["assignee"], 3);
        assert_eq!(
            unfiltered_index.search("assignee", 10).names,
            vec!["denied__stuffed", "allowed__first", "allowed__second"],
            "a denied document would change corpus statistics, ordering, and result count if admitted"
        );
        assert_eq!(
            authorized_index.search("assignee", 10).names,
            vec!["allowed__first", "allowed__second"]
        );
    }

    #[test]
    fn exact_names_and_canonical_ids_win_with_deterministic_ties() {
        let exact = definition(
            "github.list_issues",
            "github__list_issues",
            "List issues.",
            json!({"type":"object","properties":{}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let noisy = definition(
            "fixture.github_list_issues_helper",
            "fixture__github_list_issues_helper",
            "github list issues github list issues",
            json!({"type":"object","properties":{}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let index = AuthorizedToolSearchIndex::new([&noisy, &exact]);

        for query in ["github.list_issues", "github__list_issues"] {
            assert_eq!(index.search(query, 1).names, vec!["github__list_issues"]);
            assert_eq!(
                index.search(query, 1).query_class,
                SearchQueryClass::ExactIdentifier
            );
        }
    }

    #[test]
    fn equal_scores_tie_break_by_capability_id_not_provider_name() {
        let later_id_earlier_name = definition(
            "zeta.same",
            "aaa__tool",
            "shared vocabulary",
            json!({"type":"object","properties":{}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let earlier_id_later_name = definition(
            "alpha.same",
            "zzz__tool",
            "shared vocabulary",
            json!({"type":"object","properties":{}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let index =
            AuthorizedToolSearchIndex::new([&later_id_earlier_name, &earlier_id_later_name]);

        assert_eq!(
            index.search("shared vocabulary", 2).names,
            vec!["zzz__tool", "aaa__tool"]
        );
    }

    #[test]
    fn provider_terms_keep_provider_weight() {
        let tool = definition(
            "github.list_issues",
            "github__list_issues",
            "List issues.",
            json!({"type":"object","properties":{}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let document = IndexedDocument::new(&tool);

        assert_eq!(document.term_weights["github"], PROVIDER_WEIGHT);
        assert_eq!(document.term_weights["list"], NAME_WEIGHT);
        assert_eq!(document.term_weights["issues"], NAME_WEIGHT);
    }

    #[test]
    fn query_term_budget_uses_only_the_first_unique_terms() {
        let tool = definition(
            "fixture.tail",
            "fixture__tail",
            "tail_canary",
            json!({"type":"object","properties":{}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let index = AuthorizedToolSearchIndex::new([&tool]);
        let first_terms = (0..MAX_QUERY_TERMS)
            .map(|term| format!("noise{term}"))
            .collect::<Vec<_>>()
            .join(" ");

        assert!(
            index
                .search(&format!("{first_terms} tail_canary"), 1)
                .names
                .is_empty(),
            "the 33rd unique term must not affect retrieval"
        );
        assert_eq!(
            index.search(&format!("tail_canary {first_terms}"), 1).names,
            vec!["fixture__tail"]
        );
    }

    #[test]
    fn empty_index_returns_no_match() {
        let index = AuthorizedToolSearchIndex::new(std::iter::empty());

        assert_eq!(
            index.search("anything", 5),
            SearchOutcome {
                names: Vec::new(),
                query_class: SearchQueryClass::NoMatch,
            }
        );
    }

    #[test]
    fn tokenize_splits_camel_case_and_acronym_boundaries() {
        assert_eq!(tokenize("createEvent"), vec!["create", "event"]);
        assert_eq!(tokenize("HTTPUrl"), vec!["http", "url"]);
    }

    #[test]
    fn fingerprint_covers_search_metadata_but_ignores_json_object_order() {
        let first = definition(
            "fixture.lookup",
            "fixture__lookup",
            "Lookup.",
            json!({"type":"object","properties":{"alpha":{"type":"string"},"beta":{"type":"string"}}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let reordered = definition(
            "fixture.lookup",
            "fixture__lookup",
            "Lookup.",
            serde_json::from_str(
                r#"{"properties":{"beta":{"type":"string"},"alpha":{"type":"string"}},"type":"object"}"#,
            )
            .expect("valid reordered schema"),
            CapabilityDescriptionTrust::Untrusted,
        );
        let changed = definition(
            "fixture.lookup",
            "fixture__lookup",
            "Lookup.",
            json!({"type":"object","properties":{"timezone":{"type":"string"}}}),
            CapabilityDescriptionTrust::Untrusted,
        );

        assert_eq!(
            definitions_fingerprint(std::slice::from_ref(&first)),
            definitions_fingerprint(&[reordered])
        );
        assert_ne!(
            definitions_fingerprint(&[first]),
            definitions_fingerprint(&[changed])
        );
    }

    #[test]
    fn fingerprint_is_stable_across_definition_order() {
        let first = definition(
            "fixture.first",
            "fixture__first",
            "First.",
            json!({"type":"object","properties":{"alpha":{"type":"string"}}}),
            CapabilityDescriptionTrust::Untrusted,
        );
        let second = definition(
            "fixture.second",
            "fixture__second",
            "Second.",
            json!({"type":"object","properties":{"beta":{"type":"string"}}}),
            CapabilityDescriptionTrust::VerifiedCatalog,
        );

        assert_eq!(
            definitions_fingerprint(&[first.clone(), second.clone()]),
            definitions_fingerprint(&[second, first])
        );
    }

    #[derive(Debug, Deserialize)]
    struct Corpus {
        tools: Vec<CorpusTool>,
        intents: Vec<CorpusIntent>,
    }

    #[derive(Debug, Deserialize)]
    struct CorpusTool {
        kind: String,
        capability_id: String,
        name: String,
        description: String,
        #[serde(default)]
        verified: bool,
        parameters: Value,
    }

    #[derive(Debug, Deserialize)]
    struct CorpusIntent {
        class: String,
        query: String,
        relevance: BTreeMap<String, u8>,
    }

    #[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
    struct QualityMetrics {
        recall_at_1: f64,
        recall_at_5: f64,
        recall_at_10: f64,
        mrr: f64,
        ndcg_at_10: f64,
        no_match_accuracy: f64,
    }

    #[derive(Debug, Deserialize)]
    struct ScaleBaseline {
        version: u32,
        seed: u64,
        intent_count: usize,
        synthetic_namespace_count: usize,
        cases: Vec<ScaleBaselineCase>,
    }

    #[derive(Debug, Deserialize)]
    struct ScaleBaselineCase {
        tool_count: usize,
        quality: QualityMetrics,
    }

    #[derive(Debug, Serialize)]
    struct ScaleBenchmarkReport {
        version: u32,
        seed: u64,
        intent_count: usize,
        cases: Vec<ScaleBenchmarkCaseReport>,
    }

    #[derive(Debug, Serialize)]
    struct ScaleBenchmarkCaseReport {
        tool_count: usize,
        synthetic_namespace_count: usize,
        index_build_micros: u128,
        query_total_micros: u128,
        quality: QualityMetrics,
    }

    #[derive(Debug)]
    struct QueryQuality {
        class: String,
        query: String,
        relevance: BTreeMap<String, u8>,
        recall_at_5: f64,
        ndcg_at_10: f64,
        ranking: Vec<String>,
    }

    #[test]
    fn committed_corpus_quality_gate_and_benchmark_report() {
        let corpus: Corpus =
            serde_json::from_str(include_str!("../tests/fixtures/tool_search_relevance.json"))
                .expect("committed tool-search corpus is valid");
        let kinds: BTreeSet<_> = corpus.tools.iter().map(|tool| tool.kind.as_str()).collect();
        for required in ["builtin", "mcp", "wasm", "extension_lifecycle", "provider"] {
            assert!(
                kinds.contains(required),
                "corpus is missing {required} tools"
            );
        }
        assert!(
            corpus.tools.len() >= 50,
            "quality corpus must contain at least 50 tools, found {}",
            corpus.tools.len()
        );
        assert!(
            (60..=100).contains(&corpus.intents.len()),
            "quality corpus must contain 60-100 judged queries, found {}",
            corpus.intents.len()
        );
        let tool_names: BTreeSet<_> = corpus.tools.iter().map(|tool| tool.name.as_str()).collect();
        for intent in &corpus.intents {
            for (name, grade) in &intent.relevance {
                assert!(
                    tool_names.contains(name.as_str()),
                    "query {:?} judges unknown tool {name:?}",
                    intent.query
                );
                assert!(
                    (1..=3).contains(grade),
                    "query {:?} has invalid relevance grade {grade} for {name:?}",
                    intent.query
                );
            }
        }
        let definitions: Vec<_> = corpus.tools.iter().map(corpus_definition).collect();

        let build_started = Instant::now();
        let index = AuthorizedToolSearchIndex::new(definitions.iter());
        let index_build_micros = build_started.elapsed().as_micros();
        let query_started = Instant::now();
        let candidate_rankings: Vec<_> = corpus
            .intents
            .iter()
            .map(|intent| index.search(&intent.query, 10).names)
            .collect();
        let candidate_query_micros = query_started.elapsed().as_micros();
        let baseline_started = Instant::now();
        let baseline_rankings: Vec<_> = corpus
            .intents
            .iter()
            .map(|intent| legacy_rank(&definitions, &intent.query, 10))
            .collect();
        let baseline_query_micros = baseline_started.elapsed().as_micros();
        let candidate = quality_metrics(&corpus.intents, &candidate_rankings);
        let baseline = quality_metrics(&corpus.intents, &baseline_rankings);
        let candidate_queries = query_quality(&corpus.intents, &candidate_rankings);

        eprintln!(
            "tool-search corpus: baseline={baseline:?} candidate={candidate:?} index_build_us={index_build_micros} baseline_query_us={baseline_query_micros} candidate_query_us={candidate_query_micros}"
        );
        report_worst_queries(&candidate_queries);
        assert!(
            candidate.recall_at_1 >= 0.75,
            "candidate recall@1 gate: {candidate:?}"
        );
        assert!(
            candidate.recall_at_5 >= 0.90,
            "candidate recall@5 gate: {candidate:?}"
        );
        assert!(
            candidate.recall_at_10 >= 0.95,
            "candidate recall@10 gate: {candidate:?}"
        );
        assert!(candidate.mrr >= 0.85, "candidate MRR gate: {candidate:?}");
        assert!(
            candidate.ndcg_at_10 >= 0.90,
            "candidate nDCG@10 gate: {candidate:?}"
        );
        assert_eq!(
            candidate.no_match_accuracy, 1.0,
            "no-match gate: {candidate:?}"
        );
        assert!(candidate.recall_at_5 >= baseline.recall_at_5);
        assert!(candidate.recall_at_10 >= baseline.recall_at_10);
        assert!(candidate.mrr >= baseline.mrr);
        assert!(candidate.ndcg_at_10 >= baseline.ndcg_at_10);
        assert!(
            candidate.ndcg_at_10 - baseline.ndcg_at_10 >= 0.15,
            "candidate must materially improve nDCG@10 over baseline: baseline={baseline:?} candidate={candidate:?}"
        );

        let classes: BTreeSet<_> = corpus
            .intents
            .iter()
            .map(|intent| intent.class.as_str())
            .collect();
        for required in [
            "exact_name",
            "alias",
            "canonical_id",
            "parameter",
            "nested",
            "ambiguous",
            "provider",
            "hard_negative",
            "no_match",
        ] {
            assert!(
                classes.contains(required),
                "corpus is missing {required} intents"
            );
        }
        for class in classes.into_iter().filter(|class| *class != "no_match") {
            let class_queries: Vec<_> = candidate_queries
                .iter()
                .filter(|query| query.class == class)
                .collect();
            let class_recall_at_5 = mean(class_queries.iter().map(|query| query.recall_at_5));
            let class_ndcg_at_10 = mean(class_queries.iter().map(|query| query.ndcg_at_10));
            eprintln!(
                "tool-search class: class={class} queries={} recall@5={class_recall_at_5:.3} ndcg@10={class_ndcg_at_10:.3}",
                class_queries.len()
            );
            assert!(
                class_recall_at_5 >= 0.80,
                "class {class:?} recall@5 gate: {class_recall_at_5:.3}"
            );
            assert!(
                class_ndcg_at_10 >= 0.75,
                "class {class:?} nDCG@10 gate: {class_ndcg_at_10:.3}"
            );
        }
    }

    #[test]
    fn committed_scale_baseline_covers_100_500_and_1000_tools() {
        let corpus: Corpus =
            serde_json::from_str(include_str!("../tests/fixtures/tool_search_relevance.json"))
                .expect("committed tool-search corpus is valid");
        let baseline: ScaleBaseline = serde_json::from_str(include_str!(
            "../tests/fixtures/tool_search_scale_baseline.json"
        ))
        .expect("committed tool-search scale baseline is valid");

        assert_eq!(baseline.version, 1, "unknown scale baseline version");
        assert_eq!(
            baseline.intent_count,
            corpus.intents.len(),
            "scale baseline must cover the complete judged intent corpus"
        );
        assert_eq!(
            baseline
                .cases
                .iter()
                .map(|case| case.tool_count)
                .collect::<Vec<_>>(),
            vec![100, 500, 1_000]
        );
        let mut reports = Vec::new();
        for case in &baseline.cases {
            let first = scaled_definitions(&corpus, case.tool_count, baseline.seed);
            let second = scaled_definitions(&corpus, case.tool_count, baseline.seed);
            assert_eq!(first, second, "scaled catalog must be deterministic");
            assert_eq!(first.len(), case.tool_count);

            let synthetic_namespace_counts = synthetic_namespace_counts(&corpus, &first);
            assert_eq!(
                synthetic_namespace_counts.len(),
                baseline.synthetic_namespace_count,
                "every configured synthetic namespace must be represented"
            );
            let smallest_namespace = synthetic_namespace_counts
                .values()
                .min()
                .copied()
                .expect("scale fixture has synthetic namespaces");
            let largest_namespace = synthetic_namespace_counts
                .values()
                .max()
                .copied()
                .expect("scale fixture has synthetic namespaces");
            assert!(
                largest_namespace.saturating_sub(smallest_namespace) <= 1,
                "synthetic tools must be distributed evenly across namespaces: {synthetic_namespace_counts:?}"
            );

            let build_started = Instant::now();
            let index = AuthorizedToolSearchIndex::new(first.iter());
            let index_build_micros = build_started.elapsed().as_micros();
            let query_started = Instant::now();
            let rankings: Vec<_> = corpus
                .intents
                .iter()
                .map(|intent| index.search(&intent.query, 10).names)
                .collect();
            let query_total_micros = query_started.elapsed().as_micros();
            let quality = quality_metrics(&corpus.intents, &rankings);
            reports.push(ScaleBenchmarkCaseReport {
                tool_count: case.tool_count,
                synthetic_namespace_count: synthetic_namespace_counts.len(),
                index_build_micros,
                query_total_micros,
                quality,
            });
        }

        let report = ScaleBenchmarkReport {
            version: baseline.version,
            seed: baseline.seed,
            intent_count: corpus.intents.len(),
            cases: reports,
        };
        eprintln!(
            "tool-search scale baseline:\n{}",
            serde_json::to_string_pretty(&report).expect("serialize scale benchmark report")
        );
        for (actual, expected) in report.cases.iter().zip(&baseline.cases) {
            assert_quality_matches_baseline(actual.tool_count, actual.quality, expected.quality);
        }
    }

    fn scaled_definitions(
        corpus: &Corpus,
        tool_count: usize,
        seed: u64,
    ) -> Vec<ProviderToolDefinition> {
        assert!(
            tool_count >= corpus.tools.len(),
            "scale target cannot discard judged corpus tools"
        );
        let mut definitions: Vec<_> = corpus.tools.iter().map(corpus_definition).collect();
        let synthetic_count = tool_count - definitions.len();
        let namespace_offset = seed as usize % SCALE_NAMESPACES.len();
        let action_offset = seed as usize % SCALE_ACTIONS.len();
        let noun_offset = seed as usize % SCALE_NOUNS.len();
        for ordinal in 0..synthetic_count {
            let namespace = SCALE_NAMESPACES[(ordinal + namespace_offset) % SCALE_NAMESPACES.len()];
            let action = SCALE_ACTIONS[(ordinal + action_offset) % SCALE_ACTIONS.len()];
            let noun = SCALE_NOUNS[(ordinal + noun_offset) % SCALE_NOUNS.len()];
            let local_name = format!("{action}_{ordinal:04}");
            definitions.push(definition(
                &format!("{namespace}.{local_name}"),
                &format!("{namespace}__{local_name}"),
                &format!("{action} {noun} records in the {namespace} benchmark integration."),
                json!({
                    "type": "object",
                    "properties": {
                        format!("{noun}_id"): {"type": "string"},
                        "cursor": {"type": "string"},
                        "limit": {"type": "integer"}
                    },
                    "required": [format!("{noun}_id")]
                }),
                CapabilityDescriptionTrust::Untrusted,
            ));
        }
        definitions
    }

    fn corpus_definition(tool: &CorpusTool) -> ProviderToolDefinition {
        definition(
            &tool.capability_id,
            &tool.name,
            &tool.description,
            tool.parameters.clone(),
            if tool.verified {
                CapabilityDescriptionTrust::VerifiedCatalog
            } else {
                CapabilityDescriptionTrust::Untrusted
            },
        )
    }

    fn synthetic_namespace_counts(
        corpus: &Corpus,
        definitions: &[ProviderToolDefinition],
    ) -> BTreeMap<String, usize> {
        definitions.iter().skip(corpus.tools.len()).fold(
            BTreeMap::new(),
            |mut counts, definition| {
                let namespace = definition
                    .capability_id
                    .as_str()
                    .split_once('.')
                    .map_or(definition.capability_id.as_str(), |(namespace, _)| {
                        namespace
                    });
                counts
                    .entry(namespace.to_string())
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
                counts
            },
        )
    }

    fn assert_quality_matches_baseline(
        tool_count: usize,
        actual: QualityMetrics,
        expected: QualityMetrics,
    ) {
        for (name, actual, expected) in [
            ("recall_at_1", actual.recall_at_1, expected.recall_at_1),
            ("recall_at_5", actual.recall_at_5, expected.recall_at_5),
            ("recall_at_10", actual.recall_at_10, expected.recall_at_10),
            ("mrr", actual.mrr, expected.mrr),
            ("ndcg_at_10", actual.ndcg_at_10, expected.ndcg_at_10),
            (
                "no_match_accuracy",
                actual.no_match_accuracy,
                expected.no_match_accuracy,
            ),
        ] {
            assert!(
                (actual - expected).abs() <= QUALITY_BASELINE_TOLERANCE,
                "{tool_count}-tool {name} baseline drifted: expected {expected:.16}, actual {actual:.16}"
            );
        }
    }

    const QUALITY_BASELINE_TOLERANCE: f64 = 1e-12;

    const SCALE_NAMESPACES: [&str; 20] = [
        "asset_hub",
        "audit_log",
        "billing_ops",
        "compliance_archive",
        "content_registry",
        "customer_directory",
        "data_exchange",
        "device_fleet",
        "document_vault",
        "incident_queue",
        "inventory_ledger",
        "media_pipeline",
        "network_inventory",
        "quality_control",
        "research_catalog",
        "service_directory",
        "support_queue",
        "telemetry_store",
        "training_library",
        "workflow_admin",
    ];

    const SCALE_ACTIONS: [&str; 16] = [
        "archive_record",
        "compare_snapshot",
        "export_summary",
        "get_status",
        "inspect_artifact",
        "list_categories",
        "normalize_dataset",
        "record_checkpoint",
        "resolve_reference",
        "review_manifest",
        "summarize_usage",
        "sync_metadata",
        "validate_policy",
        "verify_checksum",
        "view_history",
        "write_annotation",
    ];

    const SCALE_NOUNS: [&str; 12] = [
        "artifact",
        "batch",
        "bundle",
        "checkpoint",
        "entry",
        "manifest",
        "record",
        "reference",
        "snapshot",
        "summary",
        "version",
        "workspace",
    ];

    fn legacy_rank(
        definitions: &[ProviderToolDefinition],
        query: &str,
        limit: usize,
    ) -> Vec<String> {
        let terms: Vec<_> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let mut scored = Vec::new();
        for definition in definitions {
            let blob = format!("{} {}", definition.name, definition.description).to_lowercase();
            let exact: BTreeSet<_> = blob.split_whitespace().collect();
            let score: u32 = terms
                .iter()
                .map(|term| {
                    if exact.contains(term.as_str()) {
                        10
                    } else if blob.contains(term) {
                        5
                    } else {
                        0
                    }
                })
                .sum::<u32>()
                .min(30);
            if score > 0 {
                scored.push((definition.name.to_string(), score));
            }
        }
        scored.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        scored
            .into_iter()
            .take(limit)
            .map(|(name, _)| name)
            .collect()
    }

    fn quality_metrics(intents: &[CorpusIntent], rankings: &[Vec<String>]) -> QualityMetrics {
        let relevant: Vec<_> = intents
            .iter()
            .zip(rankings)
            .filter(|(intent, _)| !intent.relevance.is_empty())
            .collect();
        let no_match: Vec<_> = intents
            .iter()
            .zip(rankings)
            .filter(|(intent, _)| intent.relevance.is_empty())
            .collect();
        // This is standard per-query recall: the denominator is the full judged
        // relevant set. Recall@1 is therefore intentionally below 1.0 for a
        // query with multiple relevant tools, even when its first result is
        // ideal. nDCG is the primary graded-ordering gate; these recall gates
        // protect breadth and are calibrated to the corpus's judgment density.
        let recall = |at: usize| {
            relevant
                .iter()
                .map(|(intent, ranking)| {
                    let retrieved = ranking
                        .iter()
                        .take(at)
                        .filter(|name| intent.relevance.contains_key(*name))
                        .count();
                    retrieved as f64 / intent.relevance.len() as f64
                })
                .sum::<f64>()
                / relevant.len() as f64
        };
        let mrr = relevant
            .iter()
            .map(|(intent, ranking)| {
                ranking
                    .iter()
                    .position(|name| intent.relevance.contains_key(name))
                    .map(|rank| 1.0 / (rank.saturating_add(1) as f64))
                    .unwrap_or(0.0)
            })
            .sum::<f64>()
            / relevant.len() as f64;
        let ndcg_at_10 = relevant
            .iter()
            .map(|(intent, ranking)| ndcg(intent, ranking, 10))
            .sum::<f64>()
            / relevant.len() as f64;
        let no_match_accuracy = if no_match.is_empty() {
            1.0
        } else {
            no_match
                .iter()
                .filter(|(_, ranking)| ranking.is_empty())
                .count() as f64
                / no_match.len() as f64
        };
        QualityMetrics {
            recall_at_1: recall(1),
            recall_at_5: recall(5),
            recall_at_10: recall(10),
            mrr,
            ndcg_at_10,
            no_match_accuracy,
        }
    }

    fn ndcg(intent: &CorpusIntent, ranking: &[String], at: usize) -> f64 {
        let dcg = ranking
            .iter()
            .take(at)
            .enumerate()
            .map(|(rank, name)| {
                let grade = intent.relevance.get(name).copied().unwrap_or_default();
                discounted_gain(grade, rank)
            })
            .sum::<f64>();
        let mut ideal: Vec<_> = intent.relevance.values().copied().collect();
        ideal.sort_unstable_by(|left, right| right.cmp(left));
        let ideal_dcg = ideal
            .into_iter()
            .take(at)
            .enumerate()
            .map(|(rank, grade)| discounted_gain(grade, rank))
            .sum::<f64>();
        if ideal_dcg == 0.0 {
            0.0
        } else {
            dcg / ideal_dcg
        }
    }

    fn discounted_gain(grade: u8, zero_based_rank: usize) -> f64 {
        (2_f64.powi(i32::from(grade)) - 1.0) / (zero_based_rank as f64 + 2.0).log2()
    }

    fn query_quality(intents: &[CorpusIntent], rankings: &[Vec<String>]) -> Vec<QueryQuality> {
        intents
            .iter()
            .zip(rankings)
            .filter(|(intent, _)| !intent.relevance.is_empty())
            .map(|(intent, ranking)| {
                let retrieved = ranking
                    .iter()
                    .take(5)
                    .filter(|name| intent.relevance.contains_key(*name))
                    .count();
                QueryQuality {
                    class: intent.class.clone(),
                    query: intent.query.clone(),
                    relevance: intent.relevance.clone(),
                    recall_at_5: retrieved as f64 / intent.relevance.len() as f64,
                    ndcg_at_10: ndcg(intent, ranking, 10),
                    ranking: ranking.clone(),
                }
            })
            .collect()
    }

    fn report_worst_queries(queries: &[QueryQuality]) {
        let mut worst: Vec<_> = queries.iter().collect();
        worst.sort_by(|left, right| {
            left.ndcg_at_10
                .total_cmp(&right.ndcg_at_10)
                .then_with(|| left.query.cmp(&right.query))
        });
        for query in worst.into_iter().take(5) {
            eprintln!(
                "tool-search worst query: class={} query={:?} relevance={:?} recall@5={:.3} ndcg@10={:.3} ranking={:?}",
                query.class,
                query.query,
                query.relevance,
                query.recall_at_5,
                query.ndcg_at_10,
                query.ranking
            );
        }
    }

    fn mean(values: impl Iterator<Item = f64>) -> f64 {
        let values: Vec<_> = values.collect();
        assert!(
            !values.is_empty(),
            "metric class must contain judged queries"
        );
        values.iter().sum::<f64>() / values.len() as f64
    }
}
