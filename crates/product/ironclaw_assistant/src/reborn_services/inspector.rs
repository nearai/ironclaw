use ironclaw_host_api::{
    ids::ThreadId,
    turn::{CapabilityActivityId, TurnRunId},
};
use ironclaw_product_contracts::{
    inspector::{
        DiagnosticCursor, DiagnosticPromptResponse, DiagnosticRunRequest, DiagnosticScope,
        DiagnosticSnapshotResponse, DiagnosticToolRequest, DiagnosticToolResponse,
    },
    surface::{ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceValidationCode},
    views::RebornViewPage,
};

use crate::inspector_store::{DiagnosticStoreError, DiagnosticStorePort};

use super::views;

fn diagnostic_scope(
    caller: ProductSurfaceCaller,
    request: DiagnosticRunRequest,
) -> Result<DiagnosticScope, ProductSurfaceError> {
    let thread_id = ThreadId::new(request.thread_id).map_err(|_| {
        ProductSurfaceError::validation("thread_id", ProductSurfaceValidationCode::InvalidId)
    })?;
    let run_id = TurnRunId::parse(&request.run_id).map_err(|_| {
        ProductSurfaceError::validation("run_id", ProductSurfaceValidationCode::InvalidId)
    })?;
    Ok(DiagnosticScope::new(
        caller.tenant_id,
        caller.user_id,
        thread_id,
        run_id,
    ))
}

fn store_error(error: DiagnosticStoreError) -> ProductSurfaceError {
    match error {
        DiagnosticStoreError::StateUnavailable => ProductSurfaceError::service_unavailable(true),
        other => ProductSurfaceError::internal_from(other),
    }
}

pub(super) fn snapshot(
    store: &dyn DiagnosticStorePort,
    caller: ProductSurfaceCaller,
    request: DiagnosticRunRequest,
) -> Result<RebornViewPage, ProductSurfaceError> {
    let scope = diagnostic_scope(caller, request)?;
    let snapshot = store.snapshot(&scope).map_err(store_error)?;
    views::view_page(DiagnosticSnapshotResponse { snapshot })
}

pub(super) fn prompt(
    store: &dyn DiagnosticStorePort,
    caller: ProductSurfaceCaller,
    request: DiagnosticRunRequest,
) -> Result<RebornViewPage, ProductSurfaceError> {
    let scope = diagnostic_scope(caller, request)?;
    let prompt = store.prompt(&scope).map_err(store_error)?;
    views::view_page(DiagnosticPromptResponse { prompt })
}

pub(super) fn tool(
    store: &dyn DiagnosticStorePort,
    caller: ProductSurfaceCaller,
    request: DiagnosticToolRequest,
) -> Result<RebornViewPage, ProductSurfaceError> {
    let activity_id = CapabilityActivityId::parse(&request.activity_id).map_err(|_| {
        ProductSurfaceError::validation("activity_id", ProductSurfaceValidationCode::InvalidId)
    })?;
    let scope = diagnostic_scope(
        caller,
        DiagnosticRunRequest {
            thread_id: request.thread_id,
            run_id: request.run_id,
        },
    )?;
    let tool = store
        .tool_execution(&scope, activity_id)
        .map_err(store_error)?;
    views::view_page(DiagnosticToolResponse { tool })
}

pub(super) fn updates(
    store: &dyn DiagnosticStorePort,
    caller: ProductSurfaceCaller,
    request: DiagnosticRunRequest,
    after_cursor: Option<String>,
) -> Result<RebornViewPage, ProductSurfaceError> {
    let scope = diagnostic_scope(caller, request)?;
    let after = after_cursor
        .as_deref()
        .map(DiagnosticCursor::parse)
        .transpose()
        .map_err(|_| {
            ProductSurfaceError::validation(
                "after_cursor",
                ProductSurfaceValidationCode::InvalidValue,
            )
        })?;
    let batch = store.updates_after(&scope, after).map_err(store_error)?;
    let next_cursor = batch.latest_cursor.map(|cursor| cursor.to_string());
    views::view_page_with_cursor(batch, next_cursor)
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};

    use super::*;

    #[test]
    fn scope_uses_authenticated_tenant_and_user() {
        let run_id = TurnRunId::new();
        let caller = ProductSurfaceCaller::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
        );
        let scope = diagnostic_scope(
            caller,
            DiagnosticRunRequest {
                thread_id: "thread-a".to_string(),
                run_id: run_id.to_string(),
            },
        )
        .expect("valid scope");
        assert_eq!(scope.tenant_id.as_str(), "tenant-a");
        assert_eq!(scope.user_id.as_str(), "user-a");
        assert_eq!(scope.thread_id.as_str(), "thread-a");
        assert_eq!(scope.run_id, run_id);
    }
}
