//! NEAR 1Click Swap WASM Tool for IronClaw.
//!
//! Interact with the Defuse 1Click API to:
//! - List all supported tokens across blockchains
//! - Request cross-chain swap quotes (NEAR, Ethereum, Bitcoin, Solana, and more)
//! - Submit a deposit transaction hash to speed up swap processing
//! - Check the execution status of a swap by deposit address
//!
//! API base: https://1click.chaindefuser.com/v0
//! No authentication required for basic usage.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://1click.chaindefuser.com/v0";

struct NearOneClickTool;

impl exports::near::agent::tool::Guest for NearOneClickTool {
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
        "Interact with the NEAR 1Click Swap API. Supports four actions: \
         'get_tokens' lists all supported tokens across blockchains (NEAR, Ethereum, Bitcoin, \
         Solana, etc.); 'get_quote' requests a cross-chain swap quote with deposit address; \
         'submit_tx_hash' notifies the system of a completed deposit transaction to speed up \
         processing; 'get_status' checks the execution status of a swap using its deposit address."
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Input types (snake_case for LLM ergonomics)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    GetTokens,
    GetQuote(QuoteParams),
    SubmitTxHash(SubmitTxHashParams),
    GetStatus(StatusParams),
}

#[derive(Debug, Deserialize)]
struct QuoteParams {
    /// Simulate without generating a real deposit address.
    #[serde(default)]
    dry: bool,
    /// EXACT_INPUT | EXACT_OUTPUT | FLEX_INPUT | ANY_INPUT. Default: EXACT_INPUT.
    #[serde(default)]
    swap_type: Option<String>,
    /// Slippage tolerance in basis points (100 = 1%). Default: 100.
    #[serde(default)]
    slippage_tolerance: Option<u32>,
    /// Source asset ID (e.g. "nep141:wrap.near").
    origin_asset: String,
    /// ORIGIN_CHAIN | INTENTS. Default: ORIGIN_CHAIN.
    #[serde(default)]
    deposit_type: Option<String>,
    /// Target asset ID (e.g. "nep141:usdt.tether-token.near").
    destination_asset: String,
    /// Amount in smallest unit (e.g. "1000000000000000000000000" for 1 wNEAR).
    amount: String,
    /// Address to refund on failure. Default: same as recipient.
    #[serde(default)]
    refund_to: Option<String>,
    /// ORIGIN_CHAIN | INTENTS. Default: ORIGIN_CHAIN.
    #[serde(default)]
    refund_type: Option<String>,
    /// Destination recipient address. Required when dry=false.
    #[serde(default)]
    recipient: Option<String>,
    /// DESTINATION_CHAIN | INTENTS. Default: DESTINATION_CHAIN.
    #[serde(default)]
    recipient_type: Option<String>,
    /// When the deposit becomes inactive (ISO 8601). Default: 24h from now.
    #[serde(default)]
    deadline: Option<String>,
    /// Optional referral identifier.
    #[serde(default)]
    referral: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SubmitTxHashParams {
    /// Blockchain transaction hash of the deposit.
    tx_hash: String,
    /// Deposit address returned by get_quote.
    deposit_address: String,
    /// Sender account (required only for NEAR blockchain transactions).
    #[serde(default)]
    near_sender_account: Option<String>,
    /// Memo used when submitting the deposit, if any.
    #[serde(default)]
    memo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusParams {
    /// Deposit address returned by get_quote.
    deposit_address: String,
    /// Required only if the quote included a deposit_memo.
    #[serde(default)]
    deposit_memo: Option<String>,
}

// ---------------------------------------------------------------------------
// API body types (camelCase for the external API)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuoteApiBody {
    dry: bool,
    swap_type: String,
    slippage_tolerance: u32,
    origin_asset: String,
    deposit_type: String,
    destination_asset: String,
    amount: String,
    refund_to: String,
    refund_type: String,
    recipient: String,
    recipient_type: String,
    deadline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    referral: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitTxHashApiBody {
    tx_hash: String,
    deposit_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    near_sender_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memo: Option<String>,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn execute_inner(params: &str) -> Result<String, String> {
    let action: Action =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    match action {
        Action::GetTokens => get_tokens(),
        Action::GetQuote(req) => get_quote(req),
        Action::SubmitTxHash(req) => submit_tx_hash(req),
        Action::GetStatus(req) => get_status(req),
    }
}

// ---------------------------------------------------------------------------
// Action implementations
// ---------------------------------------------------------------------------

fn get_tokens() -> Result<String, String> {
    let url = format!("{API_BASE}/tokens");
    let headers = json_headers();

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        "Fetching supported tokens from 1Click API",
    );

    let resp = near::agent::host::http_request("GET", &url, &headers, None, None)
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("API error (HTTP {}): {}", resp.status, body));
    }

    let body =
        String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    // Wrap in an object so the LLM gets structured output.
    let tokens: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let count = tokens.as_array().map(|a| a.len()).unwrap_or(0);
    let output = serde_json::json!({
        "token_count": count,
        "tokens": tokens,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_quote(params: QuoteParams) -> Result<String, String> {
    if params.amount.is_empty() {
        return Err("'amount' must not be empty".into());
    }
    if params.origin_asset.is_empty() {
        return Err("'origin_asset' must not be empty".into());
    }
    if params.destination_asset.is_empty() {
        return Err("'destination_asset' must not be empty".into());
    }
    if !params.dry && params.recipient.is_none() {
        return Err("'recipient' is required when dry=false".into());
    }

    // Apply defaults for optional fields.
    let swap_type = params.swap_type.unwrap_or_else(|| "EXACT_INPUT".to_string());
    let slippage_tolerance = params.slippage_tolerance.unwrap_or(100);
    let deposit_type = params.deposit_type.unwrap_or_else(|| "ORIGIN_CHAIN".to_string());
    let recipient_type = params.recipient_type.unwrap_or_else(|| "DESTINATION_CHAIN".to_string());
    let deadline = params.deadline.unwrap_or_else(|| "2099-01-01T00:00:00Z".to_string());
    let recipient = params.recipient.unwrap_or_else(|| "dry.near".to_string());
    // When refund_to/refund_type are not provided, default to INTENTS so that
    // refund_to is always a NEAR address — avoids cross-chain address format errors
    // (e.g. ETH origin assets require an 0x address with ORIGIN_CHAIN).
    let (refund_type, refund_to) = match (params.refund_type, params.refund_to) {
        (Some(rt), Some(ra)) => (rt, ra),
        (Some(rt), None) => (rt, recipient.clone()),
        (None, Some(ra)) => ("ORIGIN_CHAIN".to_string(), ra),
        (None, None) => ("INTENTS".to_string(), recipient.clone()),
    };

    let body = QuoteApiBody {
        dry: params.dry,
        swap_type,
        slippage_tolerance,
        origin_asset: params.origin_asset,
        deposit_type,
        destination_asset: params.destination_asset,
        amount: params.amount,
        refund_to,
        refund_type,
        recipient,
        recipient_type,
        deadline,
        referral: params.referral,
    };

    let body_str =
        serde_json::to_string(&body).map_err(|e| format!("Failed to serialize request: {e}"))?;

    let url = format!("{API_BASE}/quote");
    let headers = json_headers();


    let resp =
        near::agent::host::http_request("POST", &url, &headers, Some(body_str.as_bytes()), None)
            .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status == 400 {
        let err_body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Bad request: {err_body}"));
    }
    if resp.status < 200 || resp.status >= 300 {
        let err_body = String::from_utf8_lossy(&resp.body);
        return Err(format!("API error (HTTP {}): {}", resp.status, err_body));
    }

    String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))
}

fn submit_tx_hash(params: SubmitTxHashParams) -> Result<String, String> {
    if params.tx_hash.is_empty() {
        return Err("'tx_hash' must not be empty".into());
    }
    if params.deposit_address.is_empty() {
        return Err("'deposit_address' must not be empty".into());
    }

    let body = SubmitTxHashApiBody {
        tx_hash: params.tx_hash.clone(),
        deposit_address: params.deposit_address.clone(),
        near_sender_account: params.near_sender_account,
        memo: params.memo,
    };

    let body_str =
        serde_json::to_string(&body).map_err(|e| format!("Failed to serialize request: {e}"))?;

    let url = format!("{API_BASE}/deposit/submit");
    let headers = json_headers();

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!(
            "Submitting deposit tx hash {} for address {}",
            params.tx_hash, params.deposit_address
        ),
    );

    let resp =
        near::agent::host::http_request("POST", &url, &headers, Some(body_str.as_bytes()), None)
            .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status == 400 {
        let err_body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Bad request: {err_body}"));
    }
    if resp.status == 404 {
        return Err(format!(
            "Deposit address '{}' not found",
            params.deposit_address
        ));
    }
    if resp.status < 200 || resp.status >= 300 {
        let err_body = String::from_utf8_lossy(&resp.body);
        return Err(format!("API error (HTTP {}): {}", resp.status, err_body));
    }

    String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))
}

fn get_status(params: StatusParams) -> Result<String, String> {
    if params.deposit_address.is_empty() {
        return Err("'deposit_address' must not be empty".into());
    }

    let mut url = format!(
        "{API_BASE}/status?depositAddress={}",
        url_encode(&params.deposit_address)
    );
    if let Some(ref memo) = params.deposit_memo {
        if !memo.is_empty() {
            url.push_str(&format!("&depositMemo={}", url_encode(memo)));
        }
    }

    let headers = json_headers();

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Checking swap status for deposit: {}", params.deposit_address),
    );

    let resp = near::agent::host::http_request("GET", &url, &headers, None, None)
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status == 404 {
        return Err(format!(
            "Swap not found for deposit address '{}'",
            params.deposit_address
        ));
    }
    if resp.status < 200 || resp.status >= 300 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("API error (HTTP {}): {}", resp.status, body));
    }

    String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn json_headers() -> String {
    serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-NearOneClick-Tool/0.1"
    })
    .to_string()
}

/// Percent-encode a string for safe use in URL query parameters.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// JSON Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "enum": ["get_tokens", "get_quote", "submit_tx_hash", "get_status"],
            "description": "Operation to perform: 'get_tokens' lists supported tokens, 'get_quote' requests a swap quote, 'submit_tx_hash' notifies the system of a deposit TX to speed up processing, 'get_status' checks swap execution status"
        },
        "dry": {
            "type": "boolean",
            "description": "[get_quote] If true, simulates the swap without generating a real deposit address. Only origin_asset, destination_asset and amount are required in dry mode."
        },
        "origin_asset": {
            "type": "string",
            "description": "[get_quote] REQUIRED. Source asset ID (e.g. 'nep141:wrap.near')"
        },
        "destination_asset": {
            "type": "string",
            "description": "[get_quote] REQUIRED. Target asset ID (e.g. 'nep141:usdt.tether-token.near')"
        },
        "amount": {
            "type": "string",
            "description": "[get_quote] REQUIRED. Amount in the token's smallest unit (e.g. '1000000' for 1 USDC with 6 decimals, '1000000000000000000000000' for 1 wNEAR with 24 decimals)"
        },
        "recipient": {
            "type": "string",
            "description": "[get_quote] Destination recipient address. REQUIRED when dry=false. Omit for dry runs."
        },
        "swap_type": {
            "type": "string",
            "enum": ["EXACT_INPUT", "EXACT_OUTPUT", "FLEX_INPUT", "ANY_INPUT"],
            "description": "[get_quote] Optional. Default: EXACT_INPUT"
        },
        "slippage_tolerance": {
            "type": "integer",
            "description": "[get_quote] Optional. Slippage in basis points. Default: 100 (1%)",
            "minimum": 0
        },
        "deposit_type": {
            "type": "string",
            "enum": ["ORIGIN_CHAIN", "INTENTS"],
            "description": "[get_quote] Optional. Default: ORIGIN_CHAIN"
        },
        "refund_to": {
            "type": "string",
            "description": "[get_quote] Optional. Address to refund on failure. Default: same as recipient"
        },
        "refund_type": {
            "type": "string",
            "enum": ["ORIGIN_CHAIN", "INTENTS"],
            "description": "[get_quote] Optional. Default: ORIGIN_CHAIN"
        },
        "recipient_type": {
            "type": "string",
            "enum": ["DESTINATION_CHAIN", "INTENTS"],
            "description": "[get_quote] Optional. Default: DESTINATION_CHAIN"
        },
        "deadline": {
            "type": "string",
            "description": "[get_quote] Optional. ISO 8601 expiry datetime. Default: 2099-01-01T00:00:00Z"
        },
        "referral": {
            "type": "string",
            "description": "[get_quote] Optional referral identifier"
        },
        "tx_hash": {
            "type": "string",
            "description": "[submit_tx_hash] Blockchain transaction hash of the deposit"
        },
        "near_sender_account": {
            "type": "string",
            "description": "[submit_tx_hash] Sender account, required only for NEAR blockchain transactions"
        },
        "memo": {
            "type": "string",
            "description": "[submit_tx_hash] Memo used when submitting the deposit, if any"
        },
        "deposit_address": {
            "type": "string",
            "description": "[submit_tx_hash, get_status] Deposit address returned by get_quote"
        },
        "deposit_memo": {
            "type": "string",
            "description": "[get_status] Deposit memo if one was included in the quote response"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(NearOneClickTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("addr:abc123"), "addr%3Aabc123");
        assert_eq!(url_encode("simple"), "simple");
        assert_eq!(url_encode("nep141:wrap.near"), "nep141%3Awrap.near");
    }

    #[test]
    fn test_parse_get_tokens() {
        let params = r#"{"action": "get_tokens"}"#;
        let action: Action = serde_json::from_str(params).unwrap();
        assert!(matches!(action, Action::GetTokens));
    }

    #[test]
    fn test_parse_get_status() {
        let params = r#"{"action": "get_status", "deposit_address": "0xabc123"}"#;
        let action: Action = serde_json::from_str(params).unwrap();
        match action {
            Action::GetStatus(p) => {
                assert_eq!(p.deposit_address, "0xabc123");
                assert!(p.deposit_memo.is_none());
            }
            _ => panic!("Expected GetStatus"),
        }
    }

    #[test]
    fn test_parse_get_status_with_memo() {
        let params =
            r#"{"action": "get_status", "deposit_address": "0xabc", "deposit_memo": "memo123"}"#;
        let action: Action = serde_json::from_str(params).unwrap();
        match action {
            Action::GetStatus(p) => {
                assert_eq!(p.deposit_memo, Some("memo123".to_string()));
            }
            _ => panic!("Expected GetStatus"),
        }
    }

    #[test]
    fn test_parse_submit_tx_hash() {
        let params = r#"{"action": "submit_tx_hash", "tx_hash": "0xdeadbeef", "deposit_address": "0xabc123"}"#;
        let action: Action = serde_json::from_str(params).unwrap();
        match action {
            Action::SubmitTxHash(p) => {
                assert_eq!(p.tx_hash, "0xdeadbeef");
                assert_eq!(p.deposit_address, "0xabc123");
                assert!(p.near_sender_account.is_none());
                assert!(p.memo.is_none());
            }
            _ => panic!("Expected SubmitTxHash"),
        }
    }

    #[test]
    fn test_parse_submit_tx_hash_with_optional_fields() {
        let params = r#"{"action": "submit_tx_hash", "tx_hash": "0xdeadbeef", "deposit_address": "0xabc123", "near_sender_account": "alice.near", "memo": "my-memo"}"#;
        let action: Action = serde_json::from_str(params).unwrap();
        match action {
            Action::SubmitTxHash(p) => {
                assert_eq!(p.near_sender_account, Some("alice.near".to_string()));
                assert_eq!(p.memo, Some("my-memo".to_string()));
            }
            _ => panic!("Expected SubmitTxHash"),
        }
    }

    #[test]
    fn test_submit_tx_hash_body_serializes_camel_case() {
        let body = SubmitTxHashApiBody {
            tx_hash: "0xdeadbeef".to_string(),
            deposit_address: "0xabc123".to_string(),
            near_sender_account: Some("alice.near".to_string()),
            memo: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"txHash\""));
        assert!(json.contains("\"depositAddress\""));
        assert!(json.contains("\"nearSenderAccount\""));
        assert!(!json.contains("\"memo\""));
    }

    #[test]
    fn test_quote_api_body_serializes_camel_case() {
        let body = QuoteApiBody {
            dry: true,
            swap_type: "EXACT_INPUT".to_string(),
            slippage_tolerance: 100,
            origin_asset: "nep141:wrap.near".to_string(),
            deposit_type: "ORIGIN_CHAIN".to_string(),
            destination_asset: "nep141:usdt.tether-token.near".to_string(),
            amount: "1000000000000000000000000".to_string(),
            refund_to: "alice.near".to_string(),
            refund_type: "ORIGIN_CHAIN".to_string(),
            recipient: "alice.near".to_string(),
            recipient_type: "DESTINATION_CHAIN".to_string(),
            deadline: "2026-03-19T00:00:00Z".to_string(),
            referral: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"swapType\""));
        assert!(json.contains("\"originAsset\""));
        assert!(json.contains("\"depositType\""));
        assert!(json.contains("\"destinationAsset\""));
        assert!(json.contains("\"slippageTolerance\""));
        assert!(json.contains("\"refundTo\""));
        assert!(json.contains("\"refundType\""));
        assert!(json.contains("\"recipientType\""));
        // referral should be omitted when None
        assert!(!json.contains("\"referral\""));
    }

    #[test]
    fn test_invalid_action() {
        let params = r#"{"action": "invalid_action"}"#;
        assert!(serde_json::from_str::<Action>(params).is_err());
    }
}
