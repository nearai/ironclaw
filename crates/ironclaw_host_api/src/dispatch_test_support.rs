//! Shared [`CapabilityDispatcher`] test double (`test-support`).
//!
//! [`CapabilityDispatcher`](crate::dispatch::CapabilityDispatcher) is a
//! one-method port ([`dispatch_json`](crate::dispatch::CapabilityDispatcher::dispatch_json)),
//! so the ~25 hand-rolled `impl CapabilityDispatcher for …Dispatcher` doubles
//! that used to live across `ironclaw_capabilities` and `ironclaw_host_runtime`
//! tests differed only in what that single method returned and whether they
//! recorded the request. [`TestDispatcher`] is the one configurable double that
//! covers all of them:
//!
//! - `RecordingDispatcher` / `CountingDispatcher` → recording is always on
//!   ([`TestDispatcher::recorded`] / [`TestDispatcher::call_count`]).
//! - `OutputDispatcher` → [`TestDispatcher::ok`].
//! - `AuthRequiredDispatcher` / `AlwaysAuthRequired[Dispatcher]` →
//!   [`TestDispatcher::auth_required`].
//! - `FailingDispatcher` / `TerminalFailDispatcher` / `PanicDispatcher` and
//!   any request-dependent response → [`TestDispatcher::responding`].
//! - `GatingDispatcher` / `FirstCallAuthRequiredDispatcher` /
//!   `AuthRequiredOnFirstCall` (X on the first call, Y after) →
//!   [`TestDispatcher::scripted`].

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;

use crate::dispatch::{
    CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher, DispatchError,
};

type ResponderFn = dyn Fn(&CapabilityDispatchRequest, usize) -> Result<CapabilityDispatchResult, DispatchError>
    + Send
    + Sync;

/// A configurable [`CapabilityDispatcher`] test double.
///
/// Records every dispatched request and returns a response computed by its
/// responder — a fixed `Ok` value ([`Self::ok`]), an `AuthRequired` failure
/// ([`Self::auth_required`]), a scripted per-call sequence ([`Self::scripted`]),
/// or an arbitrary closure of `(request, call_index)` ([`Self::responding`]).
/// Recording is always on, so a test asserts on [`Self::recorded`] /
/// [`Self::call_count`] regardless of the response mode.
pub struct TestDispatcher {
    calls: Mutex<Vec<CapabilityDispatchRequest>>,
    responder: Box<ResponderFn>,
}

impl std::fmt::Debug for TestDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestDispatcher")
            .field("calls", &self.call_count())
            .finish_non_exhaustive()
    }
}

impl TestDispatcher {
    /// Core constructor: the response for each dispatch is
    /// `f(&request, zero_based_call_index)`.
    pub fn responding<F>(f: F) -> Self
    where
        F: Fn(&CapabilityDispatchRequest, usize) -> Result<CapabilityDispatchResult, DispatchError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            calls: Mutex::new(Vec::new()),
            responder: Box::new(f),
        }
    }

    /// Always returns `Ok(result)`.
    pub fn ok(result: CapabilityDispatchResult) -> Self {
        Self::responding(move |_, _| Ok(result.clone()))
    }

    /// Always fails with [`DispatchError::AuthRequired`] for the request's
    /// capability and no secret/credential requirements — the common auth-gate
    /// double. For non-empty requirements, use [`Self::responding`].
    pub fn auth_required() -> Self {
        Self::responding(|request, _| {
            Err(DispatchError::AuthRequired {
                capability: request.capability_id.clone(),
                required_secrets: Vec::new(),
                credential_requirements: Vec::new(),
            })
        })
    }

    /// Returns each queued response once, in order. Panics if dispatched more
    /// times than the queue length — a scripted double is expected to be
    /// exhausted exactly, so an over-dispatch is a test-authoring error, not a
    /// silent last-value repeat.
    pub fn scripted(responses: Vec<Result<CapabilityDispatchResult, DispatchError>>) -> Self {
        let queue = Mutex::new(VecDeque::from(responses));
        Self::responding(move |_, _| {
            queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
                .expect("TestDispatcher scripted queue exhausted")
        })
    }

    /// Every request dispatched so far, in call order.
    pub fn recorded(&self) -> Vec<CapabilityDispatchRequest> {
        self.lock().clone()
    }

    /// Number of dispatches so far.
    pub fn call_count(&self) -> usize {
        self.lock().len()
    }

    /// The most recent dispatched request, if any.
    pub fn last_request(&self) -> Option<CapabilityDispatchRequest> {
        self.lock().last().cloned()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<CapabilityDispatchRequest>> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl CapabilityDispatcher for TestDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let index = {
            let mut calls = self.lock();
            calls.push(request.clone());
            calls.len() - 1
        };
        (self.responder)(&request, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CapabilityId, ResourceEstimate, ResourceScope};
    use serde_json::json;

    fn request(cap: &str) -> CapabilityDispatchRequest {
        CapabilityDispatchRequest {
            capability_id: CapabilityId::new(cap).unwrap(),
            scope: ResourceScope::system(),
            authenticated_actor_user_id: None,
            run_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        }
    }

    fn auth_required_err(cap: &str) -> DispatchError {
        DispatchError::AuthRequired {
            capability: CapabilityId::new(cap).unwrap(),
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        }
    }

    // Recording is always on, so it is asserted here via the error path — the
    // `Ok`-returning `ok()` responder is a one-liner (`Ok(result.clone())`) and
    // is exercised by the downstream migrations that carry a real result.

    #[tokio::test]
    async fn records_every_request_in_call_order() {
        let d = TestDispatcher::auth_required();
        let _ = d.dispatch_json(request("test.a")).await;
        let _ = d.dispatch_json(request("test.b")).await;

        assert_eq!(d.call_count(), 2);
        let recorded = d.recorded();
        assert_eq!(recorded[0].capability_id.as_str(), "test.a");
        assert_eq!(recorded[1].capability_id.as_str(), "test.b");
        assert_eq!(d.last_request().unwrap().capability_id.as_str(), "test.b");
    }

    #[tokio::test]
    async fn auth_required_fails_with_the_requests_capability() {
        let d = TestDispatcher::auth_required();
        let err = d.dispatch_json(request("test.gated")).await.unwrap_err();
        assert!(matches!(
            err,
            DispatchError::AuthRequired { capability, .. } if capability.as_str() == "test.gated"
        ));
    }

    #[tokio::test]
    async fn scripted_returns_each_response_once_in_order() {
        let d = TestDispatcher::scripted(vec![
            Err(auth_required_err("test.first")),
            Err(DispatchError::UnknownCapability { capability: CapabilityId::new("test.other").unwrap() }),
        ]);
        assert!(matches!(
            d.dispatch_json(request("test.x")).await,
            Err(DispatchError::AuthRequired { .. })
        ));
        assert!(matches!(
            d.dispatch_json(request("test.x")).await,
            Err(DispatchError::UnknownCapability { .. })
        ));
        assert_eq!(d.call_count(), 2);
    }

    #[tokio::test]
    #[should_panic(expected = "scripted queue exhausted")]
    async fn scripted_panics_when_over_dispatched() {
        let d = TestDispatcher::scripted(vec![Err(auth_required_err("test.only"))]);
        let _ = d.dispatch_json(request("test.x")).await;
        let _ = d.dispatch_json(request("test.x")).await; // one too many
    }

    #[tokio::test]
    async fn responding_can_branch_on_request_and_call_index() {
        let d = TestDispatcher::responding(|req, idx| {
            // Fail only the first call for capability "first".
            if idx == 0 && req.capability_id.as_str() == "test.first" {
                Err(DispatchError::AuthRequired {
                    capability: req.capability_id.clone(),
                    required_secrets: Vec::new(),
                    credential_requirements: Vec::new(),
                })
            } else {
                Err(DispatchError::UnknownCapability { capability: CapabilityId::new("test.other").unwrap() })
            }
        });
        assert!(matches!(
            d.dispatch_json(request("test.first")).await,
            Err(DispatchError::AuthRequired { .. })
        ));
        assert!(matches!(
            d.dispatch_json(request("test.first")).await,
            Err(DispatchError::UnknownCapability { .. })
        ));
    }
}
