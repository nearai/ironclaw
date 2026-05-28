use ironclaw_host_api::{
    NetworkPolicy, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    RuntimeHttpSaveTarget,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpRequest};
use ironclaw_secrets::SecretStore;

use super::{HostHttpEgressService, PipelineError, runtime_network_error, runtime_response};
use crate::http_body;

pub(super) fn execute<N, S>(
    service: &HostHttpEgressService<N, S>,
    mut request: RuntimeHttpEgressRequest,
) -> Result<RuntimeHttpEgressResponse, PipelineError>
where
    N: NetworkHttpEgress,
    S: SecretStore,
{
    let network_policy = service.network_policy_for_request(&mut request)?;
    service.validate_credential_sources_for_request(&request)?;
    let save_body_to = authorize_body_store(service, &mut request)?;
    super::sanitize::validate_runtime_request(&request)?;
    let scope = request.scope.clone();
    let capability_id = request.capability_id.clone();

    let redaction_values = super::credential::apply_credential_injections(
        &service.secrets,
        service.secret_injections(),
        service.allows_direct_secret_lease(),
        &mut request,
    )?;

    let response = dispatch_network(service, request, network_policy)?;
    let credentials_injected = !redaction_values.is_empty();
    let (response, response_redacted) =
        super::sanitize::sanitize_runtime_response(response, &redaction_values)
            .map_err(PipelineError::post_transport)?;
    let (response, saved_body) = http_body::apply_body_disposition(
        response,
        save_body_to,
        service.body_store.as_deref(),
        &scope,
        &capability_id,
    )
    .map_err(PipelineError::post_transport)?;
    Ok(runtime_response(
        response,
        credentials_injected || response_redacted,
        saved_body,
    ))
}

fn authorize_body_store<N, S>(
    service: &HostHttpEgressService<N, S>,
    request: &mut RuntimeHttpEgressRequest,
) -> Result<Option<RuntimeHttpSaveTarget>, PipelineError> {
    let save_body_to = std::mem::take(&mut request.save_body_to);
    if let Some(target) = &save_body_to {
        let Some(store) = service.body_store.as_deref() else {
            return Err(PipelineError::pre_transport(
                RuntimeHttpEgressError::Request {
                    reason: "response_body_store_unavailable".to_string(),
                    request_bytes: 0,
                    response_bytes: 0,
                },
            ));
        };
        if let Err(error) = store.authorize_write(&request.scope, &request.capability_id, target) {
            return Err(PipelineError::pre_transport(
                RuntimeHttpEgressError::Request {
                    reason: format!("response_body_store_unauthorized: {}", error.reason),
                    request_bytes: 0,
                    response_bytes: 0,
                },
            ));
        }
    }
    Ok(save_body_to)
}

fn dispatch_network<N, S>(
    service: &HostHttpEgressService<N, S>,
    request: RuntimeHttpEgressRequest,
    network_policy: NetworkPolicy,
) -> Result<ironclaw_network::NetworkHttpResponse, PipelineError>
where
    N: NetworkHttpEgress,
{
    service
        .network
        .execute(NetworkHttpRequest {
            scope: request.scope,
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
            policy: network_policy,
            response_body_limit: request.response_body_limit,
            timeout_ms: request.timeout_ms,
        })
        .map_err(|error| {
            PipelineError::post_transport(runtime_network_error(
                service.unsafe_raw_diagnostics_allowed,
                error,
            ))
        })
}
