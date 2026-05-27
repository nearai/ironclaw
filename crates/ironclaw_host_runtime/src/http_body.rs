use std::fmt;

use ironclaw_filesystem::{FilesystemError, FilesystemOperation, RootFilesystem, ScopedFilesystem};
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

impl<F> RuntimeHttpBodyStore for ScopedFilesystem<F>
where
    F: RootFilesystem + ?Sized,
{
    fn authorize_write(
        &self,
        scope: &ResourceScope,
        _capability_id: &CapabilityId,
        target: &RuntimeHttpSaveTarget,
    ) -> Result<(), RuntimeHttpBodyStoreError> {
        let resolved_view;
        let view = match target.mount_view.as_ref() {
            Some(view) => view,
            None => {
                resolved_view = self
                    .mount_view(scope)
                    .map_err(runtime_http_body_store_error)?;
                &resolved_view
            }
        };
        let (_path, grant) = view
            .resolve_with_grant(&target.path)
            .map_err(runtime_http_body_store_error)?;
        if !grant.permissions.write {
            return Err(RuntimeHttpBodyStoreError {
                reason: format!(
                    "write permission denied for {} on {}",
                    FilesystemOperation::WriteFile,
                    target.path
                ),
            });
        }
        Ok(())
    }

    fn write_body(
        &self,
        scope: &ResourceScope,
        _capability_id: &CapabilityId,
        target: &RuntimeHttpSaveTarget,
        body: &[u8],
    ) -> Result<(), RuntimeHttpBodyStoreError> {
        let write = async {
            match target.mount_view.as_ref() {
                Some(view) => {
                    self.write_file_with_mount_view(view, &target.path, body)
                        .await
                }
                None => self.write_file(scope, &target.path, body).await,
            }
        };
        block_on_filesystem(write).map(|_| ())
    }
}

fn block_on_filesystem<T>(
    future: impl std::future::Future<Output = Result<T, FilesystemError>> + Send,
) -> Result<T, RuntimeHttpBodyStoreError>
where
    T: Send,
{
    let joined = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| RuntimeHttpBodyStoreError {
                        reason: "filesystem runtime unavailable".to_string(),
                    })?;
                runtime
                    .block_on(future)
                    .map_err(runtime_http_body_store_error)
            })
            .join()
    });
    joined.unwrap_or_else(|_| {
        Err(RuntimeHttpBodyStoreError {
            reason: "filesystem worker panicked".to_string(),
        })
    })
}

fn runtime_http_body_store_error(error: impl std::fmt::Display) -> RuntimeHttpBodyStoreError {
    RuntimeHttpBodyStoreError {
        reason: error.to_string(),
    }
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
