#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::{ActivityId, CapabilityId, Resolution};
use ironclaw_product::{
    ProductSurface, ProductSurfaceCaller, ProductSurfaceError, RebornGetRunStateRequest,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornViewPage, RebornViewQuery,
};

type InvokeHandler = dyn Fn(
        ProductSurfaceCaller,
        CapabilityId,
        serde_json::Value,
        ActivityId,
    ) -> Result<Resolution, ProductSurfaceError>
    + Send
    + Sync;
type QueryHandler = dyn Fn(ProductSurfaceCaller, RebornViewQuery) -> Result<RebornViewPage, ProductSurfaceError>
    + Send
    + Sync;
type StreamHandler = dyn Fn(
        ProductSurfaceCaller,
        RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, ProductSurfaceError>
    + Send
    + Sync;

#[derive(Debug, Clone)]
pub struct InvokeCall {
    pub caller: ProductSurfaceCaller,
    pub capability: CapabilityId,
    pub activity_id: ActivityId,
}

#[derive(Debug, Clone)]
pub struct QueryCall {
    pub caller: ProductSurfaceCaller,
    pub query: RebornViewQuery,
}

#[derive(Debug, Clone)]
pub struct StreamCall {
    pub caller: ProductSurfaceCaller,
    pub request: RebornStreamEventsRequest,
}

#[derive(Default)]
pub struct ProgrammableProductSurface {
    invoke_calls: Mutex<Vec<InvokeCall>>,
    query_calls: Mutex<Vec<QueryCall>>,
    stream_calls: Mutex<Vec<StreamCall>>,
    run_state_calls: Mutex<Vec<(ProductSurfaceCaller, RebornGetRunStateRequest)>>,
    invoke_handler: Mutex<Option<Box<InvokeHandler>>>,
    query_handler: Mutex<Option<Box<QueryHandler>>>,
    stream_handler: Mutex<Option<Box<StreamHandler>>>,
    stream_responses: Mutex<VecDeque<Result<RebornStreamEventsResponse, ProductSurfaceError>>>,
    stall_stream_events: Mutex<bool>,
}

impl ProgrammableProductSurface {
    pub fn set_invoke_handler(
        &self,
        handler: impl Fn(
            ProductSurfaceCaller,
            CapabilityId,
            serde_json::Value,
            ActivityId,
        ) -> Result<Resolution, ProductSurfaceError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.invoke_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn set_query_handler(
        &self,
        handler: impl Fn(
            ProductSurfaceCaller,
            RebornViewQuery,
        ) -> Result<RebornViewPage, ProductSurfaceError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.query_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn set_stream_handler(
        &self,
        handler: impl Fn(
            ProductSurfaceCaller,
            RebornStreamEventsRequest,
        ) -> Result<RebornStreamEventsResponse, ProductSurfaceError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.stream_handler.lock().expect("lock") = Some(Box::new(handler));
    }

    pub fn enqueue_stream_response(
        &self,
        response: Result<RebornStreamEventsResponse, ProductSurfaceError>,
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

    pub fn stream_calls(&self) -> Vec<StreamCall> {
        self.stream_calls.lock().expect("lock").clone()
    }

    fn unavailable() -> ProductSurfaceError {
        ProductSurfaceError::internal_from("programmable product surface response not configured")
    }
}

#[async_trait]
impl ProductSurface for ProgrammableProductSurface {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        self.invoke_calls.lock().expect("lock").push(InvokeCall {
            caller: caller.clone(),
            capability: request.operation_id.clone(),
            activity_id: request.activity_id,
        });
        if let Some(handler) = self.invoke_handler.lock().expect("lock").as_ref() {
            let output = handler(
                caller,
                request.operation_id,
                request.input,
                request.activity_id,
            )?;
            let output =
                serde_json::to_value(output).map_err(ProductSurfaceError::internal_from)?;
            return Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output });
        }
        Err(Self::unavailable())
    }

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        let query = RebornViewQuery {
            view_id: request.view_id,
            params: request.input,
            cursor: request.cursor,
        };
        self.query_calls.lock().expect("lock").push(QueryCall {
            caller: caller.clone(),
            query: query.clone(),
        });
        if let Some(handler) = self.query_handler.lock().expect("lock").as_ref() {
            let page = handler(caller, query)?;
            return Ok(ironclaw_host_api::ProductSurfaceQueryPage {
                items: vec![page.payload],
                next_cursor: page.next_cursor,
            });
        }
        Err(Self::unavailable())
    }

    async fn stream_events(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        let stream_request = RebornStreamEventsRequest {
            thread_id: request.stream_id.ok_or_else(|| {
                ProductSurfaceError::validation(
                    "stream_id",
                    ironclaw_product::ProductSurfaceValidationCode::MissingField,
                )
            })?,
            after_cursor: request
                .after_cursor
                .map(ironclaw_product::ProjectionCursor::new)
                .transpose()
                .map_err(ProductSurfaceError::internal_from)?,
        };
        self.stream_calls.lock().expect("lock").push(StreamCall {
            caller: caller.clone(),
            request: stream_request.clone(),
        });
        if *self.stall_stream_events.lock().expect("lock") {
            std::future::pending::<()>().await;
        }
        let response =
            if let Some(response) = self.stream_responses.lock().expect("lock").pop_front() {
                response?
            } else if let Some(handler) = self.stream_handler.lock().expect("lock").as_ref() {
                handler(caller, stream_request)?
            } else {
                RebornStreamEventsResponse { events: Vec::new() }
            };
        let events = response
            .events
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ProductSurfaceError::internal_from)?;
        Ok(ironclaw_host_api::ProductSurfaceStreamResponse {
            events,
            next_cursor: None,
        })
    }
}
