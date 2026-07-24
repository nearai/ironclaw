//! Schema-driven validation harness for the native memory capabilities
//! (issue #3537).
//!
//! The live `ironclaw.memory` manifest declares an extension-local
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
        let output_schema_ref = capability
            .output_schema_ref
            .as_ref()
            .map(|schema_ref| schema_ref.as_str());
        for schema_ref in
            std::iter::once(capability.input_schema_ref.as_str()).chain(output_schema_ref)
        {
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

    // Scoped-target tightening (mirrors document-read/tree.input): the `target`
    // accepts reserved names and relative document paths, but rejects blank
    // targets, absolute paths, `..` traversal, and backslash separators at the
    // schema — ahead of the provider, since a swapped provider (e.g. mem0) may
    // use the target verbatim.
    assert!(
        validator.is_valid(&json!({"target": "daily_log", "content": "x"})),
        "a reserved target name is accepted"
    );
    assert!(
        validator.is_valid(&json!({"target": "notes/sub", "content": "x"})),
        "a relative document path is accepted"
    );
    assert!(
        !validator.is_valid(&json!({"target": "/abs", "content": "x"})),
        "absolute target paths are rejected"
    );
    assert!(
        !validator.is_valid(&json!({"target": "   ", "content": "x"})),
        "blank target paths are rejected"
    );
    assert!(
        !validator.is_valid(&json!({"target": "../escape", "content": "x"})),
        "parent-dir traversal in target is rejected"
    );
    assert!(
        !validator.is_valid(&json!({"target": "notes\\evil", "content": "x"})),
        "backslash separators in target are rejected"
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
    // A single non-empty alias on its own is accepted.
    assert!(
        validator.is_valid(&json!({"pattern": "budgets"})),
        "a single non-empty alias is accepted"
    );

    assert!(
        !validator.is_valid(&json!({"limit": 5})),
        "at least one of query/q/text/pattern is required"
    );
    assert!(
        !validator.is_valid(&json!({"query": ""})),
        "an empty query is rejected by minLength"
    );
    assert!(
        !validator.is_valid(&json!({"q": ""})),
        "an empty alias is rejected by minLength"
    );
    assert!(
        !validator.is_valid(&json!({"query": "a", "text": "b"})),
        "conflicting aliases (more than one supplied) are rejected"
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
        validator.is_valid(&json!({"path": ""})),
        "an explicit empty path is the memory root"
    );
    assert!(
        validator.is_valid(&json!({"path": "notes/sub"})),
        "internal path separators are allowed"
    );

    // Scoped-path tightening (mirrors document-read.input): absolute paths and
    // traversal forms are rejected at the schema, ahead of the host filesystem gate.
    assert!(
        !validator.is_valid(&json!({"path": "/abs"})),
        "absolute paths are rejected"
    );
    assert!(
        !validator.is_valid(&json!({"path": "../escape"})),
        "parent-dir traversal is rejected"
    );
    assert!(
        !validator.is_valid(&json!({"path": "notes/../secrets"})),
        "embedded '..' traversal is rejected"
    );
    assert!(
        !validator.is_valid(&json!({"path": "notes\\evil"})),
        "backslash separators are rejected"
    );

    assert!(
        !validator.is_valid(&json!({"depth": 99})),
        "depth maximum is 10"
    );
    assert!(
        !validator.is_valid(&json!({"rogue": 1})),
        "additionalProperties is false"
    );
}

#[test]
fn document_read_output_schema_requires_word_count() {
    let validator = validator_for("schemas/memory/document-read.output.v1.json");

    // `read()` always returns word_count (via MemoryServiceReadResponse), so the
    // output schema requires it alongside path/content.
    assert!(validator.is_valid(&json!({
        "path": "notes/alpha.md",
        "content": "hello world",
        "word_count": 2
    })));
    assert!(
        !validator.is_valid(&json!({"path": "notes/alpha.md", "content": "hello world"})),
        "word_count is required"
    );
}
