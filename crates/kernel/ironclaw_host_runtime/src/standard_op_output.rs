//! Post-dispatch output validation for standard messaging operations.
//!
//! The standardized messaging framework (spec:
//! `docs/superpowers/specs/2026-07-27-standardized-messaging-framework-design.md`)
//! gives every core [`StandardMessagingOp`] a canonical output JSON Schema
//! (`StandardOpContract::output_schema`, `ironclaw_host_api::messaging`).
//! Extensions implement vendor mechanics only; they cannot narrow or widen
//! that shape. This module is the runtime-side enforcement: after a
//! capability bound to a standard op dispatches successfully, its output is
//! checked against the op's canonical schema before the outcome is allowed to
//! become `Completed`. A violation is host-detected evidence that the
//! extension's tool implementation is not honoring the canonical contract —
//! the model can retry, adjust its call, or report the broken extension.
//!
//! Bespoke capabilities (`descriptor.standard_op == None`) are never touched
//! by this module; they keep whatever shape their own manifest-declared
//! schema describes.

use std::{collections::HashMap, sync::LazyLock};

use ironclaw_host_api::messaging::StandardMessagingOp;

/// Issues are bounded to the first 3 schema violations; each is truncated so
/// a pathological schema/output pairing cannot blow up a model-visible
/// summary or a persisted run-state row.
const MAX_ISSUES: usize = 3;
const MAX_ISSUE_CHARS: usize = 200;

/// Compiled validators for every core standard messaging op (the ones whose
/// `contract()` is `Some`). Built once per process; `jsonschema::Validator`
/// compilation walks the whole schema tree, so callers on the hot dispatch
/// path must not pay that cost per invocation.
///
/// An op is present in the map iff its canonical output schema compiled.
/// Task 1's `sixteen_core_ops_have_complete_contracts` test already compiles
/// every core contract's schemas, so a missing entry for a core op is
/// impossible by construction; [`standard_op_output_violations`] still
/// handles it without a panic (see that function's fallback arm).
static VALIDATORS: LazyLock<HashMap<StandardMessagingOp, jsonschema::Validator>> =
    LazyLock::new(build_validators);

fn build_validators() -> HashMap<StandardMessagingOp, jsonschema::Validator> {
    StandardMessagingOp::ALL
        .iter()
        .filter_map(|op| {
            // `contract()` returning `None` here is not a dropped error: it is
            // the correct outcome for a reserved op (spec §4) — those must not
            // get a map entry at all, so `standard_op_output_violations` can
            // tell "reserved, nothing to enforce" apart from "core op whose
            // schema failed to compile" by whether `op.contract()` is `Some`.
            let contract = op.contract()?;
            // silent-ok: parse/compile failure here is impossible by
            // construction (Task 1's `sixteen_core_ops_have_complete_contracts`
            // test parses and compiles every core contract's schemas at
            // `cargo test` time) and this is a `LazyLock` initializer, so
            // there is no `Result` to propagate to a caller. Dropping the op
            // from the map is safe *because* `standard_op_output_violations`
            // treats "core op absent from the map" as a violation rather than
            // a silent pass — the failure mode is fail-closed, not swallowed.
            let schema: serde_json::Value = serde_json::from_str(contract.output_schema).ok()?;
            let validator = jsonschema::options()
                .should_validate_formats(true)
                .build(&schema)
                .ok()?;
            Some((*op, validator))
        })
        .collect()
}

/// Validates a standard op's dispatch output against its canonical output
/// schema.
///
/// `None` means either `standard_op` is not a core op (a reserved name with
/// no contract — nothing to enforce) or `output` satisfies the canonical
/// schema. `Some(issues)` means `output` violates the canonical contract:
/// at most 3 schema-path summaries, each stripped of instance values and
/// bounded to 200 chars, safe for model visibility and for a persisted
/// run-state row.
pub(crate) fn standard_op_output_violations(
    standard_op: StandardMessagingOp,
    output: &serde_json::Value,
) -> Option<Vec<String>> {
    // Reserved ops (`contract() == None`) have no canonical shape to enforce.
    standard_op.contract()?;

    let Some(validator) = VALIDATORS.get(&standard_op) else {
        // Impossible by construction (see the `VALIDATORS` doc comment above),
        // but panicking here would put a crash on the dispatch-completion
        // path for every standard-op capability. Fail the output instead of
        // silently waving it through.
        return Some(vec!["canonical schema failed to compile".to_string()]);
    };

    let issues: Vec<String> = validator
        .iter_errors(output)
        .take(MAX_ISSUES)
        .map(bounded_issue)
        .collect();

    (!issues.is_empty()).then_some(issues)
}

/// Renders one validation error as a bounded, value-free issue string.
///
/// The `required` keyword's message names only the missing property, drawn
/// from our own canonical schema text (never from the tool's output), so it
/// is safe to surface verbatim. Every other keyword's `Display` embeds the
/// actual offending instance value (e.g. `"{value} is not of type ..."`), so
/// those are reduced to the keyword name only — informative enough to locate
/// the problem without echoing tool-output data back through a model-visible
/// or persisted summary.
fn bounded_issue(error: jsonschema::ValidationError<'_>) -> String {
    let path = safe_output_schema_path(error.instance_path().as_str());
    let keyword = error.kind().keyword();
    let detail = if keyword == "required" {
        error.to_string()
    } else {
        format!("{keyword} constraint violated")
    };
    truncate_chars(&format!("{detail} at {path}"), MAX_ISSUE_CHARS)
}

/// Local replica of `ironclaw_loop_host`'s `safe_schema_path_summary`
/// (`capability_port/provider_input.rs`): that function is private to its
/// module, and `ironclaw_host_runtime` sits below `ironclaw_loop_host` in the
/// dependency graph, so it cannot be reached from here. JSON-pointer path
/// segments are dot-joined and bracket-stripped; this never touches instance
/// values.
fn safe_output_schema_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "root".to_string();
    }
    trimmed
        .trim_start_matches('/')
        .replace(['/', '\\', '[', ']'], ".")
        .replace(['{', '}', '`', '<', '>'], "")
}

/// Truncates to at most `max_chars` *characters* (not bytes), so multi-byte
/// UTF-8 content is never split mid-character.
fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_send_output_passes() {
        let out =
            serde_json::json!({ "message_ref": { "conversation": "C1", "message_id": "168.1" } });
        assert!(standard_op_output_violations(StandardMessagingOp::SendMessage, &out).is_none());
    }

    #[test]
    fn vendor_key_is_admitted() {
        let out = serde_json::json!({ "message_ref": { "conversation": "C1", "message_id": "1" },
                                      "vendor": { "channel_name": "eng" } });
        assert!(standard_op_output_violations(StandardMessagingOp::SendMessage, &out).is_none());
    }

    #[test]
    fn missing_message_ref_is_a_violation() {
        let out = serde_json::json!({ "ok": true, "ts": "168.1" });
        let issues = standard_op_output_violations(StandardMessagingOp::SendMessage, &out)
            .expect("violation");
        assert!(issues.iter().any(|i| i.contains("message_ref")));
        assert!(issues.len() <= 3);
    }

    #[test]
    fn reserved_op_is_never_validated() {
        // `ForwardMessage` has no contract (`contract() == None`); any shape
        // passes because there is nothing to enforce yet.
        let out = serde_json::json!({ "whatever": true });
        assert!(standard_op_output_violations(StandardMessagingOp::ForwardMessage, &out).is_none());
    }

    #[test]
    fn issues_never_contain_the_offending_value() {
        // A type-mismatch violation must not echo the actual bad value back —
        // only the keyword and a sanitized path.
        let out =
            serde_json::json!({ "message_ref": { "conversation": "C1", "message_id": 12345 } });
        let issues = standard_op_output_violations(StandardMessagingOp::SendMessage, &out)
            .expect("violation");
        assert!(
            issues
                .iter()
                .any(|i| i.contains("type constraint violated"))
        );
        for issue in &issues {
            assert!(
                !issue.contains("12345"),
                "issue leaked instance value: {issue}"
            );
        }
    }

    #[test]
    fn root_pointer_variants_are_normalized() {
        assert_eq!(safe_output_schema_path(""), "root");
        assert_eq!(safe_output_schema_path("/"), "root");
    }

    #[test]
    fn issue_truncation_preserves_utf8_character_boundaries() {
        assert_eq!(truncate_chars("abéé", 3), "abé");
        assert_eq!(truncate_chars("abé", 3), "abé");
    }
}
