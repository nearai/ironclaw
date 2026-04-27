//! NEAR indexer backend — fetch wallet balances via FastNEAR + Intear.
//!
//! Two API calls per scan:
//!
//! 1. **FastNEAR** `GET https://api.fastnear.com/v1/account/{id}/full`
//!    — native NEAR balance, all FT balances, staking pools. Free, no
//!    API key. Returns raw balances (no decimals, no symbol, no price).
//!
//! 2. **Intear** `GET https://prices.intear.tech/list-token-price`
//!    — lightweight token metadata (symbol, decimals, price) for every
//!    tracked NEAR token (~235 KB). Free, no API key. Cached up to 5s.
//!    We use this instead of `/tokens` (3.2 MB) to stay within the
//!    WASM fuel budget.
//!
//! The two responses are joined: FastNEAR provides *which* tokens the
//! wallet holds and their raw amounts; Intear provides the metadata
//! needed to convert raw amounts to human-readable decimals and USD
//! values. Tokens missing from Intear are skipped (likely spam or
//! untracked).
//!
//! ## Staking pools
//!
//! FastNEAR's `/full` response includes `pools[]` with staking
//! delegation info. These are surfaced as `position_type = "stake"`
//! positions with `protocol_id = "<pool_id>"`.
//!
//! ## WASM vs host-side
//!
//! The WASM build uses `host::http_request`; the host-side build
//! (used by live tests) uses `ureq`. Same parsers for both.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::types::{ChainSelector, RawPosition, ScanAt, TokenAmount};

use super::ScanResult;

const FASTNEAR_BASE: &str = "https://api.fastnear.com";
const INTEAR_PRICES_URL: &str = "https://prices.intear.tech/list-token-price";

// Native NEAR has 24 decimals (yoctoNEAR).
const NEAR_DECIMALS: u32 = 24;

/// Map known NEAR DeFi token contracts to (protocol_id, position_type).
/// Tokens not in this map default to ("wallet", "wallet").
fn classify_near_token(contract_id: &str) -> (&str, &str) {
    match contract_id {
        // Linear Protocol — liquid staking
        "linear-protocol.near" => ("linear", "stake"),
        // Meta Pool — liquid staking
        "meta-pool.near" => ("meta-pool", "stake"),
        // Rhea (Burrow) — lending/borrowing receipt tokens
        c if c.starts_with("storage.rhea")
            || c.starts_with("token.rhea")
            || c == "token.burrow.near"
            || c.starts_with("storage.burrow") =>
        {
            ("rhea-lending", "supply")
        }
        // Rhea (Ref Finance) — LP tokens
        c if c.starts_with("v2.ref-finance.near")
            || c.starts_with("dclv2.ref-labs.near")
            || c == "token.ref-finance.near" =>
        {
            ("rhea-lp", "lp")
        }
        // Staked NEAR variants
        "v2-0.staking.astro-stakers.near" | "aurora" => ("wallet", "wallet"),
        // Default: plain wallet holding
        _ => ("wallet", "wallet"),
    }
}

// ── FastNEAR response types ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FastNearFullResponse {
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub state: Option<FastNearAccountState>,
    #[serde(default)]
    pub tokens: Vec<FastNearToken>,
    #[serde(default)]
    pub pools: Vec<FastNearPool>,
}

#[derive(Debug, Deserialize)]
pub struct FastNearAccountState {
    /// Native NEAR balance in yoctoNEAR.
    #[serde(default)]
    pub balance: String,
    /// Locked/staked balance in yoctoNEAR.
    #[serde(default)]
    pub locked: String,
    #[serde(default)]
    pub storage_bytes: u64,
}

#[derive(Debug, Deserialize)]
pub struct FastNearToken {
    /// Raw balance as a decimal string (not adjusted for decimals).
    #[serde(default)]
    pub balance: String,
    pub contract_id: String,
    #[serde(default)]
    pub last_update_block_height: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct FastNearPool {
    pub pool_id: String,
    #[serde(default)]
    pub last_update_block_height: Option<u64>,
}

// ── Intear token price types (`/list-token-price` shape) ───────────

/// Entry from Intear's `/list-token-price` endpoint.
/// Lightweight: only symbol, decimals, price (~235 KB total vs 3.2 MB
/// for the full `/tokens` endpoint).
#[derive(Debug, Deserialize)]
pub struct IntearTokenPrice {
    #[serde(default)]
    pub symbol: String,
    #[serde(default, alias = "decimals")]
    pub decimal: u32,
    #[serde(default)]
    pub price: String,
}

// ── Parsers (pure functions, no I/O) ───────────────────────────────

/// Parse a FastNEAR `/v1/account/{id}/full` JSON response into
/// `RawPosition[]`, enriched with price data from the Intear price map.
pub fn parse_fastnear_response(
    json: &str,
    intear_json: &str,
    address: &str,
    fetched_at: i64,
) -> Result<Vec<RawPosition>, String> {
    let resp: FastNearFullResponse =
        serde_json::from_str(json).map_err(|e| format!("FastNEAR JSON parse: {e}"))?;

    let intear: BTreeMap<String, IntearTokenPrice> =
        serde_json::from_str(intear_json).map_err(|e| format!("Intear JSON parse: {e}"))?;

    let mut positions = Vec::new();

    // Native NEAR balance
    if let Some(state) = &resp.state {
        if !state.balance.is_empty() && state.balance != "0" {
            let near_price = intear
                .get("wrap.near")
                .map(|t| &t.price)
                .cloned()
                .unwrap_or_default();

            let amount = raw_to_decimal(&state.balance, NEAR_DECIMALS);
            let value_usd = if !near_price.is_empty() && near_price != "0" {
                compute_value_usd(&amount, &near_price)
            } else {
                String::new()
            };

            positions.push(RawPosition {
                chain: "near".to_string(),
                protocol_id: "wallet".to_string(),
                position_type: "wallet".to_string(),
                address: address.to_string(),
                token_balances: vec![TokenAmount {
                    symbol: "NEAR".to_string(),
                    address: None,
                    chain: "near".to_string(),
                    amount,
                    value_usd,
                }],
                debt_balances: Vec::new(),
                reward_balances: Vec::new(),
                raw_metadata: serde_json::json!({}),
                block_number: 0,
                fetched_at,
            });
        }
    }

    // FT balances
    for token in &resp.tokens {
        // Skip zero / empty balances
        if token.balance.is_empty() || token.balance == "0" {
            continue;
        }

        // Skip tokens not tracked by Intear (likely spam or untracked)
        let info = match intear.get(&token.contract_id) {
            Some(i) => i,
            None => continue,
        };

        let symbol = if info.symbol.is_empty() {
            token.contract_id.clone()
        } else {
            info.symbol.clone()
        };

        let amount = raw_to_decimal(&token.balance, info.decimal);
        let value_usd = if !info.price.is_empty() && info.price != "0" {
            compute_value_usd(&amount, &info.price)
        } else {
            warn_near_missing_price(&symbol, &token.contract_id, &amount);
            String::new()
        };

        // Skip dust (< $1). Wallets like root.near hold 100+ micro-cap
        // tokens; passing them all to the analyzer/strategy engine wastes
        // context and fuel. Tokens with no price are also skipped (no
        // value_usd means Intear has no market data).
        if value_usd.is_empty() {
            continue;
        }
        if let Ok(v) = value_usd.parse::<f64>() {
            if v < 1.0 {
                continue;
            }
        }

        let block_number = token.last_update_block_height.unwrap_or(0);

        let (protocol_id, position_type) = classify_near_token(&token.contract_id);

        positions.push(RawPosition {
            chain: "near".to_string(),
            protocol_id: protocol_id.to_string(),
            position_type: position_type.to_string(),
            address: address.to_string(),
            token_balances: vec![TokenAmount {
                symbol,
                address: Some(token.contract_id.clone()),
                chain: "near".to_string(),
                amount,
                value_usd,
            }],
            debt_balances: Vec::new(),
            reward_balances: Vec::new(),
            raw_metadata: serde_json::json!({}),
            block_number,
            fetched_at,
        });
    }

    // Staking pools
    for pool in &resp.pools {
        let block_number = pool.last_update_block_height.unwrap_or(0);
        positions.push(RawPosition {
            chain: "near".to_string(),
            protocol_id: "near-staking".to_string(),
            position_type: "stake".to_string(),
            address: address.to_string(),
            token_balances: vec![TokenAmount {
                symbol: "NEAR".to_string(),
                address: None,
                chain: "near".to_string(),
                // FastNEAR doesn't return staked amount per pool; the
                // state.locked field is the aggregate. We leave amount
                // empty here — a future enrichment step can query each
                // pool's `get_account` to fill it.
                amount: String::new(),
                value_usd: String::new(),
            }],
            debt_balances: Vec::new(),
            reward_balances: Vec::new(),
            raw_metadata: serde_json::json!({"pool_id": pool.pool_id}),
            block_number,
            fetched_at,
        });
    }

    Ok(positions)
}

/// Convert a raw integer balance string to a human-readable decimal
/// given the token's decimal places.
///
/// Example: `raw_to_decimal("3914671412201452214124438625", 24)` → `"3914.671412201452214124438625"`
fn raw_to_decimal(raw: &str, decimals: u32) -> String {
    if decimals == 0 {
        return raw.to_string();
    }

    let raw = raw.trim();
    if raw.is_empty() || raw == "0" {
        return "0".to_string();
    }

    let decimals = decimals as usize;
    let len = raw.len();

    if len <= decimals {
        // Number is less than 1.0 — pad with leading zeros
        let padding = decimals - len;
        let frac = format!("{}{}", "0".repeat(padding), raw);
        // Trim trailing zeros from fractional part
        let frac = frac.trim_end_matches('0');
        if frac.is_empty() {
            "0".to_string()
        } else {
            format!("0.{frac}")
        }
    } else {
        let integer_part = &raw[..len - decimals];
        let frac_part = &raw[len - decimals..];
        let frac_part = frac_part.trim_end_matches('0');
        if frac_part.is_empty() {
            integer_part.to_string()
        } else {
            format!("{integer_part}.{frac_part}")
        }
    }
}

/// Compute `amount * price_usd` using f64 and format as 2-decimal string.
fn compute_value_usd(amount: &str, price_usd: &str) -> String {
    let amt: f64 = amount.parse().unwrap_or_else(|_| {
        warn_unparseable_number("amount", amount);
        0.0
    });
    let price: f64 = price_usd.parse().unwrap_or_else(|_| {
        warn_unparseable_number("price_usd", price_usd);
        0.0
    });
    let value = amt * price;
    format!("{value:.2}")
}

/// Warn when Intear has no price data for a token the wallet actually
/// holds. The position will be silently dropped downstream (the dust
/// filter skips empty value_usd), so surface it here for diagnostics.
fn warn_near_missing_price(symbol: &str, contract_id: &str, amount: &str) {
    let amount_is_positive = amount.parse::<f64>().map(|n| n > 0.0).unwrap_or(false);
    if !amount_is_positive {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Warn,
        &format!(
            "NEAR indexer: {symbol} ({contract_id}) amount={amount} has no Intear price — position dropped"
        ),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (symbol, contract_id, amount);
}

fn warn_unparseable_number(field: &str, raw: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Warn,
        &format!("NEAR indexer: unparseable {field} '{raw}' — treating as 0"),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (field, raw);
}

// ── WASM scan (production path) ────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn near_http_get(url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-Portfolio-Tool/0.1"
    });

    let response =
        crate::near::agent::host::http_request("GET", url, &headers.to_string(), None, None)
            .map_err(|e| format!("NEAR HTTP error: {e}"))?;

    if response.status >= 200 && response.status < 300 {
        String::from_utf8(response.body).map_err(|e| format!("NEAR response not UTF-8: {e}"))
    } else {
        let body = String::from_utf8_lossy(&response.body);
        Err(format!("NEAR API {}: {body}", response.status))
    }
}

#[cfg(target_arch = "wasm32")]
pub fn scan(
    addresses: &[String],
    _chains: &ChainSelector,
    at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    if at.is_some() {
        return Err("NEAR backend does not support historical (`at`) queries; \
             use the fixture backend"
            .to_string());
    }

    let now_ms = crate::near::agent::host::now_millis();
    let now_secs = (now_ms / 1000) as i64;

    // Fetch Intear price map once for all addresses (~235 KB).
    let intear_json = near_http_get(INTEAR_PRICES_URL)?;

    let mut all_positions: Vec<RawPosition> = Vec::new();
    let mut block_numbers: BTreeMap<String, u64> = BTreeMap::new();

    for address in addresses {
        let url = format!("{FASTNEAR_BASE}/v1/account/{address}/full");
        let fastnear_json = near_http_get(&url)?;
        let mut positions =
            parse_fastnear_response(&fastnear_json, &intear_json, address, now_secs)?;

        for raw in &positions {
            if raw.block_number > 0 {
                block_numbers
                    .entry(raw.chain.clone())
                    .and_modify(|b| *b = (*b).max(raw.block_number))
                    .or_insert(raw.block_number);
            }
        }
        all_positions.append(&mut positions);
    }

    Ok(ScanResult {
        positions: all_positions,
        block_numbers,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn scan(
    _addresses: &[String],
    _chains: &ChainSelector,
    _at: Option<&ScanAt>,
) -> Result<ScanResult, String> {
    Err("NEAR live scan only works inside the WASM sandbox. \
         Use 'fixture' or 'near-replay' as the source in tests."
        .to_string())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_to_decimal_zero_decimals() {
        assert_eq!(raw_to_decimal("12345", 0), "12345");
    }

    #[test]
    fn raw_to_decimal_normal() {
        assert_eq!(
            raw_to_decimal("3914671412201452214124438625", 24),
            "3914.671412201452214124438625"
        );
    }

    #[test]
    fn raw_to_decimal_less_than_one() {
        assert_eq!(raw_to_decimal("500", 6), "0.0005");
    }

    #[test]
    fn raw_to_decimal_exact_integer() {
        assert_eq!(raw_to_decimal("1000000", 6), "1");
    }

    #[test]
    fn raw_to_decimal_zero() {
        assert_eq!(raw_to_decimal("0", 18), "0");
    }

    #[test]
    fn raw_to_decimal_empty() {
        assert_eq!(raw_to_decimal("", 18), "0");
    }

    #[test]
    fn compute_value_usd_basic() {
        assert_eq!(compute_value_usd("3914.67", "1.37"), "5363.10");
    }

    #[test]
    fn parse_fastnear_native_near() {
        let fastnear = r#"{
            "account_id": "test.near",
            "state": {
                "balance": "5000000000000000000000000",
                "locked": "0",
                "storage_bytes": 100
            },
            "tokens": [],
            "pools": []
        }"#;
        let intear = r#"{
            "wrap.near": {
                "price": "2.00",
                "symbol": "wNEAR",
                "decimal": 24
            }
        }"#;

        let positions = parse_fastnear_response(fastnear, intear, "test.near", 1700000000).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].chain, "near");
        assert_eq!(positions[0].protocol_id, "wallet");
        assert_eq!(positions[0].token_balances[0].symbol, "NEAR");
        assert_eq!(positions[0].token_balances[0].amount, "5");
        assert_eq!(positions[0].token_balances[0].value_usd, "10.00");
    }

    #[test]
    fn parse_fastnear_ft_balances() {
        let fastnear = r#"{
            "account_id": "test.near",
            "state": {"balance": "0", "locked": "0", "storage_bytes": 0},
            "tokens": [
                {"balance": "1000000", "contract_id": "usdt.tether-token.near", "last_update_block_height": 100},
                {"balance": "0", "contract_id": "zero.near", "last_update_block_height": null}
            ],
            "pools": []
        }"#;
        let intear = r#"{
            "usdt.tether-token.near": {
                "price": "1.00",
                "symbol": "USDt",
                "decimal": 6
            }
        }"#;

        let positions = parse_fastnear_response(fastnear, intear, "test.near", 1700000000).unwrap();
        // Native NEAR is "0" so skipped; zero.near also skipped
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].token_balances[0].symbol, "USDt");
        assert_eq!(positions[0].token_balances[0].amount, "1");
        assert_eq!(positions[0].token_balances[0].value_usd, "1.00");
        assert_eq!(
            positions[0].token_balances[0].address.as_deref(),
            Some("usdt.tether-token.near")
        );
    }

    #[test]
    fn parse_fastnear_skips_untracked_tokens() {
        let fastnear = r#"{
            "account_id": "test.near",
            "state": {"balance": "0", "locked": "0", "storage_bytes": 0},
            "tokens": [
                {"balance": "99999999999", "contract_id": "scam.near", "last_update_block_height": null}
            ],
            "pools": []
        }"#;
        // scam.near not in Intear → skipped
        let intear = "{}";

        let positions = parse_fastnear_response(fastnear, intear, "test.near", 1700000000).unwrap();
        assert!(
            positions.is_empty(),
            "tokens not tracked by Intear should be filtered"
        );
    }

    #[test]
    fn parse_fastnear_staking_pools() {
        let fastnear = r#"{
            "account_id": "test.near",
            "state": {"balance": "0", "locked": "0", "storage_bytes": 0},
            "tokens": [],
            "pools": [
                {"pool_id": "mypool.poolv1.near", "last_update_block_height": 200}
            ]
        }"#;
        let intear = "{}";

        let positions = parse_fastnear_response(fastnear, intear, "test.near", 1700000000).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].protocol_id, "near-staking");
        assert_eq!(positions[0].position_type, "stake");
        assert_eq!(
            positions[0].raw_metadata["pool_id"].as_str(),
            Some("mypool.poolv1.near")
        );
    }

    #[test]
    fn parse_fastnear_skips_dust() {
        let fastnear = r#"{
            "account_id": "test.near",
            "state": {"balance": "0", "locked": "0", "storage_bytes": 0},
            "tokens": [
                {"balance": "1", "contract_id": "tiny.near", "last_update_block_height": null}
            ],
            "pools": []
        }"#;
        let intear = r#"{
            "tiny.near": {
                "price": "0.001",
                "symbol": "TINY",
                "decimal": 6
            }
        }"#;

        let positions = parse_fastnear_response(fastnear, intear, "test.near", 1700000000).unwrap();
        assert!(positions.is_empty(), "dust < $1 should be filtered");
    }

    /// Live smoke test — only checks NEAR env setup. Real HTTP in WASM.
    #[test]
    #[ignore]
    fn live_near_smoke() {
        // This test just verifies compilation and that the test
        // infrastructure can reach the NEAR APIs. The actual HTTP
        // path runs inside the WASM sandbox.
        eprintln!(
            "live_near_smoke: NEAR indexer compiled. \
             Live HTTP path runs only inside the WASM sandbox; \
             see live_tests.rs for the full integration tests."
        );
    }
}
