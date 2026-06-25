//! Schema-driven validation harness for the native memory capabilities
//! (issue #3537).
//!
//! The live `ironclaw.memory.native` manifest declares an extension-local
//! `input_schema_ref` / `output_schema_ref` per model-facing capability
//! (`read`/`write`/`search`/`tree`). These tests prove the schema-driven
//! validation path: every declared schema resolves to a file that compiles as a
//! JSON Schema, and representative valid/invalid instances behave as expected
//! against each input schema.
//!
//! The schemas are served inline on the always-on lane (see
//! `first_party_tools::resolve_native_memory_input_schema_ref`); these tests
//! cover the bundled schema artifacts themselves.

use std::path::PathBuf;

use ironclaw_host_runtime::memory_native_extension::native_memory_manifest;
use serde_json::{Value, json};

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/memory_native")
}

fn load_schema(relative_ref: &str) -> Value {
    let path = assets_dir().join(relative_ref);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|err| panic!("schema {} must exist: {err}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|err| panic!("schema {} must be valid JSON: {err}", path.display()))
}

fn validator_for(relative_ref: &str) -> jsonschema::Validator {
    let schema = load_schema(relative_ref);
    jsonschema::validator_for(&schema)
        .unwrap_or_else(|err| panic!("schema {relative_ref} must compile: {err}"))
}

#[test]
fn every_native_capability_schema_compiles() {
    let manifest = native_memory_manifest().expect("native memory manifest must parse");
    assert!(!manifest.capabilities.is_empty());
    for capability in &manifest.capabilities {
        for schema_ref in [
            capability.input_schema_ref.as_str(),
            capability.output_schema_ref.as_str(),
        ] {
            let schema = load_schema(schema_ref);
            jsonschema::validator_for(&schema).unwrap_or_else(|err| {
                panic!(
                    "schema {schema_ref} for {} must compile: {err}",
                    capability.id
                )
            });
        }
    }
}

#[test]
fn document_read_input_schema_accepts_valid_and_rejects_invalid() {
    let validator = validator_for("schemas/memory/document-read.input.v1.json");

    assert!(validator.is_valid(&json!({"path": "notes/alpha.md"})));

    assert!(!validator.is_valid(&json!({})), "missing required path");
    assert!(
        !validator.is_valid(&json!({"path": ""})),
        "path minLength is 1"
    );
    assert!(
        !validator.is_valid(&json!({"path": "/etc/passwd"})),
        "absolute paths are rejected by the scoped-path pattern"
    );
    assert!(
        !validator.is_valid(&json!({"path": "notes/alpha.md", "rogue": 1})),
        "additionalProperties is false"
    );
}

#[test]
fn document_write_input_schema_accepts_valid_and_rejects_invalid() {
    let validator = validator_for("schemas/memory/document-write.input.v1.json");

    // The live write tool accepts flexible shapes (target defaults, append, or a
    // patch via old_string/new_string); `from_tool_input` enforces the actual
    // write-mode semantics, so the schema only fixes types + closes the object.
    assert!(validator.is_valid(&json!({"content": "hello", "target": "memory"})));
    assert!(
        validator.is_valid(&json!({"old_string": "a", "new_string": "b", "replace_all": true}))
    );

    assert!(
        !validator.is_valid(&json!({"content": "x", "rogue": 1})),
        "additionalProperties is false"
    );
    assert!(
        !validator.is_valid(&json!({"append": "yes"})),
        "append must be a boolean"
    );
}

#[test]
fn search_input_schema_accepts_valid_and_rejects_invalid() {
    let validator = validator_for("schemas/memory/search.input.v1.json");

    // `query` and its aliases (`q`/`text`/`pattern`) each satisfy the contract,
    // mirroring `MemoryServiceSearchRequest::from_tool_input`.
    assert!(validator.is_valid(&json!({"query": "budgets"})));
    assert!(validator.is_valid(&json!({"q": "budgets"})));
    assert!(validator.is_valid(&json!({"text": "budgets", "limit": 3})));

    assert!(
        !validator.is_valid(&json!({"limit": 5})),
        "at least one of query/q/text/pattern is required"
    );
    assert!(
        !validator.is_valid(&json!({"query": "x", "limit": 9999})),
        "limit maximum is 20"
    );
    assert!(
        !validator.is_valid(&json!({"query": "x", "rogue": true})),
        "additionalProperties is false"
    );
}

#[test]
fn tree_input_schema_accepts_valid_and_rejects_invalid() {
    let validator = validator_for("schemas/memory/tree.input.v1.json");

    assert!(validator.is_valid(&json!({})), "all fields are optional");
    assert!(validator.is_valid(&json!({"path": "notes", "depth": 2})));

    assert!(
        !validator.is_valid(&json!({"depth": 99})),
        "depth maximum is 10"
    );
    assert!(
        !validator.is_valid(&json!({"rogue": 1})),
        "additionalProperties is false"
    );
}
