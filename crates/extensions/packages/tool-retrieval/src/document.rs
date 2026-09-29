//! The text embedded for one tool.
//!
//! One document per authorized definition: the tool's name, its description,
//! and its parameter names (with their descriptions when the catalog that
//! supplied them was verified). Every part is bounded, so a hostile or merely
//! enormous schema cannot make one document, or the batch it rides in,
//! arbitrarily large.
//!
//! The document text is sent to the embedding endpoint and nowhere else. It is
//! never logged; only its SHA-256 digest is kept, as the vector-cache key.

use ironclaw_host_api::capability::CapabilityDescriptionTrust;
use ironclaw_loop_contracts::ProviderToolDefinition;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Upper bound on one document, in UTF-8 bytes. Well under the embeddings
/// client's default per-input budget, so a document never trips it.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 4_096;
/// Upper bound on the tool description's share of a document.
const MAX_DESCRIPTION_BYTES: usize = 1_024;
/// Upper bound on one parameter entry (name plus description).
const MAX_PARAMETER_BYTES: usize = 256;
/// How deep the parameter walk descends into nested object schemas.
const MAX_SCHEMA_DEPTH: usize = 4;
/// How many parameters one document lists at most.
const MAX_PARAMETERS: usize = 64;
/// How many schema nodes the walk visits at most, whatever their shape.
const MAX_SCHEMA_NODES: usize = 256;

/// SHA-256 of a document's text: the vector-cache key.
pub(crate) type DocumentDigest = [u8; 32];

/// The embeddable text for one definition, plus its cache key.
#[derive(Clone)]
pub(crate) struct ToolDocument {
    pub(crate) text: String,
    pub(crate) digest: DocumentDigest,
}

impl std::fmt::Debug for ToolDocument {
    // The text is schema-derived and must never reach a log line.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolDocument")
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

impl ToolDocument {
    pub(crate) fn new(definition: &ProviderToolDefinition) -> Self {
        let mut text = String::new();
        text.push_str("tool: ");
        text.push_str(&humanize_identifier(definition.capability_id.as_str()));
        text.push('\n');

        let description = definition.description.trim();
        if !description.is_empty() {
            text.push_str("description: ");
            text.push_str(truncate_to_bytes(description, MAX_DESCRIPTION_BYTES));
            text.push('\n');
        }

        let mut walk = ParameterWalk {
            trusted_descriptions: definition.description_trust
                == CapabilityDescriptionTrust::VerifiedCatalog,
            entries: Vec::new(),
            nodes: 0,
        };
        walk.collect(&definition.parameters, 0);
        if !walk.entries.is_empty() {
            text.push_str("parameters:\n");
            for entry in &walk.entries {
                text.push_str("- ");
                text.push_str(entry);
                text.push('\n');
            }
        }

        let text = truncate_to_bytes(&text, MAX_DOCUMENT_BYTES).to_string();
        let digest = Sha256::digest(text.as_bytes()).into();
        Self { text, digest }
    }
}

/// `mail.send_message` reads as `mail send message`: the separators carry no
/// meaning for an embedding model and only fragment its tokens.
fn humanize_identifier(identifier: &str) -> String {
    identifier
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The longest prefix of `value` that fits in `max` bytes and ends on a
/// character boundary.
pub(crate) fn truncate_to_bytes(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.get(..end).unwrap_or_default()
}

struct ParameterWalk {
    trusted_descriptions: bool,
    entries: Vec<String>,
    nodes: usize,
}

impl ParameterWalk {
    fn collect(&mut self, schema: &Value, depth: usize) {
        if depth > MAX_SCHEMA_DEPTH || self.nodes >= MAX_SCHEMA_NODES {
            return;
        }
        let Some(object) = schema.as_object() else {
            return;
        };
        self.nodes += 1;

        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            // Sorted explicitly: the document text is a cache key, so it must
            // not depend on whether some crate in the build turned on
            // `serde_json/preserve_order`.
            let mut properties: Vec<_> = properties.iter().collect();
            properties.sort_by(|left, right| left.0.cmp(right.0));
            for (name, property) in properties {
                if self.entries.len() >= MAX_PARAMETERS {
                    return;
                }
                let mut entry = name.clone();
                // Nested descriptions only count when the catalog that
                // supplied them was verified, matching the native ranker: an
                // unverified schema must not be able to steer ranking with
                // free text it wrote itself.
                if self.trusted_descriptions
                    && let Some(description) = property.get("description").and_then(Value::as_str)
                {
                    let description = description.trim();
                    if !description.is_empty() {
                        entry.push_str(": ");
                        entry.push_str(description);
                    }
                }
                self.entries
                    .push(truncate_to_bytes(&entry, MAX_PARAMETER_BYTES).to_string());
                self.collect(property, depth + 1);
            }
        }
        if let Some(items) = object.get("items") {
            self.collect(items, depth + 1);
        }
        for keyword in ["anyOf", "oneOf", "allOf"] {
            if let Some(variants) = object.get(keyword).and_then(Value::as_array) {
                for variant in variants {
                    if self.nodes >= MAX_SCHEMA_NODES {
                        return;
                    }
                    self.collect(variant, depth + 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::CapabilityId;
    use serde_json::json;

    fn definition(parameters: Value, trust: CapabilityDescriptionTrust) -> ProviderToolDefinition {
        let mut definition = ProviderToolDefinition::from_parts(
            CapabilityId::new("code.search_code").expect("capability id"),
            "code__search_code",
            "Search code across repositories.",
            parameters,
        )
        .expect("definition");
        definition.description_trust = trust;
        definition
    }

    #[test]
    fn document_carries_name_description_and_parameters() {
        let document = ToolDocument::new(&definition(
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search terms"},
                    "filters": {
                        "type": "object",
                        "properties": {"language": {"type": "string"}}
                    }
                }
            }),
            CapabilityDescriptionTrust::VerifiedCatalog,
        ));
        assert_eq!(
            document.text,
            "tool: code search code\n\
             description: Search code across repositories.\n\
             parameters:\n\
             - filters\n\
             - language\n\
             - query: Search terms\n"
        );
    }

    #[test]
    fn unverified_parameter_descriptions_are_left_out() {
        let document = ToolDocument::new(&definition(
            json!({
                "type": "object",
                "properties": {"query": {"type": "string", "description": "Search terms"}}
            }),
            CapabilityDescriptionTrust::Untrusted,
        ));
        assert!(document.text.contains("- query\n"));
        assert!(!document.text.contains("Search terms"));
    }

    #[test]
    fn documents_are_bounded_and_char_safe() {
        let wide: serde_json::Map<String, Value> = (0..1_000)
            .map(|index| {
                (
                    format!("parameter_{index}_é"),
                    json!({"type": "string", "description": "ü".repeat(500)}),
                )
            })
            .collect();
        let document = ToolDocument::new(&definition(
            json!({"type": "object", "properties": wide}),
            CapabilityDescriptionTrust::VerifiedCatalog,
        ));
        assert!(document.text.len() <= MAX_DOCUMENT_BYTES);
        assert!(document.text.matches("\n- ").count() <= MAX_PARAMETERS);
    }

    #[test]
    fn digest_follows_the_text() {
        let first = ToolDocument::new(&definition(
            json!({"type": "object"}),
            CapabilityDescriptionTrust::Untrusted,
        ));
        let same = ToolDocument::new(&definition(
            json!({"type": "object"}),
            CapabilityDescriptionTrust::Untrusted,
        ));
        let changed = ToolDocument::new(&definition(
            json!({"type": "object", "properties": {"query": {"type": "string"}}}),
            CapabilityDescriptionTrust::Untrusted,
        ));
        assert_eq!(first.digest, same.digest);
        assert_ne!(first.digest, changed.digest);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        assert_eq!(truncate_to_bytes("héllo", 2), "h");
        assert_eq!(truncate_to_bytes("héllo", 3), "hé");
        assert_eq!(truncate_to_bytes("abc", 10), "abc");
    }
}
