use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_first_party_extensions::{
    NEAR_ACCOUNT_CAPABILITY_ID, NEAR_FT_BALANCES_CAPABILITY_ID, NEAR_INTENTS_QUOTE_CAPABILITY_ID,
    NEAR_NFTS_CAPABILITY_ID, NEAR_TX_STATUS_CAPABILITY_ID, NEAR_VIEW_CAPABILITY_ID,
    NearDispatchError, NearDispatchRequest, NearExecutor,
};
use ironclaw_host_api::{CapabilityId, HostApiError};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub(crate) fn register_bundled_near_first_party_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    let handler = Arc::new(NearFirstPartyHandler {
        executor: NearExecutor::default(),
    });
    registry.insert_handler(
        CapabilityId::new(NEAR_ACCOUNT_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(NEAR_VIEW_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(NEAR_FT_BALANCES_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(NEAR_NFTS_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(NEAR_TX_STATUS_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(NEAR_INTENTS_QUOTE_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    Ok(())
}

struct NearFirstPartyHandler {
    executor: NearExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for NearFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let result = self
            .executor
            .dispatch(NearDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: request.services.runtime_http_egress.clone(),
            })
            .await
            .map_err(near_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn near_error(error: NearDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}
