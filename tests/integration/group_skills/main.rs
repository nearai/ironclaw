//! Group integration tests for the skill-management verbs at int tier (C-SKILL).
//!
//! `skill_list`/`skill_install`/`skill_remove` were previously covered ONLY at
//! the QA/trace tier (`with_host_runtime_skill_management_capabilities`,
//! `ironclaw_qa_smoke_scenarios_e2e.rs`), which swaps the whole model gateway and
//! skips the real `ironclaw_llm` decorator chain. This group dispatches the
//! same three verbs through the real turn → capability path, reusing the SAME
//! `HostRuntimeCapabilityHarness::skill_management_tools()` preset the
//! trace-tier harness already wires (`group_constructors.rs`), so the two
//! tiers never drift on capability ids / mounts / policy.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod scenario_install_list_remove;

use ironclaw_support::group::IronClawIntegrationGroup;

#[tokio::test]
async fn skills_group_e2e() {
    let g = IronClawIntegrationGroup::skill_management_tools()
        .await
        .expect("skill-management group builds");

    scenario_install_list_remove::run(&g)
        .await
        .expect("install_list_remove");
}
