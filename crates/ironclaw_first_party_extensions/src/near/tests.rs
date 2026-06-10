//! Unit + dispatch tests for the NEAR extension.

use super::*;
use ironclaw_host_api::{InvocationId, RuntimeHttpEgressResponse, UserId};
use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

fn scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new()).unwrap()
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn request<'a>(
    capability_id: &'a CapabilityId,
    scope: &'a ResourceScope,
    input: &'a Value,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
) -> NearDispatchRequest<'a> {
    NearDispatchRequest {
        capability_id,
        scope,
        input,
        runtime_http_egress,
    }
}

struct RecordingEgress {
    responses: StdMutex<VecDeque<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
    requests: StdMutex<Vec<Value>>,
}

impl RecordingEgress {
    fn ok_json(body: Value) -> RuntimeHttpEgressResponse {
        let bytes = serde_json::to_vec(&body).unwrap();
        RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: bytes,
            saved_body: None,
            request_bytes: 10,
            response_bytes: 20,
            redaction_applied: false,
        }
    }

    fn single(body: Value) -> Self {
        Self::queued(vec![body])
    }

    /// Queue several successful JSON responses, popped in call order.
    fn queued(bodies: Vec<Value>) -> Self {
        Self {
            responses: StdMutex::new(bodies.into_iter().map(|b| Ok(Self::ok_json(b))).collect()),
            requests: StdMutex::new(Vec::new()),
        }
    }

    /// Queue a single egress-level failure (e.g. a network error).
    fn erroring(error: RuntimeHttpEgressError) -> Self {
        Self {
            responses: StdMutex::new([Err(error)].into_iter().collect()),
            requests: StdMutex::new(Vec::new()),
        }
    }

    /// The JSON request bodies seen by `execute`, in call order.
    fn recorded_requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        self.requests.lock().unwrap().push(body);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("RecordingEgress: no more responses queued")
    }
}

/// Build a `call_function` RPC body whose `result.result` byte array decodes
/// to the given JSON string.
fn call_function_body(json_str: &str) -> Value {
    let bytes: Vec<u8> = json_str.as_bytes().to_vec();
    json!({
        "jsonrpc": "2.0",
        "id": "1",
        "result": {
            "result": bytes,
            "logs": [],
            "block_height": 12_345_678_u64,
            "block_hash": "abc",
        }
    })
}

// ----- pure-function decode tests -----

#[test]
fn encode_args_empty_for_none_and_null() {
    assert_eq!(encode_args(None).unwrap(), "");
    assert_eq!(encode_args(Some(&Value::Null)).unwrap(), "");
}

#[test]
fn encode_args_matches_known_base64() {
    // {"account_id":"alice.near"} → eyJhY2NvdW50X2lkIjoiYWxpY2UubmVhciJ9
    let args = json!({ "account_id": "alice.near" });
    assert_eq!(
        encode_args(Some(&args)).unwrap(),
        "eyJhY2NvdW50X2lkIjoiYWxpY2UubmVhciJ9"
    );
}

#[test]
fn decode_view_result_parses_object_body() {
    let body = call_function_body(r#"{"name":"Wrapped NEAR","decimals":24}"#);
    let parsed = decode_view_result(&body, 0).unwrap();
    assert_eq!(parsed["name"], "Wrapped NEAR");
    assert_eq!(parsed["decimals"], 24);
}

#[test]
fn decode_view_result_parses_ft_balance_quoted_string() {
    // ft_balance_of returns a quoted integer string.
    let body = call_function_body(r#""1000000""#);
    let parsed = decode_view_result(&body, 0).unwrap();
    assert_eq!(parsed, Value::String("1000000".to_string()));
}

#[test]
fn decode_view_result_maps_rpc_error_to_operation_failed() {
    let body = json!({"jsonrpc":"2.0","id":"1","error":{"name":"UNKNOWN_ACCOUNT"}});
    let err = decode_view_result(&body, 42).unwrap_err();
    assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    assert_eq!(err.usage().unwrap().network_egress_bytes, 42);
}

#[test]
fn decode_view_result_rejects_non_byte_array() {
    let body = json!({"result":{"result":"not-an-array"}});
    let err = decode_view_result(&body, 0).unwrap_err();
    assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputDecode);
}

// ----- async end-to-end tests -----

#[tokio::test]
async fn dispatch_returns_undeclared_capability_for_unknown_id() {
    let executor = NearExecutor::default();
    let capability = capability_id("near.unknown");
    let scope = scope();
    let input = json!({});

    let error = executor
        .dispatch(request(&capability, &scope, &input, None))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::UndeclaredCapability);
}

#[tokio::test]
async fn account_returns_network_denied_when_egress_is_none() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
    let scope = scope();
    let input = json!({"account_id":"alice.near"});

    let error = executor
        .dispatch(request(&capability, &scope, &input, None))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
}

#[tokio::test]
async fn account_rejects_missing_account_id() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
    let scope = scope();
    let input = json!({});
    let egress = Arc::new(RecordingEgress::single(json!({})));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
}

#[tokio::test]
async fn account_happy_path_returns_balance_fields() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
    let scope = scope();
    let input = json!({"account_id":"alice.near"});
    let rpc_body = json!({
        "jsonrpc": "2.0",
        "id": "1",
        "result": {
            "amount": "100000000000000000000000000",
            "locked": "0",
            "code_hash": "11111111111111111111111111111111",
            "storage_usage": 182,
            "block_height": 12_345_678_u64,
            "block_hash": "xyz",
        }
    });
    let egress = Arc::new(RecordingEgress::single(rpc_body));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap();

    assert_eq!(result.output["amount"], "100000000000000000000000000");
    assert_eq!(result.output["storage_usage"], 182);
    assert_eq!(result.output["block_height"], 12_345_678_u64);
    assert!(result.usage.network_egress_bytes > 0);
}

#[tokio::test]
async fn account_maps_rpc_error_to_operation_failed() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
    let scope = scope();
    let input = json!({"account_id":"ghost.near"});
    let rpc_body = json!({
        "jsonrpc": "2.0",
        "id": "1",
        "error": {"name": "UNKNOWN_ACCOUNT", "cause": {"name": "UNKNOWN_ACCOUNT"}}
    });
    let egress = Arc::new(RecordingEgress::single(rpc_body));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    assert!(error.usage().unwrap().network_egress_bytes > 0);
}

#[tokio::test]
async fn view_happy_path_decodes_contract_result() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_VIEW_CAPABILITY_ID);
    let scope = scope();
    let input = json!({
        "account_id": "token.v2.ref-finance.near",
        "method_name": "ft_metadata",
    });
    let egress = Arc::new(RecordingEgress::single(call_function_body(
        r#"{"spec":"ft-1.0.0","name":"Ref","symbol":"REF","decimals":18}"#,
    )));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap();

    assert_eq!(result.output["result"]["symbol"], "REF");
    assert_eq!(result.output["result"]["decimals"], 18);
    assert_eq!(result.output["block_height"], 12_345_678_u64);
    assert!(result.usage.network_egress_bytes > 0);
}

#[tokio::test]
async fn view_rejects_non_object_args() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_VIEW_CAPABILITY_ID);
    let scope = scope();
    let input = json!({
        "account_id": "x.near",
        "method_name": "foo",
        "args": "not-an-object",
    });
    let egress = Arc::new(RecordingEgress::single(json!({})));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
}

#[tokio::test]
async fn ft_balances_decodes_quoted_balances() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
    let scope = scope();
    let input = json!({
        "account_id": "alice.near",
        "token_contracts": ["usdt.tether-token.near"],
    });
    let egress = Arc::new(RecordingEgress::single(call_function_body(r#""1000000""#)));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap();

    assert_eq!(
        result.output["balances"][0]["contract"],
        "usdt.tether-token.near"
    );
    assert_eq!(result.output["balances"][0]["raw"], "1000000");
}

#[test]
fn decode_view_result_empty_bytes_returns_null() {
    // A view method that returns nothing yields an empty byte array; we
    // surface JSON null rather than failing to parse an empty slice.
    let body = json!({ "result": { "result": [] } });
    assert_eq!(decode_view_result(&body, 0).unwrap(), Value::Null);
}

#[test]
fn decode_view_result_missing_result_pointer_is_output_decode() {
    let body = json!({ "result": { "logs": [] } });
    let err = decode_view_result(&body, 7).unwrap_err();
    assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputDecode);
    assert_eq!(err.usage().unwrap().network_egress_bytes, 7);
}

#[tokio::test]
async fn ft_balances_rejects_empty_contract_list() {
    // A balance query over zero contracts is meaningless; required_string_array
    // rejects it before any RPC round-trip happens.
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
    let scope = scope();
    let input = json!({ "account_id": "alice.near", "token_contracts": [] });
    let egress = Arc::new(RecordingEgress::queued(vec![]));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    assert!(egress.recorded_requests().is_empty());
}

#[tokio::test]
async fn ft_balances_preserves_contract_order_when_concurrent() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
    let scope = scope();
    let input = json!({
        "account_id": "alice.near",
        "token_contracts": ["a.near", "b.near"],
    });
    let egress = Arc::new(RecordingEgress::queued(vec![
        call_function_body(r#""111""#),
        call_function_body(r#""222""#),
    ]));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap();

    // try_join_all preserves input order regardless of completion order.
    let balances = result.output["balances"].as_array().unwrap();
    assert_eq!(balances.len(), 2);
    assert_eq!(balances[0]["contract"], "a.near");
    assert_eq!(balances[1]["contract"], "b.near");
}

#[tokio::test]
async fn nfts_happy_path_returns_tokens() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_NFTS_CAPABILITY_ID);
    let scope = scope();
    let input = json!({ "account_id": "alice.near", "nft_contract": "nft.near" });
    let egress = Arc::new(RecordingEgress::single(call_function_body(
        r#"[{"token_id":"1","owner_id":"alice.near"}]"#,
    )));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap();

    assert_eq!(result.output["tokens"][0]["token_id"], "1");
    assert!(result.usage.network_egress_bytes > 0);
}

#[tokio::test]
async fn nfts_rejects_overlong_from_index() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_NFTS_CAPABILITY_ID);
    let scope = scope();
    let from_index = "1".repeat(MAX_FROM_INDEX_CHARS + 1);
    let input = json!({
        "account_id": "alice.near",
        "nft_contract": "nft.near",
        "from_index": from_index,
    });
    let egress = Arc::new(RecordingEgress::single(json!({})));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
}

#[tokio::test]
async fn tx_status_happy_path_returns_transaction() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_TX_STATUS_CAPABILITY_ID);
    let scope = scope();
    let input = json!({
        "tx_hash": "11111111111111111111111111111111",
        "sender_account_id": "alice.near",
    });
    let rpc_body = json!({
        "jsonrpc": "2.0",
        "id": "1",
        "result": {
            "status": { "SuccessValue": "" },
            "transaction": { "hash": "abc", "signer_id": "alice.near" },
            "receipts_outcome": [],
        }
    });
    let egress = Arc::new(RecordingEgress::single(rpc_body));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap();

    assert_eq!(result.output["transaction"]["signer_id"], "alice.near");
    assert!(result.usage.network_egress_bytes > 0);
}

#[tokio::test]
async fn tx_status_maps_rpc_error_to_operation_failed() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_TX_STATUS_CAPABILITY_ID);
    let scope = scope();
    let input = json!({ "tx_hash": "deadbeef", "sender_account_id": "alice.near" });
    let rpc_body = json!({"jsonrpc":"2.0","id":"1","error":{"name":"UNKNOWN_TRANSACTION"}});
    let egress = Arc::new(RecordingEgress::single(rpc_body));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
}

fn intents_input() -> Value {
    json!({
        "origin_asset": "nep141:wrap.near",
        "destination_asset": "nep141:usdt.tether-token.near",
        "amount": "1000000",
        "recipient": "0xabc",
        "refund_to": "alice.near",
    })
}

fn intents_quote_body() -> Value {
    json!({
        "quote": {
            "amountOut": "990000",
            "depositAddress": "intents.near",
            "fee": "1000",
            "deadline": "2026-01-01T00:00:00Z",
        }
    })
}

#[tokio::test]
async fn intents_quote_forces_dry_and_returns_fields() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
    let scope = scope();
    let input = intents_input();
    let egress = Arc::new(RecordingEgress::single(intents_quote_body()));

    let result = executor
        .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
        .await
        .unwrap();

    assert_eq!(result.output["amount_out"], "990000");
    assert_eq!(result.output["deposit_address"], "intents.near");
    // `dry: true` must always be sent — this is a read-only quote capability.
    let sent = egress.recorded_requests();
    assert_eq!(sent[0]["dry"], true);
    assert_eq!(sent[0]["swapType"], "EXACT_INPUT");
}

#[tokio::test]
async fn intents_quote_rejects_unknown_swap_type() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
    let scope = scope();
    let mut input = intents_input();
    input["swap_type"] = json!("LIMIT_ORDER");
    let egress = Arc::new(RecordingEgress::single(json!({})));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
}

#[tokio::test]
async fn intents_quote_clamps_slippage_to_ceiling() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
    let scope = scope();
    let mut input = intents_input();
    input["slippage_tolerance"] = json!(999_999);
    let egress = Arc::new(RecordingEgress::single(intents_quote_body()));

    executor
        .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
        .await
        .unwrap();

    let sent = egress.recorded_requests();
    assert_eq!(sent[0]["slippageTolerance"], MAX_SLIPPAGE_TOLERANCE);
}

#[tokio::test]
async fn intents_quote_missing_quote_key_is_operation_failed() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
    let scope = scope();
    let input = intents_input();
    // An error envelope with no `quote` must surface a failure, not null fields.
    let egress = Arc::new(RecordingEgress::single(json!({ "error": "no route" })));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
}

#[test]
fn near_account_id_charset_validation() {
    // Valid: lowercase alphanumeric with non-leading/trailing/repeating
    // separators, and a 64-char implicit account.
    assert!(is_valid_near_account_id("alice.near"));
    assert!(is_valid_near_account_id("a.near"));
    assert!(is_valid_near_account_id("token.v2.ref-finance.near"));
    assert!(is_valid_near_account_id(&"a".repeat(MAX_ACCOUNT_ID_CHARS)));
    // Invalid: uppercase, illegal char, leading/trailing/double separator,
    // and out-of-range lengths.
    assert!(!is_valid_near_account_id("Alice.near"));
    assert!(!is_valid_near_account_id("alice!.near"));
    assert!(!is_valid_near_account_id(".alice.near"));
    assert!(!is_valid_near_account_id("alice.near."));
    assert!(!is_valid_near_account_id("alice..near"));
    assert!(!is_valid_near_account_id("a"));
    assert!(!is_valid_near_account_id(
        &"a".repeat(MAX_ACCOUNT_ID_CHARS + 1)
    ));
}

#[tokio::test]
async fn account_rejects_malformed_account_id() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
    let scope = scope();
    let input = json!({ "account_id": "Alice.NEAR" });
    let egress = Arc::new(RecordingEgress::queued(vec![]));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    // Rejected at the boundary, before any RPC round-trip.
    assert!(egress.recorded_requests().is_empty());
}

#[tokio::test]
async fn ft_balances_aborts_on_partial_failure() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
    let scope = scope();
    let input = json!({
        "account_id": "alice.near",
        "token_contracts": ["a.near", "b.near"],
    });
    // One contract succeeds, the other returns an RPC error; try_join_all
    // short-circuits and the whole call fails.
    let egress = Arc::new(RecordingEgress::queued(vec![
        call_function_body(r#""111""#),
        json!({"jsonrpc":"2.0","id":"1","error":{"name":"UNKNOWN_ACCOUNT"}}),
    ]));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
}

#[tokio::test]
async fn intents_quote_maps_egress_error_to_network_denied() {
    let executor = NearExecutor::default();
    let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
    let scope = scope();
    let input = intents_input();
    // The egress itself fails (e.g. a network error); it must surface as a
    // mapped dispatch error rather than a panic or a decoded body.
    let egress = Arc::new(RecordingEgress::erroring(RuntimeHttpEgressError::Network {
        reason: "connection reset".to_string(),
        request_bytes: 10,
        response_bytes: 0,
    }));

    let error = executor
        .dispatch(request(&capability, &scope, &input, Some(egress)))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
}
