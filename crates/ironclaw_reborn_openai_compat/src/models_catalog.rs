//! Host-supplied model catalog port for `GET /v1/models`.
//!
//! The OpenAI-compatible surface exposes the deployment's configured models so
//! standard clients (OpenWebUI, LangChain, etc.) can populate model pickers.
//! The route crate must not reach into `ironclaw_llm` or the runtime directly
//! (enforced by `reborn_dependency_boundaries`), so — mirroring the projection
//! reader/streamer ports — it defines this trait and host composition injects an
//! implementation backed by the runtime's LLM configuration.

use async_trait::async_trait;

use crate::{
    OpenAiCompatAuthenticatedCaller, OpenAiCompatHttpError, OpenAiModelListResponse,
    OpenAiModelObject, unix_timestamp_now,
};

/// Default `owned_by` value when the catalog does not attribute a model to a
/// specific owner. Mirrors the v1 OpenAI-compatible proxy.
const OPENAI_COMPAT_MODEL_OWNER: &str = "ironclaw";

/// A model the deployment exposes to OpenAI-compatible clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatModelEntry {
    pub id: String,
    pub owned_by: Option<String>,
}

impl OpenAiCompatModelEntry {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            owned_by: None,
        }
    }

    pub fn with_owner(mut self, owned_by: impl Into<String>) -> Self {
        self.owned_by = Some(owned_by.into());
        self
    }
}

/// Host-supplied catalog of models surfaced through `GET /v1/models`.
///
/// Implemented by host composition over the runtime's LLM configuration. The
/// caller scope is provided so an implementation may scope the listing to the
/// authenticated tenant/user; the route enforces authentication before calling.
#[async_trait]
pub trait OpenAiCompatModelCatalog: Send + Sync {
    async fn list_models(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
    ) -> Result<Vec<OpenAiCompatModelEntry>, OpenAiCompatHttpError>;
}

/// Build the OpenAI-compatible list envelope from catalog entries, stamping a
/// shared `created` timestamp and the `model` / `list` object markers.
pub(crate) fn model_list_response(entries: Vec<OpenAiCompatModelEntry>) -> OpenAiModelListResponse {
    let created = unix_timestamp_now();
    OpenAiModelListResponse {
        object: "list".to_string(),
        data: entries
            .into_iter()
            .map(|entry| OpenAiModelObject {
                id: entry.id,
                object: "model".to_string(),
                created,
                owned_by: entry
                    .owned_by
                    .unwrap_or_else(|| OPENAI_COMPAT_MODEL_OWNER.to_string()),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_list_envelope_with_model_markers_and_default_owner() {
        let response = model_list_response(vec![
            OpenAiCompatModelEntry::new("gpt-reborn"),
            OpenAiCompatModelEntry::new("claude").with_owner("anthropic"),
        ]);
        assert_eq!(response.object, "list");
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].id, "gpt-reborn");
        assert_eq!(response.data[0].object, "model");
        assert_eq!(response.data[0].owned_by, "ironclaw");
        assert_eq!(response.data[1].owned_by, "anthropic");
        // All entries share the single stamped timestamp.
        assert_eq!(response.data[0].created, response.data[1].created);
    }

    #[test]
    fn empty_catalog_yields_empty_list() {
        let response = model_list_response(Vec::new());
        assert_eq!(response.object, "list");
        assert!(response.data.is_empty());
    }
}
