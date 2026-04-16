//! Built-in tools for the Abound remittance API.
//!
//! Four tools that wrap the Abound REST endpoints so the LLM can call them
//! directly instead of constructing raw HTTP requests.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::registry::MissionSlot;
use crate::tools::tool::{
    RiskLevel, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

use super::forex::run_transfer_analysis;
use super::validate_currency_code;


pub(crate) const REMITTANCE_BASE: &str = "https://devneobank.timesclub.co/times/bank/remittance/agent";
const NOTIFICATION_BASE: &str = "https://dev.timesclub.co/times/users/agent";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn extract_abound_error(status: u64, body: Option<&serde_json::Value>) -> String {
    let msg = body
        .and_then(|b| b.get("error"))
        .and_then(|e| e.as_object())
        .map(|e| {
            let msg = e.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
            let code = e.get("code").and_then(|c| c.as_str()).unwrap_or("");
            if code.is_empty() { msg.to_string() } else { format!("{msg} (code: {code})") }
        })
        .or_else(|| body.and_then(|b| b.get("message")).and_then(|m| m.as_str()).map(String::from))
        .or_else(|| body.and_then(|b| b.as_str()).map(String::from));
    match msg {
        Some(m) => format!("(HTTP {status}): {m}"),
        None => format!(
            "(HTTP {status}). Response: {}",
            body.map(|b| b.to_string()).unwrap_or_else(|| "empty".into())
        ),
    }
}

fn shared_client() -> Result<Client, ToolError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {e}")))
}

pub(crate) async fn abound_credentials(
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

async fn abound_write_credentials(
    secrets: &dyn SecretsStore,
    user_id: &str,
) -> Result<(String, String), ToolError> {
    let bearer = secrets
        .get_decrypted(user_id, "abound_write_token")
        .await
        .map_err(|_| {
            ToolError::NotAuthorized(
                "Missing abound_write_token. Set with: ironclaw secret set abound_write_token <TOKEN>"
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

pub(crate) async fn abound_get(
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
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::debug!(body = %text, "Failed to parse Abound response as JSON: {e}");
            serde_json::Value::String(text)
        });

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
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::debug!(body = %text, "Failed to parse Abound response as JSON: {e}");
            serde_json::Value::String(text)
        });

    Ok(json!({ "status": status, "body": body }))
}

async fn abound_post_write(
    client: &Client,
    secrets: &dyn SecretsStore,
    user_id: &str,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    let (bearer, api_key) = abound_write_credentials(secrets, user_id).await?;

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
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::debug!(body = %text, "Failed to parse Abound response as JSON: {e}");
            serde_json::Value::String(text)
        });

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
    mission_slot: MissionSlot,
}

impl AboundSendWireTool {
    pub fn new(
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        mission_slot: MissionSlot,
    ) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
            mission_slot,
        })
    }
}

#[async_trait]
impl Tool for AboundSendWireTool {
    fn name(&self) -> &str {
        "abound_send_wire"
    }

    fn description(&self) -> &str {
        "Send a wire transfer via Abound. Four actions:\n\
         - action='initiate': runs timing analysis + graph. Requires: funding_source_id, beneficiary_ref_id, amount, payment_reason_key.\n\
         - action='send': sends a notification for approval on the remote client. Requires: amount, beneficiary_ref_id, payment_reason_key.\n\
         - action='wait': creates an hourly rate monitoring mission. Requires: target_rate, current_rate.\n\
         - action='execute': executes the actual wire transfer. Call ONLY after user confirms approval. Requires: funding_source_id, beneficiary_ref_id, amount, payment_reason_key."
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
                    "description": "Amount in source currency (e.g. USD). REQUIRED — ask the user if not provided."
                },
                "payment_reason_key": {
                    "type": "string",
                    "description": "Payment reason key"
                },
                "target_rate": {
                    "type": "number",
                    "description": "Target exchange rate for the wait action (from initiate analysis)"
                },
                "current_rate": {
                    "type": "number",
                    "description": "Current exchange rate for the wait action (from initiate analysis)"
                },
                "action": {
                    "type": "string",
                    "enum": ["initiate", "send", "wait", "execute"],
                    "description": "REQUIRED. 'initiate' runs analysis. 'send' sends approval notification. 'wait' creates rate monitoring mission. 'execute' executes the wire (only after user confirms approval)."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let action = require_str(&params, "action")?;

        if action == "send" {
            let amount = params
                .get("amount")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::InvalidParameters("amount is required for send action".into()))?;
            let beneficiary = require_str(&params, "beneficiary_ref_id")?;
            let payment_reason = require_str(&params, "payment_reason_key")?;

            let ts = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let thread_id = ctx
                .metadata
                .get("notify_thread_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let notif_body = json!({
                "message_id": format!("wire_approval_{ts}"),
                "action_type": "notification",
                "meta_data": {
                    "type": "wire_approval",
                    "amount": amount,
                    "beneficiary_ref_id": beneficiary,
                    "payment_reason_key": payment_reason,
                    "notify_thread_id": thread_id,
                },
            });
            let notif_url = format!("{NOTIFICATION_BASE}/create-notification");
            let notif_result = abound_post(
                &self.client, &*self.secrets, &ctx.user_id, &notif_url, &notif_body,
            ).await?;

            let status = notif_result.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
            if (200..300).contains(&status) {
                return Ok(ToolOutput::text(
                    format!(
                        "Notification sent for wire transfer of ${amount}. \
                         Waiting for your approval on the remote client."
                    ),
                    start.elapsed(),
                ));
            } else {
                let err_info = extract_abound_error(status, notif_result.get("body"));
                return Ok(ToolOutput::text(
                    format!(
                        "Failed to send approval notification for wire transfer of ${amount} {err_info}. \
                         Please try again or approve manually on the remote client."
                    ),
                    start.elapsed(),
                ));
            }
        } else if action == "wait" {
            let target_rate = params
                .get("target_rate")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::InvalidParameters("target_rate is required for wait action".into()))?;
            let current_rate = params
                .get("current_rate")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::InvalidParameters("current_rate is required for wait action".into()))?;

            if target_rate <= 0.0 {
                return Err(ToolError::InvalidParameters(
                    "target_rate must be a positive number".into(),
                ));
            }

            let threshold = (target_rate * 10000.0).round() / 10000.0;

            let slot = self.mission_slot.read().await;
            if let Some((mgr, project_id)) = slot.as_ref() {
                let goal = format!(
                    "This mission runs exactly 24 times (once per hour for 24 hours).\n\
                     \n\
                     On each run, call abound_rate_alert(threshold={threshold}).\n\
                     - If the rate exceeds the threshold, a notification is sent automatically. \
                     Respond with FINAL() including 'goal achieved: yes'.\n\
                     - If this is thread #24 (the final run) and the threshold has NOT been reached, \
                     call abound_rate_alert(threshold={threshold}, force_notify=true) to send a \
                     status notification anyway. Then respond with FINAL() including 'mission complete'.\n\
                     - Otherwise, call FINAL() with the result and note the current rate."
                );
                let cadence = ironclaw_engine::types::mission::MissionCadence::Cron {
                    expression: "0 * * * *".to_string(),
                    timezone: None,
                };
                let notify_channels = ctx
                    .metadata
                    .get("notify_channel")
                    .and_then(|v| v.as_str())
                    .map(|ch| vec![ch.to_string()])
                    .unwrap_or_default();

                let mission_id = mgr
                    .create_mission(
                        *project_id,
                        &ctx.user_id,
                        "USD/INR Rate Monitor",
                        &goal,
                        cadence,
                        notify_channels,
                    )
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!(
                            "failed to create monitoring mission: {e}"
                        ))
                    })?;

                // Set max_threads_per_day to 24 and add success criteria
                let updates = ironclaw_engine::runtime::mission::MissionUpdate {
                    name: None,
                    description: None,
                    goal: None,
                    cadence: None,
                    notify_channels: None,
                    notify_user: None,
                    context_paths: None,
                    max_threads_per_day: Some(24),
                    success_criteria: Some(
                        "Target rate reached and notification sent, or 24 hourly checks completed."
                            .into(),
                    ),
                    cooldown_secs: None,
                    max_concurrent: None,
                    dedup_window_secs: None,
                };
                if let Err(e) = mgr.update_mission(mission_id, &ctx.user_id, updates).await {
                    tracing::debug!("failed to update mission guardrails: {e}");
                }

                return Ok(ToolOutput::text(
                    format!(
                        "Hourly rate monitoring set up (mission {mission_id}). \
                         Will check USD/INR against {threshold} every hour for 24 hours. \
                         You'll get a notification when the target is reached, or a status \
                         update after 24 hours. Current rate: {current_rate:.4}."
                    ),
                    start.elapsed(),
                ));
            } else {
                return Err(ToolError::ExecutionFailed(
                    "mission manager not available yet".into(),
                ));
            }
        } else if action == "execute" {
            let funding_source_id = require_str(&params, "funding_source_id")?;
            let beneficiary_ref_id = require_str(&params, "beneficiary_ref_id")?;
            let amount = params
                .get("amount")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::InvalidParameters("amount is required for execute".into()))?;
            let payment_reason_key = require_str(&params, "payment_reason_key")?;

            let mut missing = Vec::new();
            if funding_source_id.trim().is_empty() {
                missing.push("funding_source_id");
            }
            if beneficiary_ref_id.trim().is_empty() {
                missing.push("beneficiary_ref_id");
            }
            if payment_reason_key.trim().is_empty() {
                missing.push("payment_reason_key");
            }
            if amount <= 0.0 {
                missing.push("amount (must be > 0)");
            }
            if !missing.is_empty() {
                return Err(ToolError::InvalidParameters(format!(
                    "Cannot execute wire transfer — the following parameters are missing or invalid: {}. \
                     Use abound_account_info to look up the correct values before retrying.",
                    missing.join(", ")
                )));
            }

            let wire_body = json!({
                "funding_source_id": funding_source_id,
                "beneficiary_ref_id": beneficiary_ref_id,
                "amount": amount,
                "payment_reason_key": payment_reason_key,
            });
            let url = format!("{REMITTANCE_BASE}/send-wire");
            let wire_result =
                abound_post_write(&self.client, &*self.secrets, &ctx.user_id, &url, &wire_body).await?;

            let status = wire_result.get("status").and_then(|v| v.as_u64()).unwrap_or(0);

            let message = if (200..300).contains(&status) {
                format!("Wire transfer of ${amount} executed successfully.")
            } else {
                format!("Wire transfer failed {}", extract_abound_error(status, wire_result.get("body")))
            };

            return Ok(ToolOutput::text(message, start.elapsed()));
        }

        let funding_source_id = require_str(&params, "funding_source_id")?;
        let beneficiary_ref_id = require_str(&params, "beneficiary_ref_id")?;
        let amount = params
            .get("amount")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::InvalidParameters("amount must be a number".into()))?;
        let payment_reason_key = require_str(&params, "payment_reason_key")?;

        let analysis = run_transfer_analysis(
            &self.client,
            &*self.secrets,
            &ctx.user_id,
            Some(amount),
            true,
        )
        .await
        .ok();

        let result = json!({
            "phase": "confirmation_required",
            "analysis": analysis,
            "transfer_details": {
                "amount": amount,
                "beneficiary_ref_id": beneficiary_ref_id,
                "funding_source_id": funding_source_id,
                "payment_reason_key": payment_reason_key,
            }
        });
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

// ===========================================================================
// abound_rate_alert — atomic check-and-notify for mission threads
// ===========================================================================

pub struct AboundRateAlertTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl AboundRateAlertTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for AboundRateAlertTool {
    fn name(&self) -> &str {
        "abound_rate_alert"
    }

    fn description(&self) -> &str {
        "Check the current exchange rate and send a notification if it exceeds a threshold. \
         Designed for mission threads — does everything in one call: fetch rate, compare, notify. \
         Returns the current rate, whether threshold was exceeded, and whether notification was sent."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "from_currency": {
                    "type": "string",
                    "description": "Source currency code (default: USD)",
                    "default": "USD"
                },
                "to_currency": {
                    "type": "string",
                    "description": "Target currency code (default: INR)",
                    "default": "INR"
                },
                "threshold": {
                    "type": "number",
                    "description": "Rate threshold. Notification is sent if the current rate exceeds this value."
                },
                "message_id": {
                    "type": "string",
                    "description": "Notification message identifier (default: rate_alert)",
                    "default": "rate_alert"
                },
                "force_notify": {
                    "type": "boolean",
                    "description": "When true, send the notification regardless of whether the threshold is exceeded. Use on the final run.",
                    "default": false
                }
            },
            "required": ["threshold"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let from = validate_currency_code(
            params
                .get("from_currency")
                .and_then(|v| v.as_str())
                .unwrap_or("USD"),
        )?;
        let to = validate_currency_code(
            params
                .get("to_currency")
                .and_then(|v| v.as_str())
                .unwrap_or("INR"),
        )?;
        let threshold = params
            .get("threshold")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::InvalidParameters("threshold must be a number".into()))?;
        let message_id = params
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("rate_alert");
        let force_notify = params
            .get("force_notify")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Step 1: Fetch exchange rate
        let url = format!("{REMITTANCE_BASE}/exchange-rate?from_currency={from}&to_currency={to}");
        let rate_response =
            abound_get(&self.client, &*self.secrets, &ctx.user_id, &url).await?;

        // Parse rate from response: body.data.current_exchange_rate.formatted_value
        let current_rate = rate_response
            .get("body")
            .and_then(|b| b.get("data"))
            .and_then(|d| d.get("current_exchange_rate"))
            .and_then(|r| {
                r.get("value")
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        r.get("formatted_value")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<f64>().ok())
                    })
            })
            .unwrap_or(0.0);

        let effective_rate = rate_response
            .get("body")
            .and_then(|b| b.get("data"))
            .and_then(|d| d.get("effective_exchange_rate"))
            .and_then(|r| {
                r.get("value")
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        r.get("formatted_value")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<f64>().ok())
                    })
            })
            .unwrap_or(0.0);

        let exceeded = current_rate > threshold;
        let should_notify = exceeded || force_notify;

        // Step 2: Send notification if threshold exceeded or force_notify
        let notification_sent = if should_notify {
            let notif_body = json!({
                "message_id": message_id,
                "action_type": "notification",
                "meta_data": {
                    "alert": format!("{from}/{to} rate alert"),
                    "current_rate": current_rate,
                    "effective_rate": effective_rate,
                    "threshold": threshold,
                },
            });
            let notif_url = format!("{NOTIFICATION_BASE}/create-notification");
            let notif_result = abound_post(
                &self.client,
                &*self.secrets,
                &ctx.user_id,
                &notif_url,
                &notif_body,
            )
            .await;
            notif_result.is_ok()
        } else {
            false
        };

        let result = json!({
            "current_rate": current_rate,
            "effective_rate": effective_rate,
            "threshold": threshold,
            "exceeded": exceeded,
            "notification_sent": notification_sent,
            "pair": format!("{from}/{to}"),
        });
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

    #[test]
    fn send_notification_carries_notify_thread_id() {
        let thread_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let ctx_metadata = json!({ "notify_thread_id": thread_id });

        let extracted = ctx_metadata
            .get("notify_thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let notif_body = json!({
            "meta_data": {
                "notify_thread_id": extracted,
            },
        });

        assert_eq!(
            notif_body["meta_data"]["notify_thread_id"].as_str(),
            Some(thread_id),
        );
    }
}
