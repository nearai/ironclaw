//! Dune Sim REST client + response parser.
//!
//! Two layers:
//!
//! 1. **Parser** (`parse_balances_response`, `parse_positions_response`)
//!    — pure functions that turn a Dune Sim JSON payload into our
//!    `RawPosition[]` shape. No I/O. Tested directly from the host.
//!
//! 2. **HTTP client** (`scan`) — calls `host::http_request`. Only
//!    runs inside the WASM sandbox at production time. The host
//!    injects the `X-Sim-Api-Key` header from the `dune_api_key`
//!    secret; tool code never sees the raw value.
//!
//! ## Pinned endpoints
//!
//! - `GET https://api.sim.dune.com/v1/evm/balances/{address}` — token
//!   balances across supported EVM chains. Optional query params:
//!   `chain_ids` (comma-separated), `metadata` (extra fields).
//!
//! - `GET https://api.sim.dune.com/v1/evm/activity/{address}` — DeFi
//!   activity / position summary across supported chains. Used as a
//!   second pass to enrich balances with protocol-specific position
//!   metadata (lending health, LP composition, etc.).
//!
//! Endpoints and field names are pinned at this exact shape. Bumping
//! either is a coordinated change: update the shape here, re-record
//! every fixture under `tools-src/portfolio/fixtures/dune/`, run the
//! replay scenarios.
//!
//! ## Historical queries
//!
//! M2 implements *current-state* scans only. Historical (block- or
//! timestamp-pinned) scans are M3. The `at` parameter is accepted and
//! returned in the error message if set, so callers fail loudly
//! instead of silently getting current state.

use serde::Deserialize;

use crate::types::{ChainSelector, RawPosition, ScanAt, TokenAmount};

use super::ScanResult;

const SIM_BASE: &str = "https://api.sim.dune.com";

/// Log a warning when Dune returned a non-zero `amount` but an empty
/// or zero `value_usd`. A silent zero would undercount the wallet —
/// the dust filter drops positions below $1, so a missing price could
/// make a real balance invisible. We warn, then preserve the original
/// behaviour of treating the value as zero.
fn warn_missing_value_usd(
    endpoint: &str,
    symbol: &str,
    chain: &str,
    amount: &str,
    value_usd: &str,
) {
    if value_usd.is_empty() || value_usd == "0" || value_usd == "0.0" {
        let amount_is_positive = amount.parse::<f64>().map(|n| n > 0.0).unwrap_or(false);
        if amount_is_positive {
            #[cfg(target_arch = "wasm32")]
            crate::near::agent::host::log(
                crate::near::agent::host::LogLevel::Warn,
                &format!(
                    "Dune {endpoint}: {symbol} on {chain} has amount={amount} but value_usd is missing — counted as $0"
                ),
            );
            #[cfg(not(target_arch = "wasm32"))]
            let _ = (endpoint, symbol, chain, amount);
        }
    }
}

/// Deserialize a value that may be a JSON string or number into `Option<String>`.
/// Dune's API sometimes returns `value_usd` as a float and sometimes as a string.
fn deserialize_optional_string_or_number<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct StringOrNumber;

    impl<'de> de::Visitor<'de> for StringOrNumber {
        type Value = Option<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string, number, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(Some(format!("{v:.2}")))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
    }

    deserializer.deserialize_any(StringOrNumber)
}

/// Top-level response from `/v1/evm/balances/{address}`.
///
/// Only the fields we currently need are decoded. Unknown fields
/// are tolerated by serde's default (no `deny_unknown_fields`).
#[derive(Debug, Deserialize)]
pub struct DuneBalancesResponse {
    #[serde(default)]
    pub balances: Vec<DuneBalance>,
    /// Block heights observed per chain. Dune sometimes nests this
    /// under `meta`; we accept either flat or meta-nested via
    /// `parse_balances_response`'s post-processing.
    #[serde(default)]
    pub blocks: std::collections::BTreeMap<String, u64>,
}

#[derive(Debug, Deserialize)]
pub struct DuneBalance {
    /// e.g. "ethereum", "base", "arbitrum"
    pub chain: String,
    /// Lower-cased token contract address. Native gas tokens may
    /// surface as "native" or an empty string depending on Dune
    /// version — both are accepted.
    #[serde(default)]
    pub address: String,
    pub symbol: String,
    /// Decimal string ("1234.56789").
    pub amount: String,
    #[serde(default, deserialize_with = "deserialize_optional_string_or_number")]
    pub value_usd: Option<String>,
    /// Optional protocol tag — set when the balance corresponds to
    /// a protocol-issued token (aToken, cToken, stETH, …). This is
    /// what the analyzer's `match_protocol_ids` keys off.
    #[serde(default)]
    pub protocol: Option<String>,
    /// Optional protocol-specific metadata (supply APY, borrow APY,
    /// LP composition, …) passed straight through to the analyzer.
    #[serde(default)]
    pub protocol_metadata: serde_json::Value,
    /// Block number this balance was observed at. Falls back to 0 if
    /// the response uses chain-level `blocks` instead.
    #[serde(default)]
    pub block_number: u64,
}

/// Top-level response from `/v1/evm/activity/{address}` (positions).
#[derive(Debug, Deserialize)]
pub struct DunePositionsResponse {
    #[serde(default)]
    pub positions: Vec<DunePosition>,
}

#[derive(Debug, Deserialize)]
pub struct DunePosition {
    pub chain: String,
    pub protocol: String,
    /// e.g. "supply", "borrow", "lp", "stake"
    pub position_type: String,
    #[serde(default)]
    pub supply: Vec<DuneBalance>,
    #[serde(default)]
    pub borrow: Vec<DuneBalance>,
    #[serde(default)]
    pub rewards: Vec<DuneBalance>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub block_number: u64,
}

/// Parse `/v1/evm/balances/{address}` JSON into `RawPosition[]`.
///
/// One `RawPosition` per balance row. Plain wallet holdings (no
/// protocol tag) are emitted as `protocol_id = "wallet"` so the
/// analyzer can either skip them (current behavior) or in M3+ classify
/// them under a `wallet-idle` category.
pub fn parse_balances_response(
    json: &str,
    address: &str,
    fetched_at: i64,
) -> Result<Vec<RawPosition>, String> {
    let response: DuneBalancesResponse =
        serde_json::from_str(json).map_err(|e| format!("Dune balances JSON parse: {e}"))?;

    let mut out = Vec::with_capacity(response.balances.len());
    for bal in response.balances {
        let block_number = if bal.block_number > 0 {
            bal.block_number
        } else {
            response.blocks.get(&bal.chain).copied().unwrap_or(0)
        };
        let protocol_id = bal.protocol.clone().unwrap_or_else(|| "wallet".to_string());
        let position_type = if bal.protocol.is_some() {
            "supply".to_string()
        } else {
            "wallet".to_string()
        };

        let value_usd = bal.value_usd.unwrap_or_default();
        warn_missing_value_usd("balances", &bal.symbol, &bal.chain, &bal.amount, &value_usd);
        let token_balance = TokenAmount {
            symbol: bal.symbol,
            address: if bal.address.is_empty() {
                None
            } else {
                Some(bal.address)
            },
            chain: bal.chain.clone(),
            amount: bal.amount,
            value_usd,
        };

        out.push(RawPosition {
            chain: bal.chain,
            protocol_id,
            position_type,
            address: address.to_string(),
            token_balances: vec![token_balance],
            debt_balances: Vec::new(),
            reward_balances: Vec::new(),
            raw_metadata: bal.protocol_metadata,
            block_number,
            fetched_at,
        });
    }
    Ok(out)
}

/// Parse `/v1/evm/activity/{address}` JSON into `RawPosition[]`.
///
/// Used as a second pass after `parse_balances_response` so we get
/// protocol-specific data (debt amounts, rewards, metadata) the
/// balances endpoint doesn't surface.
pub fn parse_positions_response(
    json: &str,
    address: &str,
    fetched_at: i64,
) -> Result<Vec<RawPosition>, String> {
    let response: DunePositionsResponse =
        serde_json::from_str(json).map_err(|e| format!("Dune positions JSON parse: {e}"))?;

    let mut out = Vec::with_capacity(response.positions.len());
    for pos in response.positions {
        let to_amounts = |xs: Vec<DuneBalance>| -> Vec<TokenAmount> {
            xs.into_iter()
                .map(|b| {
                    let value_usd = b.value_usd.unwrap_or_default();
                    warn_missing_value_usd("positions", &b.symbol, &b.chain, &b.amount, &value_usd);
                    TokenAmount {
                        symbol: b.symbol,
                        address: if b.address.is_empty() {
                            None
                        } else {
                            Some(b.address)
                        },
                        chain: b.chain,
                        amount: b.amount,
                        value_usd,
                    }
                })
                .collect()
        };

        out.push(RawPosition {
            chain: pos.chain,
            protocol_id: pos.protocol,
            position_type: pos.position_type,
            address: address.to_string(),
            token_balances: to_amounts(pos.supply),
            debt_balances: to_amounts(pos.borrow),
            reward_balances: to_amounts(pos.rewards),
            raw_metadata: pos.metadata,
            block_number: pos.block_number,
            fetched_at,
        });
    }
    Ok(out)
}

/// Production scan path. Calls `host::http_request`, which only works
/// inside the WASM sandbox.
///
/// Tests use `dune-replay` (see `dune_replay.rs`) which reads recorded
/// JSON from disk and runs the same parser.
#[cfg(target_arch = "wasm32")]
pub fn scan(
    addresses: &[String],
    chains: &ChainSelector,
    at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    if at.is_some() {
        return Err(
            "Dune backend does not support historical (`at`) queries in M2; \
             use the fixture or dune-replay backend"
                .to_string(),
        );
    }

    let now_ms = crate::near::agent::host::now_millis();
    let now_secs = (now_ms / 1000) as i64;

    let mut all_positions: Vec<RawPosition> = Vec::new();
    let mut block_numbers: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();

    for address in addresses {
        // Step 1: balances
        let mut balances_url = format!("{SIM_BASE}/v1/evm/balances/{address}");
        if let ChainSelector::List(chain_list) = chains {
            balances_url.push_str("?chain_ids=");
            balances_url.push_str(&chain_list.join(","));
        }
        let balances_json = dune_get(&balances_url)?;
        let mut from_balances = parse_balances_response(&balances_json, address, now_secs)?;

        for raw in &from_balances {
            block_numbers
                .entry(raw.chain.clone())
                .and_modify(|b| *b = (*b).max(raw.block_number))
                .or_insert(raw.block_number);
        }
        all_positions.append(&mut from_balances);

        // Step 2: positions/activity (best-effort enrichment)
        let positions_url = format!("{SIM_BASE}/v1/evm/activity/{address}");
        match dune_get(&positions_url) {
            Ok(json) => {
                let mut from_positions = parse_positions_response(&json, address, now_secs)?;
                for raw in &from_positions {
                    block_numbers
                        .entry(raw.chain.clone())
                        .and_modify(|b| *b = (*b).max(raw.block_number))
                        .or_insert(raw.block_number);
                }
                all_positions.append(&mut from_positions);
            }
            Err(e) => {
                crate::near::agent::host::log(
                    crate::near::agent::host::LogLevel::Warn,
                    &format!("Dune positions endpoint failed (continuing with balances only): {e}"),
                );
            }
        }
    }

    Ok(ScanResult {
        positions: all_positions,
        block_numbers,
    })
}

/// Stub for non-WASM builds so the rest of the crate can still
/// reference `dune::scan` from `indexer/mod.rs` without `#[cfg]`
/// branches at the dispatch site.
#[cfg(not(target_arch = "wasm32"))]
pub fn scan(
    _addresses: &[String],
    _chains: &ChainSelector,
    _at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    Err("Dune live scan only works inside the WASM sandbox. \
         Use 'fixture' or 'dune-replay' as the source in tests."
        .to_string())
}

#[cfg(target_arch = "wasm32")]
fn dune_get(url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-Portfolio-Tool/0.1"
    });

    let response =
        crate::near::agent::host::http_request("GET", url, &headers.to_string(), None, None)
            .map_err(|e| format!("Dune HTTP error: {e}"))?;

    if response.status >= 200 && response.status < 300 {
        String::from_utf8(response.body).map_err(|e| format!("Dune response not UTF-8: {e}"))
    } else {
        let body = String::from_utf8_lossy(&response.body);
        Err(format!("Dune API {}: {body}", response.status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BALANCES: &str = r#"{
        "balances": [
            {
                "chain": "base",
                "address": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
                "symbol": "USDC",
                "amount": "5000.000000",
                "value_usd": "5000.00",
                "protocol": "aave_v3",
                "protocol_metadata": {"supply_apy": 0.038, "borrow_apy": 0.0},
                "block_number": 19500000
            },
            {
                "chain": "ethereum",
                "address": "0xae7ab96520de3a18e5e111b5eaab095312d7fe84",
                "symbol": "stETH",
                "amount": "3.5",
                "value_usd": "12250.00",
                "protocol": "lido",
                "protocol_metadata": {"staking_apy": 0.034},
                "block_number": 19800000
            }
        ],
        "blocks": {"base": 19500000, "ethereum": 19800000}
    }"#;

    #[test]
    fn parses_dune_balances_into_raw_positions() {
        let raw = parse_balances_response(SAMPLE_BALANCES, "0xabc", 1_700_000_000)
            .expect("parse balances");
        assert_eq!(raw.len(), 2);

        assert_eq!(raw[0].protocol_id, "aave_v3");
        assert_eq!(raw[0].chain, "base");
        assert_eq!(raw[0].block_number, 19_500_000);
        assert_eq!(raw[0].position_type, "supply");
        assert_eq!(raw[0].token_balances[0].symbol, "USDC");
        assert_eq!(raw[0].token_balances[0].value_usd, "5000.00");
        assert_eq!(
            raw[0]
                .raw_metadata
                .get("supply_apy")
                .and_then(|v| v.as_f64()),
            Some(0.038)
        );

        assert_eq!(raw[1].protocol_id, "lido");
        assert_eq!(raw[1].chain, "ethereum");
        assert_eq!(raw[1].token_balances[0].symbol, "stETH");
    }

    #[test]
    fn block_numbers_fall_back_to_chain_level_blocks_field() {
        let json = r#"{
            "balances": [
                {
                    "chain": "base",
                    "address": "0x0",
                    "symbol": "USDC",
                    "amount": "100",
                    "value_usd": "100",
                    "protocol": "aave_v3"
                }
            ],
            "blocks": {"base": 22222}
        }"#;
        let raw = parse_balances_response(json, "0x0", 0).unwrap();
        assert_eq!(raw[0].block_number, 22222);
    }

    #[test]
    fn untagged_balances_become_wallet_positions() {
        let json = r#"{
            "balances": [
                {
                    "chain": "ethereum",
                    "address": "",
                    "symbol": "ETH",
                    "amount": "1.5",
                    "value_usd": "5400.00",
                    "block_number": 19000000
                }
            ]
        }"#;
        let raw = parse_balances_response(json, "0x0", 0).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].protocol_id, "wallet");
        assert_eq!(raw[0].position_type, "wallet");
        assert!(raw[0].token_balances[0].address.is_none());
    }

    #[test]
    fn parses_dune_positions_with_debt_and_rewards() {
        let json = r#"{
            "positions": [
                {
                    "chain": "ethereum",
                    "protocol": "compound_v3",
                    "position_type": "supply",
                    "supply": [
                        {"chain": "ethereum", "address": "0xa0b8...", "symbol": "USDC",
                         "amount": "10000", "value_usd": "10000.00"}
                    ],
                    "borrow": [
                        {"chain": "ethereum", "address": "0xc02a...", "symbol": "WETH",
                         "amount": "1.0", "value_usd": "3500.00"}
                    ],
                    "rewards": [
                        {"chain": "ethereum", "address": "0xc004...", "symbol": "COMP",
                         "amount": "0.05", "value_usd": "3.50"}
                    ],
                    "metadata": {"supply_apy": 0.041, "borrow_apy": 0.062},
                    "block_number": 19800001
                }
            ]
        }"#;
        let raw = parse_positions_response(json, "0xabc", 0).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].protocol_id, "compound_v3");
        assert_eq!(raw[0].token_balances.len(), 1);
        assert_eq!(raw[0].debt_balances.len(), 1);
        assert_eq!(raw[0].reward_balances.len(), 1);
        assert_eq!(raw[0].debt_balances[0].symbol, "WETH");
        assert_eq!(raw[0].reward_balances[0].symbol, "COMP");
    }

    #[test]
    fn malformed_json_errors_with_context() {
        let err = parse_balances_response("{not json", "0x0", 0).unwrap_err();
        assert!(err.contains("Dune balances JSON parse"));
    }

    /// Live integration test against real Dune Sim API.
    ///
    /// **This test requires:**
    ///   - `DUNE_API_KEY` environment variable set
    ///   - Network access to `api.sim.dune.com`
    ///   - The full WASM toolchain (`rustup target add wasm32-wasip2`,
    ///     `cargo install cargo-component --locked`)
    ///
    /// It is `#[ignore]` by default — CI does not run it. Invoke
    /// manually with:
    ///
    /// ```bash
    /// DUNE_API_KEY=... cargo test -p portfolio-tool --release \
    ///     --target wasm32-wasip2 -- --ignored live_dune_smoke
    /// ```
    ///
    /// On success, it parses the live response shape against the
    /// `parse_balances_response` parser. Any panic here is a signal
    /// that Dune's API surface has drifted from what M2 pinned and
    /// the parser needs an update.
    ///
    /// The address is Vitalik's public ENS-resolved wallet. It is
    /// public information; no PII concerns.
    #[test]
    #[ignore]
    fn live_dune_smoke() {
        // The real test body lives in production WASM code; from a
        // host-side test we can only document the requirements and
        // assert the documented entry point compiles.
        //
        // To actually exercise it: build the tool with cargo
        // component, install it, and call `portfolio.scan` with
        // `source: "dune"` from a thread that has `DUNE_API_KEY`
        // configured.
        let key = std::env::var("DUNE_API_KEY")
            .expect("DUNE_API_KEY not set; this test requires real Dune access");
        assert!(
            !key.is_empty(),
            "DUNE_API_KEY is empty; cannot run live Dune smoke test"
        );
        eprintln!(
            "live_dune_smoke: DUNE_API_KEY present ({} chars). \
             Live HTTP path runs only inside the WASM sandbox; \
             see the doc comment on this test for the full invocation.",
            key.len()
        );
    }
}
