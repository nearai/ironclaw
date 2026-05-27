use std::fmt;

use ironclaw_host_api::{
    CapabilityId, ResourceScope, RuntimeHttpEgressError, RuntimeHttpSaveTarget,
    RuntimeHttpSavedBody,
};
use ironclaw_network::NetworkHttpResponse;
use thiserror::Error;

pub trait RuntimeHttpBodyStore: fmt::Debug + Send + Sync {
    fn authorize_write(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        target: &RuntimeHttpSaveTarget,
    ) -> Result<(), RuntimeHttpBodyStoreError>;

    fn write_body(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        target: &RuntimeHttpSaveTarget,
        body: &[u8],
    ) -> Result<(), RuntimeHttpBodyStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("runtime HTTP body store error: {reason}")]
pub struct RuntimeHttpBodyStoreError {
    pub reason: String,
}

pub(crate) fn apply_body_disposition(
    mut response: NetworkHttpResponse,
    target: Option<RuntimeHttpSaveTarget>,
    store: Option<&dyn RuntimeHttpBodyStore>,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
) -> Result<(NetworkHttpResponse, Option<RuntimeHttpSavedBody>), RuntimeHttpEgressError> {
    let Some(target) = target else {
        return Ok((response, None));
    };
    let Some(store) = store else {
        return Err(RuntimeHttpEgressError::Request {
            reason: "response_body_store_unavailable".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
    };

    let body = std::mem::take(&mut response.body);
    let bytes_written = body.len() as u64;
    store
        .write_body(scope, capability_id, &target, &body)
        .map_err(|error| {
            tracing::debug!(
                error = %error.reason,
                capability_id = %capability_id,
                "runtime HTTP response body store failed"
            );
            RuntimeHttpEgressError::Response {
                reason: format!("response_body_store_failed: {}", error.reason),
                request_bytes: response.usage.request_bytes,
                response_bytes: response.usage.response_bytes,
            }
        })?;
    drop(body);

    Ok((
        response,
        Some(RuntimeHttpSavedBody {
            path: target.path,
            bytes_written,
        }),
    ))
}
