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
        builder.add_field(capability_id, NAME_WEIGHT);
        builder.add_field(definition.name.as_str(), NAME_WEIGHT);
        if let Some(provider) = capability_id.split('.').next() {
            builder.add_field(provider, PROVIDER_WEIGHT);
        }
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
                self.collect_schema(schema, depth.saturating_add(1), trusted_descriptions);
            }
        }
        if let Some(items) = object.get("items") {
            match items {
                Value::Array(items) => {
                    for item in items.iter().take(MAX_SCHEMA_NODES) {
                        if self.schema_nodes >= MAX_SCHEMA_NODES {
                            break;
                        }
                        self.collect_schema(item, depth.saturating_add(1), trusted_descriptions);
                    }
                }
                _ => self.collect_schema(items, depth.saturating_add(1), trusted_descriptions),
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf"] {
            if let Some(variants) = object.get(keyword).and_then(Value::as_array) {
                for variant in variants.iter().take(MAX_SCHEMA_NODES) {
                    if self.schema_nodes >= MAX_SCHEMA_NODES {
                        break;
                    }
                    self.collect_schema(variant, depth.saturating_add(1), trusted_descriptions);
                }
            }
        }
        if let Some(additional) = object.get("additionalProperties")
            && additional.is_object()
        {
            self.collect_schema(additional, depth.saturating_add(1), trusted_descriptions);
        }
        for keyword in ["$defs", "definitions"] {
            if let Some(definitions) = object.get(keyword).and_then(Value::as_object) {
                for definition in definitions.values().take(MAX_SCHEMA_NODES) {
                    if self.schema_nodes >= MAX_SCHEMA_NODES {
                        break;
                    }
                    self.collect_schema(definition, depth.saturating_add(1), trusted_descriptions);
                }
            }
        }
    }
}

fn tokenize(value: &str) -> BTreeSet<String> {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_lowercase_or_digit = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if character.is_uppercase() && previous_lowercase_or_digit {
                normalized.push(' ');
            }
            for lowercase in character.to_lowercase() {
                normalized.push(lowercase);
            }
            previous_lowercase_or_digit = character.is_lowercase() || character.is_numeric();
        } else {
            normalized.push(' ');
            previous_lowercase_or_digit = false;
        }
    }
    normalized
        .split_whitespace()
        .filter(|term| !term.is_empty())
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
    use serde::Deserialize;
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
        expected: Vec<String>,
    }

    #[derive(Debug, Default)]
    struct QualityMetrics {
        recall_at_1: f64,
        recall_at_5: f64,
        recall_at_10: f64,
        mrr: f64,
        no_match_precision: f64,
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
        let definitions: Vec<_> = corpus
            .tools
            .iter()
            .map(|tool| {
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
            })
            .collect();

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

        eprintln!(
            "tool-search corpus: baseline={baseline:?} candidate={candidate:?} index_build_us={index_build_micros} baseline_query_us={baseline_query_micros} candidate_query_us={candidate_query_micros}"
        );
        assert!(
            candidate.recall_at_1 >= 0.80,
            "candidate recall@1 gate: {candidate:?}"
        );
        assert!(
            candidate.recall_at_5 >= 0.95,
            "candidate recall@5 gate: {candidate:?}"
        );
        assert!(
            candidate.recall_at_10 >= 0.95,
            "candidate recall@10 gate: {candidate:?}"
        );
        assert!(candidate.mrr >= 0.85, "candidate MRR gate: {candidate:?}");
        assert_eq!(
            candidate.no_match_precision, 1.0,
            "no-match gate: {candidate:?}"
        );
        assert!(candidate.recall_at_5 >= baseline.recall_at_5);
        assert!(candidate.recall_at_10 >= baseline.recall_at_10);
    }

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
            .filter(|(intent, _)| !intent.expected.is_empty())
            .collect();
        let no_match: Vec<_> = intents
            .iter()
            .zip(rankings)
            .filter(|(intent, _)| intent.expected.is_empty())
            .collect();
        let recall = |at: usize| {
            relevant
                .iter()
                .filter(|(intent, ranking)| {
                    ranking
                        .iter()
                        .take(at)
                        .any(|name| intent.expected.contains(name))
                })
                .count() as f64
                / relevant.len() as f64
        };
        let mrr = relevant
            .iter()
            .map(|(intent, ranking)| {
                ranking
                    .iter()
                    .position(|name| intent.expected.contains(name))
                    .map(|rank| 1.0 / (rank.saturating_add(1) as f64))
                    .unwrap_or(0.0)
            })
            .sum::<f64>()
            / relevant.len() as f64;
        let no_match_precision = if no_match.is_empty() {
            1.0
        } else {
            no_match
                .iter()
                .filter(|(_, ranking)| ranking.is_empty())
                .count() as f64
                / no_match.len() as f64
        };
        let classes: BTreeSet<_> = intents.iter().map(|intent| intent.class.as_str()).collect();
        for required in [
            "exact_name",
            "alias",
            "canonical_id",
            "parameter",
            "nested",
            "ambiguous",
            "provider",
            "no_match",
        ] {
            assert!(
                classes.contains(required),
                "corpus is missing {required} intents"
            );
        }
        QualityMetrics {
            recall_at_1: recall(1),
            recall_at_5: recall(5),
            recall_at_10: recall(10),
            mrr,
            no_match_precision,
        }
    }
}
