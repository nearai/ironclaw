//! Reverse proxy handler for exposed container ports.

use std::sync::Arc;

use axum::{
    extract::{OriginalUri, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;

#[derive(Debug, Deserialize)]
pub struct ProxyQuery {
    pub port: Option<u16>,
}

/// Reverse-proxy handler: `ANY /api/jobs/{id}/proxy/{*path}`.
///
/// Looks up the job's exposed port mappings via `ContainerPortResolver`,
/// then proxies the request to the container. The path segment after
/// `/proxy/` is forwarded to the container as-is. Query string `?port=N`
/// selects a specific container port (defaults to the first exposed port).
///
/// Requires bearer auth (same as all protected routes).
#[allow(clippy::too_many_arguments)]
pub async fn jobs_proxy_handler(
    State(state): State<Arc<GatewayState>>,
    Path((job_id_str, proxy_path)): Path<(String, String)>,
    Query(query): Query<ProxyQuery>,
    OriginalUri(orig_uri): OriginalUri,
    user: AuthenticatedUser,
    method: Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let job_id = match job_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid job ID").into_response();
        }
    };

    // Ownership check: user must own the job.
    if let Some(store) = &state.store {
        match store.sandbox_job_belongs_to_user(job_id, &user.0.user_id).await {
            Ok(false) | Err(_) => {
                return (StatusCode::NOT_FOUND, "job not found").into_response();
            }
            Ok(true) => {}
        }
    }

    // Resolve exposed ports from the in-memory container handle.
    let Some(resolver) = &state.port_resolver else {
        return (StatusCode::NOT_FOUND, "port resolver not available").into_response();
    };

    let Some(ports) = resolver.exposed_ports(job_id).await else {
        return (StatusCode::NOT_FOUND, "no exposed ports for this job").into_response();
    };

    // Select the target host port: if ?port=N is given, find the matching
    // container port; otherwise use the first exposed port.
    let target_port = if let Some(container_port) = query.port {
        match ports.iter().find(|p| p.container_port == container_port) {
            Some(ep) => ep.host_port,
            None => {
                return (StatusCode::NOT_FOUND, "requested container port is not exposed")
                    .into_response();
            }
        }
    } else {
        ports[0].host_port
    };

    // Build the proxied URL. Strip the `/api/jobs/{id}/proxy/` prefix
    // and forward the remaining path + query string to the container.
    let path_clean = proxy_path.trim_start_matches('/');
    let qs = orig_uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("http://127.0.0.1:{target_port}/{path_clean}{qs}");

    let req = reqwest::Client::new()
        .request(method, &url)
        .headers(filter_proxy_headers(&headers))
        .body(body);

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(job_id = %job_id, error = %e, "Proxy request to container failed");
            return (StatusCode::BAD_GATEWAY, "container unreachable").into_response();
        }
    };

    let status = resp.status();
    let mut response_headers = HeaderMap::new();
    for (key, value) in resp.headers() {
        if key == header::CONNECTION
            || key == header::TRANSFER_ENCODING
            || key == header::UPGRADE
        {
            continue;
        }
        if let Ok(v) = HeaderValue::from_bytes(value.as_bytes()) {
            response_headers.insert(key.clone(), v);
        }
    }

    let body_bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to read proxy response body");
            return (StatusCode::BAD_GATEWAY, "container response incomplete").into_response();
        }
    };

    (status, response_headers, body_bytes.to_vec()).into_response()
}

/// Filter headers for the outbound proxy request, stripping hop-by-hop
/// and headers that should be recomputed by the proxy layer.
fn filter_proxy_headers(incoming: &HeaderMap) -> HeaderMap {
    let skip: [header::HeaderName; 9] = [
        header::HOST,
        header::CONNECTION,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::CONTENT_LENGTH,
    ];
    let mut out = HeaderMap::new();
    for (key, value) in incoming.iter() {
        if skip.iter().any(|s| key == s) {
            continue;
        }
        if let Ok(v) = HeaderValue::from_bytes(value.as_bytes()) {
            out.insert(key.clone(), v);
        }
    }
    out
}
