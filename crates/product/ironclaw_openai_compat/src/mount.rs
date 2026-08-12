//! Host-facing route-mount assembly for the OpenAI-compatible surface.
//!
//! CHECKLIST WS6 (composition behavior evictions, the OpenAI-compat clause):
//! composition's `openai_compat_serve.rs` held both the *adapters* that
//! implement this crate's ports over runtime services — those name
//! `ironclaw_threads` / `ironclaw_turns` / `ironclaw_event_streams`, all on this
//! crate's own `BoundaryRule` forbidden list, so they stay in composition by
//! construction — and a residue that needs nothing but
//! `ironclaw_product_contracts` + `ironclaw_host_ingress`. That residue is this
//! module.
//!
//! What lives here, and why each piece belongs to the owner crate rather than
//! to the host:
//!
//! - **The router-state assembly.** Which workflow gets which port, in which
//!   order, and what a missing LLM-config service means for `GET /v1/models`
//!   are this surface's own wiring rules. Composition supplies the ports
//!   through [`OpenAiCompatRouteMountPorts`] and gets a
//!   [`ProtectedRouteMount`] back; it no longer knows the builder order.
//! - **The `/v1/models` catalog** (`LlmConfigModelCatalog`) — a projection of
//!   the operator LLM-config snapshot onto this crate's own
//!   [`OpenAiCompatModelEntry`], plus the error map onto this crate's own
//!   [`OpenAiCompatHttpError`]. Both sides of both mappings are owned here or
//!   in `ironclaw_product_contracts`.
//! - **The projection streamer** — a `ProductSurface` `stream_events` call plus
//!   a decode onto `ProductOutboundEnvelope`. Contracts vocabulary end to end.
//! - **[`product_surface_caller_from_openai_scope`]** — the scope→caller
//!   projection between two types this crate and `product_contracts` own. It is
//!   `pub` because composition's surviving projection readers still need it.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_ingress::ProtectedRouteMount;
use ironclaw_product_contracts::operator_llm::{
    LlmConfigService, LlmConfigServiceError, LlmConfigSnapshot,
};
use ironclaw_product_contracts::outbound::ProductOutboundEnvelope;
use ironclaw_product_contracts::surface::{
    BoundProductSurface, ProductSurface, ProductSurfaceCaller, ProductSurfaceError,
    ProductSurfaceStreamRequest,
};

use crate::{
    OpenAiChatCompletionProjectionReader, OpenAiChatCompletionsWorkflow,
    OpenAiChatProjectionStreamRequest, OpenAiCompatActorScope, OpenAiCompatAuthenticatedCaller,
    OpenAiCompatErrorKind, OpenAiCompatExternalToolResume, OpenAiCompatExternalToolStore,
    OpenAiCompatHttpError, OpenAiCompatModelCatalog, OpenAiCompatModelEntry,
    OpenAiCompatProjectionStreamer, OpenAiCompatRefStorePort, OpenAiCompatRouterState,
    OpenAiResponseProjectionStreamRequest, OpenAiResponsesProjectionReader,
    OpenAiResponsesWorkflow, openai_compat_router_with_state, openai_compat_routes,
};

/// The authority-bearing ports host composition supplies to mount this surface.
///
/// Every field is a port declared by this crate (or, for `llm_config`, by
/// `ironclaw_product_contracts`); composition owns the *implementations*,
/// because those reach into runtime services this crate must not name.
pub struct OpenAiCompatRouteMountPorts {
    /// The product boundary both workflows call.
    pub product_surface: Arc<dyn ProductSurface>,
    /// Durable public-id ↔ internal-ref store, shared by both workflows.
    pub ref_store: Arc<dyn OpenAiCompatRefStorePort>,
    /// Chat Completions projection read.
    pub chat_projection_reader: Arc<dyn OpenAiChatCompletionProjectionReader>,
    /// Responses projection read.
    pub responses_projection_reader: Arc<dyn OpenAiResponsesProjectionReader>,
    /// Client-supplied ("external") tool spec registration and output submit.
    pub external_tool_store: Arc<dyn OpenAiCompatExternalToolStore>,
    /// Resume for runs parked on a client-supplied tool call.
    pub external_tool_resume: Arc<dyn OpenAiCompatExternalToolResume>,
    /// Operator LLM configuration, backing `GET /v1/models`. `None` when the
    /// deployment has no root LLM provider wired; the route then stays
    /// fail-closed (501) rather than reporting an empty catalog.
    pub llm_config: Option<Arc<dyn LlmConfigService>>,
}

/// Assemble the OpenAI-compatible router and its ingress descriptors from
/// composition-supplied ports.
pub fn openai_compat_route_mount(ports: OpenAiCompatRouteMountPorts) -> ProtectedRouteMount {
    let OpenAiCompatRouteMountPorts {
        product_surface,
        ref_store,
        chat_projection_reader,
        responses_projection_reader,
        external_tool_store,
        external_tool_resume,
        llm_config,
    } = ports;

    let projection_streamer = Arc::new(OpenAiCompatRuntimeProjectionStreamer::new(Arc::clone(
        &product_surface,
    )));
    let chat_workflow = Arc::new(
        OpenAiChatCompletionsWorkflow::new(
            Arc::clone(&product_surface),
            Arc::clone(&ref_store),
            chat_projection_reader,
        )
        .with_projection_streamer(projection_streamer.clone()),
    );
    let responses_workflow = Arc::new(
        OpenAiResponsesWorkflow::new(product_surface, ref_store, responses_projection_reader)
            .with_projection_streamer(projection_streamer)
            .with_external_tools(external_tool_store, external_tool_resume),
    );
    let router_state = OpenAiCompatRouterState::with_chat_completions(chat_workflow)
        .with_responses_workflow(responses_workflow);
    // `GET /v1/models` lists the deployment's configured models from the same
    // LLM-config source the operator WebUI uses. Wired only when the root LLM
    // provider is compiled in; otherwise the route stays fail-closed (501).
    let router_state = match llm_config {
        Some(llm_config) => {
            let catalog: Arc<dyn OpenAiCompatModelCatalog> =
                Arc::new(LlmConfigModelCatalog::new(llm_config));
            router_state.with_models_catalog(catalog)
        }
        None => router_state,
    };
    ProtectedRouteMount::new(
        openai_compat_router_with_state(router_state),
        openai_compat_routes(),
    )
}

/// Maps an [`LlmConfigSnapshot`] to the OpenAI-compatible `/v1/models` listing.
///
/// The active selection (if any) is listed first, then every configured
/// provider's active or default model, de-duplicated by model id while
/// preserving order. Each entry's `owned_by` is the provider id.
fn model_entries_from_snapshot(snapshot: &LlmConfigSnapshot) -> Vec<OpenAiCompatModelEntry> {
    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Some(active) = &snapshot.active
        && let Some(model) = &active.model
    {
        candidates.push((model.clone(), active.provider_id.clone()));
    }
    for provider in &snapshot.providers {
        let model = provider
            .active_model
            .clone()
            .unwrap_or_else(|| provider.default_model.clone());
        candidates.push((model, provider.id.clone()));
    }
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|(id, _)| !id.is_empty() && seen.insert(id.clone()))
        .map(|(id, owner)| OpenAiCompatModelEntry::new(id).with_owner(owner))
        .collect()
}

/// [`OpenAiCompatModelCatalog`] backed by the operator LLM-config service.
/// Lists the deployment's configured models for `GET /v1/models`.
///
/// Private: its only construction site is [`openai_compat_route_mount`], which
/// builds it from [`OpenAiCompatRouteMountPorts::llm_config`]. Composition
/// supplies the *service*, never the catalog.
struct LlmConfigModelCatalog {
    llm_config: Arc<dyn LlmConfigService>,
}

impl LlmConfigModelCatalog {
    fn new(llm_config: Arc<dyn LlmConfigService>) -> Self {
        Self { llm_config }
    }
}

#[async_trait]
impl OpenAiCompatModelCatalog for LlmConfigModelCatalog {
    async fn list_models(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
    ) -> Result<Vec<OpenAiCompatModelEntry>, OpenAiCompatHttpError> {
        // The route authenticated the caller; carry its tenant/user scope into
        // the LLM-config read so the snapshot is scoped to the same identity.
        let product_caller = product_surface_caller_from_openai_scope(caller.scope());
        let snapshot = self
            .llm_config
            .snapshot(product_caller)
            .await
            .map_err(map_llm_config_error_to_openai)?;
        Ok(model_entries_from_snapshot(&snapshot))
    }
}

fn map_llm_config_error_to_openai(error: LlmConfigServiceError) -> OpenAiCompatHttpError {
    match error {
        LlmConfigServiceError::InvalidRequest { .. } => {
            OpenAiCompatHttpError::invalid_request(None)
        }
        LlmConfigServiceError::NotFound => OpenAiCompatHttpError::not_found(None),
        LlmConfigServiceError::Unavailable => OpenAiCompatHttpError::from_kind(
            503,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
        LlmConfigServiceError::Internal => OpenAiCompatHttpError::internal(),
    }
}

/// Project an authenticated OpenAI-compatible actor scope onto the product
/// boundary's caller identity.
pub fn product_surface_caller_from_openai_scope(
    scope: &OpenAiCompatActorScope,
) -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(
        scope.tenant_id().clone(),
        scope.user_id().clone(),
        scope.agent_id().cloned(),
        scope.project_id().cloned(),
    )
}

/// [`OpenAiCompatProjectionStreamer`] over the product boundary's
/// `stream_events`. "Runtime" names the host runtime whose `ProductSurface`
/// backs the stream — the name is unchanged from its composition home so the
/// eviction stays a move.
struct OpenAiCompatRuntimeProjectionStreamer {
    product_surface: Arc<dyn ProductSurface>,
}

impl OpenAiCompatRuntimeProjectionStreamer {
    fn new(product_surface: Arc<dyn ProductSurface>) -> Self {
        Self { product_surface }
    }
}

#[async_trait]
impl OpenAiCompatProjectionStreamer for OpenAiCompatRuntimeProjectionStreamer {
    async fn drain_chat(
        &self,
        request: OpenAiChatProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
        let surface = BoundProductSurface::new(
            Arc::clone(&self.product_surface),
            product_surface_caller_from_openai_scope(&request.actor_scope),
        );
        let response = surface
            .stream_events(ProductSurfaceStreamRequest {
                stream_id: Some(
                    request
                        .projection_subscription
                        .scope
                        .thread_id
                        .as_str()
                        .to_string(),
                ),
                after_cursor: request
                    .after_cursor
                    .map(|cursor| cursor.as_str().to_string()),
            })
            .await?;
        decode_product_outbound_events(response.events)
    }

    async fn drain_response(
        &self,
        request: OpenAiResponseProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
        let surface = BoundProductSurface::new(
            Arc::clone(&self.product_surface),
            product_surface_caller_from_openai_scope(&request.actor_scope),
        );
        let response = surface
            .stream_events(ProductSurfaceStreamRequest {
                stream_id: Some(
                    request
                        .projection_subscription
                        .scope
                        .thread_id
                        .as_str()
                        .to_string(),
                ),
                after_cursor: request
                    .after_cursor
                    .map(|cursor| cursor.as_str().to_string()),
            })
            .await?;
        decode_product_outbound_events(response.events)
    }
}

fn decode_product_outbound_events(
    events: Vec<serde_json::Value>,
) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
    events
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ProductSurfaceError::internal_from(error).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_contracts::operator_llm::{
        LlmActiveSelection, LlmConfigSnapshot, LlmProviderView,
    };

    fn provider_view(
        id: &str,
        default_model: &str,
        active: bool,
        active_model: Option<&str>,
    ) -> LlmProviderView {
        LlmProviderView {
            id: id.to_string(),
            description: String::new(),
            adapter: "open_ai_completions".to_string(),
            default_model: default_model.to_string(),
            base_url: None,
            builtin: true,
            active,
            active_model: active_model.map(str::to_string),
            api_key_required: false,
            accepts_api_key: true,
            api_key_set: false,
            can_list_models: true,
        }
    }

    #[test]
    fn model_entries_list_active_first_then_providers_deduped() {
        let snapshot = LlmConfigSnapshot {
            providers: vec![
                provider_view("openai", "gpt-4o", true, Some("gpt-4o")),
                provider_view("anthropic", "claude-opus-4", false, None),
                // Duplicate model id (same default) must not be listed twice.
                provider_view("openai-mirror", "gpt-4o", false, None),
            ],
            active: Some(LlmActiveSelection {
                provider_id: "openai".to_string(),
                model: Some("gpt-4o".to_string()),
            }),
            user_model_policy: None,
        };

        let entries = model_entries_from_snapshot(&snapshot);

        assert_eq!(entries.len(), 2, "duplicate model id must be de-duplicated");
        assert_eq!(entries[0].id, "gpt-4o", "active selection listed first");
        assert_eq!(entries[0].owned_by.as_deref(), Some("openai"));
        assert_eq!(entries[1].id, "claude-opus-4");
        assert_eq!(entries[1].owned_by.as_deref(), Some("anthropic"));
    }

    #[test]
    fn model_entries_fall_back_to_default_model_when_no_active_selection() {
        let snapshot = LlmConfigSnapshot {
            providers: vec![provider_view("anthropic", "claude-opus-4", false, None)],
            active: None,
            user_model_policy: None,
        };

        let entries = model_entries_from_snapshot(&snapshot);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "claude-opus-4");
        assert_eq!(entries[0].owned_by.as_deref(), Some("anthropic"));
    }

    #[test]
    fn llm_config_errors_map_onto_the_openai_error_envelope() {
        // The `/v1/models` catalog is the one place an operator-service error
        // reaches an OpenAI client. Each arm keeps its status: a bad request is
        // 400, a missing config 404, an unavailable service a *retryable* 503,
        // and an internal fault 500 with no operator detail leaking.
        assert_eq!(
            map_llm_config_error_to_openai(LlmConfigServiceError::InvalidRequest {
                field: None,
                reason: "bad".to_string(),
            })
            .status_code(),
            400
        );
        assert_eq!(
            map_llm_config_error_to_openai(LlmConfigServiceError::NotFound).status_code(),
            404
        );
        let unavailable = map_llm_config_error_to_openai(LlmConfigServiceError::Unavailable);
        assert_eq!(unavailable.status_code(), 503);
        assert!(
            unavailable.retryable(),
            "an unavailable LLM-config service is a retryable 503"
        );
        assert_eq!(
            map_llm_config_error_to_openai(LlmConfigServiceError::Internal).status_code(),
            500
        );
    }
}
