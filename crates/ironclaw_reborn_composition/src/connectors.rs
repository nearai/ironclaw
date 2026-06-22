//! Read-only connector proxy implementation (Composio REST).
//!
//! This is the only layer that holds both the encrypted secret store and an
//! HTTP client, so it owns the full read path the WebUI Workbench needs:
//!
//! 1. resolve the bound `composio_api_key` from the secret store, scoped to
//!    the owner — server-side only, never returned or logged;
//! 2. proxy `GET /api/v3/connected_accounts?statuses=ACTIVE` for the
//!    connected-account list;
//! 3. proxy `POST /api/v3/tools/execute/{tool}` for a single read, after
//!    enforcing the read-only allowlist
//!    ([`ironclaw_product_workflow::is_read_only_tool`]) and resolving the
//!    Composio entity (`user_id`) that owns the requested toolkit's account.
//!
//! Writes (SEND/CREATE/DELETE/MODIFY/REPLY/TRASH/UPDATE …) are rejected here
//! with `400`; they stay on the gated agent path.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountSelectionRequest,
    CredentialAccountStatus,
};
use ironclaw_host_api::{
    ExtensionId, InvocationId, ResourceScope, RuntimeCredentialAccountSetup, SecretHandle, UserId,
};
use ironclaw_product_workflow::{
    ConnectorReadError, ConnectorReadPort, ConnectorWriteKind, RebornConnectedAccount,
    RebornConnectedAccountsResponse, RebornConnectorReadRequest, RebornConnectorReadResponse,
    RebornConnectorWriteRequest, classify_connector_write, is_read_only_tool,
};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use secrecy::ExposeSecret;
use tokio::sync::Mutex;

use crate::product_auth_runtime_credentials::{
    RuntimeCredentialAccountSelectionRequest, RuntimeCredentialAccountSelectionService,
};

/// Composio REST v3 base. Matches the proven, first-party `composio` WASM tool.
const COMPOSIO_API_BASE: &str = "https://backend.composio.dev/api/v3";

/// Provider id the composio extension binds its `manual_token` credential
/// account under (`product_auth_account`, provider `composio`).
const COMPOSIO_PROVIDER: &str = "composio";

/// Convenience handle the composio WASM tool documents (`secret_exists`). Used
/// as a fallback when no `product_auth_account` record resolves an access
/// secret for the provider.
const COMPOSIO_API_KEY_HANDLE: &str = "composio_api_key";

/// Read-only Composio connector port backed by the encrypted secret store.
pub(crate) struct ComposioConnectorPort {
    http: reqwest::Client,
    secret_store: Arc<dyn SecretStore>,
    accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    owner: UserId,
    /// `toolkit slug` -> `composio entity user_id` cache, populated from the
    /// connected-accounts listing so a read can target the right entity.
    entity_by_toolkit: Mutex<HashMap<String, String>>,
    /// Whether the gateway permits actual SEND writes (deliver email / post to
    /// Slack). Off by default; draft-creation writes are always allowed. Set
    /// from `IRONCLAW_WORKBENCH_SEND_ENABLED` at construction so flipping the
    /// capability is an explicit deployment decision, not a per-request flag.
    send_enabled: bool,
}

impl std::fmt::Debug for ComposioConnectorPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComposioConnectorPort")
            .field("owner", &self.owner.as_str())
            .finish_non_exhaustive()
    }
}

impl ComposioConnectorPort {
    pub(crate) fn new(
        secret_store: Arc<dyn SecretStore>,
        accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
        owner: UserId,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            secret_store,
            accounts,
            owner,
            entity_by_toolkit: Mutex::new(HashMap::new()),
            send_enabled: send_capability_enabled(),
        }
    }

    /// Resolve the secret handle holding the composio API key.
    ///
    /// The composio extension binds the key as a `product_auth_account`
    /// (`manual_token`) credential, so the durable access secret is named by
    /// the credential-account record — not the literal `composio_api_key`. We
    /// resolve it through the product-auth selection service (same path the
    /// runtime credential injection uses for the gated tool), and fall back to
    /// the documented literal handle if no configured account is found.
    async fn resolve_api_key_handle(
        &self,
        scope: &ResourceScope,
    ) -> Result<SecretHandle, ConnectorReadError> {
        let provider =
            AuthProviderId::new(COMPOSIO_PROVIDER).map_err(|_| ConnectorReadError::Internal)?;
        let owner_scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        let requester =
            ExtensionId::new(COMPOSIO_PROVIDER).map_err(|_| ConnectorReadError::Internal)?;
        let lookup =
            CredentialAccountSelectionRequest::new(owner_scope, provider).for_extension(requester);
        // The composio API key is a manual token (no OAuth scopes), so the
        // runtime selection carries an empty provider-scope set.
        let request = RuntimeCredentialAccountSelectionRequest::new(
            lookup,
            AuthProductScope::new(scope.clone(), AuthSurface::Api),
            RuntimeCredentialAccountSetup::ManualToken,
            Vec::new(),
        );
        match self
            .accounts
            .select_unique_configured_runtime_account(request)
            .await
        {
            Ok(account)
                if account.status == CredentialAccountStatus::Configured
                    && account.access_secret.is_some() =>
            {
                Ok(account.access_secret.expect("checked is_some above"))
            }
            // No configured product-auth account (or no access secret on it):
            // fall back to the documented convenience handle.
            _ => Self::literal_api_key_handle(),
        }
    }

    /// The documented literal `composio_api_key` handle. This is the handle the
    /// `setup … {action:"configure"}` write lands under (see
    /// [`Self::configure_secrets`]) and the same one the WASM tool's
    /// `secret_exists("composio_api_key")` pre-flight checks, so the two share
    /// one binding.
    fn literal_api_key_handle() -> Result<SecretHandle, ConnectorReadError> {
        SecretHandle::new(COMPOSIO_API_KEY_HANDLE).map_err(|_| ConnectorReadError::Internal)
    }

    /// Resolve the owner-scoped composio API key from the encrypted secret
    /// store. The returned value is used only as the upstream `x-api-key`
    /// header by callers in this module — it is never logged or surfaced.
    async fn resolve_api_key(&self) -> Result<String, ConnectorReadError> {
        let scope = self
            .owner_scope()
            .map_err(|_| ConnectorReadError::Internal)?;
        let handle = self.resolve_api_key_handle(&scope).await?;

        let lease = self
            .secret_store
            .lease_once(&scope, &handle)
            .await
            .map_err(|_| ConnectorReadError::Unavailable { retryable: false })?;
        let material = self
            .secret_store
            .consume(&scope, lease.id)
            .await
            .map_err(|_| ConnectorReadError::Unavailable { retryable: false })?;
        Ok(material.expose_secret().to_string())
    }

    fn owner_scope(&self) -> Result<ResourceScope, ()> {
        ResourceScope::local_default(self.owner.clone(), InvocationId::new()).map_err(|_| ())
    }

    /// Fetch the raw active connected-accounts list from Composio.
    async fn fetch_connected_accounts(&self) -> Result<serde_json::Value, ConnectorReadError> {
        let api_key = self.resolve_api_key().await?;
        let url = format!("{COMPOSIO_API_BASE}/connected_accounts?statuses=ACTIVE");
        let response = self
            .http
            .get(&url)
            .header("x-api-key", &api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|_| ConnectorReadError::Unavailable { retryable: true })?;
        let status = response.status();
        let body: serde_json::Value =
            response
                .json()
                .await
                .map_err(|_| ConnectorReadError::Upstream {
                    message: "connected_accounts: invalid upstream JSON".to_string(),
                })?;
        if !status.is_success() {
            return Err(ConnectorReadError::Upstream {
                message: format!("connected_accounts upstream status {}", status.as_u16()),
            });
        }
        Ok(body)
    }

    /// Cache and return the `toolkit -> entity user_id` map for active accounts.
    async fn refresh_entity_map(&self) -> Result<Vec<RebornConnectedAccount>, ConnectorReadError> {
        let body = self.fetch_connected_accounts().await?;
        let items = extract_items(&body);

        let mut accounts = Vec::with_capacity(items.len());
        let mut map = self.entity_by_toolkit.lock().await;
        for item in items {
            let toolkit = extract_toolkit_slug(item);
            let user_id = item
                .get("user_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let status = item
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string();
            if let (Some(toolkit), Some(user_id)) = (toolkit.clone(), user_id.clone()) {
                map.entry(toolkit.to_ascii_lowercase()).or_insert(user_id);
            }
            if let (Some(toolkit), Some(user_id)) = (toolkit, user_id) {
                accounts.push(RebornConnectedAccount {
                    toolkit: toolkit.to_ascii_lowercase(),
                    status,
                    user_id,
                });
            }
        }
        Ok(accounts)
    }

    /// Resolve the Composio entity (`user_id`) that owns `toolkit`'s active
    /// account, using the cache and refreshing once on a miss.
    async fn entity_for_toolkit(&self, toolkit: &str) -> Result<String, ConnectorReadError> {
        let key = toolkit.to_ascii_lowercase();
        if let Some(entity) = self.entity_by_toolkit.lock().await.get(&key).cloned() {
            return Ok(entity);
        }
        self.refresh_entity_map().await?;
        self.entity_by_toolkit
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| ConnectorReadError::InvalidRequest {
                reason: format!("no active connected account for toolkit '{key}'"),
            })
    }

    /// Resolve the owner entity + server-side key and proxy a single
    /// `tools/execute/{tool}` call, returning the Composio `{successful,data,
    /// error}` envelope. Shared by the read and gated-write paths so both issue
    /// the identical upstream call; each caller enforces its own allowlist gate
    /// BEFORE invoking this. The key is sent only as the upstream header.
    async fn execute_tool(
        &self,
        toolkit: &str,
        tool: &str,
        arguments: &serde_json::Value,
    ) -> Result<RebornConnectorReadResponse, ConnectorReadError> {
        // Defense-in-depth against path traversal in the tool slug.
        if tool.contains('/') || tool.contains('\\') || tool.contains("..") {
            return Err(ConnectorReadError::InvalidRequest {
                reason: "invalid tool slug".to_string(),
            });
        }

        let entity = self.entity_for_toolkit(toolkit).await?;
        let api_key = self.resolve_api_key().await?;

        let url = format!("{COMPOSIO_API_BASE}/tools/execute/{tool}");
        let payload = serde_json::json!({
            "arguments": arguments,
            "user_id": entity,
        });

        let response = self
            .http
            .post(&url)
            .header("x-api-key", &api_key)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|_| ConnectorReadError::Unavailable { retryable: true })?;
        let status = response.status();
        let body: serde_json::Value =
            response
                .json()
                .await
                .map_err(|_| ConnectorReadError::Upstream {
                    message: "tools/execute: invalid upstream JSON".to_string(),
                })?;
        if !status.is_success() {
            return Err(ConnectorReadError::Upstream {
                message: format!("tools/execute upstream status {}", status.as_u16()),
            });
        }

        // Composio wraps tool output as { successful, data, error }.
        let successful = body
            .get("successful")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let data = body.get("data").cloned().unwrap_or(serde_json::Value::Null);
        let error = body
            .get("error")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        Ok(RebornConnectorReadResponse {
            successful,
            data,
            error,
        })
    }
}

/// Whether the gateway permits actual SEND writes. Off unless the env var is an
/// affirmative value, so enabling delivery is an explicit deployment decision.
fn send_capability_enabled() -> bool {
    matches!(
        std::env::var("IRONCLAW_WORKBENCH_SEND_ENABLED")
            .ok()
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[async_trait]
impl ConnectorReadPort for ComposioConnectorPort {
    async fn connected(&self) -> Result<RebornConnectedAccountsResponse, ConnectorReadError> {
        let accounts = self.refresh_entity_map().await?;
        Ok(RebornConnectedAccountsResponse { accounts })
    }

    async fn read(
        &self,
        request: RebornConnectorReadRequest,
    ) -> Result<RebornConnectorReadResponse, ConnectorReadError> {
        // Read-only allowlist — enforced before any upstream call so writes
        // never leave this process.
        if !is_read_only_tool(&request.tool) {
            return Err(ConnectorReadError::InvalidRequest {
                reason: format!("tool '{}' is not on the read-only allowlist", request.tool),
            });
        }
        self.execute_tool(&request.toolkit, &request.tool, &request.arguments)
            .await
    }

    async fn write(
        &self,
        request: RebornConnectorWriteRequest,
    ) -> Result<RebornConnectorReadResponse, ConnectorReadError> {
        // Explicit write allowlist — enforced before any upstream call. The
        // decision is a pure function (`connector_write_admission`) so the
        // security boundary is regression-locked by unit tests independent of the
        // upstream call.
        if let Err(reason) =
            connector_write_admission(classify_connector_write(&request.tool), self.send_enabled)
        {
            return Err(ConnectorReadError::InvalidRequest {
                reason: format!("tool '{}' {reason}", request.tool),
            });
        }
        self.execute_tool(&request.toolkit, &request.tool, &request.arguments)
            .await
    }

    async fn configure_secrets(
        &self,
        secrets: HashMap<String, String>,
    ) -> Result<(), ConnectorReadError> {
        // Only the composio API key is consumed; ignore anything else so the
        // configure surface can never be used to plant arbitrary secrets.
        let Some(api_key) = secrets.get(COMPOSIO_API_KEY_HANDLE) else {
            return Err(ConnectorReadError::InvalidRequest {
                reason: format!("configure payload is missing `{COMPOSIO_API_KEY_HANDLE}`"),
            });
        };
        if api_key.is_empty() {
            return Err(ConnectorReadError::InvalidRequest {
                reason: format!("`{COMPOSIO_API_KEY_HANDLE}` must not be empty"),
            });
        }

        // Write under the SAME owner scope + literal handle the read path
        // resolves against (`owner_scope()` + `literal_api_key_handle()`), so a
        // configure write is guaranteed to be visible to the connector reads.
        let scope = self
            .owner_scope()
            .map_err(|_| ConnectorReadError::Internal)?;
        let handle = Self::literal_api_key_handle()?;
        // Best-effort delete of any prior value so re-configuring replaces
        // rather than conflicts; a missing prior secret is not an error.
        let _ = self.secret_store.delete(&scope, &handle).await;
        self.secret_store
            .put(scope, handle, SecretMaterial::from(api_key.clone()))
            .await
            .map_err(|_| ConnectorReadError::Unavailable { retryable: false })?;
        Ok(())
    }
}

/// Extract the items array from a Composio v3 response (`{ "items": [...] }`)
/// or treat a bare array as the items.
fn extract_items(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    if let Some(items) = value.get("items").and_then(|v| v.as_array()) {
        return items.iter().collect();
    }
    if let Some(items) = value.as_array() {
        return items.iter().collect();
    }
    Vec::new()
}

/// Extract a toolkit slug from a connected-account object. v3 nests it under
/// `toolkit.slug`; falls back to `toolkit_slug` / a string `toolkit`.
fn extract_toolkit_slug(item: &serde_json::Value) -> Option<String> {
    if let Some(slug) = item
        .get("toolkit")
        .and_then(|tk| tk.get("slug"))
        .and_then(|v| v.as_str())
    {
        return Some(slug.to_string());
    }
    if let Some(slug) = item.get("toolkit_slug").and_then(|v| v.as_str()) {
        return Some(slug.to_string());
    }
    item.get("toolkit")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Pure write-admission decision for the connector write route. Drafts are
/// always permitted (they create a reviewable draft and deliver nothing); sends
/// require the gateway send capability; everything else is forbidden. Extracted
/// from `ConnectorWritePort::write` so the security boundary is unit-tested
/// independent of any upstream call. Returns `Ok(())` to permit, or `Err(reason)`
/// (a trailing clause appended to "tool '<slug>' …") to reject.
fn connector_write_admission(kind: ConnectorWriteKind, send_enabled: bool) -> Result<(), String> {
    match kind {
        ConnectorWriteKind::Draft => Ok(()),
        ConnectorWriteKind::Send if send_enabled => Ok(()),
        ConnectorWriteKind::Send => {
            Err("delivers and is disabled; the gateway send capability is off".to_string())
        }
        ConnectorWriteKind::Forbidden => Err("is not on the write allowlist".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectorWriteKind, connector_write_admission, extract_items, extract_toolkit_slug};

    #[test]
    fn write_admission_enforces_kind_x_send_enabled_matrix() {
        // Drafts: always permitted, regardless of the send capability.
        assert!(connector_write_admission(ConnectorWriteKind::Draft, false).is_ok());
        assert!(connector_write_admission(ConnectorWriteKind::Draft, true).is_ok());
        // Sends: permitted ONLY when the gateway send capability is on.
        assert!(connector_write_admission(ConnectorWriteKind::Send, true).is_ok());
        assert!(connector_write_admission(ConnectorWriteKind::Send, false).is_err());
        // Forbidden (deletes / unlisted): never permitted, even with sends on.
        assert!(connector_write_admission(ConnectorWriteKind::Forbidden, true).is_err());
        assert!(connector_write_admission(ConnectorWriteKind::Forbidden, false).is_err());
    }

    #[test]
    fn extract_items_handles_envelope_and_bare_array() {
        let env: serde_json::Value =
            serde_json::from_str(r#"{"items":[{"a":1},{"a":2}]}"#).unwrap();
        assert_eq!(extract_items(&env).len(), 2);
        let bare: serde_json::Value = serde_json::from_str(r#"[{"a":1}]"#).unwrap();
        assert_eq!(extract_items(&bare).len(), 1);
        let none: serde_json::Value = serde_json::from_str(r#"{"x":1}"#).unwrap();
        assert!(extract_items(&none).is_empty());
    }

    #[test]
    fn extract_toolkit_slug_prefers_nested() {
        let nested: serde_json::Value =
            serde_json::from_str(r#"{"toolkit":{"slug":"gmail"}}"#).unwrap();
        assert_eq!(extract_toolkit_slug(&nested).as_deref(), Some("gmail"));
        let flat: serde_json::Value = serde_json::from_str(r#"{"toolkit_slug":"slack"}"#).unwrap();
        assert_eq!(extract_toolkit_slug(&flat).as_deref(), Some("slack"));
        let str_toolkit: serde_json::Value =
            serde_json::from_str(r#"{"toolkit":"notion"}"#).unwrap();
        assert_eq!(
            extract_toolkit_slug(&str_toolkit).as_deref(),
            Some("notion")
        );
    }
}
