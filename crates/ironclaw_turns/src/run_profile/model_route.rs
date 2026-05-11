//! Model route types for resolving concrete provider+model routes from abstract model slots.
//!
//! The Reborn authority boundary requires that drivers request a model slot (purpose),
//! and only host/admin policy resolves it to a concrete route. This module defines
//! the types and resolution logic for that boundary.

use serde::{Deserialize, Serialize};

use super::refs::{ModelId, ProviderId};

/// A logical model slot representing the *purpose* of a model request.
///
/// Drivers request a slot; the host resolves it to a concrete [`ModelRoute`].
/// `#[non_exhaustive]` so future variants (`Cheap`, `Mission`, `Vision`) can
/// be added without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModelSlot {
    /// The default model slot used for general-purpose requests.
    Default,
}

/// A concrete provider+model route that identifies a specific LLM endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRoute {
    /// The provider to route to (e.g. `"openai"`, `"anthropic"`, `"ollama"`).
    pub provider_id: ProviderId,
    /// The model to use within that provider (e.g. `"gpt-4o"`, `"claude-sonnet-4-20250514"`).
    pub model_id: ModelId,
}

/// Policy governing which model routes a given actor may select.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModelSelectionPolicy {
    /// Only host-managed routes are allowed; user-provided routes are rejected.
    ManagedOnly,
    /// Users may select from an explicit allowlist of routes.
    UserSelectableAllowlist {
        /// The set of routes users are permitted to choose.
        allowed: Vec<ModelRoute>,
    },
    /// Developers may use any configured provider+model route.
    DeveloperAnyConfigured,
}

/// A resolved model route snapshot — the result of policy resolution.
///
/// Included in [`super::ResolvedRunProfile`] so that settings changes
/// don't affect in-progress runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedModelRoute {
    /// The slot that was resolved.
    pub slot: ModelSlot,
    /// The concrete route the slot resolved to.
    pub route: ModelRoute,
}

/// Errors that can occur during model route resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ModelRouteResolutionError {
    /// The requested route is not permitted by the current selection policy.
    #[error("route not allowed by selection policy")]
    RouteNotAllowed,
    /// The requested route references a provider or model that is not configured.
    #[error("route references an unconfigured provider or model")]
    RouteNotConfigured,
    /// No route is configured for the requested slot.
    #[error("no route configured for the requested slot")]
    SlotUnconfigured,
    /// The route specification is structurally invalid.
    #[error("invalid route specification")]
    InvalidRoute,
}

/// Resolves a [`ModelSlot`] to a concrete [`ResolvedModelRoute`] given a
/// requested route and the current selection policy.
///
/// Resolution is synchronous — it is pure in-memory policy logic with no I/O,
/// matching the sync pattern of [`super::RunProfileDefinition::resolve()`].
pub trait ModelRouteResolver: Send + Sync {
    /// Resolve a model slot to a concrete route.
    ///
    /// # Arguments
    /// * `slot` — the logical slot the driver is requesting
    /// * `requested_route` — the user/driver's preferred route, if any
    /// * `policy` — the selection policy governing which routes are allowed
    /// * `configured_default` — the host-configured default route for this slot, if any
    fn resolve_model_route(
        &self,
        slot: &ModelSlot,
        requested_route: Option<&ModelRoute>,
        policy: &ModelSelectionPolicy,
        configured_default: Option<&ModelRoute>,
    ) -> Result<ResolvedModelRoute, ModelRouteResolutionError>;
}

/// In-memory implementation of [`ModelRouteResolver`] that evaluates the three
/// policy modes without any I/O.
#[derive(Debug, Clone, Default)]
pub struct InMemoryModelRouteResolver;

impl ModelRouteResolver for InMemoryModelRouteResolver {
    fn resolve_model_route(
        &self,
        slot: &ModelSlot,
        requested_route: Option<&ModelRoute>,
        policy: &ModelSelectionPolicy,
        configured_default: Option<&ModelRoute>,
    ) -> Result<ResolvedModelRoute, ModelRouteResolutionError> {
        let route = match policy {
            ModelSelectionPolicy::ManagedOnly => {
                // Reject any user-provided route; only the configured default is valid.
                if requested_route.is_some() {
                    return Err(ModelRouteResolutionError::RouteNotAllowed);
                }
                configured_default
                    .ok_or(ModelRouteResolutionError::SlotUnconfigured)?
                    .clone()
            }
            ModelSelectionPolicy::UserSelectableAllowlist { allowed } => {
                if let Some(requested) = requested_route {
                    if allowed.contains(requested) {
                        requested.clone()
                    } else {
                        return Err(ModelRouteResolutionError::RouteNotAllowed);
                    }
                } else {
                    configured_default
                        .ok_or(ModelRouteResolutionError::SlotUnconfigured)?
                        .clone()
                }
            }
            ModelSelectionPolicy::DeveloperAnyConfigured => {
                if let Some(requested) = requested_route {
                    requested.clone()
                } else {
                    configured_default
                        .ok_or(ModelRouteResolutionError::SlotUnconfigured)?
                        .clone()
                }
            }
        };

        Ok(ResolvedModelRoute {
            slot: slot.clone(),
            route,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(provider: &str, model: &str) -> ModelRoute {
        ModelRoute {
            provider_id: ProviderId::new(provider).unwrap(),
            model_id: ModelId::new(model).unwrap(),
        }
    }

    #[test]
    fn developer_any_configured_accepts_any_route() {
        let resolver = InMemoryModelRouteResolver;
        let requested = route("openai", "gpt-4o");
        let result = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                Some(&requested),
                &ModelSelectionPolicy::DeveloperAnyConfigured,
                Some(&route("anthropic", "claude-sonnet")),
            )
            .unwrap();
        assert_eq!(result.slot, ModelSlot::Default);
        assert_eq!(result.route, requested);
    }

    #[test]
    fn developer_any_configured_falls_back_to_default() {
        let resolver = InMemoryModelRouteResolver;
        let default = route("anthropic", "claude-sonnet");
        let result = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                None,
                &ModelSelectionPolicy::DeveloperAnyConfigured,
                Some(&default),
            )
            .unwrap();
        assert_eq!(result.route, default);
    }

    #[test]
    fn managed_only_rejects_user_routes() {
        let resolver = InMemoryModelRouteResolver;
        let err = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                Some(&route("openai", "gpt-4o")),
                &ModelSelectionPolicy::ManagedOnly,
                Some(&route("anthropic", "claude-sonnet")),
            )
            .unwrap_err();
        assert_eq!(err, ModelRouteResolutionError::RouteNotAllowed);
    }

    #[test]
    fn managed_only_uses_configured_default() {
        let resolver = InMemoryModelRouteResolver;
        let default = route("anthropic", "claude-sonnet");
        let result = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                None,
                &ModelSelectionPolicy::ManagedOnly,
                Some(&default),
            )
            .unwrap();
        assert_eq!(result.route, default);
    }

    #[test]
    fn allowlist_accepts_listed_routes() {
        let resolver = InMemoryModelRouteResolver;
        let allowed_route = route("openai", "gpt-4o");
        let result = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                Some(&allowed_route),
                &ModelSelectionPolicy::UserSelectableAllowlist {
                    allowed: vec![allowed_route.clone(), route("anthropic", "claude-sonnet")],
                },
                None,
            )
            .unwrap();
        assert_eq!(result.route, allowed_route);
    }

    #[test]
    fn allowlist_rejects_unlisted_routes() {
        let resolver = InMemoryModelRouteResolver;
        let err = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                Some(&route("ollama", "llama3")),
                &ModelSelectionPolicy::UserSelectableAllowlist {
                    allowed: vec![route("openai", "gpt-4o")],
                },
                None,
            )
            .unwrap_err();
        assert_eq!(err, ModelRouteResolutionError::RouteNotAllowed);
    }

    #[test]
    fn slot_unconfigured_when_no_default_and_no_request() {
        let resolver = InMemoryModelRouteResolver;
        let err = resolver
            .resolve_model_route(
                &ModelSlot::Default,
                None,
                &ModelSelectionPolicy::DeveloperAnyConfigured,
                None,
            )
            .unwrap_err();
        assert_eq!(err, ModelRouteResolutionError::SlotUnconfigured);
    }

    #[test]
    fn resolved_model_route_round_trips_through_serde() {
        let resolved = ResolvedModelRoute {
            slot: ModelSlot::Default,
            route: route("openai", "gpt-4o"),
        };
        let json = serde_json::to_string(&resolved).unwrap();
        let deserialized: ResolvedModelRoute = serde_json::from_str(&json).unwrap();
        assert_eq!(resolved, deserialized);
    }
}
