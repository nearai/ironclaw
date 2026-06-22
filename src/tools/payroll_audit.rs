//! Curated, human-readable audit lines for payroll chat-agent milestones.
//!
//! When the chat agent calls one of the payroll MCP tools, we emit a single
//! plain-English line summarising the milestone onto a dedicated tracing
//! target so it surfaces in the gateway log stream (the FE log page and
//! `GET /api/logs/events`). The lines are aggregate-only — cycle id, counts,
//! totals, Merkle root, pseudonymous employee ids — never per-employee names
//! or amounts.
//!
//! Target rationale (current behaviour): the default `RUST_LOG` is
//! `t3claw=info,tower_http=warn`, and tracing's `EnvFilter` disables any
//! target that matches no directive. A bare `payroll_audit` target would be
//! dropped, so the target sits under the `t3claw::` namespace
//! (`t3claw::payroll_audit`), which `t3claw=info` enables. The same
//! `EnvFilter` gates the `WebLogLayer` that feeds the SSE stream and the DB,
//! so an `info!`/`warn!` on this target both prints and persists.
//!
//! Structure: [`format_milestone`] is a pure, panic-free formatter (unit
//! tested); [`emit_payroll_milestone`] is the thin logging wrapper the engine
//! seams call. The formatter reads both the tool input params (for the
//! organisation and recipient ids) and the tool result (for cycle id, counts,
//! totals).

use serde_json::Value;

/// Bare MCP tool names that mark a payroll milestone. The engine seam hands us
/// the MCP-prefixed form (`t3n_mcp_runPayrollComputation`); [`canonical_tool`]
/// strips that prefix before matching against these.
const RUN: &str = "runPayrollComputation";
const ESCALATIONS: &str = "submitEscalationResolutions";
const DISBURSE: &str = "executeDisbursement";
const FINALIZE: &str = "finalizeAudit";

/// Strip the t3n MCP server prefix so the bare names above match.
///
/// MCP tools are registered with the agent as `<server>_<tool>` (see
/// `mcp_tool_id` in `tools::mcp::client`); the t3n server name normalises to
/// `t3n_mcp`, so the seam hands us e.g. `t3n_mcp_runPayrollComputation`.
/// Stripping the prefix leaves an already-bare name untouched, so either seam
/// form matches.
fn canonical_tool(tool_name: &str) -> &str {
    tool_name.strip_prefix("t3n_mcp_").unwrap_or(tool_name)
}

/// True when `tool_name` is one of the payroll milestone tools (after stripping
/// the MCP server prefix). Used by the engine seams to gate cheaply before
/// parsing any tool output.
pub fn is_payroll_milestone(tool_name: &str) -> bool {
    matches!(
        canonical_tool(tool_name),
        RUN | ESCALATIONS | DISBURSE | FINALIZE
    )
}

/// Emit the milestone line, if any, onto the `t3claw::payroll_audit` target.
///
/// `params` is the tool's input arguments (carries `org_did` and, for
/// disbursement, the recipient list); `result` is the tool's returned value
/// (`ToolOutput.result`) — for MCP tools a JSON string of the
/// `{ status, message, result: … }` envelope, unpacked by
/// [`descend_to_payload`]. `error` is `Some` when the tool call failed.
/// Success lines log at `info`, failures at `warn`.
pub fn emit_payroll_milestone(
    tool_name: &str,
    params: Option<&Value>,
    result: Option<&Value>,
    error: Option<&str>,
) {
    let Some(msg) = format_milestone(tool_name, params, result, error) else {
        return;
    };

    if error.is_some() {
        tracing::warn!(target: "t3claw::payroll_audit", "{msg}");
    } else {
        tracing::info!(target: "t3claw::payroll_audit", "{msg}");
    }
}

/// Format the human-readable milestone line for a payroll tool, or `None` if
/// `tool_name` is not a payroll milestone.
///
/// Pure and panic-free: every field read is a fallible `get`/`and_then`, and
/// a milestone whose key fields are missing degrades to a minimal generic line
/// rather than returning `None` (so the milestone is always recorded once it
/// is recognised as payroll). When `error` is `Some`, a single failure line is
/// produced regardless of `result`.
pub fn format_milestone(
    tool_name: &str,
    params: Option<&Value>,
    result: Option<&Value>,
    error: Option<&str>,
) -> Option<String> {
    if !is_payroll_milestone(tool_name) {
        return None;
    }
    let canonical = canonical_tool(tool_name);

    // Navigate to the useful payload: prefer `.result` (re-parsing it when the
    // node hands it back as a JSON string), else fall back to the top-level
    // object. `payload` is `None` when no object is available; each formatter
    // then degrades to its generic line.
    let payload = result.and_then(descend_to_payload);
    let p = payload.as_ref();

    // The cycle id can come from the result payload (authoritative) or, when
    // the call failed before producing one, from the input params.
    let cycle = first_str(p, &["cycle_id"]).or_else(|| first_str(params, &["cycle_id"]));

    if let Some(err) = error {
        let step = match canonical {
            RUN => "computation",
            DISBURSE => "disbursement",
            FINALIZE => "finalisation",
            ESCALATIONS => "escalation resolution",
            _ => "step",
        };
        return Some(match cycle {
            Some(cycle) => format!("⚠️ Payroll {step} failed — cycle {cycle} · {err}"),
            None => format!("⚠️ Payroll {step} failed — {err}"),
        });
    }

    Some(match canonical {
        RUN => format_run(params, p, cycle),
        ESCALATIONS => format_escalations(p, cycle),
        DISBURSE => format_disburse(p, cycle),
        FINALIZE => format_finalize(p, cycle),
        _ => unreachable!("guarded by is_payroll_milestone"),
    })
}

/// Resolve the funnel's value to the useful payload object.
///
/// The shared funnel hands us `ToolOutput.result`. For MCP tools that is the
/// tool's textual output — a JSON *string* of the `{ status, message, result }`
/// envelope — so a top-level string is parsed first. Within the envelope,
/// `.result` may itself be an object or a re-serialised JSON string (the node
/// stringifies some contract outputs); both are handled. Falls back to the
/// envelope object itself for a flattened or unexpected shape.
fn descend_to_payload(value: &Value) -> Option<Value> {
    if let Value::String(s) = value {
        return serde_json::from_str::<Value>(s)
            .ok()
            .and_then(|v| descend_to_payload(&v));
    }
    match value.get("result") {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(Value::is_object)
            .or_else(|| value.is_object().then(|| value.clone())),
        Some(inner) if inner.is_object() => Some(inner.clone()),
        _ => value.is_object().then(|| value.clone()),
    }
}

/// First string field present among `keys`, trimmed of surrounding whitespace.
fn first_str<'a>(v: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    let obj = v?;
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// First integer field present among `keys`. Accepts JSON numbers and
/// numeric strings (the contract stringifies some counts for JS parity).
fn first_u64(v: Option<&Value>, keys: &[&str]) -> Option<u64> {
    let obj = v?;
    keys.iter().find_map(|k| {
        obj.get(*k).and_then(|field| {
            field
                .as_u64()
                .or_else(|| field.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
    })
}

/// Abbreviate a `did:t3n:<hex>` to its first 12 hex characters plus an
/// ellipsis, so a single org line stays readable. A short id (≤ 14 hex
/// characters) or a value that is not a `did:t3n:` DID is returned unchanged.
///
/// Truncation is by `chars().take(12)` rather than a byte slice so it can never
/// split a multi-byte boundary, even though hex DIDs are ASCII in practice.
fn shorten_did(did: &str) -> String {
    let Some(hex) = did.strip_prefix("did:t3n:") else {
        return did.to_string();
    };
    if hex.chars().count() <= 14 {
        return did.to_string();
    }
    let prefix: String = hex.chars().take(12).collect();
    format!("did:t3n:{prefix}…")
}

/// The organisation DID from the input params, abbreviated by [`shorten_did`].
/// Every payroll tool takes an `org_did`; `None` when it is absent.
fn org_short(params: Option<&Value>) -> Option<String> {
    first_str(params, &["org_did"]).map(shorten_did)
}

fn format_run(params: Option<&Value>, p: Option<&Value>, cycle: Option<&str>) -> String {
    let employees = first_u64(p, &["employee_count"]);
    // Prefer the MCP-enriched ready-to-render total; fall back to raw cents.
    let total = p
        .and_then(|obj| obj.get("batch_total"))
        .and_then(|m| m.get("display"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| first_str(p, &["batch_total_cents"]).map(str::to_owned));
    let flagged = first_u64(p, &["flagged_count"]).unwrap_or(0);

    match (cycle, employees, total) {
        (Some(cycle), Some(employees), Some(total)) => {
            // The org clause only appears here — it establishes org + cycle for
            // the whole run, so later lines need not repeat the long DID.
            let org_clause = match org_short(params) {
                Some(org) => format!("org {org}, "),
                None => String::new(),
            };
            let flagged_clause = if flagged > 0 {
                format!(" · {flagged} flagged")
            } else {
                String::new()
            };
            format!(
                "📋 Payroll cycle started — {org_clause}cycle {cycle} · {employees} employees · {total}{flagged_clause}"
            )
        }
        _ => "📋 Payroll cycle started".to_string(),
    }
}

fn format_escalations(p: Option<&Value>, cycle: Option<&str>) -> String {
    // Prefer explicit count fields; else derive from the resolved id arrays.
    let approved =
        first_u64(p, &["approved_count"]).or_else(|| array_len(p, "approved_employee_ids"));
    let rejected =
        first_u64(p, &["rejected_count"]).or_else(|| array_len(p, "rejected_employee_ids"));

    match (approved, rejected) {
        (Some(approved), Some(rejected)) => {
            let cycle_clause = cycle.map(|c| format!(" — cycle {c}")).unwrap_or_default();
            format!(
                "📋 Escalations resolved{cycle_clause} · {approved} approved, {rejected} rejected"
            )
        }
        _ => "📋 Escalations resolved".to_string(),
    }
}

fn format_disburse(p: Option<&Value>, cycle: Option<&str>) -> String {
    // Count succeeded rows from `disbursement_records`; fall back to explicit
    // count fields if a future shape provides them.
    let records = p
        .and_then(|obj| obj.get("disbursement_records"))
        .and_then(Value::as_array);
    let (success, total) = match records {
        Some(rows) => {
            let total = rows.len() as u64;
            let success = rows
                .iter()
                .filter(|row| row.get("status").and_then(Value::as_str) == Some("success"))
                .count() as u64;
            (Some(success), Some(total))
        }
        None => (
            first_u64(p, &["success_count", "succeeded"]),
            first_u64(p, &["total_count", "total"]),
        ),
    };

    match (success, total) {
        (Some(success), Some(total)) => {
            let cycle_clause = cycle.map(|c| format!(" — cycle {c}")).unwrap_or_default();
            let recipients = records
                .map(|rows| disburse_recipients(rows))
                .unwrap_or_default();
            format!(
                "📋 Payroll processed and paid{cycle_clause} · {success}/{total} paid{recipients}"
            )
        }
        _ => "📋 Payroll processed and paid".to_string(),
    }
}

/// Render the parenthetical recipient list for a disbursement.
///
/// Employee ids are pseudonymous (never names — names are PII the claw never
/// sees). With 1..=8 records we list ids: bare when every row succeeded, or
/// each suffixed with `✓`/`✗` when the outcome is mixed. With 0 or > 8 records
/// the parenthetical is omitted to keep the line short.
fn disburse_recipients(rows: &[Value]) -> String {
    if rows.is_empty() || rows.len() > 8 {
        return String::new();
    }
    let outcomes: Vec<(&str, bool)> = rows
        .iter()
        .filter_map(|row| {
            let id = row.get("employee_id").and_then(Value::as_str)?;
            let ok = row.get("status").and_then(Value::as_str) == Some("success");
            Some((id, ok))
        })
        .collect();
    if outcomes.is_empty() {
        return String::new();
    }
    let all_ok = outcomes.iter().all(|(_, ok)| *ok);
    let listed = outcomes
        .iter()
        .map(|(id, ok)| {
            if all_ok {
                (*id).to_string()
            } else if *ok {
                format!("{id} ✓")
            } else {
                format!("{id} ✗")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(" ({listed})")
}

fn format_finalize(p: Option<&Value>, cycle: Option<&str>) -> String {
    // The contract field is `merkle_root_hex`; accept `merkle_root` too in
    // case a future enrichment renames it.
    let root = first_str(p, &["merkle_root_hex", "merkle_root"]);
    let entries = first_u64(p, &["entry_count"]);

    match (cycle, root, entries) {
        (Some(cycle), Some(root), Some(entries)) => {
            let prefix: String = root.chars().take(10).collect();
            format!(
                "📋 Payroll completed and finalised — cycle {cycle} sealed · Merkle root {prefix}… · {entries} entries"
            )
        }
        _ => "📋 Payroll completed and finalised".to_string(),
    }
}

/// Length of the named array field, if present and an array.
fn array_len(v: Option<&Value>, key: &str) -> Option<u64> {
    v.and_then(|obj| obj.get(key))
        .and_then(Value::as_array)
        .map(|a| a.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_success_with_flagged() {
        let env = json!({
            "status": "success",
            "message": "Payroll computation completed",
            "result": {
                "status": "computed",
                "cycle_id": "2026-06",
                "employee_count": 10,
                "flagged_count": 2,
                "batch_total_cents": "1569000",
                "batch_total": { "display": "SGD 15,690.00", "currency": "SGD" }
            }
        });
        let msg = format_milestone(RUN, None, Some(&env), None).expect("payroll tool");
        assert!(msg.contains("Payroll cycle started"), "{msg}");
        assert!(msg.contains("cycle 2026-06"), "{msg}");
        assert!(msg.contains("10 employees"), "{msg}");
        assert!(msg.contains("SGD 15,690.00"), "{msg}");
        assert!(msg.contains("2 flagged"), "{msg}");
    }

    #[test]
    fn run_success_omits_flagged_when_zero() {
        let env = json!({
            "result": {
                "cycle_id": "2026-06",
                "employee_count": 10,
                "flagged_count": 0,
                "batch_total": { "display": "SGD 15,690.00" }
            }
        });
        let msg = format_milestone(RUN, None, Some(&env), None).unwrap();
        assert!(
            !msg.contains("flagged"),
            "should omit flagged clause: {msg}"
        );
        assert!(msg.contains("· SGD 15,690.00"), "{msg}");
    }

    #[test]
    fn run_success_with_org_enrichment() {
        // The started line carries the abbreviated org DID from params.
        let params = json!({
            "org_did": "did:t3n:a58bb2c562d4f8696bf68a3403591cfc6f84d82e",
            "cycle_id": "2026-06-e0971b"
        });
        let env = json!({
            "result": {
                "cycle_id": "2026-06-e0971b",
                "employee_count": 3,
                "flagged_count": 0,
                "batch_total": { "display": "SGD 15,690.00" }
            }
        });
        let msg = format_milestone(RUN, Some(&params), Some(&env), None).unwrap();
        assert!(msg.contains("org did:t3n:a58bb2c562d4…"), "{msg}");
        assert!(msg.contains("cycle 2026-06-e0971b"), "{msg}");
        assert!(msg.contains("3 employees"), "{msg}");
        assert!(msg.contains("SGD 15,690.00"), "{msg}");
    }

    #[test]
    fn run_missing_fields_generic_fallback() {
        let env = json!({ "status": "success", "message": "ok", "result": {} });
        let msg = format_milestone(RUN, None, Some(&env), None).unwrap();
        assert_eq!(msg, "📋 Payroll cycle started");
    }

    #[test]
    fn escalations_success_from_id_arrays() {
        // The contract returns id arrays, not counts; result is a JSON string.
        let inner = json!({
            "status": "ok",
            "cycle_id": "2026-06",
            "approved_employee_ids": ["e1", "e2", "e3"],
            "rejected_employee_ids": ["e4"]
        })
        .to_string();
        let env = json!({
            "status": "success",
            "message": "Escalation resolutions submitted",
            "result": inner
        });
        let msg = format_milestone(ESCALATIONS, None, Some(&env), None).unwrap();
        assert!(msg.contains("Escalations resolved"), "{msg}");
        assert!(msg.contains("cycle 2026-06"), "{msg}");
        assert!(msg.contains("3 approved"), "{msg}");
        assert!(msg.contains("1 rejected"), "{msg}");
    }

    #[test]
    fn escalations_success_from_explicit_counts() {
        let env = json!({
            "result": { "approved_count": 5, "rejected_count": 0 }
        });
        let msg = format_milestone(ESCALATIONS, None, Some(&env), None).unwrap();
        assert!(msg.contains("5 approved, 0 rejected"), "{msg}");
    }

    #[test]
    fn escalations_missing_fields_generic_fallback() {
        let env = json!({ "result": "{}" });
        let msg = format_milestone(ESCALATIONS, None, Some(&env), None).unwrap();
        assert_eq!(msg, "📋 Escalations resolved");
    }

    #[test]
    fn disburse_success_counts_rows() {
        // result is a JSON string (client.execute returns a string).
        let inner = json!({
            "status": "ok",
            "cycle_id": "2026-06",
            "disbursement_records": [
                { "employee_id": "e1", "status": "success", "reference": "r1" },
                { "employee_id": "e2", "status": "success", "reference": "r2" },
                { "employee_id": "e3", "status": "failed", "reference": "r3" }
            ]
        })
        .to_string();
        let env = json!({
            "status": "success",
            "message": "Disbursement executed",
            "result": inner
        });
        let msg = format_milestone(DISBURSE, None, Some(&env), None).unwrap();
        assert!(msg.contains("Payroll processed and paid"), "{msg}");
        assert!(msg.contains("2/3 paid"), "{msg}");
        assert!(msg.contains("cycle 2026-06"), "{msg}");
    }

    #[test]
    fn disburse_recipients_all_success_lists_bare_ids() {
        let inner = json!({
            "cycle_id": "2026-06",
            "disbursement_records": [
                { "employee_id": "E2E-E001", "status": "success" },
                { "employee_id": "E2E-E002", "status": "success" },
                { "employee_id": "E2E-E003", "status": "success" }
            ]
        });
        let env = json!({ "result": inner });
        let msg = format_milestone(DISBURSE, None, Some(&env), None).unwrap();
        assert!(msg.contains("3/3 paid"), "{msg}");
        assert!(msg.contains("(E2E-E001, E2E-E002, E2E-E003)"), "{msg}");
        // No tick/cross markers when all succeeded.
        assert!(!msg.contains('✓'), "{msg}");
        assert!(!msg.contains('✗'), "{msg}");
    }

    #[test]
    fn disburse_recipients_mixed_marks_each_outcome() {
        let inner = json!({
            "cycle_id": "2026-06",
            "disbursement_records": [
                { "employee_id": "E2E-E001", "status": "success" },
                { "employee_id": "E2E-E002", "status": "failed" },
                { "employee_id": "E2E-E003", "status": "success" }
            ]
        });
        let env = json!({ "result": inner });
        let msg = format_milestone(DISBURSE, None, Some(&env), None).unwrap();
        assert!(msg.contains("2/3 paid"), "{msg}");
        assert!(msg.contains("E2E-E001 ✓"), "{msg}");
        assert!(msg.contains("E2E-E002 ✗"), "{msg}");
        assert!(msg.contains("E2E-E003 ✓"), "{msg}");
    }

    #[test]
    fn disburse_recipients_omitted_when_too_many() {
        let rows: Vec<Value> = (0..9)
            .map(|i| json!({ "employee_id": format!("E{i}"), "status": "success" }))
            .collect();
        let env = json!({ "result": { "cycle_id": "2026-06", "disbursement_records": rows } });
        let msg = format_milestone(DISBURSE, None, Some(&env), None).unwrap();
        assert!(msg.contains("9/9 paid"), "{msg}");
        // > 8 records: no parenthetical recipient list.
        assert!(!msg.contains('('), "{msg}");
    }

    #[test]
    fn disburse_missing_fields_generic_fallback() {
        let env = json!({ "result": "not-json" });
        let msg = format_milestone(DISBURSE, None, Some(&env), None).unwrap();
        assert_eq!(msg, "📋 Payroll processed and paid");
    }

    #[test]
    fn finalize_success_truncates_root() {
        let inner = json!({
            "status": "finalised",
            "cycle_id": "2026-06",
            "merkle_root_hex": "0123456789abcdef0123456789abcdef",
            "entry_count": 42
        })
        .to_string();
        let env = json!({
            "status": "success",
            "message": "Audit finalised",
            "result": inner
        });
        let msg = format_milestone(FINALIZE, None, Some(&env), None).unwrap();
        assert!(msg.contains("Payroll completed and finalised"), "{msg}");
        assert!(msg.contains("cycle 2026-06 sealed"), "{msg}");
        // First 10 chars of the root, then an ellipsis.
        assert!(msg.contains("Merkle root 0123456789…"), "{msg}");
        assert!(msg.contains("42 entries"), "{msg}");
    }

    #[test]
    fn finalize_missing_fields_generic_fallback() {
        let env = json!({ "result": {} });
        let msg = format_milestone(FINALIZE, None, Some(&env), None).unwrap();
        assert_eq!(msg, "📋 Payroll completed and finalised");
    }

    #[test]
    fn non_payroll_tool_returns_none() {
        let env = json!({ "result": { "anything": true } });
        assert!(format_milestone("web_fetch", None, Some(&env), None).is_none());
        assert!(format_milestone("memory_write", None, None, None).is_none());
    }

    #[test]
    fn error_case_uses_failure_shape() {
        let msg = format_milestone(DISBURSE, None, None, Some("InsufficientCredit")).unwrap();
        assert_eq!(msg, "⚠️ Payroll disbursement failed — InsufficientCredit");

        let msg = format_milestone(RUN, None, None, Some("boom")).unwrap();
        assert!(
            msg.starts_with("⚠️ Payroll computation failed — boom"),
            "{msg}"
        );

        let msg = format_milestone(FINALIZE, None, None, Some("x")).unwrap();
        assert!(msg.contains("finalisation failed"), "{msg}");

        let msg = format_milestone(ESCALATIONS, None, None, Some("y")).unwrap();
        assert!(msg.contains("escalation resolution failed"), "{msg}");
    }

    #[test]
    fn error_line_includes_cycle_from_params() {
        // No result, but params carry the cycle the call was attempting.
        let params = json!({
            "org_did": "did:t3n:a58bb2c562d4f8696bf68a3403591cfc6f84d82e",
            "cycle_id": "2026-06-e0971b"
        });
        let msg =
            format_milestone(DISBURSE, Some(&params), None, Some("InsufficientCredit")).unwrap();
        assert_eq!(
            msg,
            "⚠️ Payroll disbursement failed — cycle 2026-06-e0971b · InsufficientCredit"
        );
    }

    #[test]
    fn error_takes_precedence_over_result() {
        // Even with a populated result, an error produces the failure line.
        let env = json!({ "result": { "cycle_id": "c" } });
        let msg = format_milestone(RUN, None, Some(&env), Some("nope")).unwrap();
        assert!(msg.contains("computation failed — cycle c · nope"), "{msg}");
    }

    #[test]
    fn descends_top_level_when_no_result_key() {
        // Defensive: fields present at the top level (no `.result` wrapper).
        let env = json!({
            "cycle_id": "2026-06",
            "employee_count": 4,
            "batch_total": { "display": "SGD 1,000.00" }
        });
        let msg = format_milestone(RUN, None, Some(&env), None).unwrap();
        assert!(msg.contains("cycle 2026-06"), "{msg}");
        assert!(msg.contains("4 employees"), "{msg}");
    }

    #[test]
    fn matches_mcp_prefixed_tool_names() {
        // The engine seam hands us the MCP-prefixed form `t3n_mcp_<tool>`.
        assert!(is_payroll_milestone("t3n_mcp_runPayrollComputation"));
        assert!(is_payroll_milestone("t3n_mcp_executeDisbursement"));
        assert!(is_payroll_milestone("t3n_mcp_finalizeAudit"));
        assert!(is_payroll_milestone("t3n_mcp_submitEscalationResolutions"));
        // Bare names still match (defensive — either seam form works).
        assert!(is_payroll_milestone(RUN));

        // A non-payroll t3n tool does not.
        assert!(!is_payroll_milestone("t3n_mcp_listMyContext"));

        // Success line formats through the prefix.
        let env = json!({ "result": {
            "cycle_id": "2026-06",
            "employee_count": 3,
            "batch_total": { "display": "SGD 1,000.00" }
        }});
        let msg =
            format_milestone("t3n_mcp_runPayrollComputation", None, Some(&env), None).unwrap();
        assert!(msg.contains("Payroll cycle started"), "{msg}");
        assert!(msg.contains("cycle 2026-06"), "{msg}");

        // Error line resolves the step name through the prefix too.
        let msg =
            format_milestone("t3n_mcp_executeDisbursement", None, None, Some("boom")).unwrap();
        assert_eq!(msg, "⚠️ Payroll disbursement failed — boom");
    }

    #[test]
    fn parses_string_wrapped_envelope_from_funnel() {
        // The shared funnel passes `ToolOutput.result`, which for MCP tools is
        // a JSON *string* of the envelope — the real runtime shape.
        let env_text = json!({
            "status": "success",
            "message": "Payroll computation completed",
            "result": {
                "cycle_id": "2026-06-0ba009",
                "employee_count": 3,
                "flagged_count": 0,
                "batch_total": { "display": "SGD 15,690.00" }
            }
        })
        .to_string();
        let value = Value::String(env_text);
        let msg =
            format_milestone("t3n_mcp_runPayrollComputation", None, Some(&value), None).unwrap();
        assert!(msg.contains("cycle 2026-06-0ba009"), "{msg}");
        assert!(msg.contains("3 employees"), "{msg}");
        assert!(msg.contains("SGD 15,690.00"), "{msg}");

        // Double-nested: envelope string whose `.result` is itself a string
        // (the node re-serialises some contract outputs).
        let inner = json!({
            "status": "finalised",
            "cycle_id": "2026-06-0ba009",
            "merkle_root_hex": "b4718f96589c5e46950cb8423e69736b",
            "entry_count": 2
        })
        .to_string();
        let env_text = json!({
            "status": "success",
            "message": "Audit finalised",
            "result": inner
        })
        .to_string();
        let value = Value::String(env_text);
        let msg = format_milestone("t3n_mcp_finalizeAudit", None, Some(&value), None).unwrap();
        assert!(msg.contains("Merkle root b4718f9658…"), "{msg}");
        assert!(msg.contains("2 entries"), "{msg}");

        // Disbursement rows inside a string-wrapped envelope.
        let inner = json!({
            "cycle_id": "2026-06-0ba009",
            "disbursement_records": [
                { "employee_id": "E1", "status": "success" },
                { "employee_id": "E2", "status": "success" },
                { "employee_id": "E3", "status": "success" }
            ]
        })
        .to_string();
        let value = Value::String(json!({ "result": inner }).to_string());
        let msg =
            format_milestone("t3n_mcp_executeDisbursement", None, Some(&value), None).unwrap();
        assert!(msg.contains("3/3 paid"), "{msg}");
        assert!(msg.contains("cycle 2026-06-0ba009"), "{msg}");
    }

    #[test]
    fn shorten_did_truncates_long_hex() {
        assert_eq!(
            shorten_did("did:t3n:a58bb2c562d4f8696bf68a3403591cfc6f84d82e"),
            "did:t3n:a58bb2c562d4…"
        );
    }

    #[test]
    fn shorten_did_leaves_short_and_non_did_unchanged() {
        // ≤ 14 hex chars: returned unchanged.
        assert_eq!(shorten_did("did:t3n:abcd1234"), "did:t3n:abcd1234");
        assert_eq!(
            shorten_did("did:t3n:0123456789abcd"),
            "did:t3n:0123456789abcd"
        );
        // Not a did:t3n value: returned unchanged.
        assert_eq!(shorten_did("not-a-did"), "not-a-did");
        assert_eq!(shorten_did(""), "");
    }
}
