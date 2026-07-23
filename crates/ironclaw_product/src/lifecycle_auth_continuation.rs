//! Extension-lifecycle completion for product-auth continuations.
//!
//! The auth engine emits a neutral lifecycle continuation after provider auth
//! succeeds. This workflow-owned handler re-enters the canonical lifecycle
//! command and delegates the provider-blocked-run fan-out only after the
//! caller-scoped projection is active with no remaining readiness blockers.
//!
//! When the projection is not yet active but a setup blocker remains (a
//! successful OAuth requirement with other manifest-declared setup still
//! pending), the handler must keep the completed flow **re-drivable** rather
//! than durably fenced, so a later cross-replica reconcile can complete the
//! fan-out once readiness settles. It signals that by returning a retryable
//! error instead of `Ok(())`; see the `SetupIncomplete` branch below.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};

use crate::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductSurfaceContext, LifecyclePublicState,
    ProductAuthContinuationDispatcher, ProductWorkflowError,
};

/// Wrap auth continuation delivery with canonical lifecycle readiness
/// reconciliation for lifecycle-activation continuations.
///
/// The concrete lifecycle facade is required at construction so production
/// composition cannot silently omit readiness reconciliation or defer it
/// through a process-local late-binding cell.
pub fn lifecycle_auth_continuation_dispatcher(
    facade: Arc<dyn LifecycleProductFacade>,
    inner: Arc<dyn ProductAuthContinuationDispatcher>,
) -> Arc<dyn ProductAuthContinuationDispatcher> {
    Arc::new(LifecycleAuthContinuationDispatcher { inner, facade })
}

struct LifecycleAuthContinuationDispatcher {
    inner: Arc<dyn ProductAuthContinuationDispatcher>,
    facade: Arc<dyn LifecycleProductFacade>,
}

#[async_trait]
impl ProductAuthContinuationDispatcher for LifecycleAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if let AuthContinuationRef::LifecycleActivation { package_ref } = &event.continuation
            && reconcile_lifecycle_activation(&self.facade, &event, package_ref.as_str()).await?
                == LifecycleAuthContinuationOutcome::SetupIncomplete
        {
            // This OAuth requirement completed successfully, but another
            // manifest-declared setup blocker remains, so there is nothing to fan
            // out yet.
            //
            // Crucially, do NOT settle with `Ok(())`: the caller
            // (`dispatch_completed_continuation`) treats `Ok` as "fanned out" and
            // stamps the durable `continuation_emitted_at` fence, after which a
            // later reconcile is a no-op — permanently stranding the
            // provider-blocked runs when the remaining blocker is a runtime
            // discovery that carries no continuation of its own to drive the
            // eventual fan-out.
            //
            // Surface a retryable outcome instead. The caller's LifecycleActivation
            // `BackendUnavailable` branch returns without terminalizing AND without
            // stamping the fence, so the completed flow stays re-drivable: a later
            // `reconcile_oauth_flow` (once readiness is Active) re-enters here,
            // delegates to the inner fan-out, and only THEN is the continuation
            // fenced. Setup incompleteness is genuinely a transient,
            // retry-until-ready condition, so the retryable code is behaviourally
            // correct even though it is not a store fault.
            return Err(AuthProductError::BackendUnavailable);
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
    if response.phase == LifecyclePublicState::Active && response.blockers.is_empty() {
        return Ok(LifecycleAuthContinuationOutcome::Ready);
    }
    if response.phase != LifecyclePublicState::Active && !response.blockers.is_empty() {
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
    use std::collections::VecDeque;
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

    /// Like [`RecordingLifecycleFacade`] but returns a queued sequence of
    /// responses, one per `execute` call — models a projection whose readiness
    /// transitions (SetupNeeded → Active) between the initial callback and a
    /// later reconcile.
    struct SequencedLifecycleFacade {
        responses: Mutex<VecDeque<LifecycleProductResponse>>,
        reconciled: Mutex<Vec<LifecyclePackageRef>>,
    }

    #[async_trait]
    impl LifecycleProductFacade for SequencedLifecycleFacade {
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
            Ok(self
                .responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("a queued lifecycle response for each reconcile pass"))
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
                LifecyclePublicState::Active,
                Vec::new(),
            ),
            reconciled: Mutex::new(Vec::new()),
        });
        let inner = Arc::new(RecordingInner::default());

        lifecycle_auth_continuation_dispatcher(facade.clone(), inner.clone())
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
    async fn setup_incomplete_stays_redrivable_without_delegating_or_fencing() {
        // A completed OAuth requirement with another setup blocker still pending
        // must NOT settle: it delegates nothing (no fan-out yet) and returns a
        // retryable error so the caller leaves the flow un-fenced and a later
        // reconcile can finish the fan-out. A plain `Ok(())` here would let the
        // caller stamp the durable continuation fence and permanently strand the
        // provider-blocked runs.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "hosted-docs")
            .expect("package ref");
        let facade = Arc::new(RecordingLifecycleFacade {
            response: LifecycleProductResponse::projection(
                Some(package_ref),
                LifecyclePublicState::SetupNeeded,
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

        let error = lifecycle_auth_continuation_dispatcher(facade, inner.clone())
            .dispatch_auth_continuation(event())
            .await
            .expect_err("a setup-incomplete continuation must stay re-drivable, not settle");

        // Retryable (not terminal): the caller's LifecycleActivation
        // `BackendUnavailable` branch neither terminalizes nor fences the flow.
        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert!(inner.events.lock().expect("events lock").is_empty());
    }

    #[tokio::test]
    async fn setup_incomplete_then_reconcile_at_readiness_fans_out() {
        // Cross-replica recovery: the first pass defers (setup incomplete), and a
        // later reconcile — after the remaining runtime-discovery blocker clears
        // and the projection is Active — re-drives the SAME continuation and
        // finally delegates the provider-blocked-run fan-out. Proves the flow is
        // never permanently stuck.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "hosted-docs")
            .expect("package ref");
        let facade = Arc::new(SequencedLifecycleFacade {
            responses: Mutex::new(VecDeque::from([
                LifecycleProductResponse::projection(
                    Some(package_ref.clone()),
                    LifecyclePublicState::SetupNeeded,
                    vec![
                        LifecycleReadinessBlocker::runtime(Some(
                            "hosted_mcp_discovery_pending".to_string(),
                        ))
                        .expect("blocker"),
                    ],
                ),
                LifecycleProductResponse::projection(
                    Some(package_ref),
                    LifecyclePublicState::Active,
                    Vec::new(),
                ),
            ])),
            reconciled: Mutex::new(Vec::new()),
        });
        let inner = Arc::new(RecordingInner::default());
        let dispatcher = lifecycle_auth_continuation_dispatcher(facade.clone(), inner.clone());

        // First pass: setup incomplete → deferred (retryable), no fan-out.
        let error = dispatcher
            .dispatch_auth_continuation(event())
            .await
            .expect_err("first pass defers while setup is incomplete");
        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert!(inner.events.lock().expect("events lock").is_empty());

        // Later reconcile once readiness is Active → fans out exactly once.
        dispatcher
            .dispatch_auth_continuation(event())
            .await
            .expect("reconcile at readiness completes the fan-out");
        assert_eq!(
            inner.events.lock().expect("events lock").len(),
            1,
            "the blocked-run fan-out must run exactly once, on the readiness reconcile",
        );
        assert_eq!(
            facade.reconciled.lock().expect("reconciled lock").len(),
            2,
            "both passes re-enter the idempotent canonical install action",
        );
    }

    #[tokio::test]
    async fn rejects_non_active_projection_without_an_explained_setup_blocker() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "hosted-docs")
            .expect("package ref");
        let facade = Arc::new(RecordingLifecycleFacade {
            response: LifecycleProductResponse::projection(
                Some(package_ref),
                LifecyclePublicState::SetupNeeded,
                Vec::new(),
            ),
            reconciled: Mutex::new(Vec::new()),
        });
        let inner = Arc::new(RecordingInner::default());

        let error = lifecycle_auth_continuation_dispatcher(facade, inner.clone())
            .dispatch_auth_continuation(event())
            .await
            .expect_err("unexplained incomplete readiness must fail closed");

        assert_eq!(error, AuthProductError::LifecycleActivationFailed);
        assert!(inner.events.lock().expect("events lock").is_empty());
    }
}
