//! Descriptor-backed caller and operator log projections.

use super::{
    ProductCapabilityInvoker, RebornLogQueryRequest, RebornLogQueryResponse, RebornOperatorArea,
    RebornOperatorCommandPlaneResponse, RebornOperatorLogsQuery, RebornOperatorSurfaceStatus,
    RebornServices, RebornServicesError, RebornViewDescriptor, WebUiAuthenticatedCaller,
    WebUiInboundValidationCode, WebUiInboundValidationError, bounded_log_query,
    bounded_operator_logs_query, parse_thread_id_field, validate_log_query_modes,
};

pub const LOGS_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "logs",
    paginated: true,
};

pub const OPERATOR_LOGS_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "operator_logs",
    paginated: true,
};

impl<I> RebornServices<I>
where
    I: ProductCapabilityInvoker + Clone + 'static,
{
    pub(super) async fn build_logs_view(
        &self,
        caller: WebUiAuthenticatedCaller,
        mut query: RebornLogQueryRequest,
        cursor: Option<String>,
    ) -> Result<RebornLogQueryResponse, RebornServicesError> {
        query.cursor = cursor.or(query.cursor);
        validate_log_query_modes(query.tail, query.follow)?;

        let request = bounded_log_query(query);
        let thread_id = request.thread_id.clone().ok_or_else(|| {
            RebornServicesError::validation(WebUiInboundValidationError::new(
                "thread_id",
                WebUiInboundValidationCode::MissingField,
            ))
        })?;
        let thread_id = parse_thread_id_field("thread_id", thread_id)?;
        let actor = caller.actor();
        let scope = caller.turn_scope(thread_id);
        self.resolve_thread_access_for_caller(caller.clone(), scope, &actor)
            .await?;

        self.operator_logs.query_logs(caller, request).await
    }

    pub(super) async fn build_operator_logs_view(
        &self,
        caller: WebUiAuthenticatedCaller,
        mut query: RebornOperatorLogsQuery,
        cursor: Option<String>,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        query.cursor = cursor.or(query.cursor);
        validate_log_query_modes(query.tail, query.follow)?;

        let request = bounded_operator_logs_query(query);
        let logs = self.operator_logs.query_logs(caller, request).await?;
        Ok(RebornOperatorCommandPlaneResponse {
            area: RebornOperatorArea::Logs,
            status: RebornOperatorSurfaceStatus::Available,
            message: "operator logs query completed".to_string(),
            operator_status: None,
            logs: Some(logs),
            service_lifecycle: None,
            diagnostics: Vec::new(),
        })
    }
}
