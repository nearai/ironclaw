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
    use chrono::Utc;
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_product_contracts::inspector::{
        DiagnosticActivityEvent, DiagnosticActivityKind, ToolExecutionDiagnostic,
        ToolExecutionStatus,
    };

    use crate::inspector_store::InMemoryDiagnosticStore;

    use super::*;

    fn caller(tenant: &str, user: &str) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new(tenant).expect("tenant"),
            UserId::new(user).expect("user"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
        )
    }

    #[test]
    fn scope_uses_authenticated_tenant_and_user() {
        let run_id = TurnRunId::new();
        let caller = caller("tenant-a", "user-a");
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

    #[test]
    fn reads_are_isolated_by_authenticated_scope_and_exact_resource_ids() {
        let store = InMemoryDiagnosticStore::default();
        let run_id = TurnRunId::new();
        let other_run_id = TurnRunId::new();
        let activity_id = CapabilityActivityId::new();
        let owner = caller("tenant-a", "user-a");
        let owner_scope = DiagnosticScope::new(
            owner.tenant_id.clone(),
            owner.user_id.clone(),
            ThreadId::new("thread-a").expect("thread"),
            run_id,
        );
        store
            .record_activity(
                owner_scope.clone(),
                DiagnosticActivityEvent::new(
                    Utc::now(),
                    DiagnosticActivityKind::Progress,
                    None,
                    None,
                    None,
                    Some("owner-only activity".to_string()),
                ),
            )
            .expect("record activity");
        store
            .record_tool_execution(
                owner_scope,
                ToolExecutionDiagnostic::new(
                    activity_id,
                    None,
                    "builtin.echo",
                    Some(r#"{"value":"owner-only arguments"}"#.to_string()),
                    Some("owner-only output".to_string()),
                    ToolExecutionStatus::Succeeded,
                    Some(1),
                    None,
                    None,
                    None,
                ),
            )
            .expect("record tool");

        let exact = snapshot(
            &store,
            owner.clone(),
            DiagnosticRunRequest {
                thread_id: "thread-a".to_string(),
                run_id: run_id.to_string(),
            },
        )
        .expect("owner snapshot");
        let activity = exact.payload["snapshot"]["activity"]
            .as_array()
            .expect("snapshot activity");
        assert!(activity.iter().any(|event| {
            event["event"]["summary"]["content"] == serde_json::json!("owner-only activity")
        }));

        for (read_caller, thread_id, requested_run) in [
            (caller("tenant-b", "user-a"), "thread-a", run_id),
            (caller("tenant-a", "user-b"), "thread-a", run_id),
            (owner.clone(), "thread-b", run_id),
            (owner.clone(), "thread-a", other_run_id),
        ] {
            let page = snapshot(
                &store,
                read_caller,
                DiagnosticRunRequest {
                    thread_id: thread_id.to_string(),
                    run_id: requested_run.to_string(),
                },
            )
            .expect("isolated snapshot");
            assert!(page.payload["snapshot"].is_null());
            let serialized = page.payload.to_string();
            assert!(!serialized.contains("owner-only"));
        }

        let wrong_activity = tool(
            &store,
            owner.clone(),
            DiagnosticToolRequest {
                thread_id: "thread-a".to_string(),
                run_id: run_id.to_string(),
                activity_id: CapabilityActivityId::new().to_string(),
            },
        )
        .expect("wrong activity lookup");
        assert!(wrong_activity.payload["tool"].is_null());

        let exact_tool = tool(
            &store,
            owner,
            DiagnosticToolRequest {
                thread_id: "thread-a".to_string(),
                run_id: run_id.to_string(),
                activity_id: activity_id.to_string(),
            },
        )
        .expect("owner tool lookup");
        assert_eq!(
            exact_tool.payload["tool"]["activity_id"],
            serde_json::json!(activity_id),
        );
    }
}
