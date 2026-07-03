//! E-PROJ seam smoke test: the `project_lifecycle` group surfaces the local-dev
//! synthetic `project_create` capability, and a scripted `builtin.project_create`
//! tool call dispatches through the REAL production synthetic-capability wrap
//! (`wrap_local_dev_synthetic_capabilities` + `project_create_capability`) and
//! persists a project via the real `ProjectService`.
//!
//! The result-contains assertion proves dispatch + a recorded output payload,
//! but not actual persistence: a regression that made `create_project` a
//! silent no-op while still fabricating a `{project_id, name}` success payload
//! would pass it. The read-back below closes that gap by re-querying the REAL
//! `ProjectService` (through `RebornIntegrationGroup::capability_harness` ->
//! `project_service_for_test`, the SAME instance
//! `apply_synthetic_capability_wrappers` dispatched the write through) and
//! asserting the created project is actually present — mirrors the E-PROFILE
//! `reborn_integration_profile` write -> read-back pattern.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use ironclaw_product_workflow::{ProjectCaller, RebornListProjectsRequest};
use reborn_support::assertions::ToolErrorClass;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::project_service_fault::FAULT_INJECT_DENIED_PROJECT_NAME;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn project_create_capability_dispatches_and_persists_project() {
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
    // `project_create_capability::effective_user_id` derived the caller from.
    // `effective_user_id` prefers the run scope's explicit thread owner, then
    // the run actor, and only falls back to the capability harness's fixed
    // constructor user when neither is set — this thread's binding has neither
    // an explicit owner nor an override (`project_tools()` never calls
    // `with_user_id`), so the actual dispatch caller is the thread's binding
    // actor, not `capability_harness.user_id()`.
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

/// An oversized `name` (201 ASCII bytes, over `MAX_PROJECT_NAME_BYTES=200`)
/// passes `parse_project_create_input`'s non-empty check but fails
/// `ProjectRecord::validate()` inside the real `ProjectService`, which returns
/// `ProjectServiceError::InvalidInput`. `project_service_outcome` maps that to
/// `CapabilityOutcome::Failed(CapabilityFailureKind::InvalidInput)`, persisted
/// as a `ToolResultReference` with `safe_summary` `"capability failed with
/// invalid_input: ..."` — a recoverable, model-visible tool error, not a
/// terminal `driver_unavailable` crash. Distinct from the happy-path test
/// above: this proves the reject path routes through the same
/// Completed-turn/Failed-outcome plumbing instead of aborting the run.
#[tokio::test]
async fn project_create_invalid_input_routes_to_recoverable_tool_error() {
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

/// C-SYNTH fault-injection arm — `project_create` against a
/// `ProjectService::Denied` failure. Distinct from the two tests above: those
/// exercise the real `ProjectService` end-to-end (happy path) or a real,
/// model-controlled input-validation reject (`ProjectServiceError::InvalidInput`,
/// a caller mistake the real store itself detects). This test drives a
/// genuine host-side reject the real local-dev store cannot be coerced into
/// on demand — `create_project` calling through a `FaultInjectingProjectService`
/// decorator wrapping the real service (`project_lifecycle_fault_injected()`),
/// which forces `ProjectServiceError::Denied` only for the sentinel
/// `FAULT_INJECT_DENIED_PROJECT_NAME` and delegates every other call
/// (including a same-turn happy-path create) to the real store.
/// `project_service_outcome`'s `Denied` arm maps this to a model-visible,
/// recoverable `Failed(PolicyDenied)` tool error — NOT the terminal
/// `Internal` arm, which would instead kill the whole run — proving the
/// distinction end-to-end through the real capability dispatch rather than
/// only at `project_create.rs`'s unit-test level.
///
/// Deliberately NOT `ProjectServiceError::Unavailable`/`Internal`: both route
/// through `DefaultRecoveryStrategy`'s capability-retry branch, whose retry
/// re-dispatch hits an independently confirmed production bug for
/// provider-tool-call-originated invocations under local-dev composition
/// (`LocalDevCapabilityIo::resolve_capability_input` rejects the reused
/// `input_ref` on the retry attempt with `InvalidInvocation`/"capability
/// input ref was not staged for this loop run", collapsing the documented
/// "retry twice, then a model-visible `Failed`" contract into an immediate
/// terminal `driver_unavailable`) — see issue #5608. `Denied`
/// surfaces to the model on the FIRST attempt
/// (`capability_error_is_model_visible_tool_failure`), so it exercises a
/// genuine `project_service_outcome` arm without tripping that unrelated bug.
#[tokio::test]
async fn project_create_denied_fault_routes_to_recoverable_tool_error() {
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
