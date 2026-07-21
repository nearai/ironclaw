use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthContinuationRef, AuthFlowOutcome, AuthProductError, AuthProviderId, AuthResolved,
    CredentialAccountOwnerScope,
};
use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductFacade, LifecycleProductPayload,
    LifecycleProductSurfaceContext, LifecycleReadinessBlocker,
};
use ironclaw_turns::IdempotencyKey;

use crate::RebornAuthResolutionDispatcher;

pub(crate) type LifecycleProductFacadeSlot = Arc<OnceLock<Arc<dyn LifecycleProductFacade>>>;

/// Dispatches extension-auth completion through the same lifecycle facade used
/// by WebUI activation, then delegates turn-gate continuations to the existing
/// resume/fanout dispatcher.
pub(crate) struct LifecycleAuthResolutionDispatcher {
    inner: Arc<dyn RebornAuthResolutionDispatcher>,
    lifecycle: LifecycleProductFacadeSlot,
}

impl LifecycleAuthResolutionDispatcher {
    pub(crate) fn new(
        inner: Arc<dyn RebornAuthResolutionDispatcher>,
        lifecycle: LifecycleProductFacadeSlot,
    ) -> Self {
        Self { inner, lifecycle }
    }
}

#[async_trait]
impl RebornAuthResolutionDispatcher for LifecycleAuthResolutionDispatcher {
    async fn dispatch_auth_resolved(&self, event: AuthResolved) -> Result<(), AuthProductError> {
        if !matches!(event.outcome, AuthFlowOutcome::Authorized { .. }) {
            return self.inner.dispatch_auth_resolved(event).await;
        }
        let AuthContinuationRef::LifecycleActivation { package_ref } = &event.continuation else {
            return self.inner.dispatch_auth_resolved(event).await;
        };
        let lifecycle = self.lifecycle.get().ok_or_else(|| {
            tracing::error!(
                flow_id = %event.flow_id,
                "lifecycle facade is not wired for auth continuation dispatch"
            );
            AuthProductError::BackendUnavailable
        })?;
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
                tracing::debug!(%error, flow_id = %event.flow_id, "OAuth completed but extension activation failed");
                AuthProductError::BackendUnavailable
            })?;
        if response.phase == LifecyclePhase::Active
            && matches!(
                response.payload,
                Some(LifecycleProductPayload::ExtensionActivate {
                    activated: true,
                    ..
                })
            )
        {
            return self.inner.dispatch_auth_resolved(event).await;
        }
        if matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionActivate {
                activated: false,
                ..
            })
        ) && !response.blockers.is_empty()
            && response
                .blockers
                .iter()
                .all(|blocker| matches!(blocker, LifecycleReadinessBlocker::Credential { .. }))
        {
            tracing::debug!(
                flow_id = %event.flow_id,
                remaining_credentials = response.blockers.len(),
                "OAuth credential completed; extension activation is waiting on other credentials"
            );
            return Ok(());
        }
        tracing::debug!(
            flow_id = %event.flow_id,
            phase = ?response.phase,
            "OAuth lifecycle continuation did not produce an active extension"
        );
        Err(AuthProductError::BackendUnavailable)
    }

    async fn dispatch_provider_connection(
        &self,
        delivery_key: IdempotencyKey,
        owner: CredentialAccountOwnerScope,
        provider: AuthProviderId,
    ) -> Result<(), AuthProductError> {
        self.inner
            .dispatch_provider_connection(delivery_key, owner, provider)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_auth::{
        AuthFlowId, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId,
        LifecyclePackageRef as AuthPackageRef,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
    use ironclaw_product_workflow::{
        LifecycleBlockerRef, LifecycleProductResponse, ProductWorkflowError,
    };

    use super::*;

    struct StaticLifecycleFacade {
        response: LifecycleProductResponse,
    }

    #[async_trait]
    impl LifecycleProductFacade for StaticLifecycleFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Ok(self.response.clone())
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Ok(self.response.clone())
        }
    }

    #[derive(Default)]
    struct RecordingInnerDispatcher {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl RebornAuthResolutionDispatcher for RecordingInnerDispatcher {
        async fn dispatch_auth_resolved(
            &self,
            _event: AuthResolved,
        ) -> Result<(), AuthProductError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn event() -> AuthResolved {
        AuthResolved {
            flow_id: AuthFlowId::new(),
            scope: AuthProductScope::new(
                ResourceScope::local_default(
                    UserId::new("alice").expect("user"),
                    InvocationId::new(),
                )
                .expect("scope"),
                AuthSurface::Callback,
            ),
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: AuthPackageRef::new("slack").expect("package ref"),
            },
            provider: AuthProviderId::new("slack_personal").expect("provider"),
            outcome: AuthFlowOutcome::Authorized {
                account_id: CredentialAccountId::new(),
            },
            resolved_at: chrono::Utc::now(),
        }
    }

    fn dispatcher(
        response: LifecycleProductResponse,
    ) -> (
        LifecycleAuthResolutionDispatcher,
        Arc<RecordingInnerDispatcher>,
    ) {
        let inner = Arc::new(RecordingInnerDispatcher::default());
        let slot: LifecycleProductFacadeSlot = Arc::new(OnceLock::new());
        assert!(
            slot.set(Arc::new(StaticLifecycleFacade { response }))
                .is_ok()
        );
        (
            LifecycleAuthResolutionDispatcher::new(inner.clone(), slot),
            inner,
        )
    }

    #[tokio::test]
    async fn successful_activation_delegates_to_blocked_auth_fanout() {
        let (dispatcher, inner) = dispatcher(LifecycleProductResponse {
            package_ref: None,
            phase: LifecyclePhase::Active,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids: vec!["slack.search_messages".to_string()],
                connection_required: None,
            }),
        });

        dispatcher
            .dispatch_auth_resolved(event())
            .await
            .expect("dispatch continuation");

        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn remaining_credentials_settle_without_fanout_or_failure() {
        let (dispatcher, inner) = dispatcher(LifecycleProductResponse {
            package_ref: None,
            phase: LifecyclePhase::Installed,
            blockers: vec![LifecycleReadinessBlocker::Credential {
                ref_id: Some(LifecycleBlockerRef::new("google").expect("blocker ref")),
            }],
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionActivate {
                activated: false,
                visible_capability_ids: Vec::new(),
                connection_required: None,
            }),
        });

        dispatcher
            .dispatch_auth_resolved(event())
            .await
            .expect("incomplete credentials are not an activation failure");

        assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
    }
}
