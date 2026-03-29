//! Near Intents WASM Tool for IronClaw.
//!
//! Provides token resolution, reverse lookups, and balance queries
//! for the NEAR Intents / Defuse protocol.
//!
//! No authentication required — both APIs are public.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const DEFUSE_TOKENS_URL: &str = "https://1click.chaindefuser.com/v0/tokens";
const DEFUSE_QUOTE_URL: &str = "https://1click.chaindefuser.com/v0/quote";
const NEAR_RPC_URL: &str = "https://rpc.mainnet.near.org";
const MAX_ALIAS_WORDS: usize = 5;

const CURATED_ALIASES: &[(&str, &[(&str, Option<&str>)])] = &[
    ("ethereum", &[("ETH", None)]),
    ("ether", &[("ETH", None)]),
    ("bitcoin", &[("BTC", None), ("WBTC", None)]),
    ("btc", &[("BTC", None), ("WBTC", None)]),
    ("wrapped bitcoin", &[("WBTC", None)]),
    ("wrapped near", &[("wNEAR", None)]),
    ("near", &[("wNEAR", None)]),
    ("near protocol", &[("wNEAR", None)]),
    ("solana", &[("SOL", None)]),
    ("tether", &[("USDT", None)]),
    ("usd coin", &[("USDC", None)]),
    ("usdc", &[("USDC", None)]),
    ("usdt", &[("USDT", None)]),
    ("dai stablecoin", &[("DAI", None)]),
    ("dai", &[("DAI", None)]),
    ("dogecoin", &[("DOGE", None)]),
    ("doge", &[("DOGE", None)]),
    ("shiba", &[("SHIB", None)]),
    ("shiba inu", &[("SHIB", None)]),
    ("aurora", &[("AURORA", None)]),
    ("chainlink", &[("LINK", None)]),
    ("uniswap", &[("UNI", None)]),
    ("aave", &[("AAVE", None)]),
];

const BLOCKCHAIN_ALIASES: &[(&str, &str)] = &[
    ("near", "near"),
    ("ethereum", "eth"),
    ("eth", "eth"),
    ("solana", "sol"),
    ("sol", "sol"),
    ("arbitrum", "arbitrum"),
    ("arb", "arbitrum"),
    ("base", "base"),
    ("polygon", "polygon"),
    ("matic", "polygon"),
    ("avalanche", "avalanche"),
    ("avax", "avalanche"),
    ("bnb", "bnb"),
    ("binance", "bnb"),
    ("bsc", "bnb"),
    ("ton", "ton"),
    ("optimism", "optimism"),
    ("op", "optimism"),
];

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum Action {
    #[serde(rename = "resolve_token")]
    ResolveToken {
        query: Option<String>,
        list_all: Option<bool>,
    },
    #[serde(rename = "reverse_resolve_token")]
    ReverseResolveToken { asset_id: String },
    #[serde(rename = "get_balance")]
    GetBalance {
        account_id: String,
        token_ids: Option<Vec<String>>,
    },
    #[serde(rename = "get_swap_quote")]
    GetSwapQuote {
        from_token: String,
        to_token: String,
        amount: String,
        account_id: String,
        slippage_bps: Option<u32>,
        swap_type: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct DefuseTokenRaw {
    #[serde(rename = "assetId", default)]
    asset_id: String,
    #[serde(default)]
    decimals: u32,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    blockchain: String,
    #[serde(rename = "contractAddress")]
    contract_address: Option<String>,
    price: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct TokenMatch {
    asset_id: String,
    symbol: String,
    blockchain: String,
    decimals: u32,
}

#[derive(Debug, Clone)]
struct TokenMetadata {
    defuse_asset_id: String,
    decimals: u32,
    symbol: String,
    blockchain: String,
    contract_address: Option<String>,
    price: Option<f64>,
}

#[derive(Debug, Serialize)]
struct TokenBalance {
    defuse_asset_id: String,
    symbol: Option<String>,
    raw_balance: String,
    balance: f64,
    decimals: u32,
    value_usdc: Option<f64>,
}

#[derive(Debug, Serialize)]
struct BalanceResponse {
    positions: Vec<TokenBalance>,
    total_value_usdc: f64,
}

struct NearIntentsTool;

impl exports::near::agent::tool::Guest for NearIntentsTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Near Intents tools for the Defuse protocol on NEAR. Resolves natural-language \
         token references to Defuse asset IDs, reverse-resolves asset IDs to metadata, \
         queries multi-token balances with USD values, and gets dry swap quotes. \
         No authentication required."
            .to_string()
    }
}

export!(NearIntentsTool);

fn execute_inner(params: &str) -> Result<String, String> {
    let action: Action =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    match action {
        Action::ResolveToken { query, list_all } => resolve_token(query, list_all.unwrap_or(false)),
        Action::ReverseResolveToken { asset_id } => reverse_resolve_token(&asset_id),
        Action::GetBalance {
            account_id,
            token_ids,
        } => get_balance(&normalize_account_id(&account_id), token_ids),
        Action::GetSwapQuote {
            from_token,
            to_token,
            amount,
            account_id,
            slippage_bps,
            swap_type,
        } => get_swap_quote(
            &from_token,
            &to_token,
            &amount,
            &normalize_account_id(&account_id),
            slippage_bps.unwrap_or(100),
            swap_type.as_deref().unwrap_or("EXACT_INPUT"),
        ),
    }
}

/// Recover hex address if the LLM converted it to decimal.
fn normalize_account_id(account_id: &str) -> String {
    if account_id.chars().all(|c| c.is_ascii_digit()) && account_id.len() > 40 {
        return format!("0x{}", decimal_to_hex(account_id));
    }
    account_id.to_string()
}

/// Convert a decimal string to lowercase hex (no prefix).
fn decimal_to_hex(decimal: &str) -> String {
    let mut digits: Vec<u8> = decimal
        .bytes()
        .map(|b| b - b'0')
        .collect();

    if digits.is_empty() || (digits.len() == 1 && digits[0] == 0) {
        return "0".into();
    }

    let mut hex_chars: Vec<u8> = Vec::new();
    while !(digits.is_empty() || digits.len() == 1 && digits[0] == 0) {
        let mut remainder = 0u32;
        let mut new_digits: Vec<u8> = Vec::new();
        for &d in &digits {
            let val = remainder * 10 + d as u32;
            let quotient = val / 16;
            remainder = val % 16;
            if !new_digits.is_empty() || quotient > 0 {
                new_digits.push(quotient as u8);
            }
        }
        hex_chars.push(if remainder < 10 {
            b'0' + remainder as u8
        } else {
            b'a' + (remainder as u8 - 10)
        });
        digits = new_digits;
    }

    hex_chars.reverse();
    String::from_utf8(hex_chars).unwrap_or_default()
}

fn fetch_token_list() -> Result<Vec<TokenMetadata>, String> {
    let headers = serde_json::json!({"Accept": "application/json"});
    let resp = near::agent::host::http_request("GET", DEFUSE_TOKENS_URL, &headers.to_string(), None, None)
        .map_err(|e| format!("Failed to fetch token list: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        return Err(format!(
            "Token list API returned status {}",
            resp.status
        ));
    }

    let body = String::from_utf8(resp.body)
        .map_err(|e| format!("Invalid UTF-8 in token list response: {e}"))?;

    let raw_tokens: Vec<DefuseTokenRaw> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse token list: {e}"))?;

    let tokens = raw_tokens
        .into_iter()
        .filter(|t| !t.asset_id.is_empty())
        .map(|t| TokenMetadata {
            defuse_asset_id: t.asset_id,
            decimals: t.decimals,
            symbol: t.symbol,
            blockchain: t.blockchain,
            contract_address: t.contract_address,
            price: t.price,
        })
        .collect();

    Ok(tokens)
}

struct AliasMap {
    alias_map: HashMap<String, Vec<TokenMatch>>,
    reverse_map: HashMap<String, TokenMetadata>,
    tokens: Vec<TokenMetadata>,
}

fn build_alias_map(tokens: &[TokenMetadata]) -> AliasMap {
    let mut alias_map: HashMap<String, Vec<TokenMatch>> = HashMap::new();
    let mut reverse_map: HashMap<String, TokenMetadata> = HashMap::new();

    let mut canonical_to_aliases: HashMap<&str, Vec<&str>> = HashMap::new();
    for &(alias, canonical) in BLOCKCHAIN_ALIASES {
        canonical_to_aliases.entry(canonical).or_default().push(alias);
    }

    let mut blockchain_to_names: HashMap<&str, Vec<&str>> = HashMap::new();
    for &(alias, canonical) in BLOCKCHAIN_ALIASES {
        if let Some(names) = canonical_to_aliases.get(canonical) {
            blockchain_to_names.insert(alias, names.clone());
        }
    }

    for token in tokens {
        let m = TokenMatch {
            asset_id: token.defuse_asset_id.clone(),
            symbol: token.symbol.clone(),
            blockchain: token.blockchain.clone(),
            decimals: token.decimals,
        };

        add_alias(&mut alias_map, &token.symbol, &m);

        if let Some(ref addr) = token.contract_address {
            add_alias(&mut alias_map, addr, &m);
        }

        let sym_on_chain = format!("{} on {}", token.symbol, token.blockchain);
        add_alias(&mut alias_map, &sym_on_chain, &m);

        if let Some(names) = blockchain_to_names.get(token.blockchain.as_str()) {
            for name in names {
                let key = format!("{} on {}", token.symbol, name);
                add_alias(&mut alias_map, &key, &m);
            }
        }

        reverse_map.insert(token.defuse_asset_id.clone(), token.clone());
    }

    for &(alias, targets) in CURATED_ALIASES {
        for &(symbol_filter, blockchain_filter) in targets {
            for token in tokens {
                if token.symbol != symbol_filter {
                    continue;
                }
                if let Some(bf) = blockchain_filter {
                    if token.blockchain != bf {
                        continue;
                    }
                }
                let m = TokenMatch {
                    asset_id: token.defuse_asset_id.clone(),
                    symbol: token.symbol.clone(),
                    blockchain: token.blockchain.clone(),
                    decimals: token.decimals,
                };
                add_alias(&mut alias_map, alias, &m);
            }
        }
    }

    AliasMap {
        alias_map,
        reverse_map,
        tokens: tokens.to_vec(),
    }
}

fn add_alias(map: &mut HashMap<String, Vec<TokenMatch>>, alias: &str, m: &TokenMatch) {
    let key = alias.trim().to_lowercase();
    if key.is_empty() {
        return;
    }
    let entries = map.entry(key).or_default();
    if !entries.iter().any(|e| e.asset_id == m.asset_id) {
        entries.push(m.clone());
    }
}

fn scan_for_token_hits(input_text: &str, alias_map: &HashMap<String, Vec<TokenMatch>>) -> Vec<TokenMatch> {
    let raw_tokens: Vec<&str> = input_text.split_whitespace().collect();
    let tokens: Vec<String> = raw_tokens
        .iter()
        .map(|t| normalize_token(t))
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.is_empty() {
        return Vec::new();
    }

    let mut consumed = vec![false; tokens.len()];
    let mut results: Vec<TokenMatch> = Vec::new();
    let mut seen_asset_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let max_ngram = MAX_ALIAS_WORDS.min(tokens.len());

    for n in (1..=max_ngram).rev() {
        for i in 0..=(tokens.len() - n) {
            if consumed[i..i + n].iter().any(|&c| c) {
                continue;
            }

            let ngram: String = tokens[i..i + n]
                .iter()
                .map(|t| t.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");

            let entries = match alias_map.get(&ngram) {
                Some(e) => e,
                None => continue,
            };

            if n == 1 && ngram.len() <= 2 && !is_uppercase(&tokens[i]) {
                continue;
            }

            for j in i..i + n {
                consumed[j] = true;
            }

            for entry in entries {
                if seen_asset_ids.insert(entry.asset_id.clone()) {
                    results.push(entry.clone());
                }
            }
        }
    }

    results
}

fn normalize_token(token: &str) -> String {
    let s = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '&' && c != '\'');
    s.to_string()
}

fn is_uppercase(token: &str) -> bool {
    let mut has_alpha = false;
    for ch in token.chars() {
        if ch.is_alphabetic() {
            has_alpha = true;
            if !ch.is_uppercase() {
                return false;
            }
        }
    }
    has_alpha
}

fn resolve_token(query: Option<String>, list_all: bool) -> Result<String, String> {
    let tokens = fetch_token_list()?;
    let amap = build_alias_map(&tokens);

    if list_all {
        let all: Vec<TokenMatch> = amap
            .tokens
            .iter()
            .map(|t| TokenMatch {
                asset_id: t.defuse_asset_id.clone(),
                symbol: t.symbol.clone(),
                blockchain: t.blockchain.clone(),
                decimals: t.decimals,
            })
            .collect();
        return serde_json::to_string(&all).map_err(|e| format!("Serialization error: {e}"));
    }

    let query = query.ok_or("'query' is required when list_all is false")?;
    if query.is_empty() {
        return Err("'query' must not be empty".into());
    }

    let results = scan_for_token_hits(&query, &amap.alias_map);
    serde_json::to_string(&results).map_err(|e| format!("Serialization error: {e}"))
}

fn reverse_resolve_token(asset_id: &str) -> Result<String, String> {
    if asset_id.is_empty() {
        return Err("'asset_id' must not be empty".into());
    }

    let tokens = fetch_token_list()?;
    let amap = build_alias_map(&tokens);

    match amap.reverse_map.get(asset_id) {
        Some(token) => {
            let result = serde_json::json!({
                "asset_id": token.defuse_asset_id,
                "symbol": token.symbol,
                "blockchain": token.blockchain,
                "decimals": token.decimals,
            });
            Ok(result.to_string())
        }
        None => {
            let result = serde_json::json!({"error": format!("Unknown asset ID: {asset_id}")});
            Ok(result.to_string())
        }
    }
}

fn get_balance(account_id: &str, token_ids: Option<Vec<String>>) -> Result<String, String> {
    if account_id.is_empty() {
        return Err("'account_id' must not be empty".into());
    }

    let tokens = fetch_token_list()?;
    let amap = build_alias_map(&tokens);

    let ids: Vec<String> = match token_ids {
        Some(ids) if !ids.is_empty() => ids,
        _ => amap.tokens.iter().map(|t| t.defuse_asset_id.clone()).collect(),
    };

    let args_json = serde_json::json!({
        "account_id": account_id,
        "token_ids": ids,
    });
    let args_base64 = base64_encode(args_json.to_string().as_bytes());

    let rpc_payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "dontcare",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": "intents.near",
            "method_name": "mt_batch_balance_of",
            "args_base64": args_base64,
        }
    });

    let headers = serde_json::json!({"Content-Type": "application/json"});
    let payload_bytes = rpc_payload.to_string().into_bytes();
    let resp = near::agent::host::http_request(
        "POST",
        NEAR_RPC_URL,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    )
    .map_err(|e| format!("NEAR RPC request failed: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        return Err(format!("NEAR RPC returned status {}", resp.status));
    }

    let body = String::from_utf8(resp.body)
        .map_err(|e| format!("Invalid UTF-8 in RPC response: {e}"))?;

    let rpc_resp: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse RPC response: {e}"))?;

    if let Some(err) = rpc_resp.get("error") {
        return Err(format!("NEAR RPC error: {err}"));
    }

    let result = rpc_resp
        .get("result")
        .ok_or("Missing 'result' in RPC response")?;

    if let Some(err) = result.get("error") {
        return Err(format!("NEAR RPC query error: {err}"));
    }

    // result.result is an array of bytes → decode to JSON string
    let result_bytes: Vec<u8> = result
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("Missing 'result.result' byte array in RPC response")?
        .iter()
        .map(|v| v.as_u64().unwrap_or(0) as u8)
        .collect();

    let raw_balances_str =
        String::from_utf8(result_bytes).map_err(|e| format!("Invalid UTF-8 in RPC result: {e}"))?;

    let raw_balances: Vec<String> = serde_json::from_str(&raw_balances_str)
        .map_err(|e| format!("Failed to parse balance array: {e}"))?;

    if raw_balances.len() != ids.len() {
        return Err(format!(
            "Balance count mismatch: expected {}, got {}",
            ids.len(),
            raw_balances.len()
        ));
    }

    let mut positions: Vec<TokenBalance> = Vec::new();
    let mut total_value_usdc = 0.0;

    for (token_id, raw_balance) in ids.iter().zip(raw_balances.iter()) {
        if raw_balance == "0" {
            continue;
        }

        let meta = amap.reverse_map.get(token_id);
        let decimals = meta.map(|m| m.decimals).unwrap_or(0);
        let symbol = meta.map(|m| m.symbol.clone());
        let balance = parse_balance(raw_balance, decimals);

        let value_usdc = meta.and_then(|m| m.price).map(|price| {
            let val = balance * price;
            let rounded = (val * 100.0).round() / 100.0;
            total_value_usdc += rounded;
            rounded
        });

        positions.push(TokenBalance {
            defuse_asset_id: token_id.clone(),
            symbol,
            raw_balance: raw_balance.clone(),
            balance,
            decimals,
            value_usdc,
        });
    }

    total_value_usdc = (total_value_usdc * 100.0).round() / 100.0;

    let response = BalanceResponse {
        positions,
        total_value_usdc,
    };

    serde_json::to_string(&response).map_err(|e| format!("Serialization error: {e}"))
}

fn resolve_single_token(token: &str, amap: &AliasMap) -> Result<TokenMetadata, String> {
    if token.contains(':') {
        return amap
            .reverse_map
            .get(token)
            .cloned()
            .ok_or_else(|| format!("Unknown asset ID: {token}"));
    }
    let key = token.trim().to_lowercase();
    let matches = amap
        .alias_map
        .get(&key)
        .ok_or_else(|| format!("Unknown token symbol: {token}"))?;
    if matches.len() == 1 {
        let m = &matches[0];
        return amap
            .reverse_map
            .get(&m.asset_id)
            .cloned()
            .ok_or_else(|| format!("Unknown asset ID for symbol: {token}"));
    }
    let ids: Vec<&str> = matches.iter().map(|m| m.asset_id.as_str()).collect();
    Err(format!(
        "Ambiguous symbol '{token}' matches multiple tokens: {ids:?}. \
         Please specify the full defuse asset ID."
    ))
}

#[derive(Debug, Serialize)]
struct SwapQuoteResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_in_formatted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_in_usd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_out: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_out_formatted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_out_usd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_amount_out: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_estimate: Option<u64>,
    deeplink: String,
}

fn get_swap_quote(
    from_token: &str,
    to_token: &str,
    amount: &str,
    account_id: &str,
    slippage_bps: u32,
    swap_type: &str,
) -> Result<String, String> {
    if from_token.is_empty() || to_token.is_empty() || amount.is_empty() || account_id.is_empty() {
        return Err("from_token, to_token, amount, and account_id are all required".into());
    }

    let tokens = fetch_token_list()?;
    let amap = build_alias_map(&tokens);

    let origin = resolve_single_token(from_token, &amap)?;
    let destination = resolve_single_token(to_token, &amap)?;

    let raw_amount = human_to_raw(amount, origin.decimals)?;

    let now_ms = near::agent::host::now_millis();
    let deadline_secs = now_ms / 1000 + 600; // 10 minutes
    let deadline = format_iso8601(deadline_secs);

    let request = serde_json::json!({
        "dry": true,
        "swapType": swap_type,
        "slippageTolerance": slippage_bps,
        "originAsset": origin.defuse_asset_id,
        "depositType": "INTENTS",
        "destinationAsset": destination.defuse_asset_id,
        "amount": raw_amount,
        "refundTo": account_id,
        "refundType": "INTENTS",
        "recipient": account_id,
        "recipientType": "INTENTS",
        "deadline": deadline,
    });

    let headers = serde_json::json!({"Content-Type": "application/json"});
    let payload = request.to_string().into_bytes();
    let resp = near::agent::host::http_request(
        "POST",
        DEFUSE_QUOTE_URL,
        &headers.to_string(),
        Some(&payload),
        None,
    )
    .map_err(|e| format!("Quote API request failed: {e}"))?;

    if resp.status == 400 {
        return Err(
            "Failed to get swap quote — the amount may be too small for this route. \
             Try a larger amount."
                .into(),
        );
    }
    if resp.status < 200 || resp.status >= 300 {
        return Err(format!("Quote API returned status {}", resp.status));
    }

    let body = String::from_utf8(resp.body)
        .map_err(|e| format!("Invalid UTF-8 in quote response: {e}"))?;
    let data: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse quote response: {e}"))?;

    let correlation_id = data.get("correlationId").and_then(|v| v.as_str()).map(String::from);

    let quote = data
        .get("quote")
        .ok_or("Swap quote response missing 'quote' key")?;

    let deeplink = format!(
        "https://app.defuse.org/swap?correlationId={}&from={}&to={}&amount={}",
        correlation_id.as_deref().unwrap_or(""),
        origin.defuse_asset_id,
        destination.defuse_asset_id,
        amount,
    );

    let str_field = |key: &str| quote.get(key).and_then(|v| v.as_str()).map(String::from);

    let response = SwapQuoteResponse {
        correlation_id,
        amount_in: str_field("amountIn"),
        amount_in_formatted: str_field("amountInFormatted"),
        amount_in_usd: str_field("amountInUsd"),
        amount_out: str_field("amountOut"),
        amount_out_formatted: str_field("amountOutFormatted"),
        amount_out_usd: str_field("amountOutUsd"),
        min_amount_out: str_field("minAmountOut"),
        time_estimate: quote.get("timeEstimate").and_then(|v| v.as_u64()),
        deeplink,
    };

    serde_json::to_string(&response).map_err(|e| format!("Serialization error: {e}"))
}

/// Convert a human-readable amount (e.g. "100.5") to raw integer string using decimals.
fn human_to_raw(amount: &str, decimals: u32) -> Result<String, String> {
    let amount = amount.trim();
    let parts: Vec<&str> = amount.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("Invalid amount: {amount}"));
    }
    let integer_part = parts[0];
    let frac_part = if parts.len() == 2 { parts[1] } else { "" };

    if frac_part.len() > decimals as usize {
        return Err(format!(
            "Amount has more decimal places ({}) than token supports ({decimals})",
            frac_part.len()
        ));
    }

    let padded_frac = format!("{:0<width$}", frac_part, width = decimals as usize);
    let raw_str = format!("{integer_part}{padded_frac}");

    let raw_str = raw_str.trim_start_matches('0');
    if raw_str.is_empty() {
        Ok("0".into())
    } else {
        Ok(raw_str.into())
    }
}

/// Format a Unix timestamp as ISO 8601 UTC (simplified).
fn format_iso8601(secs: u64) -> String {
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_civil(days_since_epoch as i64);

    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

/// Convert days since 1970-01-01 to (year, month, day).
fn days_to_civil(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

fn parse_balance(raw: &str, decimals: u32) -> f64 {
    // Parse as u128 to handle large balances, then divide
    match raw.parse::<u128>() {
        Ok(val) => {
            let divisor = 10u128.pow(decimals);
            if divisor == 0 {
                val as f64
            } else {
                (val as f64) / (divisor as f64)
            }
        }
        Err(_) => 0.0,
    }
}

const BASE64_CHARS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        output.push(BASE64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        output.push(BASE64_CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if i + 1 < input.len() {
            output.push(BASE64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }

        if i + 2 < input.len() {
            output.push(BASE64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }

        i += 3;
    }
    output
}

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "action": {
      "type": "string",
      "enum": ["resolve_token", "reverse_resolve_token", "get_balance", "get_swap_quote"],
      "description": "Which action to perform"
    },
    "query": {
      "type": "string",
      "description": "Token reference to resolve (for resolve_token). Examples: 'ethereum', 'USDC on arbitrum', 'wrapped near', 'bitcoin'. Uses contiguous n-gram matching, so word order matters. To target a specific chain, use format '<symbol> on <chain>' as a single phrase."
    },
    "list_all": {
      "type": "boolean",
      "description": "Set to true to return ALL tokens in the registry. Use when query returns empty results to find the correct token name/symbol.",
      "default": false
    },
    "asset_id": {
      "type": "string",
      "description": "Defuse asset ID to look up (for reverse_resolve_token). Use this to get the ticker symbol (e.g. 'WBTC') from an asset ID (e.g. 'nep141:eth-0x2260...omft.near')."
    },
    "account_id": {
      "type": "string",
      "description": "NEAR wallet address or account ID (for get_balance and get_swap_quote). Either a named account (alice.near) or a 0x-prefixed hex address — pass verbatim, do NOT convert hex to decimal."
    },
    "token_ids": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Specific defuse asset IDs to query (for get_balance). If omitted, returns all non-zero balances."
    },
    "from_token": {
      "type": "string",
      "description": "Token symbol (e.g. 'USDC') or defuse asset ID (e.g. 'nep141:usdc.near') to swap from (for get_swap_quote)."
    },
    "to_token": {
      "type": "string",
      "description": "Token symbol (e.g. 'ETH') or defuse asset ID (e.g. 'nep141:eth.near') to swap to (for get_swap_quote)."
    },
    "amount": {
      "type": "string",
      "description": "Human-readable amount to swap (e.g. '100.5') (for get_swap_quote)."
    },
    "slippage_bps": {
      "type": "integer",
      "description": "Slippage tolerance in basis points (for get_swap_quote). Default 100 (1%)."
    },
    "swap_type": {
      "type": "string",
      "description": "Swap type (for get_swap_quote): EXACT_INPUT (default) or EXACT_OUTPUT."
    }
  },
  "required": ["action"]
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tokens() -> Vec<TokenMetadata> {
        vec![
            TokenMetadata {
                defuse_asset_id: "nep141:wrap.near".into(),
                decimals: 24,
                symbol: "wNEAR".into(),
                blockchain: "near".into(),
                contract_address: Some("wrap.near".into()),
                price: Some(3.50),
            },
            TokenMetadata {
                defuse_asset_id: "nep141:usdc.near".into(),
                decimals: 6,
                symbol: "USDC".into(),
                blockchain: "near".into(),
                contract_address: Some("usdc.near".into()),
                price: Some(1.0),
            },
            TokenMetadata {
                defuse_asset_id: "nep141:eth-usdc.arb".into(),
                decimals: 6,
                symbol: "USDC".into(),
                blockchain: "arbitrum".into(),
                contract_address: Some("0xa0b8...".into()),
                price: Some(1.0),
            },
            TokenMetadata {
                defuse_asset_id: "nep141:eth.near".into(),
                decimals: 18,
                symbol: "ETH".into(),
                blockchain: "eth".into(),
                contract_address: None,
                price: Some(2500.0),
            },
            TokenMetadata {
                defuse_asset_id: "nep141:wbtc.near".into(),
                decimals: 8,
                symbol: "WBTC".into(),
                blockchain: "eth".into(),
                contract_address: None,
                price: Some(60000.0),
            },
        ]
    }

    #[test]
    fn test_normalize_token() {
        assert_eq!(normalize_token("hello"), "hello");
        assert_eq!(normalize_token("(hello)"), "hello");
        assert_eq!(normalize_token("..test.."), "test");
        assert_eq!(normalize_token("it's"), "it's");
        assert_eq!(normalize_token("R&D"), "R&D");
    }

    #[test]
    fn test_is_uppercase() {
        assert!(is_uppercase("ETH"));
        assert!(is_uppercase("BTC"));
        assert!(!is_uppercase("eth"));
        assert!(!is_uppercase("Eth"));
        assert!(is_uppercase("A1"));
        assert!(!is_uppercase("123")); // no alpha
    }

    #[test]
    fn test_alias_map_symbol_lookup() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        assert!(amap.alias_map.contains_key("wNEAR".to_lowercase().as_str()));
        assert!(amap.alias_map.contains_key("usdc"));
        assert!(amap.alias_map.contains_key("eth"));
    }

    #[test]
    fn test_alias_map_chain_specific() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        let key = "usdc on arbitrum";
        let results = amap.alias_map.get(key);
        assert!(results.is_some());
        let results = results.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].blockchain, "arbitrum");
    }

    #[test]
    fn test_alias_map_curated() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);

        // "ethereum" should map to ETH
        let results = amap.alias_map.get("ethereum").unwrap();
        assert!(results.iter().any(|r| r.symbol == "ETH"));

        // "bitcoin" should map to BTC and WBTC
        let results = amap.alias_map.get("bitcoin").unwrap();
        assert!(results.iter().any(|r| r.symbol == "WBTC"));

        // "near" should map to wNEAR
        let results = amap.alias_map.get("near").unwrap();
        assert!(results.iter().any(|r| r.symbol == "wNEAR"));
    }

    #[test]
    fn test_scan_basic() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        let results = scan_for_token_hits("I want some ethereum", &amap.alias_map);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.symbol == "ETH"));
    }

    #[test]
    fn test_scan_chain_specific() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        let results = scan_for_token_hits("USDC on arbitrum", &amap.alias_map);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].blockchain, "arbitrum");
    }

    #[test]
    fn test_scan_short_alias_requires_uppercase() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);

        // Lowercase "do" should not match (if it were an alias)
        // But uppercase short aliases should match
        let results = scan_for_token_hits("I have some ETH", &amap.alias_map);
        assert!(results.iter().any(|r| r.symbol == "ETH"));

        // lowercase "eth" with len=3 is fine (>2 chars)
        let results = scan_for_token_hits("I have some eth", &amap.alias_map);
        assert!(results.iter().any(|r| r.symbol == "ETH"));
    }

    #[test]
    fn test_scan_longest_match_wins() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        // "USDC on arbitrum" is 3 tokens and should be matched as one unit
        let results = scan_for_token_hits("send USDC on arbitrum please", &amap.alias_map);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].blockchain, "arbitrum");
    }

    #[test]
    fn test_reverse_map() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        let meta = amap.reverse_map.get("nep141:wrap.near");
        assert!(meta.is_some());
        assert_eq!(meta.unwrap().symbol, "wNEAR");
    }

    #[test]
    fn test_parse_balance() {
        assert!((parse_balance("1000000", 6) - 1.0).abs() < f64::EPSILON);
        assert!((parse_balance("1000000000000000000", 18) - 1.0).abs() < f64::EPSILON);
        assert!((parse_balance("0", 6) - 0.0).abs() < f64::EPSILON);
        assert!((parse_balance("500000", 6) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b"Hi"), "SGk=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
        assert_eq!(base64_encode(b""), "");

        // Verify a JSON args payload round-trips correctly
        let json = r#"{"account_id":"test.near","token_ids":["nep141:wrap.near"]}"#;
        let encoded = base64_encode(json.as_bytes());
        assert!(!encoded.is_empty());
        assert!(!encoded.contains('\n'));
    }

    #[test]
    fn test_action_deserialize() {
        let json = r#"{"action": "resolve_token", "query": "ethereum"}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        match action {
            Action::ResolveToken { query, list_all } => {
                assert_eq!(query.unwrap(), "ethereum");
                assert!(list_all.is_none());
            }
            _ => panic!("Wrong variant"),
        }

        let json = r#"{"action": "get_balance", "account_id": "test.near"}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        match action {
            Action::GetBalance {
                account_id,
                token_ids,
            } => {
                assert_eq!(account_id, "test.near");
                assert!(token_ids.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_normalize_account_id_passthrough() {
        assert_eq!(normalize_account_id("alice.near"), "alice.near");
        assert_eq!(normalize_account_id("0xabc123"), "0xabc123");
    }

    #[test]
    fn test_normalize_account_id_decimal_to_hex() {
        let decimal = "1271270613000041655817448348132275889066893754095";
        assert!(decimal.len() > 40);
        let result = normalize_account_id(decimal);
        assert_eq!(result, "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    }

    #[test]
    fn test_decimal_to_hex() {
        assert_eq!(decimal_to_hex("255"), "ff");
        assert_eq!(decimal_to_hex("16"), "10");
        assert_eq!(decimal_to_hex("0"), "0");
        assert_eq!(decimal_to_hex("256"), "100");
    }

    #[test]
    fn test_human_to_raw() {
        assert_eq!(human_to_raw("100", 6).unwrap(), "100000000");
        assert_eq!(human_to_raw("100.5", 6).unwrap(), "100500000");
        assert_eq!(human_to_raw("0.000001", 6).unwrap(), "1");
        assert_eq!(human_to_raw("0", 6).unwrap(), "0");
        assert_eq!(human_to_raw("1", 18).unwrap(), "1000000000000000000");
        assert!(human_to_raw("1.1234567", 6).is_err()); // too many decimals
    }

    #[test]
    fn test_format_iso8601() {
        // 2024-01-01T00:00:00Z = 1704067200
        let result = format_iso8601(1704067200);
        assert_eq!(result, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_resolve_single_token_by_asset_id() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        let meta = resolve_single_token("nep141:wrap.near", &amap).unwrap();
        assert_eq!(meta.symbol, "wNEAR");
    }

    #[test]
    fn test_resolve_single_token_by_symbol_unique() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        let meta = resolve_single_token("wNEAR", &amap).unwrap();
        assert_eq!(meta.defuse_asset_id, "nep141:wrap.near");
    }

    #[test]
    fn test_resolve_single_token_ambiguous() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);
        // USDC exists on multiple chains
        let result = resolve_single_token("USDC", &amap);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Ambiguous"));
    }

    #[test]
    fn test_action_deserialize_swap_quote() {
        let json = r#"{"action": "get_swap_quote", "from_token": "USDC", "to_token": "ETH", "amount": "100", "account_id": "alice.near"}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        match action {
            Action::GetSwapQuote {
                from_token,
                to_token,
                amount,
                account_id,
                slippage_bps,
                swap_type,
            } => {
                assert_eq!(from_token, "USDC");
                assert_eq!(to_token, "ETH");
                assert_eq!(amount, "100");
                assert_eq!(account_id, "alice.near");
                assert!(slippage_bps.is_none());
                assert!(swap_type.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_blockchain_alias_coverage() {
        let tokens = sample_tokens();
        let amap = build_alias_map(&tokens);

        // "usdc on arb" should work since "arb" is an alias for "arbitrum"
        assert!(amap.alias_map.contains_key("usdc on arb"));

        // "usdc on near" should work
        assert!(amap.alias_map.contains_key("usdc on near"));

        // "eth on ethereum" should work since ETH is on "eth" blockchain
        assert!(amap.alias_map.contains_key("eth on ethereum"));
        assert!(amap.alias_map.contains_key("eth on eth"));
    }
}
