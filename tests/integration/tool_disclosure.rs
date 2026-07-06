//! `REBORN_TOOL_DISCLOSURE=Bridged` int-tier coverage (enabler (b), #5149).
//!
//! Proves `.with_tool_disclosure_bridged()` reaches production's
//! `ToolDisclosureCapabilityDecorator` wiring
//! (`ironclaw_reborn::runtime::build_default_planned_runtime_inner`, gated on
//! `DefaultPlannedRuntimeConfig::tool_disclosure.is_bridged()`) — the same
//! lower-level factory this harness's group assembly already calls.
//!
//! Two load-bearing mechanics, both empirically verified (NOT what the
//! original plan text said — divergences noted):
//!
//! 1. **Channel**: bridged mode rewrites the `tools` argument shipped to the
//!    model — captured via `TraceLlm::captured_tool_definitions()`, the same
//!    request field real providers' native tool-calling schema travels
//!    through. It is NOT system-prompt text: tool definitions are a separate
//!    request field from the `System`-role message
//!    `assert_system_prompt_contains` reads.
//! 2. **Threshold gate**: `Bridged` mode alone does NOT defer — deferral is
//!    additionally gated on the catalog exceeding `DisclosureCaps::default()`
//!    (`max_tools: 32` / ~12k estimated schema tokens; `select_active_set`,
//!    `crates/ironclaw_reborn/src/tool_disclosure.rs`). The
//!    `GithubIssueTools` backend surfaces all 48 `github.*` manifest
//!    capabilities (`github_support::capability_ids()`), none of which is
//!    Core-tier (`CORE_TOOL_NAMES` suffix-match misses every github id), so
//!    the deferred active set is exactly the `tool_search` bridge. The
//!    13-capability `BuiltinHttpTools` backend stays UNDER the cap, so
//!    bridged mode is wired-but-inert there — pinned below as the threshold
//!    control.
//!
//! Harness note: bridged groups default to `CapabilityAllowSet::All` (see
//! `into_group`) — production's top-level resolution. Narrowed allow-sets
//! (the #5647 seam) also keep the bridge: bridge ids are host-exempt in
//! `CapabilitySurfaceProfileFilter`, pinned below.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

/// Bridge meta-tool names (`tool_disclosure.rs`'s `TOOL_SEARCH_NAME`/
/// `TOOL_DESCRIBE_NAME`/`TOOL_CALL_NAME`), hardcoded as literals: the
/// constants are `pub(crate)` inside `ironclaw_reborn` and not part of the
/// crate's public surface for a test-tree import. Only `tool_search` is
/// ADVERTISED (`advertised_bridge_tool_definitions`); `tool_describe`/
/// `tool_call` are retained internally for describe-first routing and must
/// NOT appear on the model surface.
const TOOL_SEARCH_NAME: &str = "tool_search";
const TOOL_DESCRIBE_NAME: &str = "tool_describe";
const TOOL_CALL_NAME: &str = "tool_call";

/// Representative flat github tool in provider wire form — dotted capability
/// ids are `__`-encoded on the tool surface (`encode_provider_tool_name`;
/// see `tests/snapshots/golden_payload__tool_call.snap`'s `tool_surface`).
const FLAT_GITHUB_TOOL_NAME: &str = "github__get_repo";

/// Flat first-party tool (wire form) for the below-caps threshold control.
const FLAT_HTTP_TOOL_NAME: &str = "builtin__http";

/// Bridged mode + a catalog over `DisclosureCaps::default().max_tools` (48
/// github capabilities > 32): `select_active_set` defers, so the model sees
/// the `tool_search` bridge (the only ADVERTISED bridge — discovery is
/// tool_search → capability_info → direct call) and NOT the flat `github__*`
/// list, nor the internal-only `tool_describe`/`tool_call` bridges.
#[tokio::test]
async fn bridged_mode_defers_wide_catalog_to_bridge_meta_tools() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(TOOL_SEARCH_NAME)
        .await
        .expect("deferral advertises the tool_search bridge");
    harness
        .assert_model_tools_excludes(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("deferral replaces the flat tool list, not adds to it");
    for internal_bridge in [TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_excludes(internal_bridge)
            .await
            .unwrap_or_else(|error| {
                panic!("bridge {internal_bridge:?} is internal-only, never advertised: {error}")
            });
    }
}

/// Negative control: the SAME wide catalog without
/// `.with_tool_disclosure_bridged()` surfaces the flat 48-tool list (today's
/// default, `ToolDisclosureMode::Off`) — proves the bridged assertion above
/// discriminates on the disclosure mode, not on the backend.
///
/// Pins Off-mode explicitly via `.with_tool_disclosure_off()` rather than
/// leaving this on the `from_env()` default-resolution path: without an
/// explicit pin, an ambient `REBORN_TOOL_DISCLOSURE=Bridged` in the process
/// env would silently flip this control into Bridged mode too, and the
/// assertions below would then be discriminating on nothing.
/// `apply_hermetic_env()` also scrubs the var, but the explicit builder call
/// is what makes this test's mode independent of the ambient env by
/// construction, not just by today's harness hygiene.
#[tokio::test]
async fn default_mode_surfaces_the_flat_wide_tool_list() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_off()
        .with_github_issue_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("default-disclosure harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("default mode keeps the flat tool list");
    harness
        .assert_model_tools_excludes(TOOL_SEARCH_NAME)
        .await
        .expect("default mode never surfaces the bridge meta tools");
}

/// Threshold control: bridged mode with a catalog UNDER
/// `DisclosureCaps::default()` (the 13-capability `BuiltinHttpTools` surface)
/// does NOT defer — the flat list survives and no bridge meta tool appears.
/// Pins that deferral is `mode AND caps-exceeded`, not mode alone: a harness
/// (or production surface) below the cap is wired-but-inert in Bridged mode.
#[tokio::test]
async fn bridged_mode_below_caps_keeps_the_flat_list() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("below-caps bridged harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(FLAT_HTTP_TOOL_NAME)
        .await
        .expect("below the disclosure caps the flat list is unchanged");
    harness
        .assert_model_tools_excludes(TOOL_SEARCH_NAME)
        .await
        .expect("no bridge meta tools below the disclosure caps");
}

/// #5647 regression: a narrowed capability allow-set atop Bridged-mode
/// deferral must not strip the synthetic `ironclaw.*` bridge ids — they are
/// host-exempt in `CapabilitySurfaceProfileFilter`, not granted capabilities.
#[tokio::test]
async fn bridged_mode_survives_narrowed_capability_allow_set() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(TOOL_SEARCH_NAME)
        .await
        .expect(
            "bridge ids are host-owned synthesis, not real capabilities — \
                 a narrowed allow-set must not strip them from a deferred catalog",
        );
    harness
        .assert_model_tools_excludes(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("deferral still replaces the flat list under a narrowed profile");
}

/// #5647 trust boundary: the bridge-id exemption must not widen access to
/// UNDERLYING tools. A deferred call resolves to the real capability id
/// (`github.list_issues`), which the narrowed allow-set still denies at the
/// profile filter's scope check — the exempt set admits only `ironclaw.*`.
#[tokio::test]
async fn narrowed_allow_set_still_denies_non_allowlisted_tool_through_deferral() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
        .script([RebornScriptedReply::tool_call(
            "github.list_issues",
            serde_json::json!({"owner": "octo", "repo": "demo"}),
        )])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    let run_id = harness
        .submit_turn_async("list the issues")
        .await
        .expect("turn submits");
    // Scope rejection at the profile filter discards the whole provider
    // response (model_gateway validate-then-register), surfacing as a
    // model_error-failed turn — coarse, but fails closed.
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("denied out-of-profile call fails the turn");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(failure.category(), "model_error", "got {failure:?}");
    // The load-bearing trust-boundary proof: the underlying tool NEVER
    // dispatched (github tools egress on the network lane).
    harness
        .assert_network_egress_count(0)
        .await
        .expect("a non-allowlisted underlying tool must never reach dispatch");
}

/// #5712: tool_search RESULTS are narrowed by the caller's allow-set — the
/// bridge port's catalog is built below the profile filter, so without
/// result filtering a narrowed profile reads every capability's metadata.
#[tokio::test]
async fn narrowed_allow_set_filters_tool_search_results() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
        .script([
            RebornScriptedReply::tool_call(
                "tool_search",
                serde_json::json!({"query": "repo", "limit": 20}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness
        .submit_turn("find repo tools")
        .await
        .expect("turn completes");

    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded");
    let results = output["results"].as_array().expect("results is an array");
    assert!(
        !results.is_empty(),
        "query must still match the allowlisted github.get_repo"
    );
    for result in results {
        assert_eq!(
            result["capability_id"].as_str(),
            Some("github.get_repo"),
            "non-allowlisted capability metadata leaked into tool_search results: {result}"
        );
    }
}

/// #5712: tool_describe of a non-allowlisted id reads as unknown — same
/// message as a nonexistent name, so existence itself is not disclosed.
#[tokio::test]
async fn narrowed_allow_set_denies_tool_describe_of_non_allowlisted_id() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
        .script([
            RebornScriptedReply::tool_call(
                "tool_describe",
                serde_json::json!({"name": "github.list_issues"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness
        .submit_turn("describe list_issues")
        .await
        .expect("turn completes");

    harness
        .assert_tool_error_summary_contains("tool_describe target is unknown")
        .await
        .expect("a non-allowlisted tool_describe target must read as unknown, not return schema");
}

/// #5712 control: an unnarrowed (All) caller keeps the full search catalog —
/// proves the result filter discriminates on the allow-set, not the query.
#[tokio::test]
async fn unnarrowed_allow_set_keeps_full_tool_search_catalog() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_call(
                "tool_search",
                serde_json::json!({"query": "repo", "limit": 20}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness
        .submit_turn("find repo tools")
        .await
        .expect("turn completes");

    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded");
    let ids: std::collections::BTreeSet<&str> = output["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .filter_map(|result| result["capability_id"].as_str())
        .collect();
    assert!(
        ids.len() > 1,
        "an All allow-set must surface the full catalog's matches, got only {ids:?}"
    );
}
