//! Automation list/pause/resume/rename.
//!
//! Wire source: `RebornAutomationInfo`, `RebornAutomationSource` (tagged
//! `type: schedule|once`), `RebornAutomationState` (plain snake_case string)
//! — all in `ironclaw_product_workflow::reborn_services::types`.
//!
//! Per review-round-2 override #4, the dead wire fields `scheduler_enabled`
//! (list response) and `updated` (mutation response) are dropped entirely —
//! serde ignores unknown fields on the wire, so omitting them from the
//! local wire structs is harmless; re-add with a real consumer.

use super::{ApiClient, ClientError};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AutomationSummary {
    pub automation_id: String,
    pub name: String,
    /// Raw `RebornAutomationState` wire string: active/scheduled/paused/
    /// disabled/inactive/completed/unknown.
    pub state: String,
    #[serde(default)]
    pub next_run_at: Option<String>,
    #[serde(default)]
    pub last_run_at: Option<String>,
    /// Raw `RebornAutomationRunStatus` wire string when present: ok/error.
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub is_active: bool,
}

#[derive(serde::Deserialize)]
struct ListAutomationsWire {
    automations: Vec<AutomationSummary>,
}

#[derive(serde::Deserialize)]
struct AutomationMutationWire {
    automation: Option<AutomationSummary>,
}

impl ApiClient {
    pub async fn list_automations(&self) -> Result<Vec<AutomationSummary>, ClientError> {
        let wire: ListAutomationsWire = self
            .send_json(self.http.get(self.url("/api/webchat/v2/automations")))
            .await?;
        Ok(wire.automations)
    }

    pub async fn pause_automation(&self, id: &str) -> Result<AutomationSummary, ClientError> {
        self.mutate_automation(&format!("/api/webchat/v2/automations/{id}/pause"), None)
            .await
    }

    pub async fn resume_automation(&self, id: &str) -> Result<AutomationSummary, ClientError> {
        self.mutate_automation(&format!("/api/webchat/v2/automations/{id}/resume"), None)
            .await
    }

    pub async fn rename_automation(
        &self,
        id: &str,
        name: &str,
    ) -> Result<AutomationSummary, ClientError> {
        self.mutate_automation(
            &format!("/api/webchat/v2/automations/{id}"),
            Some(serde_json::json!({ "name": name })),
        )
        .await
    }

    async fn mutate_automation(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<AutomationSummary, ClientError> {
        let mut builder = self.http.post(self.url(path));
        builder = match body {
            Some(body) => builder.json(&body),
            None => builder.json(&serde_json::json!({})),
        };
        let wire: AutomationMutationWire = self.send_json(builder).await?;
        wire.automation.ok_or_else(|| ClientError::Server {
            status: 200,
            body: "automation mutation response missing `automation`".to_string(),
        })
    }
}
