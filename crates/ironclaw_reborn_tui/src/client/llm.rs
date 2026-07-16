//! LLM provider config: catalog read, active-selection write, connection
//! probe, model listing.
//!
//! Reuses `ironclaw_product_workflow`'s response types directly (no mirror
//! needed — all-flat `String`/`bool`/`Option<String>` fields, already
//! `Serialize + Deserialize`, no `chrono`/newtype dependency):
//! `LlmConfigSnapshot`, `LlmProviderView`, `LlmActiveSelection`,
//! `LlmProbeResult`, `LlmModelsResult`.
//!
//! Per contract deviations #1/#2 (see the lane B1 plan's "Contract
//! deviations found while verifying against live code" section): request
//! bodies need local `Serialize`-only mirrors since `SetActiveLlmRequest`/
//! `LlmProbeRequest` are `Deserialize`-only on the server, and
//! `llm_list_models`/`llm_test_connection` need `adapter`/`base_url` in
//! addition to `provider_id` because `LlmProbeRequest::adapter` is
//! mandatory and isn't derivable from `provider_id` alone. Callers already
//! have `adapter` from `llm_providers()`'s `LlmProviderView::adapter`.

use ironclaw_product_workflow::{LlmConfigSnapshot, LlmModelsResult, LlmProbeResult};

use super::{ApiClient, ClientError};

#[derive(serde::Serialize)]
struct SetActiveLlmBody<'a> {
    provider_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct LlmProbeBody<'a> {
    adapter: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<&'a str>,
    provider_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
}

impl ApiClient {
    pub async fn llm_providers(&self) -> Result<LlmConfigSnapshot, ClientError> {
        self.send_json(self.http.get(self.url("/api/webchat/v2/llm/providers")))
            .await
    }

    pub async fn llm_set_active(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<LlmConfigSnapshot, ClientError> {
        self.send_json(self.http.post(self.url("/api/webchat/v2/llm/active")).json(
            &SetActiveLlmBody {
                provider_id,
                model: Some(model),
            },
        ))
        .await
    }

    pub async fn llm_list_models(
        &self,
        provider_id: &str,
        adapter: &str,
        base_url: Option<&str>,
    ) -> Result<LlmModelsResult, ClientError> {
        self.send_json(
            self.http
                .post(self.url("/api/webchat/v2/llm/list-models"))
                .json(&LlmProbeBody {
                    adapter,
                    base_url,
                    provider_id,
                    model: None,
                }),
        )
        .await
    }

    pub async fn llm_test_connection(
        &self,
        provider_id: &str,
        adapter: &str,
        base_url: Option<&str>,
    ) -> Result<LlmProbeResult, ClientError> {
        self.send_json(
            self.http
                .post(self.url("/api/webchat/v2/llm/test-connection"))
                .json(&LlmProbeBody {
                    adapter,
                    base_url,
                    provider_id,
                    model: None,
                }),
        )
        .await
    }
}
