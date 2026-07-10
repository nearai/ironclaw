use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};
use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductFacade, LifecycleProductPayload,
    LifecycleProductSurfaceContext,
};

use crate::RebornAuthContinuationDispatcher;

pub(crate) type LifecycleProductFacadeSlot = Arc<OnceLock<Arc<dyn LifecycleProductFacade>>>;

/// Dispatches extension-auth completion through the same lifecycle facade used
/// by WebUI activation, then delegates turn-gate continuations to the existing
/// resume/fanout dispatcher.
pub(crate) struct LifecycleAuthContinuationDispatcher {
    inner: Arc<dyn RebornAuthContinuationDispatcher>,
    lifecycle: LifecycleProductFacadeSlot,
}

impl LifecycleAuthContinuationDispatcher {
    pub(crate) fn new(
        inner: Arc<dyn RebornAuthContinuationDispatcher>,
        lifecycle: LifecycleProductFacadeSlot,
    ) -> Self {
        Self { inner, lifecycle }
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for LifecycleAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        let AuthContinuationRef::LifecycleActivation { package_ref } = &event.continuation else {
            return self.inner.dispatch_auth_continuation(event).await;
        };
        let lifecycle = self
            .lifecycle
            .get()
            .ok_or(AuthProductError::BackendUnavailable)?;
        let package_ref = LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            package_ref.as_str(),
        )
        .map_err(|error| {
            tracing::error!(%error, flow_id = %event.flow_id, "auth lifecycle package ref is invalid");
            AuthProductError::BackendUnavailable
        })?;
        let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: event.scope.resource.tenant_id.clone(),
            user_id: event.scope.resource.user_id.clone(),
            agent_id: event.scope.resource.agent_id.clone(),
            project_id: event.scope.resource.project_id.clone(),
        });
        let response = lifecycle
            .execute(
                context,
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, flow_id = %event.flow_id, "OAuth completed but extension activation failed");
                AuthProductError::BackendUnavailable
            })?;
        if response.phase != LifecyclePhase::Active
            || !matches!(
                response.payload,
                Some(LifecycleProductPayload::ExtensionActivate {
                    activated: true,
                    ..
                })
            )
        {
            tracing::warn!(
                flow_id = %event.flow_id,
                phase = ?response.phase,
                "OAuth lifecycle continuation did not produce an active extension"
            );
            return Err(AuthProductError::BackendUnavailable);
        }
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.inner.dispatch_canceled_auth_continuation(event).await
    }
}
