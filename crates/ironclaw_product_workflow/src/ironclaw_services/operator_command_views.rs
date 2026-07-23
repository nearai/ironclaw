//! Descriptor-backed operator command-plane read projections.

use super::{
    IronClawOperatorArea, IronClawOperatorCommandPlaneResponse, IronClawOperatorSetupResponse,
    IronClawOperatorSurfaceStatus, IronClawServices, IronClawServicesError, IronClawViewDescriptor,
    IronClawViewProvider, OperatorSetupHostState, ProductCapabilityInvoker,
    WebUiAuthenticatedCaller, llm_config, operator_config_surface_not_wired_diagnostic,
    operator_diagnostics_surface_status, operator_doctor_setup_unavailable_diagnostic,
    operator_doctor_status_diagnostic, operator_doctor_status_response,
    operator_doctor_status_unavailable_diagnostic, setup_response_from_llm_snapshot,
};

pub const OPERATOR_DIAGNOSTICS_VIEW: IronClawViewDescriptor = IronClawViewDescriptor {
    id: "operator_diagnostics",
    paginated: false,
};

pub const OPERATOR_STATUS_VIEW: IronClawViewDescriptor = IronClawViewDescriptor {
    id: "operator_status",
    paginated: false,
};

pub const OPERATOR_SETUP_VIEW: IronClawViewDescriptor = IronClawViewDescriptor {
    id: "operator_setup",
    paginated: false,
};

impl<I, V> IronClawServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: IronClawViewProvider + Clone + 'static,
{
    pub(super) async fn build_operator_setup_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<IronClawOperatorSetupResponse, IronClawServicesError> {
        let Some(llm_config) = &self.llm_config else {
            return Err(llm_config::llm_config_unavailable());
        };
        let snapshot = llm_config
            .snapshot(caller)
            .await
            .map_err(llm_config::map_llm_config_error)?;
        Ok(setup_response_from_llm_snapshot(
            snapshot,
            Vec::new(),
            OperatorSetupHostState::default(),
        ))
    }

    pub(super) async fn build_operator_diagnostics_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<IronClawOperatorCommandPlaneResponse, IronClawServicesError> {
        let mut diagnostics = Vec::new();
        let mut operator_status = None;

        match self.operator_status.status(caller.clone()).await {
            Ok(status) => {
                diagnostics.extend(
                    status
                        .checks
                        .iter()
                        .filter_map(operator_doctor_status_diagnostic),
                );
                operator_status = Some(operator_doctor_status_response(status));
            }
            Err(err) => {
                tracing::debug!(
                    error = ?err,
                    "Failed to retrieve operator status for diagnostics"
                );
                diagnostics.push(operator_doctor_status_unavailable_diagnostic());
            }
        }

        if let Some(llm_config) = &self.llm_config {
            match llm_config.snapshot(caller).await {
                Ok(snapshot) => {
                    diagnostics.extend(
                        setup_response_from_llm_snapshot(
                            snapshot,
                            Vec::new(),
                            OperatorSetupHostState::default(),
                        )
                        .diagnostics,
                    );
                }
                Err(err) => {
                    tracing::debug!(
                        error = ?err,
                        "Failed to retrieve LLM config snapshot for diagnostics"
                    );
                    diagnostics.push(operator_doctor_setup_unavailable_diagnostic(
                        "operator_setup_snapshot_unavailable",
                        "Operator setup state could not be inspected.",
                    ));
                }
            }
        } else {
            diagnostics.push(operator_doctor_setup_unavailable_diagnostic(
                "operator_setup_service_not_wired",
                "Operator setup diagnostics are unavailable because the LLM config service is not wired.",
            ));
        }

        diagnostics.push(operator_config_surface_not_wired_diagnostic());

        Ok(IronClawOperatorCommandPlaneResponse {
            area: IronClawOperatorArea::Diagnostics,
            status: operator_diagnostics_surface_status(&diagnostics),
            message: "operator diagnostics completed".to_string(),
            operator_status,
            logs: None,
            service_lifecycle: None,
            diagnostics,
        })
    }

    pub(super) async fn build_operator_status_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<IronClawOperatorCommandPlaneResponse, IronClawServicesError> {
        let status = self.operator_status.status(caller).await?;
        Ok(IronClawOperatorCommandPlaneResponse {
            area: IronClawOperatorArea::Status,
            status: IronClawOperatorSurfaceStatus::Available,
            message: "operator status is available".to_string(),
            operator_status: Some(status),
            logs: None,
            service_lifecycle: None,
            diagnostics: Vec::new(),
        })
    }
}
