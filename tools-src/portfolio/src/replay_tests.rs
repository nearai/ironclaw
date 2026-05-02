//! Replay-style tests driven by YAML scenarios under `scenarios/`.
//!
//! These tests bypass the WIT/WASM boundary and call `execute_inner`
//! directly. They exist to:
//!
//!   1. Catch regressions in the deterministic pipeline (indexer →
//!      analyzer → strategy → intents).
//!   2. Provide a data-driven way to add new scenarios without
//!      writing Rust — drop a YAML file, the harness picks it up.
//!   3. Be the seed of the M3+ replay suite, where we'll snapshot
//!      LLM-ranked outputs and widget JSON.
//!
//! Mission-level integration tests (driving through `MissionManager`)
//! land in M3 once the LLM transcripts and engine wiring stabilize.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::execute_inner;

#[derive(Debug, Deserialize)]
struct Scenario {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    #[serde(default)]
    description: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    name: String,
    action: String,
    params: Value,
    #[serde(default)]
    expect: BTreeMap<String, Value>,
    #[serde(default)]
    capture: BTreeMap<String, String>,
    /// If present, the step must fail with an error whose message
    /// contains this substring. Mutually exclusive with `expect`.
    #[serde(default)]
    expect_error_contains: Option<String>,
}

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scenarios")
}

fn load_scenarios() -> Vec<(String, Scenario)> {
    let dir = scenarios_dir();
    let mut out = Vec::new();
    walk_scenarios(&dir, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk_scenarios(dir: &std::path::Path, out: &mut Vec<(String, Scenario)>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_scenarios(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let raw =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let scenario: Scenario =
            serde_yaml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        out.push((path.display().to_string(), scenario));
    }
}

/// Substitute `$varname` placeholders inside a JSON Value tree with
/// values captured from previous steps. Only top-level string values
/// of the form `"$name"` are substituted, which is enough for the
/// scenarios we ship in M1.
fn substitute(value: &mut Value, vars: &BTreeMap<String, Value>) {
    match value {
        Value::String(s) if s.starts_with('$') => {
            let name = &s[1..];
            if let Some(replacement) = vars.get(name) {
                *value = replacement.clone();
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                substitute(item, vars);
            }
        }
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                substitute(v, vars);
            }
        }
        _ => {}
    }
}

fn run_scenario(path: &str, scenario: Scenario) {
    let mut vars: BTreeMap<String, Value> = BTreeMap::new();
    let mut last_responses: BTreeMap<String, Value> = BTreeMap::new();

    for step in scenario.steps {
        let mut params = step.params.clone();
        if let Value::Object(ref mut map) = params {
            map.insert("action".to_string(), Value::String(step.action.clone()));
        } else {
            panic!("[{path}] step '{}': params must be an object", step.name);
        }
        substitute(&mut params, &vars);

        let params_str = serde_json::to_string(&params).expect("serialize params");
        let result = execute_inner(&params_str);

        if let Some(needle) = &step.expect_error_contains {
            let err = match result {
                Err(e) => e,
                Ok(ok) => panic!(
                    "[{path}] step '{}' ({}): expected error containing '{needle}' but got Ok: {ok}",
                    step.name, step.action
                ),
            };
            assert!(
                err.contains(needle),
                "[{path}] step '{}': error message '{err}' does not contain '{needle}'",
                step.name
            );
            continue;
        }

        let result = result.unwrap_or_else(|e| {
            panic!(
                "[{path}] step '{}' ({}): execute_inner failed: {e}",
                step.name, step.action
            )
        });
        let response: Value = serde_json::from_str(&result).unwrap_or_else(|e| {
            panic!(
                "[{path}] step '{}': response is not valid JSON: {e}\n  raw: {result}",
                step.name
            )
        });

        check_expectations(path, &step, &response, &last_responses);
        capture_vars(path, &step, &response, &mut vars);
        last_responses.insert(step.name.clone(), response);
    }
}

fn check_expectations(path: &str, step: &Step, response: &Value, prior: &BTreeMap<String, Value>) {
    for (key, expected) in &step.expect {
        match key.as_str() {
            "positions_len" => {
                let len = response
                    .get("positions")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'positions' array",
                            step.name
                        )
                    });
                let want = expected.as_u64().expect("positions_len: number") as usize;
                assert_eq!(
                    len, want,
                    "[{path}] step '{}': positions_len {} != expected {}",
                    step.name, len, want
                );
            }
            "positions_min" => {
                let len = response
                    .get("positions")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'positions' array",
                            step.name
                        )
                    });
                let want = expected.as_u64().expect("positions_min: number") as usize;
                assert!(
                    len >= want,
                    "[{path}] step '{}': positions {} < min {}",
                    step.name,
                    len,
                    want
                );
            }
            "contains_protocol_ids" => {
                let positions = response
                    .get("positions")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'positions' array",
                            step.name
                        )
                    });
                let observed: std::collections::BTreeSet<String> = positions
                    .iter()
                    .filter_map(|p| {
                        p.pointer("/protocol/id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect();
                let wanted: Vec<String> = expected
                    .as_array()
                    .expect("contains_protocol_ids: array")
                    .iter()
                    .map(|v| v.as_str().expect("string").to_string())
                    .collect();
                for id in &wanted {
                    assert!(
                        observed.contains(id),
                        "[{path}] step '{}': protocol id '{id}' not found in scan output (got {:?})",
                        step.name, observed
                    );
                }
            }
            "first_position_category" => {
                let cat = response
                    .pointer("/positions/0/category")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /positions/0/category",
                            step.name
                        )
                    });
                let want = expected.as_str().expect("first_position_category: string");
                assert_eq!(
                    cat, want,
                    "[{path}] step '{}': category mismatch",
                    step.name
                );
            }
            "first_position_protocol_id" => {
                let id = response
                    .pointer("/positions/0/protocol/id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /positions/0/protocol/id",
                            step.name
                        )
                    });
                let want = expected
                    .as_str()
                    .expect("first_position_protocol_id: string");
                assert_eq!(
                    id, want,
                    "[{path}] step '{}': protocol id mismatch",
                    step.name
                );
            }
            "source" => {
                let got = response
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': response missing 'source'", step.name)
                    });
                let want = expected.as_str().expect("source: string");
                assert_eq!(got, want, "[{path}] step '{}': source mismatch", step.name);
            }
            "proposals_len" => {
                let len = response
                    .get("proposals")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'proposals' array",
                            step.name
                        )
                    });
                let want = expected.as_u64().expect("proposals_len: number") as usize;
                assert_eq!(
                    len, want,
                    "[{path}] step '{}': proposals_len {} != expected {}",
                    step.name, len, want
                );
            }
            "ready_proposals_min" => {
                let proposals = response
                    .get("proposals")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'proposals' array",
                            step.name
                        )
                    });
                let ready = proposals
                    .iter()
                    .filter(|p| p.get("status").and_then(|v| v.as_str()) == Some("ready"))
                    .count();
                let want = expected.as_u64().expect("ready_proposals_min: number") as usize;
                assert!(
                    ready >= want,
                    "[{path}] step '{}': ready proposals {} < min {}",
                    step.name,
                    ready,
                    want
                );
            }
            "first_strategy_id" => {
                let id = response
                    .pointer("/proposals/0/strategy_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /proposals/0/strategy_id",
                            step.name
                        )
                    });
                let want = expected.as_str().expect("first_strategy_id: string");
                assert_eq!(
                    id, want,
                    "[{path}] step '{}': strategy_id mismatch",
                    step.name
                );
            }
            "bundle_legs_min" => {
                let legs = response
                    .pointer("/bundle/legs")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing /bundle/legs", step.name)
                    });
                let want = expected.as_u64().expect("bundle_legs_min: number") as usize;
                assert!(
                    legs.len() >= want,
                    "[{path}] step '{}': bundle has {} legs, min {}",
                    step.name,
                    legs.len(),
                    want
                );
            }
            "bundle_schema_version" => {
                let v = response
                    .pointer("/bundle/schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /bundle/schema_version",
                            step.name
                        )
                    });
                let want = expected.as_str().expect("bundle_schema_version: string");
                assert_eq!(
                    v, want,
                    "[{path}] step '{}': schema_version mismatch",
                    step.name
                );
            }
            "bundle_first_leg_kind" => {
                let kind = response
                    .pointer("/bundle/legs/0/kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing /bundle/legs/0/kind", step.name)
                    });
                let want = expected.as_str().expect("bundle_first_leg_kind: string");
                assert_eq!(
                    kind, want,
                    "[{path}] step '{}': first bundle leg kind mismatch",
                    step.name
                );
            }
            "backtest_schema_version" => {
                let v = response
                    .get("schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing schema_version", step.name)
                    });
                let want = expected.as_str().expect("backtest_schema_version: string");
                assert_eq!(
                    v, want,
                    "[{path}] step '{}': backtest schema mismatch",
                    step.name
                );
            }
            "backtest_trades_min" => {
                let trades = response
                    .pointer("/metrics/trades")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing /metrics/trades", step.name)
                    });
                let want = expected.as_u64().expect("backtest_trades_min: number");
                assert!(
                    trades >= want,
                    "[{path}] step '{}': backtest trades {} < min {}",
                    step.name,
                    trades,
                    want
                );
            }
            "backtest_total_return_gt" => {
                let value = response
                    .pointer("/metrics/total_return_pct")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /metrics/total_return_pct",
                            step.name
                        )
                    });
                let want = expected.as_f64().expect("backtest_total_return_gt: number");
                assert!(
                    value > want,
                    "[{path}] step '{}': total_return_pct {} <= {}",
                    step.name,
                    value,
                    want
                );
            }
            "backtest_max_drawdown_le" => {
                let value = response
                    .pointer("/metrics/max_drawdown_pct")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /metrics/max_drawdown_pct",
                            step.name
                        )
                    });
                let want = expected.as_f64().expect("backtest_max_drawdown_le: number");
                assert!(
                    value <= want,
                    "[{path}] step '{}': max_drawdown_pct {} > {}",
                    step.name,
                    value,
                    want
                );
            }
            "backtest_lookahead_safe" => {
                let value = response
                    .get("lookahead_safe")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing lookahead_safe", step.name)
                    });
                let want = expected.as_bool().expect("backtest_lookahead_safe: bool");
                assert_eq!(
                    value, want,
                    "[{path}] step '{}': lookahead_safe mismatch",
                    step.name
                );
            }
            "backtest_suite_schema_version" => {
                let v = response
                    .get("schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing schema_version", step.name)
                    });
                let want = expected
                    .as_str()
                    .expect("backtest_suite_schema_version: string");
                assert_eq!(
                    v, want,
                    "[{path}] step '{}': backtest suite schema mismatch",
                    step.name
                );
            }
            "backtest_suite_ranked_min" => {
                let ranked = response
                    .get("ranked")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing ranked array", step.name)
                    });
                let want = expected
                    .as_u64()
                    .expect("backtest_suite_ranked_min: number")
                    as usize;
                assert!(
                    ranked.len() >= want,
                    "[{path}] step '{}': ranked candidates {} < min {}",
                    step.name,
                    ranked.len(),
                    want
                );
            }
            "backtest_suite_top_trades_min" => {
                let trades = response
                    .pointer("/ranked/0/metrics/trades")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': missing /ranked/0/metrics/trades",
                            step.name
                        )
                    });
                let want = expected
                    .as_u64()
                    .expect("backtest_suite_top_trades_min: number");
                assert!(
                    trades >= want,
                    "[{path}] step '{}': top-ranked trades {} < min {}",
                    step.name,
                    trades,
                    want
                );
            }
            "backtest_suite_any_passes_basic_gate" => {
                let ranked = response
                    .get("ranked")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing ranked array", step.name)
                    });
                let got = ranked.iter().any(|result| {
                    result
                        .get("passes_basic_gate")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                });
                let want = expected
                    .as_bool()
                    .expect("backtest_suite_any_passes_basic_gate: bool");
                assert_eq!(
                    got, want,
                    "[{path}] step '{}': basic-gate match mismatch",
                    step.name
                );
            }
            "equal_to_step" => {
                let other_step = expected.as_str().expect("equal_to_step: string");
                let prior_resp = prior.get(other_step).unwrap_or_else(|| {
                    panic!(
                        "[{path}] step '{}': referenced step '{other_step}' has no prior response",
                        step.name
                    )
                });
                assert_eq!(
                    response, prior_resp,
                    "[{path}] step '{}': response differs from step '{other_step}' (expected idempotent)",
                    step.name
                );
            }
            "has_ready_proposal_matching_rationale" => {
                let substr = expected
                    .as_str()
                    .expect("has_ready_proposal_matching_rationale: string");
                let proposals = response
                    .get("proposals")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'proposals' array",
                            step.name
                        )
                    });
                let found = proposals.iter().any(|p| {
                    p.get("status").and_then(|v| v.as_str()) == Some("ready")
                        && p.get("rationale")
                            .and_then(|v| v.as_str())
                            .map(|r| r.contains(substr))
                            .unwrap_or(false)
                });
                assert!(
                    found,
                    "[{path}] step '{}': no ready proposal with rationale matching '{substr}'",
                    step.name
                );
            }
            "markdown_contains" => {
                let substr = expected.as_str().expect("markdown_contains: string");
                let md = response
                    .get("markdown")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "[{path}] step '{}': response missing 'markdown' string",
                            step.name
                        )
                    });
                assert!(
                    md.contains(substr),
                    "[{path}] step '{}': markdown missing substring '{substr}'",
                    step.name
                );
            }
            "realized_apy_ge" => {
                let got = response
                    .get("realized_net_apy_7d")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing realized_net_apy_7d", step.name)
                    });
                let want = expected.as_f64().expect("realized_apy_ge: number");
                assert!(
                    got >= want - 1e-9,
                    "[{path}] step '{}': realized_apy {got} < {want}",
                    step.name
                );
            }
            "widget_schema_version" => {
                let v = response
                    .get("schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing schema_version", step.name)
                    });
                let want = expected.as_str().expect("widget_schema_version: string");
                assert_eq!(
                    v, want,
                    "[{path}] step '{}': widget schema mismatch",
                    step.name
                );
            }
            "widget_positions_min" => {
                let len = response
                    .get("positions")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let want = expected.as_u64().expect("widget_positions_min: number") as usize;
                assert!(
                    len >= want,
                    "[{path}] step '{}': widget positions {len} < min {want}",
                    step.name
                );
            }
            "widget_top_suggestions_max" => {
                let len = response
                    .get("top_suggestions")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let want = expected
                    .as_u64()
                    .expect("widget_top_suggestions_max: number")
                    as usize;
                assert!(
                    len <= want,
                    "[{path}] step '{}': widget top_suggestions {len} > max {want}",
                    step.name
                );
            }
            "widget_has_non_empty_totals" => {
                let want = expected
                    .as_bool()
                    .expect("widget_has_non_empty_totals: bool");
                let totals = response
                    .get("totals")
                    .unwrap_or_else(|| panic!("[{path}] step '{}': missing totals", step.name));
                let net = totals
                    .get("net_value_usd")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0");
                let non_empty = net.parse::<f64>().unwrap_or(0.0) > 0.0;
                assert_eq!(
                    non_empty, want,
                    "[{path}] step '{}': widget_has_non_empty_totals mismatch",
                    step.name
                );
            }
            "intents_widget_schema_version" => {
                let v = response
                    .get("schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing schema_version", step.name)
                    });
                let want = expected
                    .as_str()
                    .expect("intents_widget_schema_version: string");
                assert_eq!(
                    v, want,
                    "[{path}] step '{}': intents widget schema mismatch",
                    step.name
                );
            }
            "intents_widget_top_candidates_min" => {
                let len = response
                    .get("top_candidates")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let want = expected
                    .as_u64()
                    .expect("intents_widget_top_candidates_min: number")
                    as usize;
                assert!(
                    len >= want,
                    "[{path}] step '{}': intents widget top candidates {len} < min {want}",
                    step.name
                );
            }
            "intents_widget_intent_status" => {
                let got = response
                    .pointer("/intent/status")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing /intent/status", step.name)
                    });
                let want = expected
                    .as_str()
                    .expect("intents_widget_intent_status: string");
                assert_eq!(
                    got, want,
                    "[{path}] step '{}': intents widget intent status mismatch",
                    step.name
                );
            }
            "paid_research_schema_version" => {
                let v = response
                    .get("schema_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing schema_version", step.name)
                    });
                let want = expected
                    .as_str()
                    .expect("paid_research_schema_version: string");
                assert_eq!(
                    v, want,
                    "[{path}] step '{}': paid research schema mismatch",
                    step.name
                );
            }
            "paid_research_selected_min" => {
                let len = response
                    .get("selected_sources")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let want = expected
                    .as_u64()
                    .expect("paid_research_selected_min: number")
                    as usize;
                assert!(
                    len >= want,
                    "[{path}] step '{}': paid research selected {len} < min {want}",
                    step.name
                );
            }
            "paid_research_allocated_le" => {
                let allocated = response
                    .get("allocated_usd")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing allocated_usd", step.name)
                    });
                let want = expected
                    .as_f64()
                    .expect("paid_research_allocated_le: number");
                assert!(
                    allocated <= want + f64::EPSILON,
                    "[{path}] step '{}': paid research allocated {allocated} > {want}",
                    step.name
                );
            }
            "paid_research_has_rail" => {
                let rail = expected.as_str().expect("paid_research_has_rail: string");
                let rails = response
                    .get("payment_rails")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        panic!("[{path}] step '{}': missing payment_rails", step.name)
                    });
                let found = rails
                    .iter()
                    .any(|item| item.get("protocol").and_then(|v| v.as_str()) == Some(rail));
                assert!(
                    found,
                    "[{path}] step '{}': paid research rail '{rail}' not found",
                    step.name
                );
            }
            "paid_research_near_funding_routes_min" => {
                let len = response
                    .get("near_funding_routes")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let want = expected
                    .as_u64()
                    .expect("paid_research_near_funding_routes_min: number")
                    as usize;
                assert!(
                    len >= want,
                    "[{path}] step '{}': NEAR funding routes {len} < min {want}",
                    step.name
                );
            }
            "intents_widget_paid_sources_min" => {
                let len = response
                    .pointer("/paid_research/payable_sources")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let want = expected
                    .as_u64()
                    .expect("intents_widget_paid_sources_min: number")
                    as usize;
                assert!(
                    len >= want,
                    "[{path}] step '{}': intents widget paid sources {len} < min {want}",
                    step.name
                );
            }
            "intents_widget_paid_ready" => {
                let got = response
                    .pointer("/paid_research/ready_for_paid_fetch")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let want = expected.as_bool().expect("intents_widget_paid_ready: bool");
                assert_eq!(
                    got, want,
                    "[{path}] step '{}': paid research ready mismatch",
                    step.name
                );
            }
            other => panic!(
                "[{path}] step '{}': unknown expectation key '{other}'",
                step.name
            ),
        }
    }
}

fn capture_vars(path: &str, step: &Step, response: &Value, vars: &mut BTreeMap<String, Value>) {
    for (capture_key, var_name) in &step.capture {
        let value = match capture_key.as_str() {
            "positions_var" => response
                .get("positions")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
            "proposals_var" => response
                .get("proposals")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
            "paid_research_plan_var" => response.clone(),
            "first_ready_plan_var" => response
                .get("proposals")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|p| p.get("status").and_then(|v| v.as_str()) == Some("ready"))
                })
                .and_then(|p| p.get("movement_plan").cloned())
                .unwrap_or_else(|| {
                    panic!(
                        "[{path}] step '{}': no ready proposal to capture plan from",
                        step.name
                    )
                }),
            other => panic!(
                "[{path}] step '{}': unknown capture key '{other}'",
                step.name
            ),
        };
        vars.insert(var_name.clone(), value);
    }
}

#[test]
fn replay_all_scenarios() {
    let scenarios = load_scenarios();
    assert!(
        !scenarios.is_empty(),
        "no scenarios found under {}",
        scenarios_dir().display()
    );
    for (path, scenario) in scenarios {
        eprintln!("running scenario: {}", scenario.id);
        run_scenario(&path, scenario);
    }
}
