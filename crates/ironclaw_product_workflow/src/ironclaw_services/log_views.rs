//! Descriptor-backed caller and operator log projections.

use super::{
    IronClawLogQueryRequest, IronClawLogQueryResponse, IronClawOperatorArea,
    IronClawOperatorCommandPlaneResponse, IronClawOperatorLogsQuery, IronClawOperatorSurfaceStatus,
    IronClawServices, IronClawServicesError, IronClawViewDescriptor, IronClawViewProvider,
    ProductCapabilityInvoker, WebUiAuthenticatedCaller, WebUiInboundValidationCode,
    WebUiInboundValidationError, bounded_log_query, bounded_operator_logs_query,
    parse_thread_id_field, validate_log_query_modes,
};

pub const LOGS_VIEW: IronClawViewDescriptor = IronClawViewDescriptor {
    id: "logs",
    paginated: true,
};

pub const OPERATOR_LOGS_VIEW: IronClawViewDescriptor = IronClawViewDescriptor {
    id: "operator_logs",
    paginated: true,
};

impl<I, V> IronClawServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: IronClawViewProvider + Clone + 'static,
{
    pub(super) async fn build_logs_view(
        &self,
        caller: WebUiAuthenticatedCaller,
        mut query: IronClawLogQueryRequest,
        cursor: Option<String>,
    ) -> Result<IronClawLogQueryResponse, IronClawServicesError> {
        query.cursor = cursor.or(query.cursor);
        validate_log_query_modes(query.tail, query.follow)?;

        let request = bounded_log_query(query);
        let thread_id = request.thread_id.clone().ok_or_else(|| {
            IronClawServicesError::validation(WebUiInboundValidationError::new(
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
        mut query: IronClawOperatorLogsQuery,
        cursor: Option<String>,
    ) -> Result<IronClawOperatorCommandPlaneResponse, IronClawServicesError> {
        query.cursor = cursor.or(query.cursor);
        validate_log_query_modes(query.tail, query.follow)?;

        let request = bounded_operator_logs_query(query);
        let logs = self.operator_logs.query_logs(caller, request).await?;
        Ok(IronClawOperatorCommandPlaneResponse {
            area: IronClawOperatorArea::Logs,
            status: IronClawOperatorSurfaceStatus::Available,
            message: "operator logs query completed".to_string(),
            operator_status: None,
            logs: Some(logs),
            service_lifecycle: None,
            diagnostics: Vec::new(),
        })
    }
}
