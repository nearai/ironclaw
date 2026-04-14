//! E2E test: portfolio WASM tool with real wasmtime sandbox + fuel metering.
//!
//! Loads the compiled portfolio WASM binary into the test rig, replays an LLM
//! trace that calls portfolio.scan for a NEAR address, and verifies the WASM
//! tool executes successfully within fuel limits using canned HTTP responses.
//!
//! These tests are `#[ignore]` by default because they require a pre-compiled
//! WASM binary. Build it with:
//!   cargo component build -p portfolio-tool --target wasm32-wasip2 --release
//! Then run with:
//!   cargo test --features libsql --test e2e_wasm_portfolio -- --ignored

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use ironclaw::llm::recording::{HttpExchange, HttpExchangeRequest, HttpExchangeResponse};

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{
        LlmTrace, TraceExpects, TraceResponse, TraceStep, TraceToolCall,
    };

    const PORTFOLIO_WASM: &str =
        "tools-src/portfolio/target/wasm32-wasip2/release/portfolio_tool.wasm";
    const PORTFOLIO_CAPS: &str = "tools-src/portfolio/portfolio-tool.capabilities.json";

    /// Load a fixture file from the portfolio fixtures directory.
    fn load_fixture(name: &str) -> String {
        let path = format!("tools-src/portfolio/fixtures/near/{name}");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Missing fixture {path}: {e}. Run: cargo test -p portfolio-tool live_near_replay_fixture_record -- --ignored"))
    }

    fn json_ok(body: &str) -> HttpExchangeResponse {
        HttpExchangeResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.to_string(),
        }
    }

    /// Portfolio scan of a NEAR address via the `fixture` source.
    ///
    /// Uses the embedded fixture data (no HTTP calls), so this test
    /// validates that the WASM binary loads, executes within fuel
    /// limits, and returns valid output — without needing network
    /// access or canned HTTP responses.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_portfolio_scan_fixture_source() {
        let trace = LlmTrace {
            model_name: "test-portfolio-scan-fixture".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Scan wallet 0x1111111111111111111111111111111111111111"
                    .to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_portfolio_1".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "scan",
                                    "addresses": ["0x1111111111111111111111111111111111111111"],
                                    "source": "fixture"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Found positions in the fixture wallet.".to_string(),
                            input_tokens: 500,
                            output_tokens: 20,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                tools_used: vec!["portfolio".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("portfolio", PORTFOLIO_WASM, Some(PORTFOLIO_CAPS.into()))
            .build()
            .await;

        rig.send_message("Scan wallet 0x1111111111111111111111111111111111111111")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(30)).await;
        rig.verify_trace_expects(&trace, &responses);

        // Verify tool succeeded (not fuel-exhausted or errored)
        let completed = rig.tool_calls_completed();
        assert!(
            completed
                .iter()
                .any(|(name, success)| name == "portfolio" && *success),
            "portfolio tool should succeed with fixture source, got: {completed:?}"
        );

        rig.shutdown();
    }

    /// Portfolio scan of root.near via the NEAR indexer with canned
    /// FastNEAR + Intear HTTP responses.
    ///
    /// This is the critical test for the fuel exhaustion bug: the WASM
    /// tool must parse the Intear price response (~235 KB) within the
    /// default 100M fuel budget. The previous `/tokens` endpoint
    /// (~3.2 MB) caused fuel exhaustion; this test ensures the lighter
    /// `/list-token-price` endpoint stays within budget.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary + recorded fixtures
    async fn wasm_portfolio_scan_near_within_fuel_budget() {
        let fastnear_body = load_fixture("root.near.json");
        let intear_body = load_fixture("intear_prices.json");

        let trace = LlmTrace {
            model_name: "test-portfolio-scan-near".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Scan root.near portfolio".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_portfolio_near_1".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "scan",
                                    "addresses": ["root.near"],
                                    "source": "near"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Found NEAR positions for root.near.".to_string(),
                            input_tokens: 2000,
                            output_tokens: 50,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![
                // Intear: token prices (~235 KB) — fetched first
                HttpExchange {
                    request: HttpExchangeRequest {
                        method: "GET".to_string(),
                        url: "https://prices.intear.tech/list-token-price".to_string(),
                        headers: vec![],
                        body: None,
                    },
                    response: json_ok(&intear_body),
                },
                // FastNEAR: account balances — fetched second
                HttpExchange {
                    request: HttpExchangeRequest {
                        method: "GET".to_string(),
                        url: "https://api.fastnear.com/v1/account/root.near/full".to_string(),
                        headers: vec![],
                        body: None,
                    },
                    response: json_ok(&fastnear_body),
                },
            ],
            expects: TraceExpects {
                tools_used: vec!["portfolio".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("portfolio", PORTFOLIO_WASM, Some(PORTFOLIO_CAPS.into()))
            .build()
            .await;

        rig.send_message("Scan root.near portfolio").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(60)).await;
        rig.verify_trace_expects(&trace, &responses);

        // Verify tool succeeded — this is the fuel exhaustion regression check
        let completed = rig.tool_calls_completed();
        let portfolio_result = completed
            .iter()
            .find(|(name, _)| name == "portfolio");
        assert!(
            portfolio_result.is_some(),
            "portfolio tool was never completed — may have timed out or crashed"
        );
        let (_, success) = portfolio_result.unwrap();
        assert!(
            *success,
            "portfolio tool failed (likely fuel exhaustion). \
             Completed tools: {completed:?}"
        );

        // Verify the output contains NEAR positions
        let results = rig.tool_results();
        let portfolio_output = results
            .iter()
            .find(|(name, _)| name == "portfolio")
            .map(|(_, preview)| preview.clone())
            .unwrap_or_default();
        assert!(
            portfolio_output.contains("NEAR") || portfolio_output.contains("near"),
            "portfolio output should mention NEAR, got: {portfolio_output}"
        );

        rig.shutdown();
    }

    /// Portfolio scan with `source=auto` — verifies auto-detection routes
    /// a NEAR address to the NEAR backend (not the Dune EVM backend).
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary + recorded fixtures
    async fn wasm_portfolio_auto_detect_near_address() {
        let fastnear_body = load_fixture("root.near.json");
        let intear_body = load_fixture("intear_prices.json");

        let trace = LlmTrace {
            model_name: "test-portfolio-auto-detect".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Scan root.near".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_portfolio_auto_1".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "scan",
                                    "addresses": ["root.near"],
                                    "source": "auto"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Scanned root.near via auto-detect.".to_string(),
                            input_tokens: 2000,
                            output_tokens: 50,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![
                // Intear first, then FastNEAR (matches WASM fetch order)
                HttpExchange {
                    request: HttpExchangeRequest {
                        method: "GET".to_string(),
                        url: "https://prices.intear.tech/list-token-price".to_string(),
                        headers: vec![],
                        body: None,
                    },
                    response: json_ok(&intear_body),
                },
                HttpExchange {
                    request: HttpExchangeRequest {
                        method: "GET".to_string(),
                        url: "https://api.fastnear.com/v1/account/root.near/full".to_string(),
                        headers: vec![],
                        body: None,
                    },
                    response: json_ok(&fastnear_body),
                },
            ],
            expects: TraceExpects {
                tools_used: vec!["portfolio".to_string()],
                all_tools_succeeded: Some(true),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("portfolio", PORTFOLIO_WASM, Some(PORTFOLIO_CAPS.into()))
            .build()
            .await;

        rig.send_message("Scan root.near").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(60)).await;
        rig.verify_trace_expects(&trace, &responses);

        rig.shutdown();
    }

    /// Full scan → propose pipeline using fixture data.
    ///
    /// The fixture has a test_lending position (USDC, 3.2% APY, $5000)
    /// below the 4% stablecoin yield floor. The propose step should
    /// generate at least one proposal for it.
    ///
    /// This tests the complete WASM pipeline: scan parses fixtures,
    /// analyzer classifies positions, strategy engine matches against
    /// the yield-floor strategy, and proposals are generated — all
    /// within the fuel budget.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_portfolio_full_pipeline_scan_then_propose() {
        // Step 1: scan the fixture address. The scan response is
        // deterministic — 0x1111... has a test_lending USDC position.
        // Step 2: propose with the classified positions from step 1.
        //
        // The trace encodes both steps. The propose call uses the known
        // classified output from the fixture: test_lending position
        // classified as stablecoin-idle (USDC in lending = stablecoin-idle).
        let strategy_doc = r#"---
id: stablecoin-yield-floor
version: 1
applies_to:
  category: stablecoin-idle
  min_principal_usd: 100
constraints:
  min_projected_delta_apy_bps: 50
  max_risk_score: 3
  max_bridge_legs: 1
  gas_payback_days: 30
  prefer_same_chain: true
  prefer_near_intents: true
inputs:
  floor_apy: 0.04
---
# Stablecoin Yield Floor
Keep idle stablecoins at or above floor_apy net APY."#;

        // The classified position from scanning 0x1111... with fixture source.
        // This is the deterministic output of scan + classify for the fixture.
        let classified_position = json!({
            "protocol": {"id": "test_lending", "name": "Test Lending"},
            "category": "stablecoin-idle",
            "chain": "base",
            "address": "0x1111111111111111111111111111111111111111",
            "principal_usd": "5000.00",
            "debt_usd": "0.00",
            "net_yield_apy": 0.032,
            "unrealized_pnl_usd": "0.00",
            "risk_score": 0,
            "exit_cost_estimate_usd": "0.00",
            "withdrawal_delay_seconds": 0,
            "liquidity_tier": "instant",
            "health": null,
            "tags": [],
            "raw_position": {
                "chain": "base",
                "protocol_id": "test_lending",
                "position_type": "supply",
                "address": "0x1111111111111111111111111111111111111111",
                "token_balances": [{
                    "symbol": "USDC",
                    "address": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
                    "chain": "base",
                    "amount": "5000.000000",
                    "value_usd": "5000.00"
                }],
                "debt_balances": [],
                "reward_balances": [],
                "raw_metadata": {
                    "supply_apy": 0.032,
                    "borrow_apy": 0.0,
                    "pool_contract": "0x0000000000000000000000000000000000000abc"
                },
                "block_number": 12000000,
                "fetched_at": 1712822400
            }
        });

        let trace = LlmTrace {
            model_name: "test-portfolio-full-pipeline".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Scan and propose yield for 0x1111111111111111111111111111111111111111"
                    .to_string(),
                steps: vec![
                    // Step 1: LLM calls scan
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_scan".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "scan",
                                    "addresses": ["0x1111111111111111111111111111111111111111"],
                                    "source": "fixture"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    // Step 2: LLM calls propose with the scan results
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_propose".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "propose",
                                    "positions": [classified_position],
                                    "strategies": [strategy_doc],
                                    "config": {
                                        "floor_apy": 0.04,
                                        "max_risk_score": 3,
                                        "notify_threshold_usd": 100,
                                        "auto_intent_ceiling_usd": 1000,
                                        "max_slippage_bps": 50
                                    }
                                }),
                            }],
                            input_tokens: 500,
                            output_tokens: 50,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    // Step 3: LLM summarizes
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Found a position below the yield floor.".to_string(),
                            input_tokens: 1000,
                            output_tokens: 50,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                tools_used: vec!["portfolio".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(2),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("portfolio", PORTFOLIO_WASM, Some(PORTFOLIO_CAPS.into()))
            .build()
            .await;

        rig.send_message(
            "Scan and propose yield for 0x1111111111111111111111111111111111111111",
        )
        .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(30)).await;
        rig.verify_trace_expects(&trace, &responses);

        // Check both tool calls succeeded
        let completed = rig.tool_calls_completed();
        let portfolio_calls: Vec<_> = completed
            .iter()
            .filter(|(name, _)| name == "portfolio")
            .collect();
        assert_eq!(
            portfolio_calls.len(),
            2,
            "expected 2 portfolio calls (scan + propose), got: {completed:?}"
        );
        assert!(
            portfolio_calls.iter().all(|(_, success)| *success),
            "both portfolio calls should succeed: {completed:?}"
        );

        // Check the propose output contains a proposal (test_lending at 3.2%
        // is below the 4% floor, so it should trigger a yield-floor proposal)
        let results = rig.tool_results();
        let propose_output = results
            .iter()
            .filter(|(name, _)| name == "portfolio")
            .nth(1)
            .map(|(_, preview)| preview.clone())
            .unwrap_or_default();
        assert!(
            propose_output.contains("proposals")
                || propose_output.contains("ready")
                || propose_output.contains("below-threshold"),
            "propose output should contain proposals, got: {propose_output}"
        );

        rig.shutdown();
    }

    /// Full pipeline with NEAR address: scan root.near → propose with
    /// canned HTTP responses. Verifies the entire WASM pipeline works
    /// within fuel limits for a real NEAR wallet.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary + recorded fixtures
    async fn wasm_portfolio_near_scan_then_propose() {
        let fastnear_body = load_fixture("root.near.json");
        let intear_body = load_fixture("intear_prices.json");

        let strategy_doc = r#"---
id: stablecoin-yield-floor
version: 1
applies_to:
  category: stablecoin-idle
  min_principal_usd: 100
constraints:
  min_projected_delta_apy_bps: 50
  max_risk_score: 3
inputs:
  floor_apy: 0.04
---
# Stablecoin Yield Floor
Keep idle stablecoins at or above floor_apy net APY."#;

        // For NEAR wallet positions (category=wallet, not stablecoin-idle),
        // no yield proposals are expected — wallet holdings don't match
        // the yield-floor strategy. The test verifies the pipeline runs
        // end-to-end without errors (especially fuel exhaustion).

        // We need the classified positions from scan to pass to propose.
        // Since we can't dynamically extract them from step 1's output in
        // a trace, we pass an empty array — propose should return 0 proposals
        // without erroring.
        let trace = LlmTrace {
            model_name: "test-portfolio-near-pipeline".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Scan root.near and check yield".to_string(),
                steps: vec![
                    // Step 1: scan root.near
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_near_scan".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "scan",
                                    "addresses": ["root.near"],
                                    "source": "near"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    // Step 2: propose with empty positions (wallet
                    // positions don't match yield strategies — this
                    // verifies propose handles the "no matches" case)
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_near_propose".to_string(),
                                name: "portfolio".to_string(),
                                arguments: json!({
                                    "action": "propose",
                                    "positions": [],
                                    "strategies": [strategy_doc],
                                    "config": {
                                        "floor_apy": 0.04,
                                        "max_risk_score": 3,
                                        "notify_threshold_usd": 100,
                                        "auto_intent_ceiling_usd": 1000,
                                        "max_slippage_bps": 50
                                    }
                                }),
                            }],
                            input_tokens: 2000,
                            output_tokens: 50,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    // Step 3: summary
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Scanned root.near. Wallet holdings found but no DeFi yield positions to optimize.".to_string(),
                            input_tokens: 2500,
                            output_tokens: 80,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![
                // Intear first, then FastNEAR
                HttpExchange {
                    request: HttpExchangeRequest {
                        method: "GET".to_string(),
                        url: "https://prices.intear.tech/list-token-price".to_string(),
                        headers: vec![],
                        body: None,
                    },
                    response: json_ok(&intear_body),
                },
                HttpExchange {
                    request: HttpExchangeRequest {
                        method: "GET".to_string(),
                        url: "https://api.fastnear.com/v1/account/root.near/full".to_string(),
                        headers: vec![],
                        body: None,
                    },
                    response: json_ok(&fastnear_body),
                },
            ],
            expects: TraceExpects {
                tools_used: vec!["portfolio".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(2),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("portfolio", PORTFOLIO_WASM, Some(PORTFOLIO_CAPS.into()))
            .build()
            .await;

        rig.send_message("Scan root.near and check yield").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(60)).await;
        rig.verify_trace_expects(&trace, &responses);

        // Both calls should succeed
        let completed = rig.tool_calls_completed();
        let portfolio_calls: Vec<_> = completed
            .iter()
            .filter(|(name, _)| name == "portfolio")
            .collect();
        assert_eq!(
            portfolio_calls.len(),
            2,
            "expected 2 portfolio calls (scan + propose): {completed:?}"
        );
        assert!(
            portfolio_calls.iter().all(|(_, success)| *success),
            "both calls should succeed: {completed:?}"
        );

        rig.shutdown();
    }
}
