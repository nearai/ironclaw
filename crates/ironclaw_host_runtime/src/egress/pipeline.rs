use ironclaw_host_api::{
    NetworkPolicy, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    RuntimeHttpSaveTarget,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpRequest};
use ironclaw_secrets::SecretStore;

use super::{HostHttpEgressService, PipelineError, runtime_network_error, runtime_response};
use crate::{
    http_body::{
        self, RESPONSE_BODY_STORE_UNAUTHORIZED_REASON, RESPONSE_BODY_STORE_UNAVAILABLE_REASON,
        RuntimeHttpBodyStoreError,
    },
    latency::{
        RuntimeLatencyFields, RuntimeLatencyMetrics, started_at as latency_started_at,
        trace_runtime_error, trace_runtime_ok,
    },
};

type HttpEgressLatencyFields = RuntimeLatencyFields;

fn http_egress_latency_fields(
    request: &RuntimeHttpEgressRequest,
    allow_partial_response_body: bool,
) -> Option<HttpEgressLatencyFields> {
    if !ironclaw_observability::live_latency_enabled() {
        return None;
    }

    RuntimeLatencyFields::from_scope(
        &request.capability_id,
        &request.scope,
        request.runtime.as_str(),
        0,
    )
    .map(|fields| {
        fields.with_http_details(
            request.method.to_string(),
            request.body.len() as u64,
            request.response_body_limit.unwrap_or(0),
            request.credential_injections.len(),
            request.save_body_to.is_some(),
            allow_partial_response_body,
        )
    })
}

fn trace_http_egress_latency_ok(
    operation: &'static str,
    fields: Option<&HttpEgressLatencyFields>,
    started_at: Option<std::time::Instant>,
    request_bytes: u64,
    response_bytes: u64,
) {
    trace_runtime_ok(
        "runtime_http_egress",
        operation,
        fields,
        started_at,
        RuntimeLatencyMetrics {
            request_bytes,
            response_bytes,
            ..RuntimeLatencyMetrics::default()
        },
    );
}

fn trace_http_egress_latency_error(
    operation: &'static str,
    fields: Option<&HttpEgressLatencyFields>,
    started_at: Option<std::time::Instant>,
    error_kind: &str,
) {
    trace_runtime_error(
        "runtime_http_egress",
        operation,
        fields,
        started_at,
        error_kind,
        RuntimeLatencyMetrics::default(),
    );
}

pub(super) async fn execute<N, S>(
    service: &HostHttpEgressService<N, S>,
    request: RuntimeHttpEgressRequest,
) -> Result<RuntimeHttpEgressResponse, PipelineError>
where
    N: NetworkHttpEgress + Send + Sync,
    S: SecretStore + Send + Sync,
{
    execute_inner(service, request, false).await
}

pub(super) async fn execute_for_model_visible_output<N, S>(
    service: &HostHttpEgressService<N, S>,
    request: RuntimeHttpEgressRequest,
) -> Result<RuntimeHttpEgressResponse, PipelineError>
where
    N: NetworkHttpEgress + Send + Sync,
    S: SecretStore + Send + Sync,
{
    execute_inner(service, request, true).await
}

async fn execute_inner<N, S>(
    service: &HostHttpEgressService<N, S>,
    mut request: RuntimeHttpEgressRequest,
    allow_partial_response_body: bool,
) -> Result<RuntimeHttpEgressResponse, PipelineError>
where
    N: NetworkHttpEgress + Send + Sync,
    S: SecretStore + Send + Sync,
{
    let latency_fields = http_egress_latency_fields(&request, allow_partial_response_body);
    let policy_started_at = latency_started_at();
    let network_policy = match service.network_policy_for_request(&mut request) {
        Ok(network_policy) => {
            trace_http_egress_latency_ok(
                "policy_lookup",
                latency_fields.as_ref(),
                policy_started_at,
                0,
                0,
            );
            network_policy
        }
        Err(error) => {
            trace_http_egress_latency_error(
                "policy_lookup",
                latency_fields.as_ref(),
                policy_started_at,
                error.stable_runtime_reason(),
            );
            return Err(error);
        }
    };

    let credential_validation_started_at = latency_started_at();
    if let Err(error) = service.validate_credential_sources_for_request(&request) {
        trace_http_egress_latency_error(
            "credential_validation",
            latency_fields.as_ref(),
            credential_validation_started_at,
            error.stable_runtime_reason(),
        );
        return Err(error);
    }
    trace_http_egress_latency_ok(
        "credential_validation",
        latency_fields.as_ref(),
        credential_validation_started_at,
        0,
        0,
    );

    let body_store_started_at = latency_started_at();
    let save_body_to = match authorize_body_store(service, &mut request) {
        Ok(save_body_to) => {
            trace_http_egress_latency_ok(
                "body_store_authorize",
                latency_fields.as_ref(),
                body_store_started_at,
                0,
                0,
            );
            save_body_to
        }
        Err(error) => {
            trace_http_egress_latency_error(
                "body_store_authorize",
                latency_fields.as_ref(),
                body_store_started_at,
                error.stable_runtime_reason(),
            );
            return Err(error);
        }
    };

    let sanitize_request_started_at = latency_started_at();
    if let Err(error) = super::sanitize::validate_runtime_request(&request, service.leak_detector())
    {
        trace_http_egress_latency_error(
            "sanitize_request",
            latency_fields.as_ref(),
            sanitize_request_started_at,
            error.stable_runtime_reason(),
        );
        return Err(PipelineError::pre_transport(error));
    }
    trace_http_egress_latency_ok(
        "sanitize_request",
        latency_fields.as_ref(),
        sanitize_request_started_at,
        0,
        0,
    );

    let scope = request.scope.clone();
    let capability_id = request.capability_id.clone();
    let apply_credentials_started_at = latency_started_at();
    let redaction_values = match super::credential::apply_credential_injections(
        service.secrets(),
        service.secret_injections(),
        &mut request,
    ) {
        Ok(redaction_values) => {
            trace_http_egress_latency_ok(
                "apply_credentials",
                latency_fields.as_ref(),
                apply_credentials_started_at,
                0,
                0,
            );
            redaction_values
        }
        Err(error) => {
            trace_http_egress_latency_error(
                "apply_credentials",
                latency_fields.as_ref(),
                apply_credentials_started_at,
                error.stable_runtime_reason(),
            );
            return Err(PipelineError::pre_transport_keep_staged_secrets(error));
        }
    };

    let transport_started_at = latency_started_at();
    let response = if allow_partial_response_body {
        dispatch_network_for_model_visible_output(service, request, network_policy).await
    } else {
        dispatch_network(service, request, network_policy).await
    };
    let response = match response {
        Ok(response) => {
            trace_http_egress_latency_ok(
                "network_transport",
                latency_fields.as_ref(),
                transport_started_at,
                0,
                response.body.len() as u64,
            );
            response
        }
        Err(error) => {
            trace_http_egress_latency_error(
                "network_transport",
                latency_fields.as_ref(),
                transport_started_at,
                error.stable_runtime_reason(),
            );
            return Err(error);
        }
    };

    let credentials_injected = !redaction_values.is_empty();
    let sanitize_response_started_at = latency_started_at();
    let (response, response_redacted) = match super::sanitize::sanitize_runtime_response(
        response,
        &redaction_values,
        service.leak_detector(),
    ) {
        Ok(result) => {
            trace_http_egress_latency_ok(
                "sanitize_response",
                latency_fields.as_ref(),
                sanitize_response_started_at,
                0,
                result.0.body.len() as u64,
            );
            result
        }
        Err(error) => {
            trace_http_egress_latency_error(
                "sanitize_response",
                latency_fields.as_ref(),
                sanitize_response_started_at,
                error.stable_runtime_reason(),
            );
            return Err(PipelineError::post_transport(error));
        }
    };

    let body_disposition_started_at = latency_started_at();
    let (response, saved_body) = match http_body::apply_body_disposition(
        response,
        save_body_to,
        service.body_store(),
        &scope,
        &capability_id,
    ) {
        Ok(result) => {
            let output_bytes = result
                .1
                .as_ref()
                .map_or(result.0.body.len() as u64, |saved_body| {
                    saved_body.bytes_written
                });
            trace_http_egress_latency_ok(
                "body_disposition",
                latency_fields.as_ref(),
                body_disposition_started_at,
                0,
                output_bytes,
            );
            result
        }
        Err(error) => {
            trace_http_egress_latency_error(
                "body_disposition",
                latency_fields.as_ref(),
                body_disposition_started_at,
                error.stable_runtime_reason(),
            );
            return Err(PipelineError::post_transport(error));
        }
    };
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
    if let Some(target) = &save_body_to
        && let Err(error) =
            service
                .body_store()
                .authorize_write(&request.scope, &request.capability_id, target)
    {
        tracing::debug!(
            error = %error,
            capability_id = %request.capability_id,
            "runtime HTTP response body store authorization failed"
        );
        let reason = match error {
            RuntimeHttpBodyStoreError::Unavailable => {
                RESPONSE_BODY_STORE_UNAVAILABLE_REASON.to_string()
            }
            RuntimeHttpBodyStoreError::Unauthorized { .. }
            | RuntimeHttpBodyStoreError::Failed { .. } => {
                RESPONSE_BODY_STORE_UNAUTHORIZED_REASON.to_string()
            }
        };
        return Err(PipelineError::pre_transport(
            RuntimeHttpEgressError::Request {
                reason,
                request_bytes: 0,
                response_bytes: 0,
            },
        ));
    }
    Ok(save_body_to)
}

async fn dispatch_network<N, S>(
    service: &HostHttpEgressService<N, S>,
    mut request: RuntimeHttpEgressRequest,
    network_policy: NetworkPolicy,
) -> Result<ironclaw_network::NetworkHttpResponse, PipelineError>
where
    N: NetworkHttpEgress + Send + Sync,
{
    service
        .network()
        .execute(network_request(&mut request, network_policy))
        .await
        .map_err(|error| {
            PipelineError::post_transport(runtime_network_error(
                service.unsafe_raw_diagnostics_allowed(),
                error,
            ))
        })
}

async fn dispatch_network_for_model_visible_output<N, S>(
    service: &HostHttpEgressService<N, S>,
    mut request: RuntimeHttpEgressRequest,
    network_policy: NetworkPolicy,
) -> Result<ironclaw_network::NetworkHttpResponse, PipelineError>
where
    N: NetworkHttpEgress + Send + Sync,
{
    match service
        .network()
        .execute(network_request(&mut request, network_policy))
        .await
    {
        Ok(response) => Ok(response),
        Err(ironclaw_network::NetworkHttpError::ResponseBodyLimit {
            partial_response: Some(partial_response),
            ..
        }) => Ok(partial_response),
        Err(error) => Err(PipelineError::post_transport(runtime_network_error(
            service.unsafe_raw_diagnostics_allowed(),
            error,
        ))),
    }
}

fn network_request(
    request: &mut RuntimeHttpEgressRequest,
    network_policy: NetworkPolicy,
) -> NetworkHttpRequest {
    NetworkHttpRequest {
        scope: request.scope.clone(),
        method: request.method,
        url: std::mem::take(&mut request.url),
        headers: std::mem::take(&mut request.headers),
        body: std::mem::take(&mut request.body),
        policy: network_policy,
        response_body_limit: request.response_body_limit,
        timeout_ms: request.timeout_ms,
    }
}
