//! E-PROJ seam smoke test: the `project_lifecycle` group surfaces the local-dev
//! synthetic `project_create` capability, and a scripted `builtin.project_create`
//! tool call dispatches through the REAL production synthetic-capability wrap
//! (`wrap_local_dev_synthetic_capabilities` + `project_create_capability`) and
//! persists a project via the real `ProjectService`.
//!
//! The result-contains assertion is the mutation-catching one: if
//! `apply_synthetic_capability_wrappers` is made a no-op, the capability is not
//! provided by the port and no success result with the project name is recorded.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::group::RebornIntegrationGroup;
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
}
