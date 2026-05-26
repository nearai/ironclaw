use std::{fmt, sync::Arc};

use ironclaw_host_api::{
    CapabilityId, ResourceScope, RuntimeHttpEgressError, RuntimeHttpSaveTarget,
    RuntimeHttpSavedBody,
};
use ironclaw_network::NetworkHttpResponse;
use thiserror::Error;

pub trait RuntimeHttpBodyStore: fmt::Debug + Send + Sync {
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

#[derive(Debug, Clone)]
pub(crate) enum RuntimeHttpBodyDisposition {
    ReturnInline,
    Save {
        target: RuntimeHttpSaveTarget,
        store: Arc<dyn RuntimeHttpBodyStore>,
    },
}

impl RuntimeHttpBodyDisposition {
    pub(crate) fn for_request(
        target: Option<RuntimeHttpSaveTarget>,
        store: Option<Arc<dyn RuntimeHttpBodyStore>>,
    ) -> Result<Self, RuntimeHttpEgressError> {
        match (target, store) {
            (None, _) => Ok(Self::ReturnInline),
            (Some(target), Some(store)) => Ok(Self::Save { target, store }),
            (Some(_), None) => Err(RuntimeHttpEgressError::Request {
                reason: "response_body_store_unavailable".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            }),
        }
    }
}

pub(crate) fn apply_body_disposition(
    mut response: NetworkHttpResponse,
    disposition: RuntimeHttpBodyDisposition,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
) -> Result<(NetworkHttpResponse, Option<RuntimeHttpSavedBody>), RuntimeHttpEgressError> {
    let RuntimeHttpBodyDisposition::Save { target, store } = disposition else {
        return Ok((response, None));
    };

    store
        .write_body(scope, capability_id, &target, &response.body)
        .map_err(|error| {
            tracing::debug!(
                error = %error.reason,
                capability_id = %capability_id,
                "runtime HTTP response body store failed"
            );
            RuntimeHttpEgressError::Response {
                reason: "response_body_store_failed".to_string(),
                request_bytes: response.usage.request_bytes,
                response_bytes: response.usage.response_bytes,
            }
        })?;

    let bytes_written = response.body.len() as u64;
    response.body.clear();
    Ok((
        response,
        Some(RuntimeHttpSavedBody {
            path: target.path,
            bytes_written,
        }),
    ))
}
