//! Read-only NEAR mainnet first-party extension for IronClaw.
//!
//! Mirrors the `web_access` extension structure: a plain `NearExecutor`
//! struct with an async `dispatch` method that routes on `capability_id`,
//! adapted into the host runtime by a shim in `ironclaw_reborn_composition`.
//!
//! All NEAR queries go through the host's `Arc<dyn RuntimeHttpEgress>`
//! (never `reqwest` directly), so policy enforcement, byte accounting, and
//! private-IP denial all apply. RPC target is FastNEAR mainnet; the
//! `near.intents_quote` capability targets the 1Click solver API.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::FutureExt as _;
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};

mod input;
mod rpc;

use input::*;
use rpc::*;

pub const NEAR_EXTENSION_ID: &str = "near";
pub const NEAR_ACCOUNT_CAPABILITY_ID: &str = "near.account";
pub const NEAR_VIEW_CAPABILITY_ID: &str = "near.view";
pub const NEAR_FT_BALANCES_CAPABILITY_ID: &str = "near.ft_balances";
pub const NEAR_NFTS_CAPABILITY_ID: &str = "near.nfts";
pub const NEAR_TX_STATUS_CAPABILITY_ID: &str = "near.tx_status";
pub const NEAR_INTENTS_QUOTE_CAPABILITY_ID: &str = "near.intents_quote";

const FASTNEAR_RPC_URL: &str = "https://rpc.mainnet.fastnear.com/";
pub const FASTNEAR_RPC_HOST: &str = "rpc.mainnet.fastnear.com";
const INTENTS_QUOTE_URL: &str = "https://1click.chaindefuser.com/v0/quote";
pub const INTENTS_HOST: &str = "1click.chaindefuser.com";

pub const NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;
const RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u32 = 30_000;

const MAX_ACCOUNT_ID_CHARS: usize = 64;
const MAX_METHOD_NAME_CHARS: usize = 128;
const MAX_FT_CONTRACTS: usize = 20;
const MAX_TX_HASH_CHARS: usize = 128;
const MAX_FROM_INDEX_CHARS: usize = 64;
const DEFAULT_NFT_LIMIT: u64 = 50;
const MAX_NFT_LIMIT: u64 = 100;
/// 1Click `slippageTolerance` is expressed in basis points (100 = 1%). The
/// schema and API cap this at 10000 bp (100%).
const DEFAULT_SLIPPAGE_TOLERANCE: u64 = 100;
const MAX_SLIPPAGE_TOLERANCE: u64 = 10_000;
/// Allowed `swapType` values per the 1Click `/v0/quote` API.
const ALLOWED_SWAP_TYPES: [&str; 2] = ["EXACT_INPUT", "EXACT_OUTPUT"];

#[derive(Debug, Default)]
pub struct NearExecutor {}

pub struct NearDispatchRequest<'a> {
    pub capability_id: &'a CapabilityId,
    pub scope: &'a ResourceScope,
    pub input: &'a Value,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearDispatchResult {
    pub output: Value,
    pub usage: ResourceUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("near dispatch failed: {kind}")]
pub struct NearDispatchError {
    kind: RuntimeDispatchErrorKind,
    usage: Option<ResourceUsage>,
}

impl NearDispatchError {
    pub(crate) fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    pub(crate) fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

impl NearExecutor {
    pub async fn dispatch(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        match request.capability_id.as_str() {
            NEAR_ACCOUNT_CAPABILITY_ID => self.account(request).await,
            NEAR_VIEW_CAPABILITY_ID => self.view(request).await,
            NEAR_FT_BALANCES_CAPABILITY_ID => self.ft_balances(request).await,
            NEAR_NFTS_CAPABILITY_ID => self.nfts(request).await,
            NEAR_TX_STATUS_CAPABILITY_ID => self.tx_status(request).await,
            NEAR_INTENTS_QUOTE_CAPABILITY_ID => self.intents_quote(request).await,
            _ => Err(NearDispatchError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )),
        }
    }

    async fn account(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;

        let rpc = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "query",
            "params": {
                "request_type": "view_account",
                "finality": "final",
                "account_id": account_id,
            }
        });
        let (body, egress_bytes) = http_post_json(
            &request,
            egress,
            near_rpc_network_policy(),
            FASTNEAR_RPC_URL,
            &rpc,
        )
        .await?;
        let result = rpc_result(&body, egress_bytes)?;
        let output = json!({
            "amount": result["amount"],
            "locked": result["locked"],
            "code_hash": result["code_hash"],
            "storage_usage": result["storage_usage"],
            "block_height": result["block_height"],
        });
        Ok(success(output, egress_bytes))
    }

    async fn view(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;
        let method_name = required_method_name(request.input, "method_name")?;
        let args = optional_object(request.input, "args")?;

        let body =
            call_function(&request, egress, &account_id, &method_name, args.as_ref()).await?;
        let egress_bytes = body.egress_bytes;
        let parsed = decode_view_result(&body.value, egress_bytes)?;
        let output = json!({
            "result": parsed,
            "block_height": body.value["result"]["block_height"],
        });
        Ok(success(output, egress_bytes))
    }

    async fn ft_balances(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;
        let token_contracts = required_string_array(
            request.input,
            "token_contracts",
            MAX_FT_CONTRACTS,
            MAX_ACCOUNT_ID_CHARS,
        )?;

        let args = json!({ "account_id": account_id });
        let request_ref = &request;
        let args_ref = &args;
        // Fan the per-contract balance reads out concurrently; a serial loop
        // would pay one RPC round-trip per token. The first failure aborts the
        // batch (try_join_all short-circuits), matching the previous behavior.
        let lookups = token_contracts.into_iter().map(|contract| {
            let egress = Arc::clone(&egress);
            async move {
                let body = call_function(
                    request_ref,
                    egress,
                    &contract,
                    "ft_balance_of",
                    Some(args_ref),
                )
                .await?;
                let parsed = decode_view_result(&body.value, body.egress_bytes)?;
                // NEP-141 ft_balance_of returns a quoted integer string.
                Ok::<_, NearDispatchError>((contract, parsed, body.egress_bytes))
            }
        });
        let results = futures_util::future::try_join_all(lookups).await?;

        let mut total_egress_bytes = 0_u64;
        let mut balances = Vec::with_capacity(results.len());
        for (contract, parsed, egress_bytes) in results {
            total_egress_bytes = total_egress_bytes.saturating_add(egress_bytes);
            balances.push(json!({
                "contract": contract,
                "raw": parsed,
            }));
        }

        let output = json!({ "balances": balances });
        Ok(success(output, total_egress_bytes))
    }

    async fn nfts(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;
        let nft_contract = required_account_id(request.input, "nft_contract")?;
        let from_index = match optional_string(request.input, "from_index")? {
            Some(value) => {
                if value.chars().count() > MAX_FROM_INDEX_CHARS {
                    return Err(input_error());
                }
                value
            }
            None => "0".to_string(),
        };
        let limit = optional_u64(request.input, "limit")?
            .unwrap_or(DEFAULT_NFT_LIMIT)
            .clamp(1, MAX_NFT_LIMIT);

        let args = json!({
            "account_id": account_id,
            "from_index": from_index,
            "limit": limit,
        });
        let body = call_function(
            &request,
            egress,
            &nft_contract,
            "nft_tokens_for_owner",
            Some(&args),
        )
        .await?;
        let egress_bytes = body.egress_bytes;
        let parsed = decode_view_result(&body.value, egress_bytes)?;
        let output = json!({ "tokens": parsed });
        Ok(success(output, egress_bytes))
    }

    /// Read a transaction's status and receipt tree.
    ///
    /// TODO(near): `EXPERIMENTAL_tx_status` is an unstable NEAR JSON-RPC method
    /// and may be renamed or removed upstream; revisit when a stable equivalent
    /// covers the receipt tree. Behavior is covered by the `tx_status_*` tests.
    async fn tx_status(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let tx_hash = required_bounded(request.input, "tx_hash", MAX_TX_HASH_CHARS)?;
        let sender_account_id = required_account_id(request.input, "sender_account_id")?;

        let rpc = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "EXPERIMENTAL_tx_status",
            "params": {
                "tx_hash": tx_hash,
                "sender_account_id": sender_account_id,
            }
        });
        let (body, egress_bytes) = http_post_json(
            &request,
            egress,
            near_rpc_network_policy(),
            FASTNEAR_RPC_URL,
            &rpc,
        )
        .await?;
        let result = rpc_result(&body, egress_bytes)?;
        let output = json!({
            "status": result["status"],
            "receipts_outcome": result["receipts_outcome"],
            "transaction": result["transaction"],
        });
        Ok(success(output, egress_bytes))
    }

    async fn intents_quote(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let origin_asset = required_bounded(request.input, "origin_asset", MAX_METHOD_NAME_CHARS)?;
        let destination_asset =
            required_bounded(request.input, "destination_asset", MAX_METHOD_NAME_CHARS)?;
        let amount = required_bounded(request.input, "amount", MAX_METHOD_NAME_CHARS)?;
        let recipient = required_bounded(request.input, "recipient", MAX_METHOD_NAME_CHARS)?;
        let refund_to = required_account_id(request.input, "refund_to")?;
        let swap_type =
            optional_string(request.input, "swap_type")?.unwrap_or_else(|| "EXACT_INPUT".into());
        if !ALLOWED_SWAP_TYPES.contains(&swap_type.as_str()) {
            return Err(input_error());
        }
        // Slippage is expressed in basis points; clamp to a ceiling so a caller
        // can't request a degenerate 100%+ tolerance.
        let slippage_tolerance = optional_u64(request.input, "slippage_tolerance")?
            .unwrap_or(DEFAULT_SLIPPAGE_TOLERANCE)
            .min(MAX_SLIPPAGE_TOLERANCE);

        // `dry: true` is forced — this is a read-only quote capability and must
        // never request execution.
        let payload = json!({
            "swapType": swap_type,
            "originAsset": origin_asset,
            "destinationAsset": destination_asset,
            "amount": amount,
            "depositType": "INTENTS",
            "recipientType": "DESTINATION_CHAIN",
            "recipient": recipient,
            "refundTo": refund_to,
            "refundType": "INTENTS",
            "slippageTolerance": slippage_tolerance,
            "dry": true,
        });
        let (body, egress_bytes) = http_post_json(
            &request,
            egress,
            intents_network_policy(),
            INTENTS_QUOTE_URL,
            &payload,
        )
        .await?;
        // The 1Click quote response nests its payload under `quote`. A response
        // without it is an error envelope or an unexpected shape, not a quote, so
        // surface a failure rather than silently returning null fields.
        let quote = body
            .get("quote")
            .ok_or_else(|| operation_error(egress_bytes))?;
        let output = json!({
            "amount_out": quote["amountOut"],
            "deposit_address": quote["depositAddress"],
            "fee": quote["fee"],
            "deadline": quote["deadline"],
        });
        Ok(success(output, egress_bytes))
    }
}

#[cfg(test)]
mod tests;
