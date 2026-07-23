//! Binary-assembled web-access first-party capability wiring (extension-runtime
//! DEL-7). Composition owns the registrar seam; the concrete executor lives here.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_first_party_extensions::{
    WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID, WebAccessDispatchError,
    WebAccessDispatchRequest, WebAccessExecutor,
};
use ironclaw_reborn_composition::{
    CapabilityId, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    FirstPartyHandlerRegistrar, FirstPartyRegistrarContext, HostApiError,
};

/// Installs the web-access first-party capability handlers into the shared
/// registry. Web access needs no product-auth ports, so the registrar context
/// is unused.
pub(crate) struct WebAccessFirstPartyRegistrar;

impl FirstPartyHandlerRegistrar for WebAccessFirstPartyRegistrar {
    fn register(
        &self,
        registry: &mut FirstPartyCapabilityRegistry,
        _context: &FirstPartyRegistrarContext,
    ) -> Result<(), HostApiError> {
        let handler = Arc::new(WebAccessFirstPartyHandler {
            executor: WebAccessExecutor::default(),
        });
        registry.insert_handler(
            CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
            Arc::clone(&handler),
        );
        registry.insert_handler(
            CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
            Arc::clone(&handler),
        );
        Ok(())
    }
}

struct WebAccessFirstPartyHandler {
    executor: WebAccessExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for WebAccessFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let result = self
            .executor
            .dispatch(WebAccessDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: request.services.runtime_http_egress.clone(),
            })
            .await
            .map_err(web_access_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn web_access_error(error: WebAccessDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}
