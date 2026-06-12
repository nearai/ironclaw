//! Tests for the generated per-domain JSON Schema artifacts (epic #3036
//! slice 1 "JSON Schemas"). The schemas are generated from the same serde
//! types `parse` uses, so these tests pin the agreement between the two
//! views: documents the parser accepts validate against the schema, and
//! documents the parser rejects (unknown keys) fail it.

use ironclaw_blueprint::{blueprint_schema, domain_schemas, parse};

const FULL: &str = include_str!("fixtures/full.toml");

fn to_json(toml_src: &str) -> serde_json::Value {
    let value: toml::Value = toml::from_str(toml_src).expect("valid toml");
    serde_json::to_value(value).expect("toml converts to json")
}

fn validator() -> jsonschema::Validator {
    jsonschema::validator_for(&blueprint_schema()).expect("blueprint schema compiles")
}

#[test]
fn full_document_validates_against_blueprint_schema() {
    let doc = to_json(FULL);
    let validator = validator();
    let errors: Vec<String> = validator
        .iter_errors(&doc)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "schema rejected a valid document: {errors:?}"
    );

    // And the parser agrees the document is valid.
    parse(FULL).expect("parser accepts the same document");
}

/// `deny_unknown_fields` must surface as `additionalProperties: false` so a
/// schema-only consumer (admin web, GitOps linter) rejects the same unknown
/// keys the parser rejects.
#[test]
fn schema_rejects_unknown_top_level_key() {
    let doc = to_json("api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\nbogus = true\n");
    assert!(
        !validator().is_valid(&doc),
        "unknown top-level key must fail schema validation"
    );
}

#[test]
fn schema_rejects_unknown_nested_key() {
    let doc = to_json(
        "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
         [runtime]\nprofile = \"HostedDev\"\nmystery = 1\n",
    );
    assert!(
        !validator().is_valid(&doc),
        "unknown nested key must fail schema validation"
    );
}

/// `extensions[].config` is opaque by contract (validated against the owning
/// extension's schema at apply time) — the blueprint schema must not reject
/// arbitrary shapes inside it.
#[test]
fn schema_keeps_extension_config_opaque() {
    let doc = to_json(
        "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
         [[extensions]]\nid = \"remittance\"\n\
         [extensions.config]\nanything = { nested = [1, 2, 3] }\n",
    );
    let validator = validator();
    let errors: Vec<String> = validator.iter_errors(&doc).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "opaque config rejected: {errors:?}");
}

/// The epic acceptance criteria name these domains explicitly; the export map
/// must carry a schema for each.
#[test]
fn domain_schemas_cover_epic_domains() {
    let schemas = domain_schemas();
    for domain in [
        "providers",
        "extensions",
        "skills",
        "missions",
        "projects",
        "capability_surface",
    ] {
        assert!(
            schemas.contains_key(domain),
            "missing per-domain schema for `{domain}`"
        );
    }
    // Each per-domain schema must itself compile as a JSON Schema.
    for (domain, schema) in &schemas {
        jsonschema::validator_for(schema)
            .unwrap_or_else(|e| panic!("schema for `{domain}` does not compile: {e}"));
    }
}

/// The api_version rule is hand-mirrored into the schema's `pattern` (built
/// from the parser's own constants). Pin the agreement: for version strings,
/// schema validity must equal parser validity in both directions.
#[test]
fn schema_and_parser_agree_on_api_version() {
    let validator = validator();
    let cases = [
        ("ironclaw.config/v1", true),
        ("ironclaw.config/v1.5", true),
        ("ironclaw.config/v1.2.3", true),
        ("ironclaw.config/v2", false),
        ("ironclaw.config/v10", false),
        ("ironclaw.config/v999", false),
        ("ironclaw.config/v2/v1", false),
        ("ironclaw.config/v1.", false),
        ("ironclaw.config/v1..2", false),
        ("ironclaw.config/v1x", false),
        ("bogus/v1", false),
    ];
    for (version, expected) in cases {
        let src = format!("api_version = \"{version}\"\nkind = \"Blueprint\"\n");
        let schema_ok = validator.is_valid(&to_json(&src));
        let parse_ok = parse(&src).is_ok();
        assert_eq!(
            schema_ok, expected,
            "schema verdict for `{version}` must be {expected}"
        );
        assert_eq!(
            parse_ok, expected,
            "parser verdict for `{version}` must be {expected}"
        );
    }
}

/// The harness `id`-xor-`inline` rule is hand-mirrored into the schema via a
/// `not: required [id, inline]` clause — both the root schema and the
/// per-domain harness schema must reject the ambiguous form the parser
/// rejects.
#[test]
fn schema_rejects_harness_id_plus_inline() {
    let doc = to_json(
        "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
         [harness]\nid = \"x\"\n[harness.inline]\nid = \"y\"\n",
    );
    assert!(
        !validator().is_valid(&doc),
        "blueprint schema must reject harness id + inline"
    );

    let schemas = domain_schemas();
    let harness_schema = schemas.get("harness").expect("harness schema");
    let harness_validator = jsonschema::validator_for(harness_schema).expect("compiles");
    let entry = to_json("id = \"x\"\n[inline]\nid = \"y\"\n");
    assert!(
        !harness_validator.is_valid(&entry),
        "harness domain schema must reject id + inline"
    );
    let ok_entry = to_json("id = \"x\"\n");
    assert!(
        harness_validator.is_valid(&ok_entry),
        "id-only form is valid"
    );
}

/// A single `[[extensions]]` entry validates against the per-domain entry
/// schema — the shape an editor or admin form validates field-by-field.
#[test]
fn extension_entry_validates_against_domain_schema() {
    let schemas = domain_schemas();
    let entry_schema = schemas.get("extensions").expect("extensions schema");
    let validator = jsonschema::validator_for(entry_schema).expect("compiles");

    let entry = to_json(
        "id = \"github-mcp\"\nversion = \"^0.4\"\ntrust = \"user_trusted\"\n\
         config = { default_org = \"acme-corp\" }\n",
    );
    assert!(validator.is_valid(&entry), "valid entry must pass");

    let bad = to_json("id = \"github-mcp\"\nunknown_field = true\n");
    assert!(
        !validator.is_valid(&bad),
        "unknown key in entry must fail the domain schema"
    );
}
