//! `REBORN_TOOL_DISCLOSURE=Bridged` int-tier coverage (enabler (b), #5149).
//!
//! Proves `.with_tool_disclosure_bridged()` reaches production's
//! `ToolDisclosureCapabilityDecorator` wiring
//! (`ironclaw_turn_runner::runtime::build_default_planned_runtime_inner`, gated on
//! `DefaultPlannedRuntimeConfig::tool_disclosure.is_enabled()`) — the same
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
//!    `crates/loop/ironclaw_loop_host/src/tool_disclosure.rs`). The
//!    `GithubIssueTools` backend surfaces all 48 `github.*` manifest
//!    capabilities (`github_support::capability_ids()`), none of which is
//!    Core-tier (`CORE_TOOL_NAMES` suffix-match misses every github id). The
//!    production interactive profile may additionally keep reviewed pins
//!    visible beside the complete discovery bridge set. The
//!    13-capability `BuiltinHttpTools` backend stays UNDER the cap, so
//!    bridged mode is wired-but-inert there — pinned below as the threshold
//!    control.
//!
//! Harness note: bridged groups default to `CapabilitySurfacePolicy::allow_all()`
//! (see `into_group`) — production's top-level resolution. Narrowed policies
//! (the #5647 seam) still keep the synthetic disclosure bridge when deferral is
//! needed, while its catalog contents are filtered by that exact policy.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_host_api::capability_surface::CapabilitySurfacePolicy;
use ironclaw_loop_contracts::BatchPolicyKind;
use ironclaw_loop_host::ToolDisclosureMode;
use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::extension_surface::BUNDLED_EXTENSION_CAPABILITY_IDS;
use reborn_support::reply::RebornScriptedReply;

/// Bridge meta-tool names (`tool_disclosure.rs`'s `TOOL_SEARCH_NAME`/
/// `TOOL_DESCRIBE_NAME`/`TOOL_CALL_NAME`), hardcoded as literals: the
/// constants are `pub(crate)` inside `ironclaw_loop_host` (the cluster moved
/// there with the WS3 runner sheds) and are not part of that crate's public
/// surface for a test-tree import.
const TOOL_SEARCH_NAME: &str = "tool_search";
const TOOL_DESCRIBE_NAME: &str = "tool_describe";
const TOOL_CALL_NAME: &str = "tool_call";

/// Representative flat github tool in provider wire form — dotted capability
/// ids are `__`-encoded on the tool surface (`encode_provider_tool_name`;
/// see `tests/snapshots/golden_payload__tool_call.snap`'s `tool_surface`).
const FLAT_GITHUB_TOOL_NAME: &str = "github__get_repo";

/// Flat first-party tool (wire form) for the below-caps threshold control.
const FLAT_HTTP_TOOL_NAME: &str = "builtin__http";

/// Globally disabled in the production planned-runtime configuration. The
/// disclosure catalog must never reveal it even though the spawn decorator
/// installs its definition below disclosure.
const SPAWN_SUBAGENT_CAPABILITY_ID: &str = "builtin.spawn_subagent";
const SPAWN_SUBAGENT_TOOL_NAME: &str = "builtin__spawn_subagent";

fn deferred_bridge_script() -> [RebornScriptedReply; 4] {
    [
        RebornScriptedReply::tool_call(
            TOOL_SEARCH_NAME,
            serde_json::json!({"query": "get repository", "limit": 5}),
        ),
        RebornScriptedReply::tool_call(
            TOOL_DESCRIBE_NAME,
            serde_json::json!({"name": FLAT_GITHUB_TOOL_NAME}),
        ),
        RebornScriptedReply::tool_call(
            TOOL_CALL_NAME,
            serde_json::json!({
                "name": FLAT_GITHUB_TOOL_NAME,
                "arguments": r#"{"owner":"nearai","repo":"ironclaw"}"#
            }),
        ),
        RebornScriptedReply::text("done"),
    ]
}

async fn assert_deferred_bridge_flow(harness: &RebornIntegrationHarness) {
    harness
        .assert_model_tool_result_content_occurrences("get repository", 1)
        .await
        .expect("bounded search result reaches the next production model request");
    harness
        .assert_model_tool_result_content_occurrences("additionalProperties", 1)
        .await
        .expect("describe schema reaches the next production model request");
    harness
        .assert_tool_invoked("github.get_repo")
        .await
        .expect("tool_call dispatches the selected target through the inner capability port");
    harness
        .assert_network_egress_header_contains(
            "api.github.com/repos/nearai/ironclaw",
            "authorization",
            "token ghp_fake_fixture_token",
        )
        .await
        .expect("tool_call target reaches mediated GitHub egress with injected credentials");
    harness
        .assert_reply_contains("done")
        .await
        .expect("turn completes after the deferred capability result");
}

/// More than `DisclosureCaps::default().max_tools` GitHub capabilities while
/// still excluding the tail of the catalog, so narrowed deferred-mode tests
/// exercise both bridge availability and metadata filtering.
const WIDE_EFFECTIVE_GITHUB_CAPABILITY_COUNT: usize = 33;

fn wide_effective_github_allowlist() -> impl Iterator<Item = &'static str> {
    BUNDLED_EXTENSION_CAPABILITY_IDS[..WIDE_EFFECTIVE_GITHUB_CAPABILITY_COUNT]
        .iter()
        .copied()
}

/// Bridged mode + a catalog over `DisclosureCaps::default().max_tools` (48
/// github capabilities > 32): `select_active_set` defers, so the model sees
/// the complete advertised `tool_search` → `tool_describe` → `tool_call`
/// bridge set and NOT the ordinary flat `github__*` list (reviewed profile pins
/// may remain directly visible).
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

    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_contains(bridge)
            .await
            .unwrap_or_else(|error| panic!("deferral must advertise bridge {bridge:?}: {error}"));
    }
    harness
        .assert_model_tools_excludes(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("deferral replaces the flat tool list, not adds to it");
}

/// Regression for prod run df55a9c5: the model discovered the globally
/// disabled spawn tool through `tool_search`, loaded its schema, then retried
/// the denied invocation until the run failed. A capability excluded by the
/// resolved model-surface policy must be absent from every bridged disclosure
/// path, not merely from the flat provider tool list.
#[tokio::test]
async fn bridged_disclosure_never_advertises_globally_disabled_spawn_subagent() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "spawn subagent", "limit": 20}),
            ),
            RebornScriptedReply::tool_call(
                TOOL_DESCRIBE_NAME,
                serde_json::json!({"name": SPAWN_SUBAGENT_TOOL_NAME}),
            ),
            RebornScriptedReply::tool_call(
                TOOL_CALL_NAME,
                serde_json::json!({
                    "name": SPAWN_SUBAGENT_TOOL_NAME,
                    "arguments": r#"{"goal":"inspect the repository"}"#,
                }),
            ),
            RebornScriptedReply::text("done without spawning"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness
        .submit_turn("inspect the repository without spawning a subagent")
        .await
        .expect("disabled spawn remains a recoverable unknown tool");

    harness
        .assert_model_tools_contains(TOOL_SEARCH_NAME)
        .await
        .expect("allowed disclosure bridge remains advertised");
    harness
        .assert_model_tools_excludes(SPAWN_SUBAGENT_TOOL_NAME)
        .await
        .expect("disabled spawn is absent from the flat provider tool list");
    harness
        .assert_model_tool_description_excludes(TOOL_SEARCH_NAME, SPAWN_SUBAGENT_TOOL_NAME)
        .await
        .expect("disabled spawn is absent from tool_search's catalog index");

    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded");
    assert!(
        output["results"]
            .as_array()
            .expect("results is an array")
            .iter()
            .all(|result| result["capability_id"].as_str() != Some(SPAWN_SUBAGENT_CAPABILITY_ID)),
        "disabled spawn capability leaked into tool_search results: {output}"
    );

    harness
        .assert_tool_error_summary_contains("tool_describe target is unknown")
        .await
        .expect("disabled spawn schema is not describable");
    harness
        .assert_tool_error_summary_contains("tool_call target is not a known tool")
        .await
        .expect("disabled spawn is not resolvable through the bridge");
    harness
        .assert_tool_not_invoked(SPAWN_SUBAGENT_CAPABILITY_ID)
        .await
        .expect("disabled spawn never reaches dispatch");
    harness
        .assert_capability_result_count(SPAWN_SUBAGENT_CAPABILITY_ID, 0)
        .await
        .expect("disabled spawn produces no capability result");
    harness
        .assert_reply_contains("done without spawning")
        .await
        .expect("the run continues after the recoverable unknown-tool results");
}

/// Search and describe are read-only catalog lookups, so one model response
/// may safely probe them together. A recoverable bad describe must not prevent
/// its valid siblings from completing.
#[tokio::test]
async fn discovery_batch_classifies_parallel_and_preserves_valid_siblings() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_calls([
                (
                    TOOL_SEARCH_NAME,
                    serde_json::json!({"query": "get repository", "limit": 5}),
                ),
                (
                    TOOL_DESCRIBE_NAME,
                    serde_json::json!({"name": FLAT_GITHUB_TOOL_NAME}),
                ),
                (
                    TOOL_DESCRIBE_NAME,
                    serde_json::json!({"name": "missing__catalog_tool"}),
                ),
            ]),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness
        .submit_turn("probe the github catalog")
        .await
        .expect("discovery batch completes despite one recoverable lookup failure");

    harness
        .assert_capability_batch_policy(3, BatchPolicyKind::Parallel)
        .await
        .expect("side-effect-free discovery bridges classify parallel");
    harness
        .assert_model_tool_result_content_occurrences("get repository", 1)
        .await
        .expect("valid bounded search sibling reaches the next model request");
    harness
        .assert_model_tool_result_content_occurrences("additionalProperties", 1)
        .await
        .expect("valid describe schema sibling reaches the next model request");
    harness
        .assert_model_tool_result_content_occurrences("tool_describe target is unknown", 1)
        .await
        .expect("recoverable invalid describe reaches the next model request");
    harness
        .assert_reply_contains("done")
        .await
        .expect("turn continues after the recoverable bad describe");
}

/// Negative control: the SAME wide catalog under explicit
/// `ToolDisclosureMode::Off` surfaces the flat 48-tool list — proves the
/// bridged assertion above discriminates on the disclosure mode, not on the
/// backend.
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
async fn explicit_off_surfaces_the_flat_wide_tool_list() {
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
        .expect("explicit Off keeps the flat tool list");
    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_excludes(bridge)
            .await
            .unwrap_or_else(|error| {
                panic!("explicit Off must exclude discovery bridge {bridge:?}: {error}")
            });
    }
}

/// The production default enables progressive disclosure for a wide catalog.
/// The `ironclaw_loop_host` unit contract separately proves that an unset or
/// empty environment value resolves to this default.
#[tokio::test]
async fn production_default_defers_wide_catalog_to_bridge_meta_tools() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("production-default disclosure harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_contains(bridge)
            .await
            .unwrap_or_else(|error| {
                panic!("production default must advertise bridge {bridge:?}: {error}")
            });
    }
    harness
        .assert_model_tools_excludes(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("production default defers the flat wide catalog");
    harness
        .assert_model_tools_excludes("github__search_code")
        .await
        .expect("namespace-summary production default does not expose opt-in profile pins");
}

#[tokio::test]
async fn bridged_mode_exposes_authorized_profile_pin_for_matching_profile() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("bridged profile-pin harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains("github__search_code")
        .await
        .expect("matching interactive profile exposes its authorized pin");
    harness
        .assert_model_tools_excludes(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("an unrelated deferred GitHub tool remains deferred");
}

#[tokio::test]
async fn comparison_arms_are_selectable_through_the_production_caller_path() {
    let compact = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_mode(ToolDisclosureMode::Compact)
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "get repository", "limit": 1}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("compact comparison harness builds");
    compact
        .submit_turn("find repo tool")
        .await
        .expect("compact turn");
    let compact_output = compact
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("compact search output");
    assert!(
        compact_output["results"][0]
            .get("schema_complete")
            .is_none()
    );
    compact
        .assert_model_tool_description_excludes(TOOL_SEARCH_NAME, "Namespaces:")
        .await
        .expect("compact arm uses the legacy preview");

    let namespaces = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_mode(ToolDisclosureMode::Namespaces)
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "get repository", "limit": 1}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("namespace comparison harness builds");
    namespaces
        .submit_turn("find repo tool")
        .await
        .expect("namespace turn");
    let namespace_output = namespaces
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("namespace search output");
    assert_eq!(namespace_output["results"][0]["schema_complete"], true);
    namespaces
        .assert_model_tool_description_contains(TOOL_SEARCH_NAME, "Namespaces:")
        .await
        .expect("namespace arm uses the fair namespace preview");
    namespaces
        .assert_model_tools_excludes("github__search_code")
        .await
        .expect("namespace arm does not enable the production pin set");
}

#[tokio::test]
async fn denied_profile_pin_is_absent_from_surface_preview_search_and_calls() {
    let allowed = BUNDLED_EXTENSION_CAPABILITY_IDS
        .iter()
        .copied()
        .filter(|id| *id != "github.search_code")
        .take(WIDE_EFFECTIVE_GITHUB_CAPABILITY_COUNT)
        .collect::<Vec<_>>();
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(allowed)
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "search code", "limit": 20}),
            ),
            RebornScriptedReply::tool_call(
                TOOL_CALL_NAME,
                serde_json::json!({
                    "name": "github__search_code",
                    "arguments": r#"{"query":"repo:nearai/ironclaw ToolDisclosureMode"}"#,
                }),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("denied-pin harness builds");

    harness
        .submit_turn("search code")
        .await
        .expect("turn completes");
    harness
        .assert_model_tools_excludes("github__search_code")
        .await
        .expect("denied pin is absent from the direct surface");
    harness
        .assert_model_tool_description_excludes(TOOL_SEARCH_NAME, "github__search_code")
        .await
        .expect("denied pin is absent from the passive preview");
    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded");
    assert!(
        output["results"].as_array().is_some_and(|results| results
            .iter()
            .all(|result| result["capability_id"] != "github.search_code")),
        "denied pin must not leak through explicit search: {output}"
    );
    harness
        .assert_tool_error_summary_contains("tool_call target is not a known tool")
        .await
        .expect("denied pin is not callable through the bridge");
}

/// General harnesses pin Off rather than inheriting the production environment,
/// so unrelated integration tests remain stable when the production default can
/// safely change after the authorization prerequisite lands.
#[tokio::test]
async fn hermetic_harness_defaults_to_off_for_wide_catalogs() {
    let harness = RebornIntegrationHarness::test_default()
        .with_github_issue_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("hermetic default harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("hermetic default keeps the flat tool list");
    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_excludes(bridge)
            .await
            .unwrap_or_else(|error| {
                panic!("hermetic default must pin Off and exclude {bridge:?}: {error}")
            });
    }
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
    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_excludes(bridge)
            .await
            .unwrap_or_else(|error| {
                panic!("below-threshold surfaces must not advertise bridge {bridge:?}: {error}")
            });
    }
}

/// Caller-path proof for the provider-facing protocol: every advertised bridge
/// registers through the production capability-port factory, and the final
/// `tool_call` dispatches the real bundled GitHub capability through mediated
/// host egress.
#[tokio::test]
async fn deferred_search_describe_call_flow_uses_production_capability_chain() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script(deferred_bridge_script())
        .build()
        .await
        .expect("bridged-disclosure harness builds");
    harness
        .submit_turn("find and inspect the ironclaw repository")
        .await
        .expect("search, describe, and call complete");
    assert_deferred_bridge_flow(&harness).await;
}

/// Selector-budget regression: a physically wide catalog with only one
/// permitted capability is effectively below the disclosure caps. The one
/// permitted tool stays flat and directly callable; no bridge is advertised.
#[tokio::test]
async fn bridged_mode_single_permitted_tool_stays_flat_and_direct() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(["github.get_repo"])
        .script([
            RebornScriptedReply::tool_call(
                "github.get_repo",
                serde_json::json!({"owner": "octo", "repo": "demo"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness
        .submit_turn("inspect the permitted repository")
        .await
        .expect("direct permitted tool call completes");

    harness
        .assert_model_tools_contains(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("the sole permitted tool stays on the flat model surface");
    harness
        .assert_model_tools_excludes(TOOL_SEARCH_NAME)
        .await
        .expect("an effectively below-cap surface must not advertise tool_search");
    harness
        .assert_network_egress_count(1)
        .await
        .expect("the permitted flat tool remains directly callable");
}

/// Empty effective surface: no real tool or synthetic bridge is advertised.
/// The test drives a normal text turn and inspects the actual model tool list;
/// it does not guess a bridge name that the empty surface never offered.
#[tokio::test]
async fn empty_policy_advertises_no_tools() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test([])
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("empty-policy bridged-disclosure harness builds");

    harness
        .submit_turn("answer without tools")
        .await
        .expect("text-only turn completes");

    harness
        .assert_model_tools_empty()
        .await
        .expect("an empty effective policy advertises no tools or bridges");
    harness
        .assert_network_egress_count(0)
        .await
        .expect("an empty surface performs no capability side effects");
}

/// The disclosure catalog must consume the complete policy-qualified visible
/// surface, not rebuild a broader corpus by checking capability IDs alone.
/// An empty runtime dimension therefore hides host-runtime tools and must not
/// synthesize discovery bridges for their excluded metadata, even though every
/// capability ID remains permitted. Host-synthetic core tools are outside that
/// runtime catalog and remain governed by their owning surface.
#[tokio::test]
async fn runtime_excluded_policy_advertises_no_runtime_tools_or_disclosure_bridges() {
    let mut policy = CapabilitySurfacePolicy::allow_all();
    policy.allowed_runtimes.clear();
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_capability_surface_policy_for_bridged_test(policy)
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("runtime-excluded bridged-disclosure harness builds");

    harness
        .submit_turn("answer without tools")
        .await
        .expect("text-only turn completes");

    harness
        .assert_model_tools_excludes(FLAT_GITHUB_TOOL_NAME)
        .await
        .expect("a runtime-excluded policy advertises no host-runtime tools");
    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        harness
            .assert_model_tools_excludes(bridge)
            .await
            .expect("runtime-excluded metadata must not synthesize a disclosure bridge");
    }
    harness
        .assert_network_egress_count(0)
        .await
        .expect("a runtime-excluded surface performs no capability side effects");
}

/// #5647 trust boundary: the bridge-id exemption must not widen access to
/// UNDERLYING tools. Even a malformed deferred call that would otherwise take
/// the describe-first recovery path resolves to the real capability id
/// (`github.list_issues`), which the narrowed policy still denies at the
/// profile filter's scope check — the exempt set admits only `ironclaw.*`.
#[tokio::test]
async fn narrowed_policy_still_denies_non_allowlisted_tool_through_deferral() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(["github.get_repo"])
        .script([RebornScriptedReply::tool_call(
            "github.list_issues",
            serde_json::json!({}),
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
    // model_unavailable-failed turn — coarse, but fails closed (#5692 renamed
    // this category from the generic "model_error").
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("denied out-of-profile call fails the turn");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(failure.category(), "model_unavailable", "got {failure:?}");
    // The load-bearing trust-boundary proof: the underlying tool NEVER
    // dispatched (github tools egress on the network lane).
    harness
        .assert_network_egress_count(0)
        .await
        .expect("a non-allowlisted underlying tool must never reach dispatch");
}

/// #5659-w6 follow-up: a genuinely wide effective policy still defers and
/// keeps the tool_search bridge, whose own advertised *description*
/// (the always-on catalog index of discoverable tool names, see
/// `catalog_index_tool_search_description`) must be narrowed by the caller's
/// policy too — not just tool_search RESULTS and tool_describe (#5712).
/// The bridge is synthesized outside the filtered base surface (#5647), so the
/// disclosure catalog must apply the same policy before constructing the text.
#[tokio::test]
async fn bridged_mode_wide_effective_policy_keeps_narrowed_tool_search_description() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(wide_effective_github_allowlist())
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(TOOL_SEARCH_NAME)
        .await
        .expect("bridge ids stay advertised under a narrowed policy (#5647)");
    harness
        .assert_model_tool_description_contains(TOOL_SEARCH_NAME, FLAT_GITHUB_TOOL_NAME)
        .await
        .expect(
            "the allowlisted tool's name must still be discoverable via tool_search's own \
                 advertised description index — narrowing must not empty the index outright",
        );
    harness
        .assert_model_tool_description_excludes(TOOL_SEARCH_NAME, "github__handle_webhook")
        .await
        .expect(
            "non-allowlisted tool name must not leak via tool_search's own \
                 advertised description index",
        );
}

/// #5712: tool_search RESULTS are narrowed by the caller's policy — the
/// bridge port's catalog is built below the profile filter, so without
/// result filtering a narrowed profile reads every capability's metadata.
#[tokio::test]
async fn narrowed_policy_filters_tool_search_results() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(wide_effective_github_allowlist())
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
    let allowed: std::collections::BTreeSet<&str> = wide_effective_github_allowlist().collect();
    for result in results {
        let capability_id = result["capability_id"]
            .as_str()
            .expect("search result capability id");
        assert!(
            allowed.contains(capability_id),
            "non-allowlisted capability metadata leaked into tool_search results: {result}"
        );
    }
}

/// #7177: retrieval must use admitted schema vocabulary, not only provider
/// names and top-level descriptions. `committer` exists in the GitHub file
/// mutation schemas but not in their catalog descriptions.
#[tokio::test]
async fn tool_search_discovers_authorized_tools_by_parameter_only_vocabulary() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "committer", "limit": 10}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness
        .submit_turn("find a tool that accepts committer identity")
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
        ids.contains("github.delete_file") || ids.contains("github.create_or_update_file"),
        "parameter-only query must discover a matching authorized file tool, got {ids:?}"
    );
    harness
        .assert_model_tool_description_excludes(TOOL_SEARCH_NAME, "committer")
        .await
        .expect("richer internal search metadata must not grow the prompt-side names-only index");
}

/// #5712: tool_describe of a non-allowlisted id reads as unknown — same
/// message as a nonexistent name, so existence itself is not disclosed.
///
/// A substring check on `safe_summary` alone would pass even for an empty
/// index, and would miss an existence oracle hiding in the envelope's other
/// fields (`model_observation`'s structured diagnostic, in particular). This
/// scripts BOTH a non-allowlisted target (`github.list_issues`, present in
/// the catalog but outside the policy) and a target that is not in the
/// catalog at all, then asserts their persisted `ToolResultReferenceEnvelope`s
/// are identical modulo `result_ref` — which is derived from
/// `RebornScriptedReply::tool_call`'s process-global call-id counter (see
/// `synthetic_provider_error_result_ref`) and so differs between the two
/// calls by construction, carrying no policy/existence information.
#[tokio::test]
async fn narrowed_policy_denies_tool_describe_of_non_allowlisted_id() {
    const NONEXISTENT_TARGET: &str = "totally_nonexistent_tool";
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(["github.get_repo"])
        .script([
            RebornScriptedReply::tool_call(
                "tool_describe",
                serde_json::json!({"name": "github.list_issues"}),
            ),
            RebornScriptedReply::tool_call(
                "tool_describe",
                serde_json::json!({"name": NONEXISTENT_TARGET}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness
        .submit_turn("describe list_issues, then a made-up tool")
        .await
        .expect("turn completes");

    harness
        .assert_tool_error_summary_contains("tool_describe target is unknown")
        .await
        .expect("a non-allowlisted tool_describe target must read as unknown, not return schema");

    let envelopes = harness
        .persisted_tool_result_envelopes()
        .await
        .expect("both tool_describe calls persist a ToolResultReference");
    assert_eq!(
        envelopes.len(),
        2,
        "expected exactly one ToolResultReference per scripted tool_describe call, got {envelopes:?}"
    );
    let (non_allowlisted, nonexistent) = (&envelopes[0], &envelopes[1]);
    assert_ne!(
        non_allowlisted.result_ref, nonexistent.result_ref,
        "sanity: the two calls' result_refs must differ (distinct scripted call ids) — \
         otherwise this test isn't actually comparing two separate persisted results"
    );
    assert_eq!(
        non_allowlisted.version, nonexistent.version,
        "envelope schema version must not vary by target"
    );
    assert_eq!(
        non_allowlisted.safe_summary, nonexistent.safe_summary,
        "non-allowlisted vs nonexistent tool_describe must read byte-identical safe_summary"
    );
    assert_eq!(
        non_allowlisted.model_observation, nonexistent.model_observation,
        "non-allowlisted vs nonexistent tool_describe must read byte-identical model_observation \
         — a differing diagnostic/status here would be an existence oracle the safe_summary check alone would miss"
    );
}

/// A host-exempt `tool_call` bridge must not expose whether a requested target
/// exists outside the caller's effective policy. Both a catalog-known denied
/// target and a genuinely nonexistent target stay on the same recoverable bridge
/// path and persist byte-equivalent result envelopes modulo their run-scoped ids.
#[tokio::test]
async fn narrowed_policy_denies_tool_call_of_non_allowlisted_id_without_existence_oracle() {
    const NONEXISTENT_TARGET: &str = "totally_nonexistent_tool";
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_surface_policy_for_bridged_test(["github.get_repo"])
        .script([
            RebornScriptedReply::tool_call(
                TOOL_CALL_NAME,
                serde_json::json!({"name": "github.list_issues", "arguments": {}}),
            ),
            RebornScriptedReply::tool_call(
                TOOL_CALL_NAME,
                serde_json::json!({"name": NONEXISTENT_TARGET, "arguments": {}}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness
        .submit_turn("call list_issues, then a made-up tool")
        .await
        .expect("turn completes");

    harness
        .assert_tool_error_summary_contains("tool_call target is not a known tool")
        .await
        .expect("a non-allowlisted tool_call target must read as unknown");

    let envelopes = harness
        .persisted_tool_result_envelopes()
        .await
        .expect("both tool_call attempts persist a ToolResultReference");
    assert_eq!(
        envelopes.len(),
        2,
        "expected exactly one ToolResultReference per scripted tool_call, got {envelopes:?}"
    );
    let (non_allowlisted, nonexistent) = (&envelopes[0], &envelopes[1]);
    assert_ne!(
        non_allowlisted.result_ref, nonexistent.result_ref,
        "sanity: distinct scripted calls must carry distinct run-scoped result refs"
    );
    assert_eq!(non_allowlisted.version, nonexistent.version);
    assert_eq!(
        non_allowlisted.safe_summary, nonexistent.safe_summary,
        "non-allowlisted and nonexistent tool_call targets must have identical summaries"
    );
    assert_eq!(
        non_allowlisted.model_observation, nonexistent.model_observation,
        "non-allowlisted and nonexistent tool_call targets must have identical model observations"
    );
}

/// #5712 control: an unnarrowed (All) caller keeps the full search catalog —
/// proves the result filter discriminates on the policy, not the query.
#[tokio::test]
async fn unnarrowed_policy_keeps_full_tool_search_catalog() {
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
        "an unrestricted policy must surface the full catalog's matches, got only {ids:?}"
    );
}

/// A tool ranker that ignores the query and returns a fixed ranking, so the
/// test can tell its output apart from the native BM25F ranker's. It records
/// the size of every corpus it is fitted on.
#[derive(Debug)]
struct FixedRankingRetrieval {
    ranking: Vec<ironclaw_loop_contracts::RankedTool>,
    fitted_corpus_sizes: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
}

#[derive(Debug)]
struct FixedRankingIndex {
    ranking: Vec<ironclaw_loop_contracts::RankedTool>,
}

#[async_trait::async_trait]
impl ironclaw_loop_contracts::ToolRetrievalIndex for FixedRankingIndex {
    async fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<
        ironclaw_loop_contracts::ToolSearchOutcome,
        ironclaw_loop_contracts::ToolRetrievalError,
    > {
        Ok(ironclaw_loop_contracts::ToolSearchOutcome {
            ranked: self.ranking.clone(),
            query_class: ironclaw_loop_contracts::ToolSearchQueryClass::Lexical,
        })
    }
}

#[async_trait::async_trait]
impl ironclaw_loop_contracts::ToolRetrievalProvider for FixedRankingRetrieval {
    fn ranker_version(&self) -> &str {
        "fixed-ranking-integration-v1"
    }

    async fn fit(
        &self,
        definitions: &[ironclaw_loop_contracts::ProviderToolDefinition],
    ) -> Result<
        std::sync::Arc<dyn ironclaw_loop_contracts::ToolRetrievalIndex>,
        ironclaw_loop_contracts::ToolRetrievalError,
    > {
        self.fitted_corpus_sizes
            .lock()
            .expect("fit recorder lock")
            .push(definitions.len());
        Ok(std::sync::Arc::new(FixedRankingIndex {
            ranking: self.ranking.clone(),
        }))
    }
}

/// A deployment binds an alternate tool ranker through the runtime build path
/// (`DefaultPlannedRuntimeParts::tool_retrieval_provider` ->
/// `ToolDisclosureCapabilityDecorator::with_retrieval_provider`), and
/// `tool_search` returns that ranker's order rather than native BM25F's.
/// The host still drops a name the ranker invented outside the authorized
/// corpus.
#[tokio::test]
async fn bound_tool_retrieval_provider_ranks_tool_search_through_the_runtime_build_path() {
    use ironclaw_loop_contracts::RankedTool;

    let fitted_corpus_sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = std::sync::Arc::new(FixedRankingRetrieval {
        ranking: vec![
            RankedTool::new("github__create_issue", 0.9),
            RankedTool::new("invented__not_authorized", 0.8),
            RankedTool::new(FLAT_GITHUB_TOOL_NAME, 0.3),
        ],
        fitted_corpus_sizes: std::sync::Arc::clone(&fitted_corpus_sizes),
    });
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_tool_retrieval_provider(provider)
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "list open issues", "limit": 5}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness
        .submit_turn("find issue tools")
        .await
        .expect("turn completes");

    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded");
    let names: Vec<&str> = output["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .filter_map(|result| result["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec!["github__create_issue", FLAT_GITHUB_TOOL_NAME],
        "tool_search must return the bound ranker's order, minus names outside the corpus"
    );
    let fitted = fitted_corpus_sizes
        .lock()
        .expect("fit recorder lock")
        .clone();
    assert!(
        fitted.iter().all(|size| *size > 32),
        "the bound ranker is fitted over the whole wide authorized catalog: {fitted:?}"
    );
    assert!(!fitted.is_empty(), "the bound ranker was never fitted");
    harness
        .assert_reply_contains("done")
        .await
        .expect("turn completes after the search");
}

/// Bag-of-words stand-in for an embedding endpoint: one dimension per known
/// word, plus a constant so no vector is all zeros. Records every text it is
/// asked to embed. With `fail_queries`, it embeds tool documents but fails
/// every other text, which is how a dense side fails at search time.
struct KeywordEmbedder {
    embedded: std::sync::Mutex<Vec<String>>,
    fail_queries: bool,
}

impl KeywordEmbedder {
    fn new(fail_queries: bool) -> Self {
        Self {
            embedded: std::sync::Mutex::new(Vec::new()),
            fail_queries,
        }
    }
}

const KEYWORD_EMBEDDER_VOCABULARY: &[&str] =
    &["create", "issue", "comment", "pull", "list", "repo"];

#[async_trait::async_trait]
impl ironclaw_llm::embeddings::EmbeddingProvider for KeywordEmbedder {
    fn model_name(&self) -> &str {
        "keyword-embedder"
    }

    fn dimension(&self) -> Option<usize> {
        Some(KEYWORD_EMBEDDER_VOCABULARY.len() + 1)
    }

    async fn embed(
        &self,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ironclaw_llm::embeddings::EmbeddingError> {
        self.embedded
            .lock()
            .expect("embedder lock")
            .extend(texts.iter().cloned());
        if self.fail_queries && texts.iter().any(|text| !text.starts_with("tool: ")) {
            return Err(ironclaw_llm::embeddings::EmbeddingError::RequestFailed {
                reason: "query embedding refused by the test embedder".to_string(),
            });
        }
        Ok(texts
            .iter()
            .map(|text| {
                let lower = text.to_lowercase();
                let words: Vec<&str> = lower
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .collect();
                KEYWORD_EMBEDDER_VOCABULARY
                    .iter()
                    .map(|term| words.iter().filter(|word| *word == term).count() as f32)
                    .chain(std::iter::once(0.01))
                    .collect()
            })
            .collect())
    }
}

/// The dense tool ranker package, bound through the same runtime build path,
/// serves `tool_search`: it is fitted over the authorized catalog, embeds the
/// model's query, and its cosine order is what the model gets back.
#[tokio::test]
async fn dense_tool_ranker_serves_tool_search_through_the_runtime_build_path() {
    let embedder = std::sync::Arc::new(KeywordEmbedder::new(false));
    let provider = std::sync::Arc::new(ironclaw_tool_retrieval::DenseToolRetrievalProvider::new(
        std::sync::Arc::clone(&embedder)
            as std::sync::Arc<dyn ironclaw_llm::embeddings::EmbeddingProvider>,
    ));
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_tool_retrieval_provider(provider)
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "create issue", "limit": 3}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");

    harness
        .submit_turn("open a bug report")
        .await
        .expect("turn completes");

    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded");
    let names: Vec<&str> = output["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .filter_map(|result| result["name"].as_str())
        .collect();
    assert_eq!(
        names.first(),
        Some(&"github__create_issue"),
        "the closest tool by cosine similarity ranks first: {names:?}"
    );
    assert!(names.len() <= 3, "the limit holds: {names:?}");

    let embedded = embedder.embedded.lock().expect("embedder lock").clone();
    assert!(
        embedded
            .iter()
            .filter(|text| text.starts_with("tool: "))
            .count()
            > 32,
        "the ranker embeds the whole wide authorized catalog"
    );
    assert!(
        embedded.iter().any(|text| text == "create issue"),
        "the ranker embeds the model's query"
    );
    harness
        .assert_reply_contains("done")
        .await
        .expect("turn completes after the search");
}

/// Run one `tool_search` for "create issue" (limit 3) through the runtime
/// build path, with `provider` bound or (for `None`) the native ranker, and
/// return the ranked names the model got back.
async fn tool_search_names_through_the_runtime_build_path(
    provider: Option<std::sync::Arc<dyn ironclaw_loop_contracts::ToolRetrievalProvider>>,
) -> Vec<String> {
    let mut builder = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools();
    if let Some(provider) = provider {
        builder = builder.with_tool_retrieval_provider(provider);
    }
    let harness = builder
        .script([
            RebornScriptedReply::tool_call(
                TOOL_SEARCH_NAME,
                serde_json::json!({"query": "create issue", "limit": 3}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("bridged-disclosure harness builds");
    harness
        .submit_turn("open a bug report")
        .await
        .expect("turn completes");
    let output = harness
        .tool_result_output("ironclaw.tool_search")
        .await
        .expect("tool_search result recorded, never a failure");
    harness
        .assert_reply_contains("done")
        .await
        .expect("turn completes after the search");
    output["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .filter_map(|result| result["name"].as_str().map(str::to_string))
        .collect()
}

fn hybrid_over(
    embedder: &std::sync::Arc<KeywordEmbedder>,
) -> std::sync::Arc<dyn ironclaw_loop_contracts::ToolRetrievalProvider> {
    let dense = std::sync::Arc::new(ironclaw_tool_retrieval::DenseToolRetrievalProvider::new(
        std::sync::Arc::clone(embedder)
            as std::sync::Arc<dyn ironclaw_llm::embeddings::EmbeddingProvider>,
    ));
    std::sync::Arc::new(ironclaw_loop_host::HybridToolRetrieval::new(Some(dense)))
}

/// Hybrid ranking (BM25F fused with the dense ranker), bound through the same
/// runtime build path, serves `tool_search`: the dense side is fitted over the
/// catalog and consulted with the model's query, and fusion puts the tool both
/// rankers agree on first. When the dense side fails the search, the model
/// gets exactly the native BM25F ranking rather than a failed search.
#[tokio::test]
async fn hybrid_tool_ranker_serves_tool_search_and_degrades_to_bm25f() {
    let native = tool_search_names_through_the_runtime_build_path(None).await;
    assert!(!native.is_empty(), "BM25F answers the query");

    let embedder = std::sync::Arc::new(KeywordEmbedder::new(false));
    let fused =
        tool_search_names_through_the_runtime_build_path(Some(hybrid_over(&embedder))).await;
    assert_eq!(
        fused.first().map(String::as_str),
        Some("github__create_issue"),
        "fusion ranks the tool both rankers agree on first: {fused:?} (native {native:?})"
    );
    assert!(fused.len() <= 3, "the limit holds: {fused:?}");
    let embedded = embedder.embedded.lock().expect("embedder lock").clone();
    assert!(
        embedded.iter().any(|text| text.starts_with("tool: ")),
        "the hybrid fit embeds the authorized catalog"
    );
    assert!(
        embedded.iter().any(|text| text == "create issue"),
        "the hybrid search consults the dense side with the model's query"
    );

    let failing = std::sync::Arc::new(KeywordEmbedder::new(true));
    let degraded =
        tool_search_names_through_the_runtime_build_path(Some(hybrid_over(&failing))).await;
    assert!(
        failing
            .embedded
            .lock()
            .expect("embedder lock")
            .iter()
            .any(|text| text == "create issue"),
        "the failing dense side was asked"
    );
    assert_eq!(
        degraded, native,
        "a dense failure degrades to BM25F exactly"
    );
}

// ---------------------------------------------------------------------
// Turn-start tool selection (`REBORN_TOOL_PREFETCH`)
// ---------------------------------------------------------------------

/// Lexical selection with the production defaults (no score thresholds,
/// a 32,000-token budget, 16 messages of 2 KiB segments) and the production
/// re-selection defaults: re-select once the prompt cache has been idle for
/// an hour plus a minute.
fn lexical_prefetch(always: &[&str]) -> ironclaw_loop_host::ToolPrefetchConfig {
    lexical_prefetch_with_cache_lifetime(always, std::time::Duration::from_secs(3_600))
}

/// Lexical selection whose cache lifetime (for a provider the host cannot
/// know, as the scripted one) is `lifetime`, with no margin.
fn lexical_prefetch_with_cache_lifetime(
    always: &[&str],
    lifetime: std::time::Duration,
) -> ironclaw_loop_host::ToolPrefetchConfig {
    let margin = if lifetime >= std::time::Duration::from_secs(3_600) {
        std::time::Duration::from_secs(60)
    } else {
        std::time::Duration::ZERO
    };
    ironclaw_loop_host::ToolPrefetchConfig::new(
        ironclaw_loop_host::ToolPrefetchRanking::Lexical,
        100,
        32_000,
        0.0,
        0.0,
        always.iter().map(|name| name.to_string()).collect(),
    )
    .expect("valid prefetch config")
    .with_reselection(ironclaw_loop_host::ToolReselectionConfig::new(
        true, lifetime, margin,
    ))
}

/// [`lexical_prefetch`] with an explicit relative threshold of 0.3 (the
/// former lexical default): a tool is kept only when it scores at least 0.3
/// of its segment's top tool. Thresholds are off by default and still cut
/// when set; these scenarios need a narrow array to show that a deferred
/// tool stays deferred.
fn thresholded_lexical_prefetch() -> ironclaw_loop_host::ToolPrefetchConfig {
    ironclaw_loop_host::ToolPrefetchConfig::new(
        ironclaw_loop_host::ToolPrefetchRanking::Lexical,
        100,
        32_000,
        0.0,
        0.3,
        Vec::new(),
    )
    .expect("valid prefetch config")
    .with_reselection(ironclaw_loop_host::ToolReselectionConfig::new(
        true,
        std::time::Duration::from_secs(3_600),
        std::time::Duration::from_secs(60),
    ))
}

/// A GitHub opening request, with turn-start selection on, advertises the
/// GitHub tools it predicts plus the always-on floor — and not the whole
/// catalog — so the model calls the repository tool natively on its first
/// reply, with no discovery round trip. The selection is recorded as the
/// conversation's `initial` history entry.
#[tokio::test]
async fn turn_start_selection_serves_a_github_opening_request_without_discovery() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(thresholded_lexical_prefetch())
        .script([
            RebornScriptedReply::tool_call(
                FLAT_GITHUB_TOOL_NAME,
                serde_json::json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("prefetch harness builds");

    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw and summarise it")
        .await
        .expect("turn completes");

    let opening = harness.model_tool_names(0).expect("first model request");
    for expected in [
        FLAT_GITHUB_TOOL_NAME,
        TOOL_SEARCH_NAME,
        TOOL_DESCRIBE_NAME,
        TOOL_CALL_NAME,
        "builtin__result_read",
    ] {
        assert!(
            opening.iter().any(|name| name == expected),
            "the opening request must advertise {expected:?}: {opening:?}"
        );
    }
    let github_advertised = opening
        .iter()
        .filter(|name| name.starts_with("github__"))
        .count();
    assert!(
        github_advertised
            < reborn_support::github::capability_ids()
                .expect("github capability ids")
                .len(),
        "only the predicted GitHub tools are advertised, not all of them: {opening:?}"
    );
    assert!(
        !opening
            .iter()
            .any(|name| name == "github__merge_pull_request"),
        "an unrelated GitHub tool stays deferred: {opening:?}"
    );

    // Served natively: the first reply's direct call executed, and the turn
    // needed exactly two model calls (the call and the answer).
    harness
        .assert_tool_invoked("github.get_repo")
        .await
        .expect("the advertised tool is called directly");
    assert_eq!(
        harness.scripted_llm.captured_tool_definitions().len(),
        2,
        "no discovery round trip"
    );
    harness
        .assert_model_tool_definitions_identical()
        .await
        .expect("the tools array does not change within the turn");

    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("the opening selection is recorded");
    assert_eq!(history.entries.len(), 1);
    let entry = &history.entries[0];
    assert_eq!(entry.reason, ironclaw_threads::ToolSelectionReason::Initial);
    assert_eq!(
        entry.advertised, opening,
        "the record is the exact tools array"
    );
    assert!(
        entry
            .scores
            .iter()
            .any(|score| score.name == FLAT_GITHUB_TOOL_NAME && score.score > 0.0),
        "the selected tool's score is recorded: {:?}",
        entry.scores
    );
}

/// Semantic turn-start selection ranks the opening request with the ranker
/// bound behind `tool_search` (here hybrid: BM25F fused with a dense ranker),
/// not with BM25F alone: the dense side embeds the opening request, the
/// predicted tool is advertised and called directly, and the recorded entry
/// names the hybrid ranker whose scale its scores are on.
#[tokio::test]
async fn semantic_turn_start_selection_ranks_with_the_bound_hybrid_ranker() {
    const OPENING: &str = "Please create an issue about the flaky test";
    let embedder = std::sync::Arc::new(KeywordEmbedder::new(false));
    let semantic = ironclaw_loop_host::ToolPrefetchConfig::new(
        ironclaw_loop_host::ToolPrefetchRanking::Semantic,
        100,
        32_000,
        0.0,
        0.0,
        Vec::new(),
    )
    .expect("valid prefetch config");
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_retrieval_provider(hybrid_over(&embedder))
        .with_tool_prefetch(semantic)
        .script([
            RebornScriptedReply::tool_call(
                "github__create_issue",
                serde_json::json!({"owner": "nearai", "repo": "ironclaw", "title": "Flaky test"}),
            ),
            RebornScriptedReply::text("filed"),
        ])
        .build()
        .await
        .expect("semantic prefetch harness builds");
    harness.submit_turn(OPENING).await.expect("turn completes");

    assert!(
        embedder
            .embedded
            .lock()
            .expect("embedder lock")
            .iter()
            .any(|text| text == OPENING),
        "the dense side ranks the opening request"
    );
    let opening = harness.model_tool_names(0).expect("first model request");
    assert!(
        opening.iter().any(|name| name == "github__create_issue"),
        "{opening:?}"
    );
    harness
        .assert_tool_invoked("github.create_issue")
        .await
        .expect("the predicted tool is called directly");
    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("history recorded");
    assert!(
        history.entries[0]
            .ranker_version
            .as_deref()
            .is_some_and(|version| version.starts_with("hybrid-rrf-v1")),
        "{:?}",
        history.entries[0].ranker_version
    );
}

/// A hybrid ranker whose dense side persists its vectors in `root`'s
/// per-user `/tool-vectors` store, the store the runtime build binds.
fn persisting_hybrid_over(
    embedder: &std::sync::Arc<KeywordEmbedder>,
    root: &std::sync::Arc<ironclaw_filesystem::InMemoryBackend>,
) -> std::sync::Arc<dyn ironclaw_loop_contracts::ToolRetrievalProvider> {
    let store = ironclaw_tool_retrieval::ToolVectorStoreSlot::new();
    store.bind(ironclaw_tool_retrieval::FilesystemToolVectorStore::new(
        ironclaw_composition::wrap_scoped(std::sync::Arc::clone(root)),
        ironclaw_tool_retrieval::DEFAULT_STORED_VECTORS_PER_OWNER,
    ));
    let dense = std::sync::Arc::new(
        ironclaw_tool_retrieval::DenseToolRetrievalProvider::with_vector_store(
            std::sync::Arc::clone(embedder)
                as std::sync::Arc<dyn ironclaw_llm::embeddings::EmbeddingProvider>,
            store,
        ),
    );
    std::sync::Arc::new(ironclaw_loop_host::HybridToolRetrieval::new(Some(dense)))
}

/// Tool vectors survive a restart: the first process embeds the catalog at
/// its first selection and persists the vectors per user; a second process
/// (a fresh ranker with an empty memory cache, over the same store) selects
/// for a new conversation embedding only the opening request.
#[tokio::test]
async fn semantic_selection_after_a_restart_embeds_only_the_opening_request() {
    const OPENING: &str = "Please create an issue about the flaky test";
    let root = std::sync::Arc::new(ironclaw_filesystem::InMemoryBackend::new());
    let semantic = || {
        ironclaw_loop_host::ToolPrefetchConfig::new(
            ironclaw_loop_host::ToolPrefetchRanking::Semantic,
            100,
            16_000,
            0.35,
            0.0,
            Vec::new(),
        )
        .expect("valid prefetch config")
    };
    let run_process = |embedder: std::sync::Arc<KeywordEmbedder>| {
        let provider = persisting_hybrid_over(&embedder, &root);
        let semantic = semantic();
        async move {
            let harness = RebornIntegrationHarness::test_default()
                .with_tool_disclosure_production_default()
                .with_github_issue_tools()
                .with_tool_retrieval_provider(provider)
                .with_tool_prefetch(semantic)
                .script([RebornScriptedReply::text("noted")])
                .build()
                .await
                .expect("semantic prefetch harness builds");
            harness.submit_turn(OPENING).await.expect("turn completes");
            harness
                .assert_reply_contains("noted")
                .await
                .expect("turn completes");
            embedder.embedded.lock().expect("embedder lock").clone()
        }
    };

    let first = run_process(std::sync::Arc::new(KeywordEmbedder::new(false))).await;
    assert!(
        first
            .iter()
            .filter(|text| text.starts_with("tool: "))
            .count()
            > 32,
        "the first process embeds the catalog"
    );

    let restarted = run_process(std::sync::Arc::new(KeywordEmbedder::new(false))).await;
    let documents: Vec<&String> = restarted
        .iter()
        .filter(|text| text.starts_with("tool: "))
        .collect();
    assert!(
        documents.is_empty(),
        "after a restart no tool document is embedded again: {} were",
        documents.len()
    );
    assert!(
        restarted.iter().any(|text| text == OPENING),
        "the opening request is still embedded, to rank it"
    );
}

/// The #6987 shape: with turn-start selection on (re-selection included, at
/// its production defaults; an explicit threshold keeps the opening array
/// narrow), every model call made while the prompt cache
/// could be warm carries the byte-identical `tools` array — even after the
/// model finds a deferred tool with `tool_search` and calls it with
/// `tool_call` (nothing is promoted), and after a compaction (which rewrites
/// only the messages behind the tools) — so the cached prompt prefix
/// survives, and the history keeps its single `initial` entry.
#[tokio::test]
async fn turn_start_selection_keeps_the_tools_array_identical_while_the_cache_is_warm() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(thresholded_lexical_prefetch())
        .script([
            RebornScriptedReply::tool_call(
                FLAT_GITHUB_TOOL_NAME,
                serde_json::json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            RebornScriptedReply::text("here is the repository"),
        ])
        .build()
        .await
        .expect("prefetch harness builds");
    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw and summarise it")
        .await
        .expect("opening turn completes");
    let opening = harness.model_tool_names(0).expect("first model request");
    assert!(
        !opening.iter().any(|name| name == "github__create_issue"),
        "the follow-up tool starts deferred: {opening:?}"
    );

    harness.push_script([
        RebornScriptedReply::tool_call(
            TOOL_SEARCH_NAME,
            serde_json::json!({"query": "create issue", "limit": 5}),
        ),
        RebornScriptedReply::tool_call(
            TOOL_CALL_NAME,
            serde_json::json!({
                "name": "github__create_issue",
                "arguments": r#"{"owner":"nearai","repo":"ironclaw","title":"Flaky test"}"#
            }),
        ),
        RebornScriptedReply::text("filed it"),
    ]);
    harness
        .submit_turn("Now open an issue about the flaky test")
        .await
        .expect("follow-up turn completes");
    harness
        .assert_tool_invoked("github.create_issue")
        .await
        .expect("the deferred tool is reached through tool_search and tool_call");

    harness.push_script([RebornScriptedReply::text("you're welcome")]);
    harness
        .submit_turn("thanks")
        .await
        .expect("closing turn completes");

    assert_eq!(harness.scripted_llm.captured_tool_definitions().len(), 6);
    harness
        .assert_model_tool_definitions_identical()
        .await
        .expect("the tools array never changes while the cache could be warm");
    harness
        .assert_prompt_cache_prefix_stable()
        .await
        .expect("the cached prefix is not rewritten either");

    // A compaction rewrites the messages, which follow the tools in the
    // cached prefix; it never re-selects the tools.
    harness
        .create_compaction_summary_for_test(1, 2, "the user asked about the repository", None)
        .await
        .expect("compaction summary");
    harness.push_script([RebornScriptedReply::text("anything else?")]);
    harness
        .submit_turn("one more thing about the repository")
        .await
        .expect("turn after compaction completes");
    assert_eq!(harness.scripted_llm.captured_tool_definitions().len(), 7);
    harness
        .assert_model_tool_definitions_identical()
        .await
        .expect("compaction does not change the tools array");
    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("history recorded");
    assert_eq!(
        history.entries.len(),
        1,
        "nothing is added after the opening selection"
    );
}

/// Scripted availability for turn-start selection: one capability's account
/// is missing until `connect` is called; every other tool is available.
#[derive(Debug)]
struct AccountMissingFor {
    capability: &'static str,
    connected: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl ironclaw_loop_contracts::ToolAvailabilityPredicate for AccountMissingFor {
    async fn availability(
        &self,
        _run_context: &ironclaw_loop_contracts::LoopRunContext,
        capability_ids: &[ironclaw_host_api::ids::CapabilityId],
    ) -> std::collections::BTreeMap<
        ironclaw_host_api::ids::CapabilityId,
        ironclaw_loop_contracts::ToolAvailability,
    > {
        let connected = self.connected.load(std::sync::atomic::Ordering::SeqCst);
        capability_ids
            .iter()
            .map(|id| {
                let answer = if id.as_str() == self.capability && !connected {
                    ironclaw_loop_contracts::ToolAvailability::Unavailable(
                        ironclaw_loop_contracts::ToolUnavailableReason::CredentialMissing,
                    )
                } else {
                    ironclaw_loop_contracts::ToolAvailability::Available
                };
                (id.clone(), answer)
            })
            .collect()
    }
}

/// #7836's rule on turn-start selection: a tool whose account is not
/// connected is not advertised even when the opening request predicts it,
/// yet stays callable through `tool_call` (where its setup prompt would
/// appear). Connecting the account later does not add it to the frozen list.
#[tokio::test]
async fn turn_start_selection_leaves_out_a_tool_whose_account_is_missing() {
    let availability = std::sync::Arc::new(AccountMissingFor {
        capability: "github.get_repo",
        connected: std::sync::atomic::AtomicBool::new(false),
    });
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(lexical_prefetch(&[]))
        .with_tool_availability(std::sync::Arc::clone(&availability)
            as std::sync::Arc<dyn ironclaw_loop_contracts::ToolAvailabilityPredicate>)
        .script([
            RebornScriptedReply::tool_call(
                TOOL_CALL_NAME,
                serde_json::json!({
                    "name": FLAT_GITHUB_TOOL_NAME,
                    "arguments": r#"{"owner":"nearai","repo":"ironclaw"}"#
                }),
            ),
            RebornScriptedReply::text("here is the repository"),
        ])
        .build()
        .await
        .expect("prefetch harness builds");
    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw and summarise it")
        .await
        .expect("opening turn completes");

    let opening = harness.model_tool_names(0).expect("first model request");
    assert!(
        !opening.iter().any(|name| name == FLAT_GITHUB_TOOL_NAME),
        "a tool whose account is missing is not advertised: {opening:?}"
    );
    assert!(
        opening.iter().any(|name| name.starts_with("github__")),
        "the request's other GitHub tools are still selected: {opening:?}"
    );
    harness
        .assert_tool_invoked("github.get_repo")
        .await
        .expect("the left-out tool is still callable through tool_call");

    availability
        .connected
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness.push_script([RebornScriptedReply::text("you're welcome")]);
    harness
        .submit_turn("thanks")
        .await
        .expect("closing turn completes");
    harness
        .assert_model_tool_definitions_identical()
        .await
        .expect("a tool that becomes available is not added to the frozen list");
    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("history recorded");
    assert_eq!(history.entries.len(), 1);
    assert!(
        !history.entries[0]
            .advertised
            .iter()
            .any(|name| name == FLAT_GITHUB_TOOL_NAME)
    );
}

/// Revocation through the whole stack: a selected tool whose account is
/// disconnected between turns (a definite "unavailable" answer) leaves the
/// frozen `tools` array on the next turn, which appends a `revoked` entry to
/// the conversation's selection history. Nothing else changes: the rest of
/// the array stays in order, and no tool is added.
#[tokio::test]
async fn a_selected_tool_whose_account_is_disconnected_is_revoked_on_the_next_turn() {
    let availability = std::sync::Arc::new(AccountMissingFor {
        capability: "github.get_repo",
        connected: std::sync::atomic::AtomicBool::new(true),
    });
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(lexical_prefetch(&[]))
        .with_tool_availability(std::sync::Arc::clone(&availability)
            as std::sync::Arc<dyn ironclaw_loop_contracts::ToolAvailabilityPredicate>)
        .script([
            RebornScriptedReply::tool_call(
                FLAT_GITHUB_TOOL_NAME,
                serde_json::json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            RebornScriptedReply::text("here is the repository"),
        ])
        .build()
        .await
        .expect("prefetch harness builds");
    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw and summarise it")
        .await
        .expect("opening turn completes");
    let opening = harness.model_tool_names(0).expect("first model request");
    assert!(
        opening.iter().any(|name| name == FLAT_GITHUB_TOOL_NAME),
        "the connected tool is selected: {opening:?}"
    );

    availability
        .connected
        .store(false, std::sync::atomic::Ordering::SeqCst);
    harness.push_script([RebornScriptedReply::text("you're welcome")]);
    harness
        .submit_turn("thanks")
        .await
        .expect("closing turn completes");

    let closing = harness
        .model_tool_names(2)
        .expect("the closing turn's request");
    let expected: Vec<String> = opening
        .iter()
        .filter(|name| *name != FLAT_GITHUB_TOOL_NAME)
        .cloned()
        .collect();
    assert_eq!(
        closing, expected,
        "the revoked tool leaves the array; nothing else moves or is added"
    );
    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("history recorded");
    assert_eq!(history.entries.len(), 2);
    assert_eq!(
        history.entries[1].reason,
        ironclaw_threads::ToolSelectionReason::Revoked
    );
    assert_eq!(history.entries[1].advertised, closing);
}

/// Prompt text follows the advertised surface: the delivery guidance names
/// both delivery tools, so with turn-start selection on it renders only when
/// the selection advertises both — here, only once an operator lists them as
/// always-on extras. With the ordinary surface (selection off) the three-tool
/// catalog is advertised whole and the guidance renders, which pins that the
/// absence below is the selection's doing.
#[tokio::test]
async fn delivery_guidance_renders_only_when_the_selection_advertises_both_delivery_tools() {
    const DELIVERY_GUIDANCE: &str = "To put content on ANOTHER surface";

    async fn opening_prompt(
        prefetch: Option<ironclaw_loop_host::ToolPrefetchConfig>,
        conversation: &str,
    ) -> RebornIntegrationHarness {
        let provider =
            reborn_support::comm_context::RecordingCommunicationContextProvider::with_notification_count_and_channel(
                1,
                "reborn-prefetch-channel",
            );
        let mut group = reborn_support::group::RebornIntegrationGroup::builder()
            .with_tool_disclosure_mode(ToolDisclosureMode::Namespaces)
            .communication_context_provider(provider);
        if let Some(prefetch) = prefetch {
            group = group.tool_prefetch(prefetch);
        }
        let group = group
            .outbound_target_tools()
            .await
            .expect("outbound group builds");
        let harness = group
            .thread(conversation)
            .script([RebornScriptedReply::text("ok")])
            .build()
            .await
            .expect("thread builds");
        harness
            .submit_turn("What is the capital of France?")
            .await
            .expect("turn completes");
        harness
    }

    let ordinary = opening_prompt(None, "conv-prefetch-delivery-off").await;
    ordinary
        .assert_model_request_contains(DELIVERY_GUIDANCE)
        .await
        .expect("the ordinary surface advertises both delivery tools");

    let selected = opening_prompt(Some(lexical_prefetch(&[])), "conv-prefetch-delivery-on").await;
    let tools = selected.model_tool_names(0).expect("first model request");
    assert!(
        !tools.iter().any(|name| name == "builtin__outbound_deliver"),
        "an unrelated question does not select the delivery tool: {tools:?}"
    );
    selected
        .assert_model_request_excludes(DELIVERY_GUIDANCE)
        .await
        .expect("guidance naming an unadvertised tool is not rendered");

    let pinned = opening_prompt(
        Some(lexical_prefetch(&[
            "outbound_deliver",
            "outbound_delivery_targets_list",
        ])),
        "conv-prefetch-delivery-pinned",
    )
    .await;
    pinned
        .assert_model_request_contains(DELIVERY_GUIDANCE)
        .await
        .expect("listing both delivery tools as extras brings the guidance back");
}

/// Prompt text follows the advertised surface for the skill listing too: it
/// tells the model to activate skills with `builtin.skill_activate`, so with
/// turn-start selection on it renders only when the selection advertises
/// that tool. The runtime-context time line likewise drops its `profile_set`
/// and `time` mentions when the selection advertises neither. With the
/// ordinary surface (selection off) both render as they always have, which
/// pins that the absences below are the selection's doing. Every turn also
/// completes, which proves the model port re-resolved the gated skill
/// snippets to the same refs the prompt bundle used.
#[tokio::test]
async fn skill_listing_and_runtime_tool_mentions_follow_the_selection() {
    const LISTING: &str = "- greet: greets the user warmly";
    const SKILL_ACTIVATE_HEADER: &str = "builtin.skill_activate";

    async fn opening_prompt(
        prefetch: Option<ironclaw_loop_host::ToolPrefetchConfig>,
        conversation: &str,
    ) -> RebornIntegrationHarness {
        let mut group = reborn_support::group::RebornIntegrationGroup::builder()
            .with_tool_disclosure_mode(ToolDisclosureMode::Namespaces);
        if let Some(prefetch) = prefetch {
            group = group.tool_prefetch(prefetch);
        }
        let group = group
            .skill_activation_tools()
            .await
            .expect("skill group builds");
        let harness = group
            .thread(conversation)
            .script([RebornScriptedReply::text("ok")])
            .build()
            .await
            .expect("thread builds");
        harness
            .submit_turn("What is the capital of France?")
            .await
            .expect("turn completes");
        harness
    }

    let ordinary = opening_prompt(None, "conv-prefetch-skills-off").await;
    for rendered in [LISTING, SKILL_ACTIVATE_HEADER, "profile_set capability"] {
        ordinary
            .assert_model_request_contains(rendered)
            .await
            .expect("the ordinary surface renders the tool-naming text unchanged");
    }

    let selected = opening_prompt(Some(lexical_prefetch(&[])), "conv-prefetch-skills-on").await;
    let tools = selected.model_tool_names(0).expect("first model request");
    assert!(
        !tools.iter().any(|name| name == "builtin__skill_activate"),
        "an unrelated question does not select skill_activate: {tools:?}"
    );
    for withheld in [
        LISTING,
        SKILL_ACTIVATE_HEADER,
        "profile_set capability",
        "time capability",
    ] {
        selected
            .assert_model_request_excludes(withheld)
            .await
            .expect("text naming an unadvertised tool is not rendered");
    }

    let pinned = opening_prompt(
        Some(lexical_prefetch(&["skill_activate"])),
        "conv-prefetch-skills-pinned",
    )
    .await;
    pinned
        .assert_model_request_contains(LISTING)
        .await
        .expect("listing skill_activate as an extra brings the skill listing back");
}

// ---------------------------------------------------------------------
// Turn-start selection through the Jev classifier (`classifier = "jev"`)
// ---------------------------------------------------------------------

/// The path the stub decisions endpoint serves: no provider's, so a request
/// that reaches it went to the configured URL as given.
const JEV_STUB_PATH: &str = "/stub/jev/decisions";

/// A loopback stand-in for a Jev decisions endpoint, configured as the
/// classifier's endpoint: one request per connection, answered by `reply`
/// with the question ids asked, or `404` for a request to any path but
/// [`JEV_STUB_PATH`]. Counts the requests it served. Nothing here reaches a
/// real provider.
async fn jev_stub(
    reply: impl Fn(&[String]) -> (u16, String) + Send + Sync + 'static,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub");
    let port = listener.local_addr().expect("stub address").port();
    let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reply = std::sync::Arc::new(reply);
    let counter = std::sync::Arc::clone(&served);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let reply = std::sync::Arc::clone(&reply);
            let counter = std::sync::Arc::clone(&counter);
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 8192];
                let (header_end, length) = loop {
                    let Ok(read) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while buffer.len() < header_end + length {
                    match stream.read(&mut chunk).await {
                        Ok(read) if read > 0 => buffer.extend_from_slice(&chunk[..read]),
                        _ => return,
                    }
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&buffer[header_end..header_end + length])
                        .unwrap_or_default();
                let asked: Vec<String> = body["questions"]
                    .as_object()
                    .map(|questions| questions.keys().cloned().collect())
                    .unwrap_or_default();
                let request_line = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (status, text) = if path == JEV_STUB_PATH {
                    reply(&asked)
                } else {
                    (404, "{}".to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://127.0.0.1:{port}{JEV_STUB_PATH}"), served)
}

fn jev_prefetch(url: &str, max_tools: usize) -> ironclaw_loop_host::ToolPrefetchConfig {
    let classifier = ironclaw_tool_selection_jev::with_stub_endpoint(
        ironclaw_tool_selection_jev::JevToolClassifier::new(
            ironclaw_tool_selection_jev::JevEndpoint::default(),
            ironclaw_tool_selection_jev::DEFAULT_JEV_MODEL,
            ironclaw_tool_selection_jev::JevApiKey::new("jev-integration-key").expect("key"),
            std::time::Duration::from_millis(2_000),
        )
        .expect("classifier"),
        url,
        ironclaw_host_api::action::NetworkPolicy {
            allowed_targets: vec![ironclaw_host_api::action::NetworkTargetPattern {
                scheme: Some(ironclaw_host_api::action::NetworkScheme::Http),
                host_pattern: "127.0.0.1".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: false,
            max_egress_bytes: None,
        },
    );
    ironclaw_loop_host::ToolPrefetchConfig::new(
        ironclaw_loop_host::ToolPrefetchRanking::Lexical,
        max_tools,
        32_000,
        0.0,
        0.0,
        Vec::new(),
    )
    .expect("valid prefetch config")
    .with_classifier(std::sync::Arc::new(classifier))
}

/// With the Jev classifier bound, the opening request's `tools` array is the
/// floor plus Jev's most probable tools (here room for three), the model
/// calls the predicted tool directly, and the recorded entry carries the
/// probabilities on the Jev scale.
#[tokio::test]
async fn jev_turn_start_selection_advertises_the_most_probable_tools() {
    let (url, served) = jev_stub(|asked| {
        let answers: serde_json::Map<String, serde_json::Value> = asked
            .iter()
            .map(|id| {
                let probability = match id.as_str() {
                    FLAT_GITHUB_TOOL_NAME => 0.97,
                    "github__list_issues" => 0.6,
                    "github__create_issue" => 0.5,
                    _ => 0.01,
                };
                (
                    id.clone(),
                    serde_json::json!({"type": "noul", "noul": probability}),
                )
            })
            .collect();
        (
            200,
            serde_json::json!({"answers": answers, "usage": {"input_tokens": 1200}}).to_string(),
        )
    })
    .await;
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(jev_prefetch(&url, 7))
        .script([
            RebornScriptedReply::tool_call(
                FLAT_GITHUB_TOOL_NAME,
                serde_json::json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("jev harness builds");
    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw and summarise it")
        .await
        .expect("turn completes");

    let opening = harness.model_tool_names(0).expect("first model request");
    let github: Vec<&String> = opening
        .iter()
        .filter(|name| name.starts_with("github__"))
        .collect();
    assert_eq!(
        github,
        vec![
            "github__create_issue",
            "github__get_repo",
            "github__list_issues"
        ],
        "{opening:?}"
    );
    for floor in [
        TOOL_SEARCH_NAME,
        TOOL_DESCRIBE_NAME,
        TOOL_CALL_NAME,
        "builtin__result_read",
    ] {
        assert!(opening.iter().any(|name| name == floor), "{opening:?}");
    }
    harness
        .assert_tool_invoked("github.get_repo")
        .await
        .expect("the predicted tool is called directly");
    let entry = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("selection recorded")
        .entries
        .remove(0);
    assert_eq!(entry.advertised, opening);
    assert_eq!(entry.ranker_version.as_deref(), Some("jev:jev-latest"));
    assert_eq!(entry.fallback_reason, None);
    assert_eq!(
        entry
            .scores
            .first()
            .map(|score| (score.name.as_str(), score.score)),
        Some((FLAT_GITHUB_TOOL_NAME, 0.97))
    );
    assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// When Jev fails at a conversation's first turn, the conversation freezes
/// the core tool set it would have without selection (no GitHub tool is
/// core), records that with the failure's label, and a later turn reuses it
/// without calling Jev again. A refused key (`401`) and an account that
/// cannot pay (`402`) both fall back at once, with no retry.
#[tokio::test]
async fn a_jev_failure_freezes_the_core_tool_set_for_the_conversation() {
    for (status, label) in [(401, "unauthorized"), (402, "payment_required")] {
        a_jev_failure_freezes_the_core_tool_set(status, label).await;
    }
}

async fn a_jev_failure_freezes_the_core_tool_set(status: u16, label: &str) {
    let (url, served) = jev_stub(move |_| (status, r#"{"error":"refused"}"#.to_string())).await;
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(jev_prefetch(&url, 100))
        .script([RebornScriptedReply::text("hello")])
        .build()
        .await
        .expect("jev harness builds");
    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw")
        .await
        .expect("turn completes despite the classifier failure");
    let opening = harness.model_tool_names(0).expect("first model request");
    assert!(
        !opening.iter().any(|name| name.starts_with("github__")),
        "{opening:?}"
    );
    for floor in [
        TOOL_SEARCH_NAME,
        TOOL_DESCRIBE_NAME,
        TOOL_CALL_NAME,
        "builtin__result_read",
    ] {
        assert!(opening.iter().any(|name| name == floor), "{opening:?}");
    }
    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("the fallback is recorded");
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].advertised, opening);
    assert_eq!(history.entries[0].fallback_reason.as_deref(), Some(label));

    harness.push_script([RebornScriptedReply::text("again")]);
    harness
        .submit_turn("and the issues?")
        .await
        .expect("second turn completes");
    harness
        .assert_model_tool_definitions_identical()
        .await
        .expect("the fallback stays frozen");
    assert_eq!(
        served.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{status}: not retried, and Jev is not asked again"
    );
}

/// Cold-cache re-selection: after an idle gap past the prompt-cache lifetime
/// (shortened to one second for the test), the next turn's tools come from
/// the conversation so far. The conversation drifts from a GitHub repository
/// to creating a project (the `system` namespace, deferred at the opening),
/// and the second turn calls the project tool directly, with no discovery
/// round trip. The tool the first turn called stays advertised.
///
/// The second message runs far past the 32 terms a ranker reads of one
/// query, and each message is ranked on its own and the rankings merged, so
/// the second turn's tools come from both topics: a GitHub tool that shares
/// no word with the second message, and that the first turn never called,
/// is advertised too. One query joined newest first would have read only
/// the project message.
#[tokio::test]
async fn after_an_idle_gap_the_next_turn_is_served_tools_for_the_new_topic() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_github_issue_tools()
        .with_tool_prefetch(lexical_prefetch_with_cache_lifetime(
            &[],
            std::time::Duration::from_secs(1),
        ))
        .script([
            RebornScriptedReply::tool_call(
                FLAT_GITHUB_TOOL_NAME,
                serde_json::json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            RebornScriptedReply::text("here is the repository"),
        ])
        .build()
        .await
        .expect("prefetch harness builds");
    harness
        .submit_turn("Get the GitHub repository nearai/ironclaw and summarise it")
        .await
        .expect("opening turn completes");
    let opening = harness.model_tool_names(0).expect("first model request");
    assert!(
        !opening.iter().any(|name| name == "builtin__project_create"),
        "the project tool starts deferred: {opening:?}"
    );
    assert_eq!(
        harness.model_tool_names(1).expect("second model request"),
        opening,
        "the array is identical within the opening turn"
    );

    // Idle past the one-second lifetime.
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    harness.push_script([
        RebornScriptedReply::tool_call(
            "builtin__project_create",
            serde_json::json!({"name": "launch plan"}),
        ),
        RebornScriptedReply::text("created the project"),
    ]);
    let filler: Vec<String> = (0..48).map(|index| format!("lorem{index}")).collect();
    let drift = format!(
        "Create a new project called launch plan. {}",
        filler.join(" ")
    );
    harness
        .submit_turn(&drift)
        .await
        .expect("drifted turn completes");

    let drifted = harness.model_tool_names(2).expect("third model request");
    assert!(
        drifted.iter().any(|name| name == "builtin__project_create"),
        "the drifted turn advertises the project tool: {drifted:?}"
    );
    assert!(
        drifted.iter().any(|name| name == FLAT_GITHUB_TOOL_NAME),
        "the tool the conversation called stays advertised: {drifted:?}"
    );
    assert!(
        drifted.iter().any(|name| name == "github__list_branches"),
        "the opening message's topic is still ranked: {drifted:?}"
    );
    assert_eq!(
        harness.model_tool_names(3).expect("fourth model request"),
        drifted,
        "the new array is identical within the drifted turn"
    );
    harness
        .assert_tool_invoked("builtin.project_create")
        .await
        .expect("the project tool is called directly");
    assert_eq!(
        harness.scripted_llm.captured_tool_definitions().len(),
        4,
        "no discovery round trip in either turn"
    );

    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("history recorded");
    assert_eq!(
        history
            .entries
            .iter()
            .map(|entry| entry.reason)
            .collect::<Vec<_>>(),
        [
            ironclaw_threads::ToolSelectionReason::Initial,
            ironclaw_threads::ToolSelectionReason::CacheCold
        ]
    );
    assert_eq!(history.entries[1].advertised, drifted);
}

// ---------------------------------------------------------------------
// A deployment shaped like the tool-discovery benchmark: several MCP
// packages, a large result paged with `result_read`, and more than one turn.
// ---------------------------------------------------------------------

/// Six mock MCP packages of six tools each (36 tools, over the disclosure
/// cap of 32, so the ordinary modes defer too). Every MCP tool name is
/// unique across packages because one mock server answers for all of them.
fn benchmark_shaped_mcp_packages() -> Vec<reborn_support::harness_mcp::MockMcpPackage> {
    const PACKAGES: [(&str, [(&str, &str); 6]); 6] = [
        (
            "tracker",
            [
                (
                    "search_issues",
                    "Search tracker issues by text, state and label",
                ),
                (
                    "create_issue",
                    "Open a new tracker issue with a title and body",
                ),
                ("close_issue", "Close a tracker issue by its number"),
                ("comment_issue", "Add a comment to a tracker issue"),
                ("assign_issue", "Assign a tracker issue to a teammate"),
                ("label_issue", "Add labels to a tracker issue"),
            ],
        ),
        (
            "calendar",
            [
                (
                    "create_event",
                    "Create a calendar event with a start and end time",
                ),
                ("list_events", "List calendar events in a time range"),
                ("delete_event", "Delete a calendar event by its id"),
                ("move_event", "Move a calendar event to a new time"),
                ("invite_attendee", "Invite an attendee to a calendar event"),
                ("free_busy", "Report free and busy time for a calendar"),
            ],
        ),
        (
            "crm",
            [
                ("list_customers", "List customer accounts in the CRM"),
                ("get_customer", "Read one CRM customer account"),
                ("create_customer", "Create a CRM customer account"),
                ("update_customer", "Update fields of a CRM customer account"),
                ("list_deals", "List open sales deals in the CRM"),
                ("close_deal", "Mark a CRM sales deal as won or lost"),
            ],
        ),
        (
            "docs",
            [
                ("search_documents", "Search shared documents by text"),
                ("get_document", "Fetch one shared document by its id"),
                ("create_document", "Create a shared document"),
                ("append_document", "Append text to a shared document"),
                ("share_document", "Share a document with a teammate"),
                ("archive_document", "Archive a shared document"),
            ],
        ),
        (
            "billing",
            [
                ("list_invoices", "List invoices for a billing account"),
                ("get_invoice", "Fetch one invoice by its number"),
                ("refund_payment", "Refund a captured payment"),
                ("list_payments", "List payments for a billing account"),
                ("create_invoice", "Draft a new invoice"),
                ("void_invoice", "Void an unpaid invoice"),
            ],
        ),
        (
            "chat",
            [
                ("post_message", "Post a message to a chat channel"),
                ("list_channels", "List chat channels"),
                ("read_channel", "Read recent messages from a chat channel"),
                ("react_message", "Add a reaction to a chat message"),
                ("pin_message", "Pin a chat message in its channel"),
                ("invite_member", "Invite a member to a chat channel"),
            ],
        ),
    ];
    PACKAGES
        .iter()
        .map(
            |(provider, tools)| reborn_support::harness_mcp::MockMcpPackage {
                provider_id: provider.to_string(),
                tools: tools
                    .iter()
                    .map(
                        |(tool, description)| reborn_support::harness_mcp::MockMcpTool {
                            capability_id: format!("{provider}.{tool}"),
                            description: description.to_string(),
                            parameters_schema: serde_json::json!({
                                "type": "object",
                                "properties": {
                                    "query": {"type": "string", "description": "What to look for"},
                                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                                },
                                "required": ["query"],
                                "additionalProperties": false
                            }),
                        },
                    )
                    .collect(),
            },
        )
        .collect()
}

/// A mock MCP server answering every benchmark-shaped tool, with a result
/// for `search_issues` large enough to be stored durably and paged with
/// `result_read`.
async fn benchmark_shaped_mcp_server() -> support::mock_mcp_server::MockMcpServer {
    let issues: Vec<serde_json::Value> = (0..400)
        .map(|number| {
            serde_json::json!({
                "number": number,
                "title": format!("Login outage report {number}"),
                "state": "open",
                "body": "Users cannot sign in after the latest deploy; retries fail.",
            })
        })
        .collect();
    let responses = benchmark_shaped_mcp_packages()
        .into_iter()
        .flat_map(|package| package.tools)
        .map(|tool| {
            let name = tool
                .capability_id
                .split_once('.')
                .map(|(_, name)| name.to_string())
                .expect("mock capability ids are provider-qualified");
            let content = if name == "search_issues" {
                serde_json::json!({"issues": issues})
            } else {
                serde_json::json!({"ok": true, "tool": name})
            };
            support::mock_mcp_server::MockToolResponse { name, content }
        })
        .collect();
    support::mock_mcp_server::start_mock_mcp_server(responses).await
}

/// The tool-discovery benchmark's shape with turn-start selection on: six
/// MCP packages, a large MCP result stored durably, a second turn that pages
/// it with `result_read` and then reaches a deferred tool through
/// `tool_search` and `tool_call`, and a third turn. Every model call carries
/// the byte-identical `tools` array, including the `tool_search` catalog
/// index, and the history keeps its single `initial` entry.
#[tokio::test]
async fn selection_keeps_the_tools_array_identical_with_mcp_packages_and_a_paged_result() {
    let server = benchmark_shaped_mcp_server().await;
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_production_default()
        .with_mock_mcp_packages(server.mcp_url(), benchmark_shaped_mcp_packages())
        .with_tool_prefetch(lexical_prefetch(&[]))
        .script([
            RebornScriptedReply::tool_call(
                "tracker__search_issues",
                serde_json::json!({"query": "login outage"}),
            ),
            RebornScriptedReply::text("found the outage issues"),
        ])
        .build()
        .await
        .expect("MCP packages harness builds");
    harness
        .submit_turn("Search the tracker issues about the login outage")
        .await
        .expect("opening turn completes");
    let opening = harness.model_tool_names(0).expect("first model request");
    for expected in [
        "tracker__search_issues",
        "builtin__result_read",
        TOOL_SEARCH_NAME,
    ] {
        assert!(
            opening.iter().any(|name| name == expected),
            "the opening request advertises {expected:?}: {opening:?}"
        );
    }
    assert!(
        !opening.iter().any(|name| name == "calendar__create_event"),
        "the calendar tool starts deferred: {opening:?}"
    );

    let result_ref = harness
        .latest_tool_result_ref()
        .await
        .expect("the large MCP result is stored durably");
    harness.push_script([
        RebornScriptedReply::tool_call(
            "builtin__result_read",
            serde_json::json!({"result_ref": result_ref, "offset": 0, "max_bytes": 8_000}),
        ),
        RebornScriptedReply::tool_call(
            TOOL_SEARCH_NAME,
            serde_json::json!({"query": "calendar event", "limit": 5}),
        ),
        RebornScriptedReply::tool_call(
            TOOL_CALL_NAME,
            serde_json::json!({
                "name": "calendar__create_event",
                "arguments": r#"{"query":"triage meeting"}"#
            }),
        ),
        RebornScriptedReply::text("read the issues and booked the triage meeting"),
    ]);
    harness
        .submit_turn("Read the rest of those issues, then book a triage meeting")
        .await
        .expect("second turn completes");
    harness
        .assert_tool_invoked("builtin.result_read")
        .await
        .expect("the large result is paged with result_read");
    harness
        .assert_tool_invoked("calendar.create_event")
        .await
        .expect("the deferred MCP tool is reached through tool_call");
    assert!(
        server.recorded_requests().iter().any(|request| {
            request.method == "tools/call"
                && request
                    .params
                    .as_ref()
                    .and_then(|params| params.get("name"))
                    .and_then(|name| name.as_str())
                    == Some("create_event")
        }),
        "the MCP server receives the deferred tool's call"
    );

    harness.push_script([RebornScriptedReply::text("you're welcome")]);
    harness
        .submit_turn("thanks")
        .await
        .expect("closing turn completes");

    let captured = harness.scripted_llm.captured_tool_definitions();
    assert_eq!(captured.len(), 7, "two, four and one model calls");
    for (index, tools) in captured.iter().enumerate() {
        let differing = reborn_support::assertions::differing_tool_definitions(&captured[0], tools);
        assert!(
            differing.is_empty(),
            "model call {index} rendered these tools differently from call 0: {differing:?}"
        );
    }
    harness
        .assert_model_tool_definitions_identical()
        .await
        .expect("the tools array is byte-identical on every model call");
    let history = harness
        .tool_selection_history_for_test()
        .await
        .expect("history read")
        .expect("history recorded");
    assert_eq!(history.entries.len(), 1, "only the initial selection");
}

/// The same deployment without turn-start selection, in each ordinary
/// deferred disclosure mode. Calling a deferred tool through `tool_call`
/// promotes it into the next turn's array, by design; apart from that one
/// added definition, every model call carries the byte-identical array.
#[tokio::test]
async fn ordinary_disclosure_changes_the_tools_array_only_by_promotion() {
    const PROMOTED: &str = "tracker__search_issues";
    for mode in [
        ToolDisclosureMode::Compact,
        ToolDisclosureMode::Signatures,
        ToolDisclosureMode::Namespaces,
        ToolDisclosureMode::Bridged,
    ] {
        let server = benchmark_shaped_mcp_server().await;
        let harness = RebornIntegrationHarness::test_default()
            .with_tool_disclosure_mode(mode)
            .with_mock_mcp_packages(server.mcp_url(), benchmark_shaped_mcp_packages())
            .script([
                RebornScriptedReply::tool_call(
                    TOOL_SEARCH_NAME,
                    serde_json::json!({"query": "tracker issues", "limit": 5}),
                ),
                RebornScriptedReply::tool_call(
                    TOOL_CALL_NAME,
                    serde_json::json!({
                        "name": PROMOTED,
                        "arguments": r#"{"query":"login outage"}"#
                    }),
                ),
                RebornScriptedReply::text("found the outage issues"),
            ])
            .build()
            .await
            .expect("MCP packages harness builds");
        harness
            .submit_turn("Search the tracker issues about the login outage")
            .await
            .expect("opening turn completes");
        let result_ref = harness
            .latest_tool_result_ref()
            .await
            .expect("the large MCP result is stored durably");
        harness.push_script([
            RebornScriptedReply::tool_call(
                "builtin__result_read",
                serde_json::json!({"result_ref": result_ref, "offset": 0, "max_bytes": 8_000}),
            ),
            RebornScriptedReply::text("read the issues"),
        ]);
        harness
            .submit_turn("Read the rest of those issues")
            .await
            .expect("second turn completes");
        harness.push_script([RebornScriptedReply::text("you're welcome")]);
        harness
            .submit_turn("thanks")
            .await
            .expect("closing turn completes");

        let captured = harness.scripted_llm.captured_tool_definitions();
        assert_eq!(
            captured.len(),
            6,
            "{mode:?}: three, two and one model calls"
        );
        assert!(
            !captured[0].iter().any(|tool| tool.name == PROMOTED),
            "{mode:?}: the MCP tool starts deferred"
        );
        assert!(
            captured[3].iter().any(|tool| tool.name == PROMOTED),
            "{mode:?}: the called tool is promoted into the next turn's array"
        );
        for (index, tools) in captured.iter().enumerate() {
            let differing =
                reborn_support::assertions::differing_tool_definitions(&captured[0], tools);
            assert!(
                differing.iter().all(|name| name == PROMOTED),
                "{mode:?}: model call {index} changed more than the promoted tool: {differing:?}"
            );
        }
        for (index, tools) in captured.iter().enumerate().skip(3) {
            let differing =
                reborn_support::assertions::differing_tool_definitions(&captured[3], tools);
            assert!(
                differing.is_empty(),
                "{mode:?}: model call {index} changed after the promotion took effect: \
                 {differing:?}"
            );
        }
    }
}
