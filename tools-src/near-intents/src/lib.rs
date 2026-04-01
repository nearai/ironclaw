//! Read-only NEAR Intents / Defuse WASM tool for IronClaw.
//!
//! This Phase 0 tool intentionally stops at the read plane:
//! - token resolution and reverse lookup
//! - protocol metadata
//! - balance queries
//! - dry swap quotes
//!
//! It does not authenticate, store custody material, sign, or execute trades.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

const DEFUSE_TOKENS_URL: &str = "https://1click.chaindefuser.com/v0/tokens";
const DEFUSE_QUOTE_URL: &str = "https://1click.chaindefuser.com/v0/quote";
const NEAR_RPC_URL: &str = "https://rpc.mainnet.near.org";
const INTENTS_CONTRACT_ID: &str = "intents.near";
const MAX_ALIAS_WORDS: usize = 5;
const DEFAULT_SLIPPAGE_BPS: u32 = 100;
const QUOTE_DEADLINE_SECS: u64 = 600;

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
    ("usd coin", &[("USDC", None)]),
    ("usdc", &[("USDC", None)]),
    ("tether", &[("USDT", None)]),
    ("usdt", &[("USDT", None)]),
    ("dai", &[("DAI", None)]),
    ("dogecoin", &[("DOGE", None)]),
    ("doge", &[("DOGE", None)]),
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

struct NearIntentsTool;

impl exports::near::agent::tool::Guest for NearIntentsTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(error),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Read-only NEAR Intents / Defuse tool for resolving tokens, inspecting \
         protocol metadata, querying balances, and requesting dry swap quotes. \
         No authentication or signing is performed."
            .to_string()
    }
}

export!(NearIntentsTool);

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
    #[serde(rename = "get_protocol_metadata")]
    GetProtocolMetadata {},
}

#[derive(Debug, Clone, Deserialize)]
struct DefuseTokenRaw {
    #[serde(rename = "assetId")]
    asset_id: String,
    decimals: u32,
    symbol: String,
    blockchain: String,
    #[serde(rename = "contractAddress")]
    contract_address: Option<String>,
    price: Option<f64>,
}

#[derive(Debug, Clone)]
struct TokenMetadata {
    asset_id: String,
    symbol: String,
    blockchain: String,
    decimals: u32,
    contract_address: Option<String>,
    price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct TokenMatch {
    asset_id: String,
    symbol: String,
    blockchain: String,
    decimals: u32,
}

#[derive(Debug, Serialize)]
struct TokenBalance {
    asset_id: String,
    symbol: String,
    blockchain: String,
    raw_balance: String,
    balance: f64,
    decimals: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    value_usdc: Option<f64>,
}

#[derive(Debug, Serialize)]
struct BalanceResponse {
    account_id: String,
    positions: Vec<TokenBalance>,
    total_value_usdc: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skipped_token_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReverseResolveResponse {
    asset_id: String,
    symbol: String,
    blockchain: String,
    decimals: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<f64>,
}

#[derive(Debug, Serialize)]
struct ProtocolMetadataResponse {
    protocol: String,
    balance_contract: String,
    token_count: usize,
    chains: Vec<String>,
    actions: Vec<String>,
    public_endpoints: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SwapQuoteResponse {
    from_asset_id: String,
    to_asset_id: String,
    requested_amount: String,
    requested_amount_raw: String,
    slippage_bps: u32,
    swap_type: String,
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

struct AliasMap {
    alias_map: HashMap<String, Vec<TokenMatch>>,
    reverse_map: HashMap<String, TokenMetadata>,
    tokens: Vec<TokenMetadata>,
}

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
            slippage_bps.unwrap_or(DEFAULT_SLIPPAGE_BPS),
            swap_type
                .as_deref()
                .unwrap_or("EXACT_INPUT")
                .to_ascii_uppercase()
                .as_str(),
        ),
        Action::GetProtocolMetadata {} => get_protocol_metadata(),
    }
}

fn resolve_token(query: Option<String>, list_all: bool) -> Result<String, String> {
    let tokens = fetch_token_list()?;
    let alias_map = build_alias_map(&tokens);

    if list_all {
        let mut matches: Vec<TokenMatch> = alias_map
            .tokens
            .iter()
            .map(TokenMatch::from_metadata)
            .collect();
        sort_matches(&mut matches);
        return serialize(&matches);
    }

    let query = query.ok_or("'query' is required when list_all is false")?;
    if query.trim().is_empty() {
        return Err("'query' must not be empty".into());
    }

    let mut results = scan_for_token_hits(&query, &alias_map.alias_map);
    sort_matches(&mut results);
    serialize(&results)
}

fn reverse_resolve_token(asset_id: &str) -> Result<String, String> {
    if asset_id.trim().is_empty() {
        return Err("'asset_id' must not be empty".into());
    }

    let tokens = fetch_token_list()?;
    let alias_map = build_alias_map(&tokens);
    let token = alias_map
        .reverse_map
        .get(asset_id)
        .ok_or_else(|| format!("Unknown asset ID: {asset_id}"))?;

    serialize(&ReverseResolveResponse {
        asset_id: token.asset_id.clone(),
        symbol: token.symbol.clone(),
        blockchain: token.blockchain.clone(),
        decimals: token.decimals,
        contract_address: token.contract_address.clone(),
        price: round_option(token.price),
    })
}

fn get_protocol_metadata() -> Result<String, String> {
    let tokens = fetch_token_list()?;
    let mut chains: Vec<String> = tokens
        .iter()
        .map(|token| token.blockchain.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    chains.sort();

    serialize(&ProtocolMetadataResponse {
        protocol: "NEAR Intents / Defuse".to_string(),
        balance_contract: INTENTS_CONTRACT_ID.to_string(),
        token_count: tokens.len(),
        chains,
        actions: vec![
            "resolve_token".to_string(),
            "reverse_resolve_token".to_string(),
            "get_balance".to_string(),
            "get_swap_quote".to_string(),
            "get_protocol_metadata".to_string(),
        ],
        public_endpoints: vec![
            DEFUSE_TOKENS_URL.to_string(),
            DEFUSE_QUOTE_URL.to_string(),
            NEAR_RPC_URL.to_string(),
        ],
    })
}

fn get_balance(account_id: &str, token_ids: Option<Vec<String>>) -> Result<String, String> {
    if account_id.trim().is_empty() {
        return Err("'account_id' must not be empty".into());
    }

    let tokens = fetch_token_list()?;
    let alias_map = build_alias_map(&tokens);

    let query_token_ids = match token_ids {
        Some(ids) if !ids.is_empty() => ids,
        _ => alias_map
            .tokens
            .iter()
            .map(|token| token.asset_id.clone())
            .collect(),
    };

    let raw_balances = query_balance_rpc(account_id, &query_token_ids)?;

    if raw_balances.len() != query_token_ids.len() {
        return Err(format!(
            "Balance count mismatch: expected {}, got {}",
            query_token_ids.len(),
            raw_balances.len()
        ));
    }

    let mut positions = Vec::new();
    let mut skipped_token_ids = Vec::new();
    let mut total_value_usdc = 0.0;

    for (token_id, raw_balance) in query_token_ids.iter().zip(raw_balances.iter()) {
        if raw_balance == "0" {
            continue;
        }

        let Some(metadata) = alias_map.reverse_map.get(token_id) else {
            skipped_token_ids.push(token_id.clone());
            continue;
        };

        let balance = parse_balance(raw_balance, metadata.decimals)?;
        let value_usdc = metadata.price.map(|price| round2(balance * price));

        if let Some(value) = value_usdc {
            total_value_usdc += value;
        }

        positions.push(TokenBalance {
            asset_id: metadata.asset_id.clone(),
            symbol: metadata.symbol.clone(),
            blockchain: metadata.blockchain.clone(),
            raw_balance: raw_balance.clone(),
            balance: round6(balance),
            decimals: metadata.decimals,
            value_usdc,
        });
    }

    positions.sort_by(|a, b| {
        b.value_usdc
            .unwrap_or(0.0)
            .partial_cmp(&a.value_usdc.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.blockchain.cmp(&b.blockchain))
    });

    serialize(&BalanceResponse {
        account_id: account_id.to_string(),
        positions,
        total_value_usdc: round2(total_value_usdc),
        skipped_token_ids,
    })
}

fn get_swap_quote(
    from_token: &str,
    to_token: &str,
    amount: &str,
    account_id: &str,
    slippage_bps: u32,
    swap_type: &str,
) -> Result<String, String> {
    if from_token.trim().is_empty()
        || to_token.trim().is_empty()
        || amount.trim().is_empty()
        || account_id.trim().is_empty()
    {
        return Err("from_token, to_token, amount, and account_id are all required".into());
    }

    if swap_type != "EXACT_INPUT" && swap_type != "EXACT_OUTPUT" {
        return Err(format!(
            "Unsupported swap_type '{swap_type}'; expected EXACT_INPUT or EXACT_OUTPUT"
        ));
    }

    let tokens = fetch_token_list()?;
    let alias_map = build_alias_map(&tokens);
    let from_metadata = resolve_single_token(from_token, &alias_map)?;
    let to_metadata = resolve_single_token(to_token, &alias_map)?;
    let raw_amount = human_to_raw(amount, from_metadata.decimals)?;

    let deadline = format_iso8601(near::agent::host::now_millis() / 1000 + QUOTE_DEADLINE_SECS);
    let request = serde_json::json!({
        "dry": true,
        "swapType": swap_type,
        "slippageTolerance": slippage_bps,
        "originAsset": from_metadata.asset_id,
        "depositType": "INTENTS",
        "destinationAsset": to_metadata.asset_id,
        "amount": raw_amount,
        "refundTo": account_id,
        "refundType": "INTENTS",
        "recipient": account_id,
        "recipientType": "INTENTS",
        "deadline": deadline
    });

    let headers = serde_json::json!({
        "Accept": "application/json",
        "Content-Type": "application/json"
    });
    let payload = request.to_string().into_bytes();
    let response = near::agent::host::http_request(
        "POST",
        DEFUSE_QUOTE_URL,
        &headers.to_string(),
        Some(&payload),
        None,
    )
    .map_err(|e| format!("Quote API request failed: {e}"))?;

    if response.status < 200 || response.status >= 300 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Quote API returned status {}: {}",
            response.status, body
        ));
    }

    let body = String::from_utf8(response.body)
        .map_err(|e| format!("Invalid UTF-8 in quote response: {e}"))?;
    let data: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse quote response: {e}"))?;

    let quote = data
        .get("quote")
        .ok_or("Swap quote response missing 'quote' object")?;
    let correlation_id = string_field(&data, "correlationId");
    let from_asset_id = from_metadata.asset_id.clone();
    let to_asset_id = to_metadata.asset_id.clone();

    serialize(&SwapQuoteResponse {
        from_asset_id: from_asset_id.clone(),
        to_asset_id: to_asset_id.clone(),
        requested_amount: amount.to_string(),
        requested_amount_raw: raw_amount,
        slippage_bps,
        swap_type: swap_type.to_string(),
        correlation_id: correlation_id.clone(),
        amount_in: string_field(quote, "amountIn"),
        amount_in_formatted: string_field(quote, "amountInFormatted"),
        amount_in_usd: string_field(quote, "amountInUsd"),
        amount_out: string_field(quote, "amountOut"),
        amount_out_formatted: string_field(quote, "amountOutFormatted"),
        amount_out_usd: string_field(quote, "amountOutUsd"),
        min_amount_out: string_field(quote, "minAmountOut"),
        time_estimate: quote.get("timeEstimate").and_then(|value| value.as_u64()),
        deeplink: format!(
            "https://app.defuse.org/swap?correlationId={}&from={}&to={}&amount={}",
            correlation_id.unwrap_or_default(),
            from_asset_id,
            to_asset_id,
            amount
        ),
    })
}

fn fetch_token_list() -> Result<Vec<TokenMetadata>, String> {
    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-NearIntents-Tool/0.1"
    });

    let response =
        near::agent::host::http_request("GET", DEFUSE_TOKENS_URL, &headers.to_string(), None, None)
            .map_err(|e| format!("Failed to fetch token list: {e}"))?;

    if response.status < 200 || response.status >= 300 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Token list API returned status {}: {}",
            response.status, body
        ));
    }

    let body = String::from_utf8(response.body)
        .map_err(|e| format!("Invalid UTF-8 in token list response: {e}"))?;
    let raw_tokens: Vec<DefuseTokenRaw> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse token list: {e}"))?;

    let mut tokens: Vec<TokenMetadata> = raw_tokens
        .into_iter()
        .filter(|token| {
            !token.asset_id.trim().is_empty()
                && !token.symbol.trim().is_empty()
                && !token.blockchain.trim().is_empty()
        })
        .map(|token| TokenMetadata {
            asset_id: token.asset_id,
            symbol: token.symbol,
            blockchain: token.blockchain,
            decimals: token.decimals,
            contract_address: token.contract_address,
            price: token.price,
        })
        .collect();

    tokens.sort_by(|a, b| {
        a.symbol
            .cmp(&b.symbol)
            .then_with(|| a.blockchain.cmp(&b.blockchain))
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });

    Ok(tokens)
}

fn build_alias_map(tokens: &[TokenMetadata]) -> AliasMap {
    let mut alias_map: HashMap<String, Vec<TokenMatch>> = HashMap::new();
    let mut reverse_map = HashMap::new();
    let mut canonical_to_aliases: HashMap<&str, Vec<&str>> = HashMap::new();

    for &(alias, canonical) in BLOCKCHAIN_ALIASES {
        canonical_to_aliases
            .entry(canonical)
            .or_default()
            .push(alias);
    }

    for token in tokens {
        let token_match = TokenMatch::from_metadata(token);
        add_alias(&mut alias_map, &token.symbol, &token_match);
        add_alias(&mut alias_map, &token.asset_id, &token_match);

        if let Some(address) = &token.contract_address {
            add_alias(&mut alias_map, address, &token_match);
        }

        add_alias(
            &mut alias_map,
            &format!("{} on {}", token.symbol, token.blockchain),
            &token_match,
        );

        if let Some(chain_names) = canonical_to_aliases.get(token.blockchain.as_str()) {
            for chain_name in chain_names {
                add_alias(
                    &mut alias_map,
                    &format!("{} on {}", token.symbol, chain_name),
                    &token_match,
                );
            }
        }

        reverse_map.insert(token.asset_id.clone(), token.clone());
    }

    for &(alias, targets) in CURATED_ALIASES {
        for &(symbol_filter, chain_filter) in targets {
            for token in tokens {
                if token.symbol != symbol_filter {
                    continue;
                }
                if let Some(required_chain) = chain_filter {
                    if token.blockchain != required_chain {
                        continue;
                    }
                }
                add_alias(&mut alias_map, alias, &TokenMatch::from_metadata(token));
            }
        }
    }

    AliasMap {
        alias_map,
        reverse_map,
        tokens: tokens.to_vec(),
    }
}

fn add_alias(map: &mut HashMap<String, Vec<TokenMatch>>, alias: &str, token_match: &TokenMatch) {
    let key = alias.trim().to_ascii_lowercase();
    if key.is_empty() {
        return;
    }

    let entries = map.entry(key).or_default();
    if !entries
        .iter()
        .any(|existing| existing.asset_id == token_match.asset_id)
    {
        entries.push(token_match.clone());
    }
}

fn scan_for_token_hits(
    input_text: &str,
    alias_map: &HashMap<String, Vec<TokenMatch>>,
) -> Vec<TokenMatch> {
    let normalized_tokens: Vec<String> = input_text
        .split_whitespace()
        .map(normalize_token)
        .filter(|token| !token.is_empty())
        .collect();

    if normalized_tokens.is_empty() {
        return Vec::new();
    }

    let mut consumed = vec![false; normalized_tokens.len()];
    let mut seen_asset_ids = HashSet::new();
    let mut results = Vec::new();
    let max_ngram = MAX_ALIAS_WORDS.min(normalized_tokens.len());

    for n in (1..=max_ngram).rev() {
        for start in 0..=(normalized_tokens.len() - n) {
            if consumed[start..start + n].iter().any(|already| *already) {
                continue;
            }

            let ngram = normalized_tokens[start..start + n]
                .iter()
                .map(|token| token.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(" ");

            let Some(matches) = alias_map.get(&ngram) else {
                continue;
            };

            if n == 1 && ngram.len() <= 2 && !is_uppercase(&normalized_tokens[start]) {
                continue;
            }

            for idx in start..start + n {
                consumed[idx] = true;
            }

            for token_match in matches {
                if seen_asset_ids.insert(token_match.asset_id.clone()) {
                    results.push(token_match.clone());
                }
            }
        }
    }

    results
}

fn resolve_single_token(input: &str, alias_map: &AliasMap) -> Result<TokenMetadata, String> {
    if input.contains(':') {
        return alias_map
            .reverse_map
            .get(input)
            .cloned()
            .ok_or_else(|| format!("Unknown asset ID: {input}"));
    }

    let key = input.trim().to_ascii_lowercase();
    let matches = alias_map
        .alias_map
        .get(&key)
        .ok_or_else(|| format!("Unknown token reference: {input}"))?;

    if matches.len() == 1 {
        return alias_map
            .reverse_map
            .get(&matches[0].asset_id)
            .cloned()
            .ok_or_else(|| format!("Unknown asset ID: {}", matches[0].asset_id));
    }

    let choices = matches
        .iter()
        .map(|item| format!("{} ({})", item.asset_id, item.blockchain))
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Ambiguous token reference '{input}'. Use a chain-qualified symbol or asset ID. Matches: {choices}"
    ))
}

fn query_balance_rpc(account_id: &str, token_ids: &[String]) -> Result<Vec<String>, String> {
    let args_json = serde_json::json!({
        "account_id": account_id,
        "token_ids": token_ids
    });
    let args_base64 = base64_encode(args_json.to_string().as_bytes());

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "ironclaw-near-intents",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": INTENTS_CONTRACT_ID,
            "method_name": "mt_batch_balance_of",
            "args_base64": args_base64
        }
    });

    let headers = serde_json::json!({
        "Accept": "application/json",
        "Content-Type": "application/json"
    });
    let request_body = payload.to_string().into_bytes();
    let response = near::agent::host::http_request(
        "POST",
        NEAR_RPC_URL,
        &headers.to_string(),
        Some(&request_body),
        None,
    )
    .map_err(|e| format!("NEAR RPC request failed: {e}"))?;

    if response.status < 200 || response.status >= 300 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "NEAR RPC returned status {}: {}",
            response.status, body
        ));
    }

    let body = String::from_utf8(response.body)
        .map_err(|e| format!("Invalid UTF-8 in RPC response: {e}"))?;
    let rpc_response: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse RPC response: {e}"))?;

    if let Some(error) = rpc_response.get("error") {
        return Err(format!("NEAR RPC error: {error}"));
    }

    let result = rpc_response
        .get("result")
        .ok_or("Missing 'result' in RPC response")?;
    if let Some(error) = result.get("error") {
        return Err(format!("NEAR RPC query error: {error}"));
    }

    let result_bytes = result
        .get("result")
        .and_then(|value| value.as_array())
        .ok_or("Missing 'result.result' byte array in RPC response")?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| "RPC byte array contained a non-integer value".to_string())
                .and_then(|byte| {
                    u8::try_from(byte)
                        .map_err(|_| "RPC byte array contained a value above 255".to_string())
                })
        })
        .collect::<Result<Vec<u8>, String>>()?;

    let raw_balances_str =
        String::from_utf8(result_bytes).map_err(|e| format!("Invalid UTF-8 in RPC result: {e}"))?;
    serde_json::from_str(&raw_balances_str)
        .map_err(|e| format!("Failed to parse balance array: {e}"))
}

fn normalize_account_id(account_id: &str) -> String {
    if account_id.chars().all(|ch| ch.is_ascii_digit()) && account_id.len() > 40 {
        return format!("0x{}", decimal_to_hex(account_id));
    }
    account_id.to_string()
}

fn normalize_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '&' && ch != '\'')
        .to_string()
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

fn parse_balance(raw_balance: &str, decimals: u32) -> Result<f64, String> {
    let value = raw_balance
        .parse::<u128>()
        .map_err(|e| format!("Invalid raw balance '{raw_balance}': {e}"))?;
    let divisor = 10u128
        .checked_pow(decimals)
        .ok_or_else(|| format!("Unsupported decimal precision: {decimals}"))?;
    if divisor == 0 {
        return Err("Invalid zero divisor when parsing balance".into());
    }
    Ok((value as f64) / (divisor as f64))
}

fn human_to_raw(amount: &str, decimals: u32) -> Result<String, String> {
    let amount = amount.trim();
    if amount.is_empty() {
        return Err("'amount' must not be empty".into());
    }
    if amount.starts_with('-') {
        return Err("'amount' must not be negative".into());
    }

    let parts: Vec<&str> = amount.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("Invalid amount: {amount}"));
    }

    let integer = parts[0];
    let fraction = if parts.len() == 2 { parts[1] } else { "" };
    if !integer.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(format!("Invalid amount: {amount}"));
    }
    if fraction.len() > decimals as usize {
        return Err(format!(
            "Amount has more decimal places ({}) than token supports ({decimals})",
            fraction.len()
        ));
    }

    let padded_fraction = format!("{fraction:0<width$}", width = decimals as usize);
    let raw = format!("{integer}{padded_fraction}");
    let trimmed = raw.trim_start_matches('0');
    if trimmed.is_empty() {
        Ok("0".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn decimal_to_hex(decimal: &str) -> String {
    let mut digits: Vec<u8> = decimal.bytes().map(|byte| byte - b'0').collect();
    if digits.is_empty() || (digits.len() == 1 && digits[0] == 0) {
        return "0".to_string();
    }

    let mut hex_chars = Vec::new();
    while !(digits.is_empty() || (digits.len() == 1 && digits[0] == 0)) {
        let mut remainder = 0u32;
        let mut next_digits = Vec::new();

        for digit in &digits {
            let value = remainder * 10 + u32::from(*digit);
            let quotient = value / 16;
            remainder = value % 16;
            if !next_digits.is_empty() || quotient > 0 {
                next_digits.push(quotient as u8);
            }
        }

        let ch = if remainder < 10 {
            b'0' + remainder as u8
        } else {
            b'a' + (remainder as u8 - 10)
        };
        hex_chars.push(ch);
        digits = next_digits;
    }

    hex_chars.reverse();
    String::from_utf8(hex_chars).unwrap_or_else(|_| String::new())
}

fn format_iso8601(seconds_since_epoch: u64) -> String {
    let days_since_epoch = seconds_since_epoch / 86_400;
    let time_of_day = seconds_since_epoch % 86_400;

    let hours = time_of_day / 3_600;
    let minutes = (time_of_day % 3_600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_civil(days_since_epoch as i64);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_civil(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u32;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let adjusted_year = if month <= 2 { year + 1 } else { year };
    (adjusted_year as i32, month, day)
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    let field = value.get(key)?;
    match field {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(num) => Some(num.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn round_option(value: Option<f64>) -> Option<f64> {
    value.map(round6)
}

fn sort_matches(matches: &mut [TokenMatch]) {
    matches.sort_by(|a, b| {
        a.symbol
            .cmp(&b.symbol)
            .then_with(|| a.blockchain.cmp(&b.blockchain))
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });
}

fn serialize<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("Serialization error: {e}"))
}

impl TokenMatch {
    fn from_metadata(metadata: &TokenMetadata) -> Self {
        Self {
            asset_id: metadata.asset_id.clone(),
            symbol: metadata.symbol.clone(),
            blockchain: metadata.blockchain.clone(),
            decimals: metadata.decimals,
        }
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut index = 0;

    while index < input.len() {
        let b0 = input[index] as u32;
        let b1 = if index + 1 < input.len() {
            input[index + 1] as u32
        } else {
            0
        };
        let b2 = if index + 2 < input.len() {
            input[index + 2] as u32
        } else {
            0
        };

        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(BASE64_ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        output.push(BASE64_ALPHABET[((triple >> 12) & 0x3f) as usize] as char);

        if index + 1 < input.len() {
            output.push(BASE64_ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }

        if index + 2 < input.len() {
            output.push(BASE64_ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }

        index += 3;
    }

    output
}

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "action": {
      "type": "string",
      "enum": [
        "resolve_token",
        "reverse_resolve_token",
        "get_balance",
        "get_swap_quote",
        "get_protocol_metadata"
      ],
      "description": "Which read-only action to perform."
    },
    "query": {
      "type": "string",
      "description": "Token reference to resolve. Examples: 'ethereum', 'USDC on arbitrum', 'wrapped near', 'bitcoin'."
    },
    "list_all": {
      "type": "boolean",
      "description": "Return all known Defuse tokens instead of resolving a specific query.",
      "default": false
    },
    "asset_id": {
      "type": "string",
      "description": "Defuse asset ID to reverse-resolve."
    },
    "account_id": {
      "type": "string",
      "description": "NEAR account ID or wallet address for balance and quote queries."
    },
    "token_ids": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Specific Defuse asset IDs to query for balances."
    },
    "from_token": {
      "type": "string",
      "description": "Source token symbol or Defuse asset ID for a dry quote."
    },
    "to_token": {
      "type": "string",
      "description": "Destination token symbol or Defuse asset ID for a dry quote."
    },
    "amount": {
      "type": "string",
      "description": "Human-readable amount for a dry quote."
    },
    "slippage_bps": {
      "type": "integer",
      "description": "Slippage tolerance in basis points. Defaults to 100."
    },
    "swap_type": {
      "type": "string",
      "description": "Swap type for the dry quote: EXACT_INPUT or EXACT_OUTPUT."
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
                asset_id: "nep141:wrap.near".into(),
                symbol: "wNEAR".into(),
                blockchain: "near".into(),
                decimals: 24,
                contract_address: Some("wrap.near".into()),
                price: Some(3.5),
            },
            TokenMetadata {
                asset_id: "nep141:usdc.near".into(),
                symbol: "USDC".into(),
                blockchain: "near".into(),
                decimals: 6,
                contract_address: Some("usdc.near".into()),
                price: Some(1.0),
            },
            TokenMetadata {
                asset_id: "nep141:usdc.arb".into(),
                symbol: "USDC".into(),
                blockchain: "arbitrum".into(),
                decimals: 6,
                contract_address: Some("0xa0b8".into()),
                price: Some(1.0),
            },
            TokenMetadata {
                asset_id: "nep141:eth.near".into(),
                symbol: "ETH".into(),
                blockchain: "eth".into(),
                decimals: 18,
                contract_address: None,
                price: Some(2500.0),
            },
            TokenMetadata {
                asset_id: "nep141:wbtc.near".into(),
                symbol: "WBTC".into(),
                blockchain: "eth".into(),
                decimals: 8,
                contract_address: None,
                price: Some(60000.0),
            },
        ]
    }

    #[test]
    fn alias_map_supports_chain_qualified_lookup() {
        let alias_map = build_alias_map(&sample_tokens());
        let hits = alias_map
            .alias_map
            .get("usdc on arbitrum")
            .expect("chain-qualified alias");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].asset_id, "nep141:usdc.arb");
    }

    #[test]
    fn alias_map_supports_curated_aliases() {
        let alias_map = build_alias_map(&sample_tokens());
        let hits = alias_map.alias_map.get("bitcoin").expect("bitcoin alias");
        assert!(hits.iter().any(|item| item.symbol == "WBTC"));
    }

    #[test]
    fn scan_prefers_longest_match() {
        let alias_map = build_alias_map(&sample_tokens());
        let hits = scan_for_token_hits("swap usdc on arbitrum to eth", &alias_map.alias_map);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|item| item.asset_id == "nep141:usdc.arb"));
        assert!(hits.iter().any(|item| item.asset_id == "nep141:eth.near"));
    }

    #[test]
    fn resolve_single_token_rejects_ambiguous_symbol() {
        let alias_map = build_alias_map(&sample_tokens());
        let error = resolve_single_token("USDC", &alias_map).expect_err("ambiguous symbol");
        assert!(error.contains("Ambiguous token reference"));
    }

    #[test]
    fn normalize_account_id_keeps_standard_values() {
        assert_eq!(normalize_account_id("alice.near"), "alice.near");
        assert_eq!(normalize_account_id("0xabc123"), "0xabc123");
    }

    #[test]
    fn normalize_account_id_recovers_hex_from_decimal() {
        let decimal = "1271270613000041655817448348132275889066893754095";
        assert_eq!(
            normalize_account_id(decimal),
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        );
    }

    #[test]
    fn parse_balance_respects_decimals() {
        let balance = parse_balance("1000000", 6).expect("balance");
        assert!((balance - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn human_to_raw_converts_fractional_amounts() {
        assert_eq!(human_to_raw("100.5", 6).expect("raw"), "100500000");
        assert_eq!(human_to_raw("0.000001", 6).expect("raw"), "1");
    }

    #[test]
    fn human_to_raw_rejects_extra_precision() {
        let error = human_to_raw("1.1234567", 6).expect_err("too many decimals");
        assert!(error.contains("more decimal places"));
    }

    #[test]
    fn base64_encode_matches_known_output() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[test]
    fn format_iso8601_uses_expected_utc_date() {
        assert_eq!(format_iso8601(1_704_067_200), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn action_deserialize_supports_protocol_metadata() {
        let json = r#"{"action":"get_protocol_metadata"}"#;
        let action: Action = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(action, Action::GetProtocolMetadata {}));
    }

    #[test]
    fn action_deserialize_supports_swap_quote() {
        let json = r#"{
            "action":"get_swap_quote",
            "from_token":"USDC",
            "to_token":"ETH",
            "amount":"100",
            "account_id":"alice.near"
        }"#;
        let action: Action = serde_json::from_str(json).expect("deserialize");
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
            _ => panic!("wrong variant"),
        }
    }
}
