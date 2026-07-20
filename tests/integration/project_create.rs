//! E-PROJ seam smoke test: the `project_lifecycle` group surfaces the local-dev
//! synthetic `project_create` capability; a scripted `builtin.project_create`
//! call dispatches through the REAL synthetic-capability wrap
//! (`wrap_synthetic_capabilities` + `project_create_capability`) and
//! persists a project via the real `ProjectService`.
//!
//! A result-contains assertion alone would pass a silent-no-op regression that
//! still fabricates a success payload, so the read-back below re-queries the
//! SAME `ProjectService` instance the write dispatched through
//! (`capability_harness` -> `project_service_for_test`) and asserts the
//! project is actually present — mirrors the E-PROFILE write -> read-back
//! pattern.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_product_workflow::{ProjectCaller, RebornListProjectsRequest};
use reborn_support::assertions::ToolErrorClass;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::project_service_fault::FAULT_INJECT_DENIED_PROJECT_NAME;
use reborn_support::reply::RebornScriptedReply;

#[test]
fn project_create_capability_dispatches_and_persists_project() {
    run_async_test_with_stack(
        "project_create_capability_dispatches_and_persists_project",
        project_create_capability_dispatches_and_persists_project_impl,
    );
}

async fn project_create_capability_dispatches_and_persists_project_impl() {
    let group = RebornIntegrationGroup::project_lifecycle()
        .await
        .expect("project-lifecycle group builds");
    let harness = group
        .thread("conv-project-create")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.project_create",
                serde_json::json!({"name": "My Project", "description": "a test project"}),
            ),
            RebornScriptedReply::text("created your project"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("create a project named My Project")
        .await
        .expect("turn completes");

    harness
        .assert_tool_invoked("builtin.project_create")
        .await
        .expect("project_create dispatched through the synthetic-capability port");
    // Mutation-catching assertion: a successful project_create records its
    // `{project_id, name}` output through the recording result writer.
    harness
        .assert_tool_result_contains("My Project")
        .await
        .expect("project_create returned the created project");

    // Persistence read-back (E-PROJ): re-fetch through the SAME `ProjectService`
    // instance the capability wrote through, scoped to the same `(tenant, user)`
    // `project_create_capability::effective_user_id` derived the caller from —
    // here, the thread's binding actor (`project_tools()` never calls
    // `with_user_id`, so it's not `capability_harness.user_id()`).
    let capability_harness = group
        .capability_harness()
        .expect("project_lifecycle always uses HostRuntime");
    let project_service = capability_harness
        .project_service_for_test()
        .expect("project_tools() always wires a ProjectService");
    let caller = ProjectCaller {
        tenant_id: harness.binding.tenant_id.clone(),
        user_id: harness.binding.actor_user_id.clone(),
    };
    let projects = project_service
        .list_projects(caller, RebornListProjectsRequest::default())
        .await
        .expect("list_projects succeeds")
        .projects;
    assert!(
        projects.iter().any(|project| project.name == "My Project"),
        "project_create's write must be readable back through the real \
         ProjectService — a no-op create_project that still fabricates a \
         success payload must fail this assertion; got projects: {projects:?}"
    );
}

/// An oversized `name` (201 bytes, over `MAX_PROJECT_NAME_BYTES=200`) passes
/// input parsing but fails `ProjectRecord::validate()` inside the real
/// `ProjectService`; `project_service_outcome` maps the resulting
/// `InvalidInput` error to a recoverable, model-visible `Failed` tool error —
/// proving the reject path routes through Completed-turn/Failed-outcome
/// plumbing rather than aborting the run.
#[test]
fn project_create_invalid_input_routes_to_recoverable_tool_error() {
    run_async_test_with_stack(
        "project_create_invalid_input_routes_to_recoverable_tool_error",
        project_create_invalid_input_routes_to_recoverable_tool_error_impl,
    );
}

async fn project_create_invalid_input_routes_to_recoverable_tool_error_impl() {
    let group = RebornIntegrationGroup::project_lifecycle()
        .await
        .expect("project-lifecycle group builds");
    let oversized_name = "a".repeat(201);
    let harness = group
        .thread("conv-project-create-invalid")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.project_create",
                serde_json::json!({"name": oversized_name}),
            ),
            RebornScriptedReply::text("that name didn't work"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("create a project with a very long name")
        .await
        .expect("turn completes despite the rejected project_create");

    harness
        .assert_tool_invoked("builtin.project_create")
        .await
        .expect("project_create dispatched through the synthetic-capability port");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .expect("oversized name surfaces as a Failed(InvalidInput) capability outcome");
}

/// C-SYNTH fault-injection arm — `project_create` against a genuine host-side
/// `ProjectService::Denied` reject the real local-dev store can't be coerced
/// into on demand: `create_project` calls through a
/// `FaultInjectingProjectService` decorator (`project_lifecycle_fault_injected()`)
/// that forces `Denied` only for `FAULT_INJECT_DENIED_PROJECT_NAME` and
/// delegates everything else to the real store. `project_service_outcome`'s
/// `Denied` arm maps this to a recoverable `Failed(PolicyDenied)` tool error on
/// the FIRST attempt — not the terminal `Internal` arm.
///
/// Deliberately NOT `Unavailable`/`Internal`: both route through the
/// capability-retry branch, whose retry re-dispatch hits an unrelated,
/// independently confirmed bug for provider-tool-call-originated invocations
/// under local-dev composition (issue #5608) that collapses the "retry twice,
/// then Failed" contract into an immediate `driver_unavailable`.
#[test]
fn project_create_denied_fault_routes_to_recoverable_tool_error() {
    run_async_test_with_stack(
        "project_create_denied_fault_routes_to_recoverable_tool_error",
        project_create_denied_fault_routes_to_recoverable_tool_error_impl,
    );
}

async fn project_create_denied_fault_routes_to_recoverable_tool_error_impl() {
    let group = RebornIntegrationGroup::project_lifecycle_fault_injected()
        .await
        .expect("project-lifecycle fault-injection group builds");
    let harness = group
        .thread("conv-project-create-fault")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.project_create",
                serde_json::json!({"name": FAULT_INJECT_DENIED_PROJECT_NAME}),
            ),
            RebornScriptedReply::text("you are not permitted to create that project"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("create a project that will hit a service fault")
        .await
        .expect("turn completes despite the injected fault");

    harness
        .assert_tool_invoked("builtin.project_create")
        .await
        .expect("project_create dispatched through the fault-injecting decorator");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "not permitted to create")
        .await
        .expect("injected Denied fault surfaces as a recoverable Failed tool error");
    harness
        .assert_reply_contains("not permitted")
        .await
        .expect("run recovered and finalized instead of dying at the fault");
}

fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
