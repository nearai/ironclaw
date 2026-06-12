//! Per-domain JSON Schema export — the "JSON Schemas" deliverable of epic
//! #3036 slice 1.
//!
//! The schemas are *generated* from the same serde types [`crate::parse`]
//! validates with, so the structural view cannot drift: `deny_unknown_fields`
//! becomes `additionalProperties: false`, optional fields stay optional, and
//! the opaque `extensions[].config` table stays opaque (it is validated
//! against the owning extension's schema at apply time, not here).
//!
//! On top of the generated structure, [`refine`] hand-mirrors the *small*
//! subset of `parser.rs` semantic validation that JSON Schema can express:
//! the api_version major lock (built from the same constants the parser
//! uses) and the harness `id`-xor-`inline` rule. Both are pinned by
//! agreement tests in `tests/schema.rs`.
//!
//! **The schemas are a pre-filter, not the authority.** Checks that JSON
//! Schema cannot express remain parser-only: the inline-secret scan, the
//! typed-ID grammars from `ironclaw_host_api`, extension `version`
//! requirement syntax, and file-ref containment. A document that passes
//! these schemas may still fail [`crate::parse`]; a document that fails them
//! will always fail `parse`. Schema-only consumers (admin-web import UI,
//! editor tooling, GitOps linters) get early structural feedback and must
//! still treat `parse` as the final gate.

use std::collections::BTreeMap;

use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Value, json};

use crate::parser::{API_VERSION_PREFIX, SUPPORTED_MAJOR};
use crate::schema::{
    AgentLoop, Blueprint, CapabilitySurface, Extension, HarnessBinding, Mission, Project,
    Providers, Runtime, Scope, Skill, SystemPrompt,
};

/// JSON Schema (draft 2020-12) for the whole `ironclaw.config/v1` document.
///
/// Generated from the same serde types [`crate::parse`] validates with
/// (structure cannot drift), plus two hand-mirrored semantic rules: the
/// api_version major lock and the harness `id`-xor-`inline` rule. **A
/// pre-filter, not the authority**: the inline-secret scan, typed-ID
/// grammars, extension `version` syntax, and file-ref containment are
/// parser-only — a document that passes this schema may still fail
/// [`crate::parse`]; one that fails it always fails `parse`.
pub fn blueprint_schema() -> Value {
    let mut schema = schema_of::<Blueprint>();
    if let Some(api_version) = schema.pointer_mut("/properties/api_version") {
        inject_api_version_pattern(api_version);
    }
    if let Some(harness) = schema.pointer_mut("/$defs/HarnessBinding") {
        inject_harness_exclusivity(harness);
    }
    schema
}

/// Per-domain JSON Schemas keyed by the blueprint table they describe.
///
/// Array-of-tables domains (`extensions`, `skills`, `missions`, `projects`)
/// map to their *entry* schema — the shape of one `[[...]]` element — which is
/// what a per-domain validator or editor needs. Single-table domains map to
/// the table shape. Every top-level v1 domain is present. The same
/// pre-filter-not-authority caveat as [`blueprint_schema`] applies.
pub fn domain_schemas() -> BTreeMap<&'static str, Value> {
    let mut harness = schema_of::<HarnessBinding>();
    inject_harness_exclusivity(&mut harness);
    BTreeMap::from([
        ("scope", schema_of::<Scope>()),
        ("system_prompt", schema_of::<SystemPrompt>()),
        ("providers", schema_of::<Providers>()),
        ("runtime", schema_of::<Runtime>()),
        ("agent_loop", schema_of::<AgentLoop>()),
        ("extensions", schema_of::<Extension>()),
        ("skills", schema_of::<Skill>()),
        ("missions", schema_of::<Mission>()),
        ("projects", schema_of::<Project>()),
        ("capability_surface", schema_of::<CapabilitySurface>()),
        ("harness", harness),
    ])
}

fn schema_of<T: JsonSchema>() -> Value {
    SchemaGenerator::default()
        .into_root_schema_for::<T>()
        .to_value()
}

/// Mirror of `parser::validate_api_version`: exact supported major plus any
/// `.digits` minor/patch tail. Built from the parser's own constants so the
/// two cannot disagree on prefix or major; the shape is pinned by an
/// agreement test over both accepted and rejected version strings.
fn inject_api_version_pattern(api_version: &mut Value) {
    // `.` is the only regex metacharacter in the prefix.
    let prefix = API_VERSION_PREFIX.replace('.', r"\.");
    let pattern = format!(r"^{prefix}{SUPPORTED_MAJOR}(\.[0-9]+)*$");
    if let Some(object) = api_version.as_object_mut() {
        object.insert("pattern".to_string(), Value::String(pattern));
    }
}

/// Mirror of the parser's harness rule: bind a registered harness by `id` or
/// define one `inline`, never both.
fn inject_harness_exclusivity(harness: &mut Value) {
    if let Some(object) = harness.as_object_mut() {
        object.insert("not".to_string(), json!({ "required": ["id", "inline"] }));
    }
}
