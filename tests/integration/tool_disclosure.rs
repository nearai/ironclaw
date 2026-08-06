//! `REBORN_TOOL_DISCLOSURE=Bridged` int-tier coverage (enabler (b), #5149).
//!
//! Proves `.with_tool_disclosure_bridged()` reaches production's
//! `ToolDisclosureCapabilityDecorator` wiring
//! (`ironclaw_turn_runner::runtime::build_default_planned_runtime_inner`, gated on
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
//!    `crates/ironclaw_loop_host/src/tool_disclosure.rs`). The
//!    `GithubIssueTools` backend surfaces all 48 `github.*` manifest
//!    capabilities (`github_support::capability_ids()`), none of which is
//!    Core-tier (`CORE_TOOL_NAMES` suffix-match misses every github id), so
//!    the deferred active set is exactly the complete discovery bridge set. The
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
        .assert_model_message_content_contains("tool_search returned catalog matches")
        .await
        .expect("tool_search completion reaches the next production model request");
    harness
        .assert_model_message_content_contains("tool_describe returned schema")
        .await
        .expect("tool_describe completion reaches the next production model request");
    harness
        .assert_tool_invoked("github.get_repo")
        .await
        .expect("tool_call dispatches the selected target through the inner capability port");
    harness
        .assert_network_egress_header_contains(
            "api.github.com/repos/nearai/ironclaw",
            "authorization",
            "Bearer ghp_fake_fixture_token",
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
/// bridge set and NOT the flat `github__*` list.
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
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
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
async fn empty_allow_set_advertises_no_tools() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test([])
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("empty-allow-set bridged-disclosure harness builds");

    harness
        .submit_turn("answer without tools")
        .await
        .expect("text-only turn completes");

    harness
        .assert_model_tools_empty()
        .await
        .expect("an empty effective allow-set advertises no tools or bridges");
    harness
        .assert_network_egress_count(0)
        .await
        .expect("an empty surface performs no capability side effects");
}

/// #5647 trust boundary: the bridge-id exemption must not widen access to
/// UNDERLYING tools. Even a malformed deferred call that would otherwise take
/// the describe-first recovery path resolves to the real capability id
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

/// #5659-w6 follow-up: a genuinely wide effective allow-set still defers and
/// keeps the tool_search bridge, whose own advertised *description*
/// (the always-on catalog index of discoverable tool names, see
/// `catalog_index_tool_search_description`) must be narrowed by the caller's
/// allow-set too — not just tool_search RESULTS and tool_describe (#5712).
/// The bridge id is host-exempt from the outer `CapabilitySurfaceProfileFilter`
/// (#5647) so nothing else strips a leaked name out of that description text.
#[tokio::test]
async fn bridged_mode_wide_effective_allow_set_keeps_narrowed_tool_search_description() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(wide_effective_github_allowlist())
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("narrowed bridged-disclosure harness builds");

    harness.submit_turn("hello").await.expect("turn completes");

    harness
        .assert_model_tools_contains(TOOL_SEARCH_NAME)
        .await
        .expect("bridge ids stay advertised under a narrowed allow-set (#5647)");
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

/// #5712: tool_search RESULTS are narrowed by the caller's allow-set — the
/// bridge port's catalog is built below the profile filter, so without
/// result filtering a narrowed profile reads every capability's metadata.
#[tokio::test]
async fn narrowed_allow_set_filters_tool_search_results() {
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(wide_effective_github_allowlist())
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

/// #5712: tool_describe of a non-allowlisted id reads as unknown — same
/// message as a nonexistent name, so existence itself is not disclosed.
///
/// A substring check on `safe_summary` alone would pass even for an empty
/// index, and would miss an existence oracle hiding in the envelope's other
/// fields (`model_observation`'s structured diagnostic, in particular). This
/// scripts BOTH a non-allowlisted target (`github.list_issues`, present in
/// the catalog but outside the allow-set) and a target that is not in the
/// catalog at all, then asserts their persisted `ToolResultReferenceEnvelope`s
/// are identical modulo `result_ref` — which is derived from
/// `RebornScriptedReply::tool_call`'s process-global call-id counter (see
/// `synthetic_provider_error_result_ref`) and so differs between the two
/// calls by construction, carrying no allow-set/existence information.
#[tokio::test]
async fn narrowed_allow_set_denies_tool_describe_of_non_allowlisted_id() {
    const NONEXISTENT_TARGET: &str = "totally_nonexistent_tool";
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
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
/// exists outside the caller's effective allow-set. Both a catalog-known denied
/// target and a genuinely nonexistent target stay on the same recoverable bridge
/// path and persist byte-equivalent result envelopes modulo their run-scoped ids.
#[tokio::test]
async fn narrowed_allow_set_denies_tool_call_of_non_allowlisted_id_without_existence_oracle() {
    const NONEXISTENT_TARGET: &str = "totally_nonexistent_tool";
    let harness = RebornIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .with_narrowed_capability_allow_set_for_bridged_test(["github.get_repo"])
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
