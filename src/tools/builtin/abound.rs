//! Built-in tools for the Abound remittance API.
//!
//! Four tools that wrap the Abound REST endpoints so the LLM can call them
//! directly instead of constructing raw HTTP requests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::tool::{
    ApprovalRequirement, RiskLevel, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

use super::validate_currency_code;


const REMITTANCE_BASE: &str = "https://devneobank.timesclub.co/times/bank/remittance/agent";
const NOTIFICATION_BASE: &str = "https://dev.timesclub.co/times/users/agent";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn shared_client() -> Result<Client, ToolError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {e}")))
}

async fn abound_credentials(
    secrets: &dyn SecretsStore,
    user_id: &str,
) -> Result<(String, String), ToolError> {
    let bearer = secrets
        .get_decrypted(user_id, "abound_read_token")
        .await
        .map_err(|_| {
            ToolError::NotAuthorized(
                "Missing abound_read_token. Set with: ironclaw secret set abound_read_token <TOKEN>"
                    .into(),
            )
        })?;
    let api_key = secrets
        .get_decrypted(user_id, "abound_api_key")
        .await
        .map_err(|_| {
            ToolError::NotAuthorized(
                "Missing abound_api_key. Set with: ironclaw secret set abound_api_key <KEY>".into(),
            )
        })?;
    Ok((bearer.expose().to_owned(), api_key.expose().to_owned()))
}

async fn abound_get(
    client: &Client,
    secrets: &dyn SecretsStore,
    user_id: &str,
    url: &str,
) -> Result<serde_json::Value, ToolError> {
    let (bearer, api_key) = abound_credentials(secrets, user_id).await?;

    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {bearer}"))
        .header("X-API-KEY", &api_key)
        .header("device-type", "WEB")
        .send()
        .await
        .map_err(|e| ToolError::ExternalService(e.to_string()))?;

    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| ToolError::ExternalService(e.to_string()))?;
    let body: serde_json::Value =
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));

    Ok(json!({ "status": status, "body": body }))
}

async fn abound_post(
    client: &Client,
    secrets: &dyn SecretsStore,
    user_id: &str,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    let (bearer, api_key) = abound_credentials(secrets, user_id).await?;

    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {bearer}"))
        .header("X-API-KEY", &api_key)
        .header("device-type", "WEB")
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| ToolError::ExternalService(e.to_string()))?;

    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| ToolError::ExternalService(e.to_string()))?;
    let body: serde_json::Value =
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));

    Ok(json!({ "status": status, "body": body }))
}

// ===========================================================================
// abound_account_info
// ===========================================================================

pub struct AboundAccountInfoTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl AboundAccountInfoTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for AboundAccountInfoTool {
    fn name(&self) -> &str {
        "abound_account_info"
    }

    fn description(&self) -> &str {
        "Get Abound account information including limits, saved recipients, and funding sources."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let url = format!("{REMITTANCE_BASE}/account/info");
        let result = abound_get(&self.client, &*self.secrets, &ctx.user_id, &url).await?;
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }
}

// ===========================================================================
// abound_exchange_rate
// ===========================================================================

pub struct AboundExchangeRateTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl AboundExchangeRateTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for AboundExchangeRateTool {
    fn name(&self) -> &str {
        "abound_exchange_rate"
    }

    fn description(&self) -> &str {
        "Get the current exchange rate for a currency pair from Abound (e.g. USD to INR)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "from_currency": {
                    "type": "string",
                    "description": "Source currency code, e.g. USD"
                },
                "to_currency": {
                    "type": "string",
                    "description": "Target currency code, e.g. INR"
                }
            },
            "required": ["from_currency", "to_currency"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let from = validate_currency_code(require_str(&params, "from_currency")?)?;
        let to = validate_currency_code(require_str(&params, "to_currency")?)?;
        let url = format!("{REMITTANCE_BASE}/exchange-rate?from_currency={from}&to_currency={to}");
        let result = abound_get(&self.client, &*self.secrets, &ctx.user_id, &url).await?;
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }
}

// ===========================================================================
// abound_send_wire
// ===========================================================================

pub struct AboundSendWireTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl AboundSendWireTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for AboundSendWireTool {
    fn name(&self) -> &str {
        "abound_send_wire"
    }

    fn description(&self) -> &str {
        "Send a wire transfer via Abound. ALWAYS call analyze_transfer before calling this tool. Requires funding source, beneficiary, amount, and payment reason."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "funding_source_id": {
                    "type": "string",
                    "description": "Funding source ID from account info"
                },
                "beneficiary_ref_id": {
                    "type": "string",
                    "description": "Beneficiary reference ID from account info"
                },
                "amount": {
                    "type": "number",
                    "description": "Amount in source currency (e.g. USD)"
                },
                "payment_reason_key": {
                    "type": "string",
                    "description": "Payment reason key from account info (e.g. family_maintenance, gift, education_support, medical_support)"
                }
            },
            "required": ["funding_source_id", "beneficiary_ref_id", "amount", "payment_reason_key"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let funding_source_id = require_str(&params, "funding_source_id")?;
        let beneficiary_ref_id = require_str(&params, "beneficiary_ref_id")?;
        let amount = params
            .get("amount")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::InvalidParameters("amount must be a number".into()))?;
        let payment_reason_key = require_str(&params, "payment_reason_key")?;

        let body = json!({
            "funding_source_id": funding_source_id,
            "beneficiary_ref_id": beneficiary_ref_id,
            "amount": amount,
            "payment_reason_key": payment_reason_key,
        });

        let url = format!("{REMITTANCE_BASE}/send-wire");
        let result = abound_post(&self.client, &*self.secrets, &ctx.user_id, &url, &body).await?;
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::High
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }
}

// ===========================================================================
// abound_create_notification
// ===========================================================================

pub struct AboundCreateNotificationTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl AboundCreateNotificationTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for AboundCreateNotificationTool {
    fn name(&self) -> &str {
        "abound_create_notification"
    }

    fn description(&self) -> &str {
        "Send a notification through Abound (e.g. after a successful transfer)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message_id": {
                    "type": "string",
                    "description": "Unique message identifier"
                },
                "action_type": {
                    "type": "string",
                    "description": "Notification action type (e.g. notification)"
                },
                "meta_data": {
                    "type": "object",
                    "description": "Additional metadata for the notification"
                }
            },
            "required": ["message_id", "action_type"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let message_id = require_str(&params, "message_id")?;
        let action_type = require_str(&params, "action_type")?;
        let meta_data = params
            .get("meta_data")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let body = json!({
            "message_id": message_id,
            "action_type": action_type,
            "meta_data": meta_data,
        });

        let url = format!("{NOTIFICATION_BASE}/create-notification");
        let result = abound_post(&self.client, &*self.secrets, &ctx.user_id, &url, &body).await?;
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_currency_code_valid() {
        assert_eq!(validate_currency_code("USD").unwrap(), "USD");
        assert_eq!(validate_currency_code("inr").unwrap(), "INR");
    }

    #[test]
    fn test_validate_currency_code_rejects_injection() {
        assert!(validate_currency_code("USD/../../admin").is_err());
        assert!(validate_currency_code("U$D").is_err());
        assert!(validate_currency_code("AB").is_err());
        assert!(validate_currency_code("ABCDE").is_err());
        assert!(validate_currency_code("").is_err());
    }
}
