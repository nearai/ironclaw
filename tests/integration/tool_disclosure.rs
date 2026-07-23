//! `IRONCLAW_TOOL_DISCLOSURE=Bridged` int-tier coverage (enabler (b), #5149).
//!
//! Proves `.with_tool_disclosure_bridged()` reaches production's
//! `ToolDisclosureCapabilityDecorator` wiring
//! (`ironclaw_runner::runtime::build_default_planned_runtime_inner`, gated on
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
//!    `crates/ironclaw_runner/src/tool_disclosure.rs`). The
//!    `GithubIssueTools` backend surfaces all 48 `github.*` manifest
//!    capabilities (`github_support::capability_ids()`), none of which is
//!    Core-tier (`CORE_TOOL_NAMES` suffix-match misses every github id), so
//!    the deferred active set is exactly the `tool_search` bridge. The
//!    13-capability `BuiltinHttpTools` backend stays UNDER the cap, so
//!    bridged mode is wired-but-inert there — pinned below as the threshold
//!    control.
//!
//! Harness note: bridged groups resolve `CapabilityAllowSet::All` (see
//! `into_group`) — production's top-level resolution — because the harness's
//! default granted-ids allowlist would strip the synthetic `ironclaw.*`
//! bridge ids at `CapabilitySurfaceProfileFilter` and ship ZERO tools.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;

/// Bridge meta-tool names (`tool_disclosure.rs`'s `TOOL_SEARCH_NAME`/
/// `TOOL_DESCRIBE_NAME`/`TOOL_CALL_NAME`), hardcoded as literals: the
/// constants are `pub(crate)` inside `ironclaw_runner` and not part of the
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
    let harness = IronClawIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_github_issue_tools()
        .script([IronClawScriptedReply::text("done")])
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
/// explicit pin, an ambient `IRONCLAW_TOOL_DISCLOSURE=Bridged` in the process
/// env would silently flip this control into Bridged mode too, and the
/// assertions below would then be discriminating on nothing.
/// `apply_hermetic_env()` also scrubs the var, but the explicit builder call
/// is what makes this test's mode independent of the ambient env by
/// construction, not just by today's harness hygiene.
#[tokio::test]
async fn default_mode_surfaces_the_flat_wide_tool_list() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_tool_disclosure_off()
        .with_github_issue_tools()
        .script([IronClawScriptedReply::text("done")])
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
    let harness = IronClawIntegrationHarness::test_default()
        .with_tool_disclosure_bridged()
        .with_builtin_http_tools()
        .script([IronClawScriptedReply::text("done")])
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
