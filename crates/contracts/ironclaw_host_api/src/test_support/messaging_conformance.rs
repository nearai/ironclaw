//! Conformance test-support for the standardized messaging framework
//! (`docs/internal/superpowers/specs/2026-07-27-standardized-messaging-framework-design.md`
//! §10.4).
//!
//! Every extension that binds a tool to a [`StandardMessagingOp`]
//! (`ironclaw_host_api::messaging`) promises that tool's input/output shape
//! matches the host's canonical JSON Schema for that operation. These
//! helpers let an extension's own tests — first-party adapter tests, WASM
//! conformance suites (spec §10.4) — assert that promise directly against
//! the Task 1 registry, without hand-rolling `jsonschema::validator_for`
//! calls and schema plumbing per test.
//!
//! Every assertion here panics loudly, with the full issue list and instance
//! paths, on failure. That is a deliberate contrast with the runtime's
//! model-visible output enforcement
//! (`ironclaw_host_runtime::standard_op_output`, which bounds issues to 3
//! and strips instance values so nothing sensitive reaches a model or a
//! persisted row): this module only ever runs inside a test binary (the
//! `test-support` feature is never enabled by a shipped artifact), so
//! showing every issue and the offending value is the point, not a leak —
//! these are debugging surfaces for extension authors.

use crate::messaging::{StandardMessagingOp, StandardOpContract};

/// `op`'s canonical input JSON Schema, parsed. Panics if `op` is reserved
/// (`contract()` is `None`) — asking for a canonical contract on a reserved
/// op is a test-authoring mistake, not a condition to handle gracefully.
pub fn canonical_input_schema(op: StandardMessagingOp) -> serde_json::Value {
    parse_schema(op, "input", contract_or_panic(op).input_schema)
}

/// `op`'s canonical output JSON Schema, parsed. Panics if `op` is reserved
/// (`contract()` is `None`).
pub fn canonical_output_schema(op: StandardMessagingOp) -> serde_json::Value {
    parse_schema(op, "output", contract_or_panic(op).output_schema)
}

/// Panics with the issue list when `output` violates `op`'s canonical
/// output schema.
pub fn assert_canonical_output(op: StandardMessagingOp, output: &serde_json::Value) {
    let schema = canonical_output_schema(op);
    let validator = compile_validator(op, "output", &schema);
    let issues = collect_issues(&validator, output);
    if issues.is_empty() {
        return;
    }
    let op_name = op.op_name();
    let count = issues.len();
    let listed = issues.join("\n");
    panic!(
        "{op_name} output failed its canonical output schema ({count} issue(s)):\n\
         {listed}\n\
         output: {output}"
    );
}

/// Panics when `input` is NOT accepted by `op`'s canonical input schema.
pub fn assert_canonical_input_accepted(op: StandardMessagingOp, input: &serde_json::Value) {
    let schema = canonical_input_schema(op);
    let validator = compile_validator(op, "input", &schema);
    let issues = collect_issues(&validator, input);
    if issues.is_empty() {
        return;
    }
    let op_name = op.op_name();
    let count = issues.len();
    let listed = issues.join("\n");
    panic!(
        "{op_name} input was expected to be ACCEPTED by the canonical input \
         schema but was rejected ({count} issue(s)):\n\
         {listed}\n\
         input: {input}"
    );
}

/// Panics when `input` IS accepted by `op`'s canonical input schema — for
/// `additionalProperties`/closed-input checks that expect rejection.
pub fn assert_canonical_input_rejected(op: StandardMessagingOp, input: &serde_json::Value) {
    let schema = canonical_input_schema(op);
    let validator = compile_validator(op, "input", &schema);
    if !validator.is_valid(input) {
        return;
    }
    let op_name = op.op_name();
    panic!(
        "{op_name} input was expected to be REJECTED by the canonical input \
         schema (e.g. an additionalProperties/closed-input check) but was \
         accepted: {input}"
    );
}

/// Extracts the `message_ref` object from a write output (send/edit) for
/// the evidence loop: send's ref feeds edit/delete/react inputs. Panics if
/// `output` has no `message_ref` field.
pub fn message_ref_from_output(output: &serde_json::Value) -> serde_json::Value {
    output.get("message_ref").cloned().unwrap_or_else(|| {
        panic!("output has no \"message_ref\" field to extract as evidence: {output}")
    })
}

/// `op`'s contract, or a caller-bug-shaped panic for a reserved op.
fn contract_or_panic(op: StandardMessagingOp) -> &'static StandardOpContract {
    op.contract()
        .unwrap_or_else(|| panic!("{} is reserved, no canonical contract", op.op_name()))
}

/// Parses one `&'static str` canonical schema (`label` is `"input"` or
/// `"output"`, for the panic message only). Parse failure here is
/// impossible by construction — Task 1's own `sixteen_core_ops_have_complete_contracts`
/// test in `crate::messaging` already parses and compiles every core
/// contract's schemas — but a thin wrapper still fails loud instead of
/// silently producing `Value::Null` if that construction invariant is ever
/// broken.
fn parse_schema(
    op: StandardMessagingOp,
    label: &str,
    schema_text: &'static str,
) -> serde_json::Value {
    serde_json::from_str(schema_text).unwrap_or_else(|e| {
        panic!(
            "{} canonical {label} schema is not valid JSON: {e}",
            op.op_name()
        )
    })
}

/// Compiles a canonical schema `Value` into a [`jsonschema::Validator`].
/// Same "impossible by construction, still fails loud" reasoning as
/// [`parse_schema`].
fn compile_validator(
    op: StandardMessagingOp,
    label: &str,
    schema: &serde_json::Value,
) -> jsonschema::Validator {
    jsonschema::options()
        .should_validate_formats(true)
        .build(schema)
        .unwrap_or_else(|e| {
            panic!(
                "{} canonical {label} schema does not compile: {e}",
                op.op_name()
            )
        })
}

/// Every schema-validation issue against `instance`, formatted as
/// `"<instance path>: <message>"`. Unbounded and untruncated — see the
/// module doc comment for why that is correct here and not in the
/// runtime's model-visible enforcement.
fn collect_issues(validator: &jsonschema::Validator, instance: &serde_json::Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|error| {
            let path = error.instance_path().as_str();
            let path = if path.is_empty() { "<root>" } else { path };
            format!("{path}: {error}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::panic::{self, AssertUnwindSafe};

    use serde_json::json;

    use super::*;
    use crate::messaging::StandardMessagingOp::{EditMessage, ForwardMessage, SendMessage};

    #[test]
    fn assert_canonical_output_passes_for_a_valid_send_message_output() {
        let output = json!({ "message_ref": { "conversation": "C1", "message_id": "1" } });
        assert_canonical_output(SendMessage, &output);
    }

    #[test]
    fn assert_canonical_output_panics_when_message_ref_is_missing() {
        let output = json!({ "ok": true });
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            assert_canonical_output(SendMessage, &output)
        }));
        let payload = result.expect_err("expected a panic for a shape missing message_ref");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        // `{"ok": true}` against send_message's output schema fails two
        // independent keywords: `additionalProperties` (on "ok") and
        // `required` (missing "message_ref") — both issues, and the
        // issue-count marker, must survive in the panic message.
        assert!(message.contains("message_ref"), "{message}");
        assert!(message.contains("2 issue(s)"), "{message}");
    }

    #[test]
    fn assert_canonical_input_rejected_passes_for_an_extra_property() {
        // additionalProperties: false on send_message's input — closed-input check.
        let input = json!({ "conversation": "C", "text": "t", "extra": 1 });
        assert_canonical_input_rejected(SendMessage, &input);
    }

    #[test]
    fn assert_canonical_input_rejected_panics_when_input_is_actually_accepted() {
        let input = json!({ "conversation": "C", "text": "t" });
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            assert_canonical_input_rejected(SendMessage, &input)
        }));
        assert!(
            result.is_err(),
            "expected a panic: input should have been accepted, not rejected"
        );
    }

    #[test]
    fn assert_canonical_input_accepted_panics_when_input_is_rejected() {
        let input = json!({ "text": "missing the required conversation field" });
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            assert_canonical_input_accepted(SendMessage, &input)
        }));
        let payload = result.expect_err("expected a panic for a missing required field");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        // The single violation is the missing required "conversation"
        // property — both the offending property name and the issue-count
        // marker must survive in the panic message.
        assert!(message.contains("conversation"), "{message}");
        assert!(message.contains("1 issue(s)"), "{message}");
    }

    #[test]
    fn message_ref_round_trips_from_send_output_into_edit_input() {
        let send_output = json!({ "message_ref": { "conversation": "C1", "message_id": "42" } });
        assert_canonical_output(SendMessage, &send_output);

        let message_ref = message_ref_from_output(&send_output);
        let edit_input = json!({ "message_ref": message_ref, "text": "x" });
        assert_canonical_input_accepted(EditMessage, &edit_input);
    }

    #[test]
    fn message_ref_from_output_panics_when_the_field_is_absent() {
        let output = json!({ "ok": true });
        let result = panic::catch_unwind(AssertUnwindSafe(|| message_ref_from_output(&output)));
        assert!(result.is_err());
    }

    #[test]
    fn reserved_op_panics_with_a_clear_message() {
        let result =
            panic::catch_unwind(AssertUnwindSafe(|| canonical_input_schema(ForwardMessage)));
        let payload = result.expect_err("a reserved op must panic, not return a schema");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        assert!(message.contains("reserved"), "{message}");
        assert!(message.contains("no canonical contract"), "{message}");
    }

    #[test]
    fn canonical_schemas_parse_for_every_core_op() {
        use crate::messaging::StandardMessagingOp;

        for op in StandardMessagingOp::ALL
            .iter()
            .filter(|op| op.contract().is_some())
        {
            let _ = canonical_input_schema(*op);
            let _ = canonical_output_schema(*op);
        }
    }
}
