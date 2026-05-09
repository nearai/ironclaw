//! Caller-tier (Tier B) coverage for runtime-policy filtering of the
//! model-facing tool list.
//!
//! These tests build a `TestRig` with `with_runtime_overrides(...)`, run a
//! short text-only trace, then inspect
//! [`TraceLlm::captured_tool_definitions()`] — the new sibling of the
//! existing `captured_requests()` accessor — to assert *exactly* what tool
//! surface the dispatcher shipped to the model on each iteration.
//!
//! Why this matters: the existing
//! `tests/runtime_policy_tool_visibility_integration.rs` proves
//! `ToolRegistry::tool_definitions_visible_under(policy)` filters at the
//! registry boundary. It does **not** prove the dispatcher actually calls
//! that method on every chat turn. Tier B closes that gap by asserting on
//! the captured payload — a regression that bypasses the filter (shipping
//! the unfiltered tool list to the model) is caught here even if the
//! registry test still passes.
//!
//! ## Known gap (future PR)
//!
//! A hallucinated tool call (e.g. the LLM emits `shell` under HostedDev
//! where `shell` is hidden) is **not** rejected at dispatch today —
//! `tools::execute::execute_tool_with_safety` looks up the tool by name in
//! the registry and ignores the runtime policy. A "policy gate at execute
//! time" enhancement would cover that. We do not lock it in here because
//! locking the *current* behavior would freeze a security gap.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;
    use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};

    /// Tools that declare `ToolRuntimeAffordance::HostFilesystem` under PR
    /// #3243. These resolve as visible iff `policy.filesystem_backend ==
    /// HostWorkspace`. Hosted profiles resolve to `TenantWorkspace` and
    /// must hide all of these.
    const HOST_FS_AFFORDED_TOOLS: &[&str] = &["read_file", "write_file", "list_dir", "apply_patch"];

    // Note on `shell`: it declares `AnyProcess` (not `LocalShell`), and
    // HostedDev's `process_backend == TenantSandbox` satisfies `AnyProcess`
    // — so `shell` is currently visible under HostedDev and we do NOT
    // assert it is hidden. The substrate's
    // `shell_is_hidden_under_hosted_dev_policy_through_dispatcher_path`
    // integration test documents this gap and the future affordance-tightening
    // path. When/if `shell` is retightened to `LocalShell`, both tests
    // flip together.
    fn captured_tool_names_per_call(rig: &crate::support::test_rig::TestRig) -> Vec<Vec<String>> {
        let trace_llm = rig
            .trace_llm()
            .expect("rig must be built with .with_trace(...)");
        trace_llm
            .captured_tool_definitions()
            .into_iter()
            .map(|tools| tools.into_iter().map(|t| t.name).collect())
            .collect()
    }

    #[tokio::test]
    async fn hosted_dev_policy_filters_shell_from_model_facing_tool_list_in_caller_tier() {
        // Build a rig with HostedMultiTenant + HostedDev. The dispatcher must
        // call `tool_definitions_visible_under(policy)` for every chat turn,
        // so the captured tool defs on iteration 1 must exclude every tool
        // declaring an affordance the policy can't satisfy.
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/runtime_policy/hosted_dev_no_shell.json"
        ))
        .expect("failed to load hosted_dev_no_shell.json fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_runtime_overrides(
                DeploymentMode::HostedMultiTenant,
                RuntimeProfile::HostedDev,
                false,
            )
            .build()
            .await;

        rig.run_and_verify_trace(&trace, Duration::from_secs(20))
            .await;

        let captured = captured_tool_names_per_call(&rig);
        assert!(
            !captured.is_empty(),
            "rig must have captured at least one LLM call",
        );
        for (call_index, names) in captured.iter().enumerate() {
            for hidden in HOST_FS_AFFORDED_TOOLS {
                assert!(
                    !names.iter().any(|n| n.as_str() == *hidden),
                    "call #{call_index}: {hidden} must be filtered under HostedDev; got {names:?}",
                );
            }
        }

        rig.shutdown();
    }

    #[tokio::test]
    async fn local_dev_policy_keeps_full_tool_surface_shipped_to_model_in_caller_tier() {
        // Counter-test: under LocalSingleUser + LocalDev the full surface
        // resolves (LocalHost process, HostWorkspace fs, Direct network) —
        // every PR-#3243-affected tool must be visible.
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/runtime_policy/local_dev_text_only.json"
        ))
        .expect("failed to load local_dev_text_only.json fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_runtime_overrides(
                DeploymentMode::LocalSingleUser,
                RuntimeProfile::LocalDev,
                false,
            )
            .build()
            .await;

        rig.run_and_verify_trace(&trace, Duration::from_secs(20))
            .await;

        let captured = captured_tool_names_per_call(&rig);
        assert!(
            !captured.is_empty(),
            "rig must have captured at least one LLM call",
        );
        // The first iteration's captured tool defs must include every
        // host-fs-afforded tool (and shell, which declares AnyProcess).
        // Filtering is per-iteration, but for a text-only trace there's
        // only one captured call.
        let first_call = &captured[0];
        for visible in HOST_FS_AFFORDED_TOOLS {
            assert!(
                first_call.iter().any(|n| n.as_str() == *visible),
                "{visible} must be visible under LocalDev; got {first_call:?}",
            );
        }
        assert!(
            first_call.iter().any(|n| n.as_str() == "shell"),
            "shell must be visible under LocalDev; got {first_call:?}",
        );

        rig.shutdown();
    }

    #[tokio::test]
    async fn runtime_policy_none_preserves_full_tool_surface_in_caller_tier() {
        // Sanity counter-test: when the rig is built WITHOUT
        // `.with_runtime_overrides(...)`, `AgentDeps.runtime_policy` stays
        // `None` (the historical default for every existing test) and the
        // dispatcher must NOT filter — it must call the unfiltered
        // `tool_definitions()`. This locks the new helper's "no opt-in by
        // accident" property: a regression that defaulted the helper to
        // some non-`None` policy would be caught here.
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/runtime_policy/no_policy_text_only.json"
        ))
        .expect("failed to load no_policy_text_only.json fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.run_and_verify_trace(&trace, Duration::from_secs(20))
            .await;

        let captured = captured_tool_names_per_call(&rig);
        assert!(
            !captured.is_empty(),
            "rig must have captured at least one LLM call",
        );
        let first_call = &captured[0];
        // Same affordance-bearing tools as the LocalDev test. The
        // production default is "no filter" so they must all be visible.
        for visible in HOST_FS_AFFORDED_TOOLS {
            assert!(
                first_call.iter().any(|n| n.as_str() == *visible),
                "{visible} must be in unfiltered tool surface (runtime_policy=None); got {first_call:?}",
            );
        }
        assert!(
            first_call.iter().any(|n| n.as_str() == "shell"),
            "shell must be in unfiltered tool surface; got {first_call:?}",
        );

        rig.shutdown();
    }
}
