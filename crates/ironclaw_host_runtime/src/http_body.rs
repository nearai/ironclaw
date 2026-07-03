use std::{fmt, sync::LazyLock};

use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityId, MountView, ResourceScope, RuntimeHttpEgressError, RuntimeHttpSaveTarget,
    RuntimeHttpSavedBody,
};
use ironclaw_network::NetworkHttpResponse;
use thiserror::Error;

pub(crate) const RESPONSE_BODY_STORE_UNAVAILABLE_REASON: &str = "response_body_store_unavailable";
pub(crate) const RESPONSE_BODY_STORE_UNAUTHORIZED_REASON: &str = "response_body_store_unauthorized";
pub(crate) const RESPONSE_BODY_STORE_FAILED_REASON: &str = "response_body_store_failed";

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
pub enum RuntimeHttpBodyStoreError {
    #[error("runtime HTTP body store unavailable")]
    Unavailable,
    #[error("runtime HTTP body store unauthorized: {reason}")]
    Unauthorized { reason: String },
    #[error("runtime HTTP body store failed: {reason}")]
    Failed { reason: String },
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UnsupportedRuntimeHttpBodyStore;

impl RuntimeHttpBodyStore for UnsupportedRuntimeHttpBodyStore {
    fn authorize_write(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
        _target: &RuntimeHttpSaveTarget,
    ) -> Result<(), RuntimeHttpBodyStoreError> {
        Err(RuntimeHttpBodyStoreError::Unavailable)
    }

    fn write_body(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
        _target: &RuntimeHttpSaveTarget,
        _body: &[u8],
    ) -> Result<(), RuntimeHttpBodyStoreError> {
        Err(RuntimeHttpBodyStoreError::Unavailable)
    }
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
        let mut resolved_view = None;
        let view = target_mount_view(
            self,
            scope,
            target,
            &mut resolved_view,
            runtime_http_body_store_unauthorized_error,
        )?;
        let (_path, grant) = view
            .resolve_with_grant(&target.path)
            .map_err(runtime_http_body_store_unauthorized_error)?;
        if !grant.permissions.write {
            return Err(RuntimeHttpBodyStoreError::Unauthorized {
                reason: "write permission denied".to_string(),
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
        let mut resolved_view = None;
        let view = target_mount_view(
            self,
            scope,
            target,
            &mut resolved_view,
            runtime_http_body_store_failed_error,
        )?;
        let write = async {
            self.write_bytes_with_mount_view(view, &target.path, body)
                .await
        };
        block_on_filesystem(write).map(|_| ())
    }
}

fn target_mount_view<'a, F>(
    filesystem: &'a ScopedFilesystem<F>,
    scope: &ResourceScope,
    target: &RuntimeHttpSaveTarget,
    resolved_view: &'a mut Option<MountView>,
    map_error: fn(String) -> RuntimeHttpBodyStoreError,
) -> Result<&'a MountView, RuntimeHttpBodyStoreError>
where
    F: RootFilesystem + ?Sized,
{
    let view = match target.mount_grant.as_ref() {
        Some(grant) => {
            MountView::new(vec![grant.clone()]).map_err(|error| map_error(error.to_string()))?
        }
        None => filesystem
            .mount_view(scope)
            .map_err(|error| map_error(error.to_string()))?,
    };
    Ok(resolved_view.insert(view))
}

static FILESYSTEM_RUNTIME: LazyLock<Result<tokio::runtime::Runtime, RuntimeHttpBodyStoreError>> =
    LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("runtime-http-body-store")
            .enable_all()
            .build()
            .map_err(|_| RuntimeHttpBodyStoreError::Unavailable)
    });

fn block_on_filesystem<T>(
    future: impl std::future::Future<Output = Result<T, FilesystemError>> + Send,
) -> Result<T, RuntimeHttpBodyStoreError>
where
    T: Send,
{
    let runtime = FILESYSTEM_RUNTIME.as_ref().map_err(|error| error.clone())?;
    let joined = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                runtime
                    .block_on(future)
                    .map_err(runtime_http_body_store_failed_error)
            })
            .join()
    });
    joined.unwrap_or_else(|_| {
        Err(RuntimeHttpBodyStoreError::Failed {
            reason: "filesystem worker panicked".to_string(),
        })
    })
}

fn runtime_http_body_store_unauthorized_error(
    _error: impl std::fmt::Display,
) -> RuntimeHttpBodyStoreError {
    RuntimeHttpBodyStoreError::Unauthorized {
        reason: "write authorization failed".to_string(),
    }
}

fn runtime_http_body_store_failed_error(
    error: impl std::fmt::Display,
) -> RuntimeHttpBodyStoreError {
    RuntimeHttpBodyStoreError::Failed {
        reason: error.to_string(),
    }
}

pub(crate) fn apply_body_disposition(
    mut response: NetworkHttpResponse,
    target: Option<RuntimeHttpSaveTarget>,
    store: &dyn RuntimeHttpBodyStore,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
) -> Result<(NetworkHttpResponse, Option<RuntimeHttpSavedBody>), RuntimeHttpEgressError> {
    let Some(target) = target else {
        return Ok((response, None));
    };

    let body = std::mem::take(&mut response.body);
    let bytes_written = body.len() as u64;
    store
        .write_body(scope, capability_id, &target, &body)
        .map_err(|error| {
            tracing::debug!(
                error = %error,
                capability_id = %capability_id,
                "runtime HTTP response body store failed"
            );
            RuntimeHttpEgressError::Response {
                reason: RESPONSE_BODY_STORE_FAILED_REASON.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_filesystem_maps_worker_panic_to_failed_error() {
        let result = block_on_filesystem(async {
            panic!("filesystem worker test panic");
            #[allow(unreachable_code)]
            Ok::<(), FilesystemError>(())
        });

        assert_eq!(
            result,
            Err(RuntimeHttpBodyStoreError::Failed {
                reason: "filesystem worker panicked".to_string(),
            })
        );
    }
}
