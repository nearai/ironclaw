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
    /// Mirrors `RebornAutomationInfo::active_hold`; present while the
    /// automation's active fire is gate-parked or still running (#5886).
    #[serde(default)]
    pub active_hold: Option<AutomationActiveHold>,
    /// Mirrors `RebornAutomationInfo::recent_runs`, newest-first (bounded by
    /// the server's `run_limit`).
    #[serde(default)]
    pub recent_runs: Vec<AutomationRecentRun>,
}

/// Mirrors `RebornAutomationActiveHold`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AutomationActiveHold {
    /// Raw `RebornAutomationHoldReason` wire string: approval/auth/
    /// in_progress/other.
    pub reason: String,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub elapsed_occurrences: Option<u32>,
    #[serde(default)]
    pub elapsed_occurrences_capped: bool,
}

/// Mirrors `RebornAutomationRecentRunInfo`. Only the fields the TUI needs
/// (deciding which thread `Enter` opens) are carried; `run_id`/`fire_slot`/
/// `submitted_at`/`completed_at` are dropped per the same unknown-field-
/// tolerant convention as the rest of this file.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AutomationRecentRun {
    /// `None` until fire acceptance (see server doc comment) — the TUI must
    /// not treat this run as openable when absent.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Raw `RebornAutomationRecentRunStatus` wire string: running/ok/error/
    /// unknown.
    #[serde(default)]
    pub status: String,
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
    /// Always requests `?include_completed=true` so a fired one-time
    /// automation (`TriggerState::Completed`) stays visible in the panel
    /// instead of silently vanishing the moment it fires — see
    /// `ListAutomationsQuery::include_completed` in `ironclaw_webui_v2`.
    pub async fn list_automations(&self) -> Result<Vec<AutomationSummary>, ClientError> {
        let wire: ListAutomationsWire = self
            .send_json(
                self.http
                    .get(self.url("/api/webchat/v2/automations"))
                    .query(&[("include_completed", "true")]),
            )
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::{RawQuery, State};
    use axum::routing::get;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct CapturedQuery(Arc<Mutex<Option<String>>>);

    async fn list_automations_stub(
        State(captured): State<CapturedQuery>,
        RawQuery(query): RawQuery,
    ) -> axum::Json<serde_json::Value> {
        *captured.0.lock().expect("lock captured query") = query;
        axum::Json(serde_json::json!({ "automations": [] }))
    }

    /// Item 1: `list_automations()` must always request `include_completed`
    /// so a fired one-time automation stays visible instead of vanishing —
    /// asserts on the actual query string sent over the wire, not just on
    /// what the client method returns.
    #[tokio::test]
    async fn list_automations_requests_include_completed() {
        let captured = CapturedQuery::default();
        let router = Router::new()
            .route("/api/webchat/v2/automations", get(list_automations_stub))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub listener");
        let addr = listener.local_addr().expect("stub listener addr");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("stub server serve");
        });

        let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
        client
            .list_automations()
            .await
            .expect("list_automations succeeds against stub");

        let query = captured.0.lock().expect("lock captured query").clone();
        assert_eq!(
            query.as_deref(),
            Some("include_completed=true"),
            "list_automations must send ?include_completed=true"
        );
    }
}
