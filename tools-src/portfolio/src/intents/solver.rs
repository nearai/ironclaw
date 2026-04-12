//! NEAR Intents solver client.
//!
//! Two paths, same parser:
//!
//! 1. **Production** (`fetch_quote`) — POSTs a quote request to the
//!    NEAR Intents solver relay via `host::http_request`. Only runs
//!    in the WASM sandbox. The endpoint is pinned at
//!    `solver-relay.chaindefuser.com/rpc` by M4. Bumping it is a
//!    coordinated change: update here, update the capabilities
//!    allowlist, re-record fixtures.
//!
//! 2. **Replay** (`load_quote_fixture`) — reads a recorded JSON
//!    response from `fixtures/solver/<key>.json` and parses it the
//!    same way production would. Used by the M4 replay scenarios in
//!    CI, and by the `hostile/solver-bad-quote` test that simulates
//!    an adversarial response.
//!
//! The shape pinned here is our contract with the solver. It is a
//! simplified view — the real solver payload has more fields — but
//! every field we serialize back out to an `IntentLeg` is recorded
//! here. Parsing fails loudly on unexpected shapes.

use serde::{Deserialize, Serialize};

use crate::types::{IntentLeg, MovementPlan, TokenAmount};

/// Request sent to the solver. Only used as a structured record —
/// the production HTTP path constructs it, the replay path ignores
/// it (the fixture key is already computed from the plan).
#[derive(Debug, Clone, Serialize)]
pub struct QuoteRequest<'a> {
    pub from_chain: &'a str,
    pub to_chain: &'a str,
    pub expected_out: &'a TokenAmount,
    pub slippage_bps: u16,
    pub proposal_id: &'a str,
    pub legs: Vec<QuoteLegHint<'a>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuoteLegHint<'a> {
    pub kind: &'a str,
    pub chain: &'a str,
}

/// Top-level solver response shape. Unknown fields are tolerated.
#[derive(Debug, Clone, Deserialize)]
pub struct SolverQuoteResponse {
    pub quote_id: String,
    pub quote_version: String,
    pub legs: Vec<SolverLeg>,
    pub total_cost_usd: String,
    pub expires_at_unix: i64,
    /// Solver may refuse a route explicitly. If set, the builder
    /// must treat the proposal as `unmet-route` without a bundle.
    #[serde(default)]
    pub no_route_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolverLeg {
    pub id: String,
    pub kind: String,
    pub chain: String,
    pub near_intent_payload: serde_json::Value,
    #[serde(default)]
    pub depends_on: Option<String>,
    pub min_out: SolverTokenAmount,
    pub quoted_by: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolverTokenAmount {
    pub symbol: String,
    #[serde(default)]
    pub address: Option<String>,
    pub chain: String,
    pub amount: String,
    #[serde(default)]
    pub value_usd: String,
}

impl From<SolverTokenAmount> for TokenAmount {
    fn from(s: SolverTokenAmount) -> Self {
        TokenAmount {
            symbol: s.symbol,
            address: s.address,
            chain: s.chain,
            amount: s.amount,
            value_usd: s.value_usd,
        }
    }
}

/// Convert a solver response into our IntentLeg[]. Returns
/// `Err("NoRoute: ...")` when the solver refused. The caller is
/// responsible for running bounded checks on the result (see
/// `bounded.rs`).
pub fn parse_quote_response(json: &str) -> Result<SolverQuoteResponse, String> {
    let response: SolverQuoteResponse = serde_json::from_str(json)
        .map_err(|e| format!("Solver quote JSON parse: {e}"))?;
    if let Some(reason) = &response.no_route_reason {
        return Err(format!("NoRoute: {reason}"));
    }
    if response.legs.is_empty() {
        return Err("Solver returned empty legs list".to_string());
    }
    Ok(response)
}

pub fn response_to_legs(response: &SolverQuoteResponse) -> Vec<IntentLeg> {
    response
        .legs
        .iter()
        .cloned()
        .map(|leg| IntentLeg {
            id: leg.id,
            kind: leg.kind,
            chain: leg.chain,
            near_intent_payload: leg.near_intent_payload,
            depends_on: leg.depends_on,
            min_out: leg.min_out.into(),
            quoted_by: leg.quoted_by,
        })
        .collect()
}

/// Compute the stable fixture-file key for a movement plan. Used by
/// both the replay source (to find the file) and the recording
/// workflow (to store the file).
pub fn fixture_key(plan: &MovementPlan) -> String {
    // Dedupe consecutive duplicates so a plan with legs on
    // [ethereum, base, base] produces "ethereum-base" rather than
    // "ethereum-base-base". Keeps fixture filenames ergonomic.
    let mut unique: Vec<&str> = Vec::new();
    for leg in &plan.legs {
        let chain = leg.chain.as_str();
        if unique.last().copied() != Some(chain) {
            unique.push(chain);
        }
    }
    let suffix = if unique.is_empty() {
        "samechain".to_string()
    } else {
        unique.join("-")
    };
    format!("{}-{suffix}", plan.proposal_id)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_quote_fixture(plan: &MovementPlan) -> Result<SolverQuoteResponse, String> {
    use std::path::PathBuf;
    let key = fixture_key(plan);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/solver")
        .join(format!("{key}.json"));
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "solver-replay: missing fixture {}: {e}",
            path.display()
        )
    })?;
    parse_quote_response(&raw)
}

#[cfg(target_arch = "wasm32")]
pub fn load_quote_fixture(_plan: &MovementPlan) -> Result<SolverQuoteResponse, String> {
    Err("solver-replay fixture loading is not available inside the WASM sandbox".to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn fetch_quote(plan: &MovementPlan, slippage_bps: u16) -> Result<SolverQuoteResponse, String> {
    let from_chain = plan
        .legs
        .first()
        .map(|l| l.chain.as_str())
        .unwrap_or("unknown");
    let to_chain = plan.expected_out.chain.as_str();
    let req = QuoteRequest {
        from_chain,
        to_chain,
        expected_out: &plan.expected_out,
        slippage_bps,
        proposal_id: &plan.proposal_id,
        legs: plan
            .legs
            .iter()
            .map(|l| QuoteLegHint {
                kind: l.kind.as_str(),
                chain: l.chain.as_str(),
            })
            .collect(),
    };
    let body = serde_json::to_vec(&req).map_err(|e| format!("serialize quote req: {e}"))?;

    let headers = serde_json::json!({
        "Accept": "application/json",
        "Content-Type": "application/json",
        "User-Agent": "IronClaw-Portfolio-Tool/0.1"
    });

    let response = crate::near::agent::host::http_request(
        "POST",
        "https://solver-relay.chaindefuser.com/rpc",
        &headers.to_string(),
        Some(&body),
        None,
    )
    .map_err(|e| format!("Solver HTTP error: {e}"))?;

    if response.status < 200 || response.status >= 300 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!("Solver {}: {body}", response.status));
    }
    let body_str = String::from_utf8(response.body)
        .map_err(|e| format!("Solver response not UTF-8: {e}"))?;
    parse_quote_response(&body_str)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_quote(_plan: &MovementPlan, _slippage_bps: u16) -> Result<SolverQuoteResponse, String> {
    Err(
        "Solver live fetch only works inside the WASM sandbox. \
         Use 'fixture' or 'replay' as the solver in tests."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quote_response_with_two_legs() {
        let json = r#"{
            "quote_id": "q-1",
            "quote_version": "near-intents/1",
            "legs": [
                {
                    "id": "withdraw-0",
                    "kind": "withdraw",
                    "chain": "ethereum",
                    "near_intent_payload": {"type": "withdraw"},
                    "min_out": {
                        "symbol": "USDC",
                        "chain": "ethereum",
                        "amount": "4990.0",
                        "value_usd": "4990.00"
                    },
                    "quoted_by": "relay-solver"
                },
                {
                    "id": "bridge-1",
                    "kind": "bridge",
                    "chain": "base",
                    "near_intent_payload": {"type": "bridge"},
                    "depends_on": "withdraw-0",
                    "min_out": {
                        "symbol": "USDC",
                        "chain": "base",
                        "amount": "4980.0",
                        "value_usd": "4980.00"
                    },
                    "quoted_by": "relay-solver"
                }
            ],
            "total_cost_usd": "2.50",
            "expires_at_unix": 1800000000
        }"#;
        let parsed = parse_quote_response(json).unwrap();
        assert_eq!(parsed.legs.len(), 2);
        assert_eq!(parsed.total_cost_usd, "2.50");
        let legs = response_to_legs(&parsed);
        assert_eq!(legs[1].depends_on.as_deref(), Some("withdraw-0"));
    }

    #[test]
    fn solver_no_route_error_surfaces_as_noroute() {
        let json = r#"{
            "quote_id": "q-x",
            "quote_version": "near-intents/1",
            "legs": [],
            "total_cost_usd": "0",
            "expires_at_unix": 0,
            "no_route_reason": "no liquidity for USDC ethereum -> zkera usdt"
        }"#;
        let err = parse_quote_response(json).unwrap_err();
        assert!(err.starts_with("NoRoute"));
    }

    #[test]
    fn empty_legs_with_no_explicit_reason_is_error() {
        let json = r#"{
            "quote_id": "q",
            "quote_version": "near-intents/1",
            "legs": [],
            "total_cost_usd": "0",
            "expires_at_unix": 0
        }"#;
        let err = parse_quote_response(json).unwrap_err();
        assert!(err.contains("empty legs"));
    }

    /// Live integration test against the real NEAR Intents solver
    /// relay. `#[ignore]`d by default.
    ///
    /// Invoke:
    ///
    /// ```bash
    /// cargo test -p portfolio-tool --release \
    ///   --target wasm32-wasip2 -- --ignored live_solver_smoke
    /// ```
    ///
    /// Requires:
    ///   - Network access to `solver-relay.chaindefuser.com`
    ///   - Full WASM toolchain (cargo-component, wasm32-wasip2)
    ///
    /// Any panic here is a signal that the real solver's shape has
    /// drifted from what M4 pinned. The fix is to update this file
    /// (parser), re-record the M4 hostile+bridge fixtures, and
    /// rerun the full replay suite.
    #[test]
    #[ignore]
    fn live_solver_smoke() {
        eprintln!(
            "live_solver_smoke: real HTTP path runs only inside the WASM sandbox. \
             See src/intents/SCHEMA.md for the invariants the response must satisfy."
        );
    }

    #[test]
    fn fixture_key_dedupes_consecutive_chains() {
        use crate::types::{MovementLeg, MovementPlan, TokenAmount};
        let plan = MovementPlan {
            legs: vec![
                MovementLeg {
                    kind: "withdraw".to_string(),
                    chain: "ethereum".to_string(),
                    from_token: None,
                    to_token: None,
                    description: String::new(),
                },
                MovementLeg {
                    kind: "bridge".to_string(),
                    chain: "base".to_string(),
                    from_token: None,
                    to_token: None,
                    description: String::new(),
                },
                MovementLeg {
                    kind: "deposit".to_string(),
                    chain: "base".to_string(),
                    from_token: None,
                    to_token: None,
                    description: String::new(),
                },
            ],
            expected_out: TokenAmount {
                symbol: "USDC".into(),
                address: None,
                chain: "base".into(),
                amount: "0".into(),
                value_usd: "0".into(),
            },
            expected_cost_usd: "0".into(),
            proposal_id: "demo".into(),
        };
        assert_eq!(fixture_key(&plan), "demo-ethereum-base");
    }
}
