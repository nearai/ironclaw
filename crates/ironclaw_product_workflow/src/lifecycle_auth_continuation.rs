//! Extension-lifecycle completion for product-auth continuations.
//!
//! The auth engine emits a neutral lifecycle continuation after provider auth
//! succeeds. This workflow-owned handler re-enters the canonical lifecycle
//! command and accepts completion only after the caller-scoped projection is
//! active with no remaining readiness blockers.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};
use ironclaw_host_api::InstallationState;

use crate::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductSurfaceContext, ProductAuthContinuationDispatcher,
    ProductWorkflowError,
};

#[derive(Clone, Default)]
pub struct LifecycleAuthContinuationSlot {
    facade: Arc<OnceLock<Arc<dyn LifecycleProductFacade>>>,
}

impl LifecycleAuthContinuationSlot {
    pub fn fill(&self, facade: Arc<dyn LifecycleProductFacade>) -> Result<(), &'static str> {
        self.facade
            .set(facade)
            .map_err(|_| "extension lifecycle continuation facade was already configured")
    }

    pub fn wrap(
        &self,
        inner: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Arc<dyn ProductAuthContinuationDispatcher> {
        Arc::new(LifecycleAuthContinuationDispatcher {
            inner,
            facade: Arc::clone(&self.facade),
        })
    }
}

struct LifecycleAuthContinuationDispatcher {
    inner: Arc<dyn ProductAuthContinuationDispatcher>,
    facade: Arc<OnceLock<Arc<dyn LifecycleProductFacade>>>,
}

#[async_trait]
impl ProductAuthContinuationDispatcher for LifecycleAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if let AuthContinuationRef::LifecycleActivation { package_ref } = &event.continuation {
            let facade = self
                .facade
                .get()
                .ok_or(AuthProductError::BackendUnavailable)?;
            if reconcile_lifecycle_activation(facade, &event, package_ref.as_str()).await?
                == LifecycleAuthContinuationOutcome::SetupIncomplete
            {
                // This OAuth requirement completed successfully, but another
                // manifest-declared setup blocker remains. Settle this flow
                // without resuming provider-blocked runs; the final setup
                // requirement will reconcile readiness and delegate fan-out.
                return Ok(());
            }
        }
        self.inner.dispatch_auth_continuation(event).await
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.inner.dispatch_canceled_auth_continuation(event).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleAuthContinuationOutcome {
    Ready,
    SetupIncomplete,
}

async fn reconcile_lifecycle_activation(
    facade: &Arc<dyn LifecycleProductFacade>,
    event: &AuthContinuationEvent,
    package_ref: &str,
) -> Result<LifecycleAuthContinuationOutcome, AuthProductError> {
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_ref)
        .map_err(|_| AuthProductError::LifecycleActivationFailed)?;
    let resource = &event.scope.resource;
    let response = facade
        .execute(
            LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                tenant_id: resource.tenant_id.clone(),
                user_id: resource.user_id.clone(),
                agent_id: resource.agent_id.clone(),
                project_id: resource.project_id.clone(),
            }),
            LifecycleProductAction::ExtensionInstall { package_ref },
        )
        .await
        .map_err(map_lifecycle_readiness_error)?;
    if response.phase == InstallationState::Active && response.blockers.is_empty() {
        return Ok(LifecycleAuthContinuationOutcome::Ready);
    }
    if response.phase != InstallationState::Active && !response.blockers.is_empty() {
        return Ok(LifecycleAuthContinuationOutcome::SetupIncomplete);
    }
    Err(AuthProductError::LifecycleActivationFailed)
}

fn map_lifecycle_readiness_error(error: ProductWorkflowError) -> AuthProductError {
    match error {
        ProductWorkflowError::Transient { .. } => AuthProductError::BackendUnavailable,
        other => {
            tracing::warn!(
                error = %other,
                "lifecycle readiness reconciliation failed after authentication"
            );
            AuthProductError::LifecycleActivationFailed
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Utc;
    use ironclaw_auth::{
        AuthFlowId, AuthProductScope, AuthProviderId, AuthSurface,
        LifecyclePackageRef as AuthPackageRef,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, UserId};

    use super::*;
    use crate::{LifecycleProductResponse, LifecycleReadinessBlocker};

    struct RecordingLifecycleFacade {
        response: LifecycleProductResponse,
        reconciled: Mutex<Vec<LifecyclePackageRef>>,
    }

    #[async_trait]
    impl LifecycleProductFacade for RecordingLifecycleFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            let LifecycleProductAction::ExtensionInstall { package_ref } = action else {
                panic!("OAuth continuation must re-enter the idempotent install action")
            };
            self.reconciled
                .lock()
                .expect("reconciled lock")
                .push(package_ref);
            Ok(self.response.clone())
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            panic!("OAuth continuation must enter the canonical readiness command")
        }
    }

    #[derive(Default)]
    struct RecordingInner {
        events: Mutex<Vec<AuthContinuationEvent>>,
    }

    #[async_trait]
    impl ProductAuthContinuationDispatcher for RecordingInner {
        async fn dispatch_auth_continuation(
            &self,
            event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events.lock().expect("events lock").push(event);
            Ok(())
        }

        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    fn event() -> AuthContinuationEvent {
        let resource = ResourceScope::local_default(
            UserId::new("oauth-user").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        AuthContinuationEvent {
            flow_id: AuthFlowId::new(),
            scope: AuthProductScope::new(resource, AuthSurface::Callback),
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: AuthPackageRef::new("hosted-docs").expect("auth package ref"),
            },
            provider: AuthProviderId::new("docs-vendor").expect("provider"),
            credential_account_id: None,
            emitted_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn reconciles_readiness_before_delegating() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "hosted-docs")
            .expect("package ref");
        let facade = Arc::new(RecordingLifecycleFacade {
            response: LifecycleProductResponse::projection(
                Some(package_ref.clone()),
                InstallationState::Active,
                Vec::new(),
            ),
            reconciled: Mutex::new(Vec::new()),
        });
        let inner = Arc::new(RecordingInner::default());
        let slot = LifecycleAuthContinuationSlot::default();
        slot.fill(facade.clone()).expect("fill facade");

        slot.wrap(inner.clone())
            .dispatch_auth_continuation(event())
            .await
            .expect("ready extension continues to turn fanout");

        assert_eq!(
            facade
                .reconciled
                .lock()
                .expect("reconciled lock")
                .as_slice(),
            &[package_ref]
        );
        assert_eq!(inner.events.lock().expect("events lock").len(), 1);
    }

    #[tokio::test]
    async fn settles_completed_credential_without_delegating_before_readiness_is_active() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "hosted-docs")
            .expect("package ref");
        let facade = Arc::new(RecordingLifecycleFacade {
            response: LifecycleProductResponse::projection(
                Some(package_ref),
                InstallationState::Configured,
                vec![
                    LifecycleReadinessBlocker::runtime(Some(
                        "hosted_mcp_discovery_pending".to_string(),
                    ))
                    .expect("blocker"),
                ],
            ),
            reconciled: Mutex::new(Vec::new()),
        });
        let inner = Arc::new(RecordingInner::default());
        let slot = LifecycleAuthContinuationSlot::default();
        slot.fill(facade).expect("fill facade");

        slot.wrap(inner.clone())
            .dispatch_auth_continuation(event())
            .await
            .expect("the completed credential flow settles while setup remains incomplete");

        assert!(inner.events.lock().expect("events lock").is_empty());
    }

    #[tokio::test]
    async fn rejects_non_active_projection_without_an_explained_setup_blocker() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "hosted-docs")
            .expect("package ref");
        let facade = Arc::new(RecordingLifecycleFacade {
            response: LifecycleProductResponse::projection(
                Some(package_ref),
                InstallationState::Configured,
                Vec::new(),
            ),
            reconciled: Mutex::new(Vec::new()),
        });
        let inner = Arc::new(RecordingInner::default());
        let slot = LifecycleAuthContinuationSlot::default();
        slot.fill(facade).expect("fill facade");

        let error = slot
            .wrap(inner.clone())
            .dispatch_auth_continuation(event())
            .await
            .expect_err("unexplained incomplete readiness must fail closed");

        assert_eq!(error, AuthProductError::LifecycleActivationFailed);
        assert!(inner.events.lock().expect("events lock").is_empty());
    }
}
