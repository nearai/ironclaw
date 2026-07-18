//! Descriptor-driven operator WebUI route authorization.
//!
//! The WebUI v2 descriptors are the route-policy contract. This module
//! derives the operator-only request matcher from descriptors so auth
//! enforcement cannot drift from the route table mounted by composition.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::Method;
use ironclaw_host_api::ingress::IngressRouteDescriptor;

use crate::webui_route_match::{network_method_to_axum, parse_pattern, segments_match};

#[derive(Debug, Clone)]
struct OperatorWebuiConfigRoute {
    method: Method,
    segments: Vec<Option<String>>,
}

/// Shared state for checking whether an authenticated request targets an
/// operator-only WebUI configuration route.
#[derive(Clone, Default)]
pub(crate) struct OperatorWebuiConfigRouteState {
    routes: Arc<Vec<OperatorWebuiConfigRoute>>,
}

impl std::fmt::Debug for OperatorWebuiConfigRouteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperatorWebuiConfigRouteState")
            .field("routes", &self.routes.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn build_operator_webui_config_route_state(
    descriptors: &[IngressRouteDescriptor],
) -> OperatorWebuiConfigRouteState {
    let routes = descriptors
        .iter()
        .map(|descriptor| OperatorWebuiConfigRoute {
            method: network_method_to_axum(descriptor.method()),
            segments: parse_pattern(descriptor.route_pattern().as_str()),
        })
        .collect();
    OperatorWebuiConfigRouteState {
        routes: Arc::new(routes),
    }
}

impl OperatorWebuiConfigRouteState {
    pub(crate) fn requires_operator_webui_config(&self, request: &Request) -> bool {
        let method = request.method();
        let path = request.uri().path();
        self.routes.iter().any(|route| {
            method_matches_route(&route.method, method) && segments_match(&route.segments, path)
        })
    }
}

fn method_matches_route(route_method: &Method, request_method: &Method) -> bool {
    route_method == request_method
        || (request_method == Method::HEAD && route_method == Method::GET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webui_v2::is_webui_v2_operator_webui_config_route_id;
    use axum::body::Body;
    use axum::http::Method;

    fn request(method: Method, path: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("request")
    }

    #[test]
    fn operator_routes_are_matched_from_descriptors() {
        let descriptors: Vec<_> = crate::webui_v2::webui_v2_routes()
            .into_iter()
            .filter(|descriptor| {
                is_webui_v2_operator_webui_config_route_id(descriptor.route_id().as_str())
            })
            .collect();
        let state = build_operator_webui_config_route_state(&descriptors);

        assert!(state.requires_operator_webui_config(&request(
            Method::GET,
            "/api/webchat/v2/llm/providers",
        )));
        assert!(state.requires_operator_webui_config(&request(
            Method::HEAD,
            "/api/webchat/v2/llm/providers",
        )));
        assert!(state.requires_operator_webui_config(&request(
            Method::POST,
            "/api/webchat/v2/llm/providers/openai/delete",
        )));
        assert!(!state.requires_operator_webui_config(&request(
            Method::GET,
            "/api/webchat/v2/llm/providers/openai/delete",
        )));
        assert!(
            !state
                .requires_operator_webui_config(&request(Method::POST, "/api/webchat/v2/threads",))
        );
        assert!(
            !state
                .requires_operator_webui_config(&request(Method::HEAD, "/api/webchat/v2/threads",))
        );
    }
}
