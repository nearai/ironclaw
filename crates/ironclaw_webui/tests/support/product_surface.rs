#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::{ActivityId, CapabilityId, Resolution};
use ironclaw_product_workflow::{
    ProductCapabilityInput, ProductOperationRequest, ProductOperationResponse, ProductSurface,
    RebornGetRunStateRequest, RebornGetRunStateResponse, RebornServicesError,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornStreamEventsSubscription,
    RebornViewPage, RebornViewQuery, WebUiAuthenticatedCaller,
};

type InvokeHandler = dyn Fn(
        WebUiAuthenticatedCaller,
        CapabilityId,
        ProductCapabilityInput,
        ActivityId,
    ) -> Result<Resolution, RebornServicesError>
    + Send
    + Sync;
type QueryHandler = dyn Fn(WebUiAuthenticatedCaller, RebornViewQuery) -> Result<RebornViewPage, RebornServicesError>
    + Send
    + Sync;
type CommandHandler = dyn Fn(
        WebUiAuthenticatedCaller,
        ProductOperationRequest,
    ) -> Result<ProductOperationResponse, RebornServicesError>
    + Send
    + Sync;
type StreamHandler = dyn Fn(
        WebUiAuthenticatedCaller,
        RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError>
    + Send
    + Sync;

#[derive(Debug, Clone)]
pub struct InvokeCall {
    pub caller: WebUiAuthenticatedCaller,
    pub capability: CapabilityId,
    pub activity_id: ActivityId,
}

#[derive(Debug, Clone)]
pub struct QueryCall {
    pub caller: WebUiAuthenticatedCaller,
    pub query: RebornViewQuery,
}

#[derive(Debug, Clone)]
pub struct CommandCall {
    pub caller: WebUiAuthenticatedCaller,
    pub request: ProductOperationRequest,
}

#[derive(Debug, Clone)]
pub struct StreamCall {
    pub caller: WebUiAuthenticatedCaller,
    pub request: RebornStreamEventsRequest,
}

#[derive(Default)]
pub struct ProgrammableProductSurface {
    invoke_calls: Mutex<Vec<InvokeCall>>,
    query_calls: Mutex<Vec<QueryCall>>,
    command_calls: Mutex<Vec<CommandCall>>,
    stream_calls: Mutex<Vec<StreamCall>>,
    run_state_calls: Mutex<Vec<(WebUiAuthenticatedCaller, RebornGetRunStateRequest)>>,
    invoke_handler: Mutex<Option<Box<InvokeHandler>>>,
    query_handler: Mutex<Option<Box<QueryHandler>>>,
    command_handler: Mutex<Option<Box<CommandHandler>>>,
    stream_handler: Mutex<Option<Box<StreamHandler>>>,
    stream_responses: Mutex<VecDeque<Result<RebornStreamEventsResponse, RebornServicesError>>>,
    stall_stream_events: Mutex<bool>,
}

impl ProgrammableProductSurface {
    pub fn set_invoke_handler(
        &self,
        handler: impl Fn(
            WebUiAuthenticatedCaller,
            CapabilityId,
            ProductCapabilityInput,
            ActivityId,
        ) -> Result<Resolution, RebornServicesError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.invoke_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn set_query_handler(
        &self,
        handler: impl Fn(
            WebUiAuthenticatedCaller,
            RebornViewQuery,
        ) -> Result<RebornViewPage, RebornServicesError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.query_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn set_command_handler(
        &self,
        handler: impl Fn(
            WebUiAuthenticatedCaller,
            ProductOperationRequest,
        ) -> Result<ProductOperationResponse, RebornServicesError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.command_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn set_stream_handler(
        &self,
        handler: impl Fn(
            WebUiAuthenticatedCaller,
            RebornStreamEventsRequest,
        ) -> Result<RebornStreamEventsResponse, RebornServicesError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.stream_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn enqueue_stream_response(
        &self,
        response: Result<RebornStreamEventsResponse, RebornServicesError>,
    ) {
        self.stream_responses
            .lock()
            .expect("lock")
            .push_back(response);
    }

    pub fn stall_stream_events(&self) {
        *self.stall_stream_events.lock().expect("lock") = true;
    }

    pub fn invoke_calls(&self) -> Vec<InvokeCall> {
        self.invoke_calls.lock().expect("lock").clone()
    }

    pub fn query_calls(&self) -> Vec<QueryCall> {
        self.query_calls.lock().expect("lock").clone()
    }

    pub fn command_calls(&self) -> Vec<CommandCall> {
        self.command_calls.lock().expect("lock").clone()
    }

    pub fn stream_calls(&self) -> Vec<StreamCall> {
        self.stream_calls.lock().expect("lock").clone()
    }

    fn unavailable() -> RebornServicesError {
        RebornServicesError::internal_from("programmable product surface response not configured")
    }
}

#[async_trait]
impl ProductSurface for ProgrammableProductSurface {
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: ProductCapabilityInput,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        self.invoke_calls.lock().expect("lock").push(InvokeCall {
            caller: caller.clone(),
            capability: capability.clone(),
            activity_id,
        });
        if let Some(handler) = self.invoke_handler.lock().expect("lock").as_ref() {
            return handler(caller, capability, input, activity_id);
        }
        Err(Self::unavailable())
    }

    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        query: RebornViewQuery,
    ) -> Result<RebornViewPage, RebornServicesError> {
        self.query_calls.lock().expect("lock").push(QueryCall {
            caller: caller.clone(),
            query: query.clone(),
        });
        if let Some(handler) = self.query_handler.lock().expect("lock").as_ref() {
            return handler(caller, query);
        }
        Err(Self::unavailable())
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        self.stream_calls.lock().expect("lock").push(StreamCall {
            caller: caller.clone(),
            request: request.clone(),
        });
        if *self.stall_stream_events.lock().expect("lock") {
            std::future::pending::<()>().await;
        }
        if let Some(response) = self.stream_responses.lock().expect("lock").pop_front() {
            return response;
        }
        if let Some(handler) = self.stream_handler.lock().expect("lock").as_ref() {
            return handler(caller, request);
        }
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }

    async fn subscribe_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsSubscription, RebornServicesError> {
        Err(Self::unavailable())
    }

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        self.run_state_calls
            .lock()
            .expect("lock")
            .push((caller, request));
        Err(Self::unavailable())
    }

    async fn execute_command(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: ProductOperationRequest,
    ) -> Result<ProductOperationResponse, RebornServicesError> {
        self.command_calls.lock().expect("lock").push(CommandCall {
            caller: caller.clone(),
            request: request.clone(),
        });
        if let Some(handler) = self.command_handler.lock().expect("lock").as_ref() {
            return handler(caller, request);
        }
        Err(Self::unavailable())
    }
}
