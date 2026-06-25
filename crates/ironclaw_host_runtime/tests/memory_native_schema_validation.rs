//! Schema-driven validation harness for the native memory capabilities
//! (issue #3537).
//!
//! The `ironclaw.memory.native` manifest declares an extension-local
//! `input_schema_ref` / `output_schema_ref` per capability. These tests prove
//! the schema-driven validation path: every declared schema resolves to a file
//! that compiles as a JSON Schema, and a representative instance validates (and
//! an invalid one is rejected) against the context-retrieve input schema.
//!
//! Live, `HostPortView`-mediated dispatch of these `host_internal` capabilities
//! (validate-input-pre-exec, validate-output-post-exec) is part of the
//! feature-gated SQL storage stack — see `docs/adr/0002-...`. These tests cover
//! the validation harness itself over the real bundled schemas.

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
fn context_retrieve_input_schema_accepts_valid_and_rejects_invalid() {
    let schema = load_schema("schemas/memory/context-retrieve.input.v1.json");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    // Valid: required `query` + `limit` within bounds.
    assert!(validator.is_valid(&json!({"query": "what did I say about budgets", "limit": 5})));

    // Invalid: `limit` exceeds the schema maximum (50).
    assert!(!validator.is_valid(&json!({"query": "x", "limit": 9999})));

    // Invalid: missing the required `query`.
    assert!(!validator.is_valid(&json!({"limit": 5})));

    // Invalid: unexpected property (schema is additionalProperties: false).
    assert!(!validator.is_valid(&json!({"query": "x", "limit": 5, "rogue": true})));
}
