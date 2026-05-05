use async_trait::async_trait;
use std::sync::Arc;

use crate::{
    AdmissionRejection, CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest,
    ResumeTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnError, TurnRunState,
    TurnStateStore,
};

pub trait TurnAdmissionPolicy: Send + Sync {
    fn check_submit(&self, request: &SubmitTurnRequest) -> Result<(), AdmissionRejection>;
}

#[derive(Debug, Default)]
pub struct AllowAllTurnAdmissionPolicy;

impl TurnAdmissionPolicy for AllowAllTurnAdmissionPolicy {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        Ok(())
    }
}

#[async_trait]
pub trait TurnCoordinator: Send + Sync {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError>;

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError>;

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError>;

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError>;
}

pub struct DefaultTurnCoordinator<S> {
    store: Arc<S>,
    admission_policy: Arc<dyn TurnAdmissionPolicy>,
}

impl<S> DefaultTurnCoordinator<S>
where
    S: TurnStateStore,
{
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            admission_policy: Arc::new(AllowAllTurnAdmissionPolicy),
        }
    }

    pub fn with_admission_policy(mut self, policy: Arc<dyn TurnAdmissionPolicy>) -> Self {
        self.admission_policy = policy;
        self
    }
}

#[async_trait]
impl<S> TurnCoordinator for DefaultTurnCoordinator<S>
where
    S: TurnStateStore + 'static,
{
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.admission_policy
            .check_submit(&request)
            .map_err(TurnError::AdmissionRejected)?;
        self.store.submit_turn(request).await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.store.resume_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.store.request_cancel(request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.store.get_run_state(request).await
    }
}
