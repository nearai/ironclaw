use ironclaw_host_api::NetworkMethod;

use crate::{
    error::NetworkHttpError,
    policy::StaticNetworkPolicyEnforcer,
    resolver::{NetworkResolver, SystemNetworkResolver, resolve_public_ips},
    transport::reject_caller_host_header,
    types::{NetworkHttpRequest, NetworkHttpResponse, NetworkRequest, NetworkTransportRequest},
    url_target::network_target_for_url,
};

pub trait NetworkHttpEgress: Send + Sync {
    fn execute(&self, request: NetworkHttpRequest)
    -> Result<NetworkHttpResponse, NetworkHttpError>;
}

pub trait NetworkHttpTransport: Send + Sync {
    fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError>;
}

#[derive(Debug, Clone)]
pub struct PolicyNetworkHttpEgress<T, R = SystemNetworkResolver> {
    transport: T,
    resolver: R,
}

impl<T> PolicyNetworkHttpEgress<T, SystemNetworkResolver> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            resolver: SystemNetworkResolver,
        }
    }
}

impl<T, R> PolicyNetworkHttpEgress<T, R> {
    pub fn new_with_resolver(transport: T, resolver: R) -> Self {
        Self {
            transport,
            resolver,
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T, R> NetworkHttpEgress for PolicyNetworkHttpEgress<T, R>
where
    T: NetworkHttpTransport,
    R: NetworkResolver,
{
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let estimated_request_bytes = estimate_http_request_bytes(
            request.method,
            &request.url,
            &request.headers,
            &request.body,
        );
        reject_caller_host_header(&request.headers)?;
        let target = network_target_for_url(&request.url, estimated_request_bytes)?;
        let permit = StaticNetworkPolicyEnforcer::new(request.policy.clone())
            .authorize_blocking(NetworkRequest {
                scope: request.scope,
                target: target.clone(),
                method: request.method,
                estimated_bytes: Some(estimated_request_bytes),
            })
            .map_err(|error| NetworkHttpError::PolicyDenied {
                reason: error.to_string(),
                request_bytes: estimated_request_bytes,
                response_bytes: 0,
            })?;
        let resolved_ips = resolve_public_ips(
            &target,
            &request.policy,
            &self.resolver,
            estimated_request_bytes,
        )?;
        let transport_request = NetworkTransportRequest {
            method: permit.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
            resolved_ips,
            response_body_limit: request.response_body_limit,
            timeout_ms: request.timeout_ms,
        };
        self.transport.execute(transport_request)
    }
}

fn estimate_http_request_bytes(
    method: NetworkMethod,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> u64 {
    let mut total = 0_u64;
    add_len(&mut total, method_label(method).len());
    add_len(&mut total, " ".len());
    add_len(&mut total, url.len());
    add_len(&mut total, " HTTP/1.1\r\n".len());
    for (name, value) in headers {
        add_len(&mut total, name.len());
        add_len(&mut total, ": ".len());
        add_len(&mut total, value.len());
        add_len(&mut total, "\r\n".len());
    }
    add_len(&mut total, "\r\n".len());
    add_len(&mut total, body.len());
    total
}

fn add_len(total: &mut u64, len: usize) {
    *total = total.saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
}

fn method_label(method: NetworkMethod) -> &'static str {
    match method {
        NetworkMethod::Get => "GET",
        NetworkMethod::Post => "POST",
        NetworkMethod::Put => "PUT",
        NetworkMethod::Patch => "PATCH",
        NetworkMethod::Delete => "DELETE",
        NetworkMethod::Head => "HEAD",
    }
}
