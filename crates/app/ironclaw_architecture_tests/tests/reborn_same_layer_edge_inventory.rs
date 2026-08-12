//! Same-layer dependency inventory — the guard the layer matrix cannot be
//! (target-architecture epic #3773, CHECKLIST WS10; issue #7149).
//!
//! # The hole
//!
//! `layer_allows_dependency` in `reborn_dependency_boundaries.rs` is
//! **reflexive**: every layer permits itself. So an edge between two crates in
//! the same layer is legal by construction, no exception entry is required,
//! and `LAYER_MATRIX_EXCEPTIONS` never sees it. Measured on `origin/main`
//! @ `676d86ce02`: **391 workspace normal edges, 73 of them same-layer** — and
//! that count sat flat across the entire restructure while the *reported*
//! exception count fell 20 → 6. (**70** on the merged tree at `acbf1d89e8`;
//! #7143's `host_ingress` re-layer took three of them cross-layer, and the
//! stale-row half of this gate is what named all three.) The improvement was real; it was also
//! measuring a category that excludes most of the coupling. Every same-layer
//! pair that is not yet an edge is silently addable:
//! `ironclaw_assistant → ironclaw_webui` compiles and passes the whole
//! architecture suite today.
//!
//! # The instrument
//!
//! [`SAME_LAYER_EDGE_INVENTORY`] is that missing default guard, built to the
//! same shape as `LAYER_MATRIX_EXCEPTIONS`:
//!
//! 1. **Complete.** Every live same-layer edge must be inventoried. A 74th is
//!    a red build, which is the property the layer matrix cannot provide.
//! 2. **Not stale.** An inventoried edge that no longer exists is a red build,
//!    so deletions are banked rather than left as headroom.
//! 3. **Shrink-only, both directions.** The count is pinned by an equality.
//!    Growth is new coupling; slack is an unclaimed budget for it — the exact
//!    defect #7147 found in two other ratchets on this same tree.
//! 4. **Tracked.** Every entry names an owner and the workstream that decides
//!    its disposition, and placeholders count as missing.
//!
//! ⚠ **What `decided_in` does and does not claim.** It is *not* a promise that
//! every inventoried edge is scheduled for deletion. Same-layer edges are
//! legal by the matrix and some are permanent by charter — a provider
//! implementing its own contract (`memory_native → memory`), the contracts
//! family's internal vocabulary (`product_contracts → host_api`). The field
//! names the workstream that owns the **consumer** crate's family and will make
//! that call, taken from CHECKLIST's own workstream headings. What the
//! inventory guarantees is narrower and enforceable: no same-layer edge appears
//! without a reviewer seeing it, and the total cannot drift upward.
//!
//! # The second rule: a downward re-layer needs a consumer-side pin
//!
//! Moving a crate *down* a layer widens who may reach it, and the layer matrix
//! by construction reports that widening as an improvement. #7143 demoted
//! `ironclaw_host_ingress` to `substrates` with no such pin, leaving it
//! reachable by every crate above `substrates` with nothing objecting —
//! and when that PR merged into this branch, the rule below went red on it
//! by name and the pin was supplied here.
//! [`CRATE_LAYER_ORIGINS`] makes the trigger mechanical rather than
//! remembered: each crate's **first** declared layer is frozen, so a live layer
//! below it is a permanent, detectable demotion, and the gate then demands a
//! [`DowngradePin`] whose frozen consumer set is enforced on every commit.
//!
//! Both origin rows were derived from `git log`, not assumed: across all 67
//! layered crates exactly one downward re-layer has ever happened
//! (`ironclaw_extension_registry` `loops` → `substrates`, #7094 / WS2), alongside two
//! promotions (`hooks` `substrates` → `loops`, `runner` `kernel` → `loops`)
//! which need no pin because moving up narrows reach.
//!
//! ✎ **That census is a snapshot of when this gate was authored and is no
//! longer the live count — read [`DOWNGRADE_PINS`], not this paragraph.** Three
//! more demotions have landed since: `ironclaw_host_ingress` `products` →
//! `substrates` (#7143, the move that motivated the rule), `ironclaw_skills`
//! `loops` → `substrates` (#7141 / WS4), and `ironclaw_extension_support`
//! `loops` → `runtimes` (WS3 closeout). The last of those is worth naming here
//! because it is the first demotion taken *for* the exception register rather
//! than alongside it: it deleted `LAYER_MATRIX_EXCEPTIONS`' final entry, so the
//! consumer-side pin is now the only structural check standing over that edge.
//! Kept as four rows rather than folded into a count, since a pin's whole value
//! is naming who may reach the demoted crate.
//!
//! ⚠ **Every test function here must keep its `reborn_` prefix.** The file name
//! is not what selects it: `code_style.yml` runs
//! `cargo test -p ironclaw_architecture_tests reborn`, and that argument is a **test
//! name** filter, not a path filter. Written without the prefix these gates
//! compiled, passed locally, and reported `running 0 tests` under the exact
//! command CI uses — inert in one of the two lanes that run them, which is the
//! failure mode this whole file exists to stop. Verified by running that command
//! and reading the count.

#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;

use ratchet_support::workspace_root;

/// The layer ladder, lowest first. Mirrors `IRONCLAW_CRATE_LAYERS` in
/// `reborn_dependency_boundaries.rs` minus `legacy`, which is not a rung — it
/// is the v1 escape hatch that may depend on anything, so "below" is undefined
/// for it. A crate declaring a layer this list does not know fails loudly
/// rather than being skipped: an unknown layer that silently drops out of the
/// walk is how a scan passes having measured less than it claims.
const LAYER_LADDER: &[&str] = &[
    "contracts",
    "substrates",
    "runtimes",
    "kernel",
    "loops",
    "products",
    "app",
];

/// One inventoried same-layer edge.
struct SameLayerEdge {
    crate_name: &'static str,
    dependency_name: &'static str,
    /// The layer both ends sit in. Checked against `cargo metadata`, so this
    /// field cannot quietly describe a world that no longer exists.
    layer: &'static str,
    /// The workstream that owns the consumer crate's family (CHECKLIST
    /// headings) — who answers for the edge.
    owner: &'static str,
    /// The workstream in which this edge's disposition is decided. See the
    /// module note: not a deletion promise.
    decided_in: &'static str,
}

/// Every same-layer dependency edge in the workspace, one row each.
///
/// `owner` is the consumer crate's **§5 family directory** (PROPOSAL §5 / §9's
/// crate mapping). `decided_in` is the CHECKLIST workstream that owns that
/// family — WS1 contracts, WS2 extensions, WS3 kernel, WS4 loop, WS5 product,
/// WS6 app + domains. Only those five families have a dedicated workstream
/// heading; `substrates/`, `events/` and `lanes/` do not, so their crates take
/// the workstream their per-crate rows actually sit in (WS6 for the eviction
/// and cleanup rows; WS3 for `lanes/`, which owns the sandbox merge and the
/// `mcp` registry flip). `first_party_extension_ports` has no target family at
/// all — §9 marks it delete-after-migration — so it carries WS8.
///
/// Rows are grouped by layer and sorted, which is also the order the failure
/// messages print, so a diff against a fresh `cargo metadata` reads cleanly.
const SAME_LAYER_EDGE_INVENTORY: &[SameLayerEdge] = &[
    // ---- app ----
    SameLayerEdge {
        crate_name: "ironclaw",
        dependency_name: "ironclaw_composition",
        layer: "app",
        owner: "app/",
        decided_in: "WS6",
    },
    // ---- contracts ----
    SameLayerEdge {
        crate_name: "ironclaw_extension_contracts",
        dependency_name: "ironclaw_host_api",
        layer: "contracts",
        owner: "contracts/",
        decided_in: "WS1",
    },
    SameLayerEdge {
        crate_name: "ironclaw_loop_contracts",
        dependency_name: "ironclaw_extension_contracts",
        layer: "contracts",
        owner: "contracts/",
        decided_in: "WS1",
    },
    SameLayerEdge {
        crate_name: "ironclaw_loop_contracts",
        dependency_name: "ironclaw_host_api",
        layer: "contracts",
        owner: "contracts/",
        decided_in: "WS1",
    },
    SameLayerEdge {
        crate_name: "ironclaw_product_contracts",
        dependency_name: "ironclaw_extension_contracts",
        layer: "contracts",
        owner: "contracts/",
        decided_in: "WS1",
    },
    SameLayerEdge {
        crate_name: "ironclaw_product_contracts",
        dependency_name: "ironclaw_host_api",
        layer: "contracts",
        owner: "contracts/",
        decided_in: "WS1",
    },
    // ---- kernel ----
    SameLayerEdge {
        crate_name: "ironclaw_approvals",
        dependency_name: "ironclaw_authorization",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_approvals",
        dependency_name: "ironclaw_runtime_policy",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS6 (the profile approval gate evicted from the composition root consumes `MinimalApprovalBypass`, the classification `runtime_policy` owns per §4.4)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_approvals",
        dependency_name: "ironclaw_trust",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS6 (same eviction: the gate implements `authorization`'s `TrustAwareCapabilityDispatchAuthorizer`, whose signature names `ironclaw_trust::TrustDecision`)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_processes",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3 (#7141: processes re-layered runtimes -> kernel; the edge predates the move and was downward-legal before it)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_processes",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3 (#7141: processes re-layered runtimes -> kernel; previously downward-legal)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_processes",
        dependency_name: "ironclaw_resources",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3 (#7141: the former LAYER_MATRIX_EXCEPTIONS entry became same-layer when processes re-layered to kernel; the ratchet reported it stale and the register shrank 5 -> 4)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_turns",
        dependency_name: "ironclaw_processes",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3 (#7141: processes re-layered runtimes -> kernel; previously downward-legal, #6696's journal dependency)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_authorization",
        dependency_name: "ironclaw_trust",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_approvals",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_authorization",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_resources",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_runtime_policy",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_trust",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_turns",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_approvals",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_authorization",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_capabilities",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_resources",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_runtime_policy",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_trust",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    SameLayerEdge {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_turns",
        layer: "kernel",
        owner: "kernel/",
        decided_in: "WS3",
    },
    // ---- loops ----
    // ✎ WS8, 2026-08-05: `first_party_extension_ports -> loop_host` is gone —
    // the crate dissolved into `ironclaw_loop_host` (PROPOSAL §9 row 55), which
    // is what "dissolved (no target family)" was waiting for. The baseline
    // below drops with it; this is the equality doing its job.
    SameLayerEdge {
        crate_name: "ironclaw_turn_runner",
        dependency_name: "ironclaw_agent_loop",
        layer: "loops",
        owner: "loop/",
        decided_in: "WS4",
    },
    SameLayerEdge {
        crate_name: "ironclaw_turn_runner",
        dependency_name: "ironclaw_hooks",
        layer: "loops",
        owner: "loop/",
        decided_in: "WS4",
    },
    SameLayerEdge {
        crate_name: "ironclaw_turn_runner",
        dependency_name: "ironclaw_loop_host",
        layer: "loops",
        owner: "loop/",
        decided_in: "WS4",
    },
    SameLayerEdge {
        crate_name: "ironclaw_extension_host",
        dependency_name: "ironclaw_loop_host",
        layer: "loops",
        owner: "extensions/",
        decided_in: "the WS2 flip (extension_host products -> loops; the loop_host edge predates the flip and becomes same-layer with it)",
    },
    // ---- products ----
    SameLayerEdge {
        crate_name: "ironclaw_extension_manager",
        dependency_name: "ironclaw_assistant",
        layer: "products",
        owner: "extensions/",
        decided_in: "WS2",
    },
    SameLayerEdge {
        crate_name: "ironclaw_openai_compat",
        dependency_name: "ironclaw_assistant",
        layer: "products",
        owner: "product/",
        decided_in: "WS5",
    },
    SameLayerEdge {
        crate_name: "ironclaw_webui",
        dependency_name: "ironclaw_assistant",
        layer: "products",
        owner: "product/",
        decided_in: "WS5",
    },
    SameLayerEdge {
        crate_name: "ironclaw_webui",
        dependency_name: "ironclaw_openai_compat",
        layer: "products",
        owner: "product/",
        decided_in: "WS5",
    },
    // ---- runtimes ----
    SameLayerEdge {
        crate_name: "ironclaw_wasm",
        dependency_name: "ironclaw_wasm_limiter",
        layer: "runtimes",
        owner: "lanes/",
        decided_in: "WS3",
    },
    // ---- substrates ----
    SameLayerEdge {
        crate_name: "ironclaw_attachments",
        dependency_name: "ironclaw_extractors",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_attachments",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        // The web-app subscription store rides the scoped-filesystem plane
        // (per database.md), the same edge every persisting domain crate
        // carries; both sit at the substrates layer by family rule.
        crate_name: "ironclaw_web_app",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "web-app channel (2026-08-08)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_attachments",
        dependency_name: "ironclaw_threads",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_auth",
        dependency_name: "ironclaw_event_log",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_auth",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_auth",
        dependency_name: "ironclaw_secrets",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_conversations",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_conversations",
        dependency_name: "ironclaw_triggers",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_event_projections",
        dependency_name: "ironclaw_event_log",
        layer: "substrates",
        owner: "events/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_event_streams",
        dependency_name: "ironclaw_event_projections",
        layer: "substrates",
        owner: "events/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_event_streams",
        dependency_name: "ironclaw_outbound",
        layer: "substrates",
        owner: "events/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_extension_registry",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "extensions/",
        decided_in: "WS2",
    },
    SameLayerEdge {
        crate_name: "ironclaw_skills",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "substrates/",
        decided_in: "WS4 (#7141: skills re-layered loops -> substrates per the WS4 row; its existing filesystem dep became same-layer)",
    },
    SameLayerEdge {
        crate_name: "ironclaw_filesystem",
        dependency_name: "ironclaw_libsql_runtime",
        layer: "substrates",
        owner: "substrates/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_filesystem",
        dependency_name: "ironclaw_observability",
        layer: "substrates",
        owner: "substrates/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_filesystem",
        dependency_name: "ironclaw_safety",
        layer: "substrates",
        owner: "substrates/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_llm",
        dependency_name: "ironclaw_safety",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_memory_mem0",
        dependency_name: "ironclaw_memory",
        layer: "substrates",
        owner: "extensions/packages/mem0/",
        decided_in: "WS2",
    },
    SameLayerEdge {
        crate_name: "ironclaw_memory_native",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "extensions/packages/memory-native/",
        decided_in: "WS2",
    },
    SameLayerEdge {
        crate_name: "ironclaw_memory_native",
        dependency_name: "ironclaw_memory",
        layer: "substrates",
        owner: "extensions/packages/memory-native/",
        decided_in: "WS2",
    },
    SameLayerEdge {
        crate_name: "ironclaw_memory_native",
        dependency_name: "ironclaw_safety",
        layer: "substrates",
        owner: "extensions/packages/memory-native/",
        decided_in: "WS2",
    },
    SameLayerEdge {
        crate_name: "ironclaw_outbound",
        dependency_name: "ironclaw_attachments",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_outbound",
        dependency_name: "ironclaw_event_projections",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_outbound",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_event_store",
        dependency_name: "ironclaw_event_log",
        layer: "substrates",
        owner: "events/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_event_store",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "events/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_identity",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_trace_commons",
        dependency_name: "ironclaw_llm",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_trace_commons",
        dependency_name: "ironclaw_safety",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_secrets",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "substrates/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_threads",
        dependency_name: "ironclaw_filesystem",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_threads",
        dependency_name: "ironclaw_safety",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    SameLayerEdge {
        crate_name: "ironclaw_triggers",
        dependency_name: "ironclaw_libsql_runtime",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
    // The trusted-trigger prompt scan moved *behind* the seam: it now runs in
    // `TrustedTriggerSubmitRequest::new`, so "this prompt passed the scan" is
    // an invariant of the sealed type rather than a step one submitter
    // performs. `ironclaw_safety` is an I/O-free `substrates` leaf with no
    // ironclaw dependencies, so this is a peer edge, not a reach upward — and
    // it is the counterpart of the edge `ironclaw_conversations` *lost* in the
    // same change, which `reborn_dependency_boundaries.rs` now forbids by name.
    // (PROPOSAL §6.4.2/§6.4.3.)
    SameLayerEdge {
        crate_name: "ironclaw_triggers",
        dependency_name: "ironclaw_safety",
        layer: "substrates",
        owner: "domains/",
        decided_in: "WS6",
    },
];

/// The same-layer edge count on `origin/main` @ `676d86ce02` (2026-08-04).
///
/// Measured from `cargo metadata --no-deps`, counting **deduplicated
/// `(crate, dependency)` pairs where both ends declare the same layer and the
/// dependency is a `normal` kind** — the same filter
/// `reborn_workspace_crates_declare_layers_and_follow_layer_matrix` applies
/// when it consults the layer matrix, so the two gates measure one graph.
/// Recounted here rather than inherited: #7149 quotes 68 from an earlier tree,
/// and 68 is not what this tree holds.
///
/// ✎ **73 → 70 when #7143 merged.** Its `host_ingress` re-layer turned
/// `extension_host`/`operator`/`webui → host_ingress` from `products →
/// products` into legal cross-layer edges. The stale-row half of the gate
/// caught all three by name, and the baseline is lowered in the same change
/// so the improvement is banked as a floor rather than left as headroom.
/// Recounted on the merged tree, not derived by subtracting three.
///
/// ✎ **72 → 74 (WS6, the composition policy eviction).** Two edges are *added*
/// — the only growth this number has taken — and both are the same kernel crate
/// paying for a module that left the app layer: `approvals → trust` and
/// `approvals → runtime_policy`, both arriving with the profile approval gate
/// (`profile_gate.rs` / `profile_gate_policy.rs`) evicted from
/// `ironclaw_composition`. Neither is avoidable at the destination: the
/// gate *implements* `ironclaw_authorization`'s
/// `TrustAwareCapabilityDispatchAuthorizer`, whose method signature names
/// `ironclaw_trust::TrustDecision`, and it consumes `MinimalApprovalBypass`,
/// which §4.4 pins to `ironclaw_runtime_policy` as "the one place that
/// classification lives". The trade is deliberate and stated rather than
/// hidden: 2,178 lines of authorization semantics stop living in the assembly
/// root, at the cost of two edges inside a kernel family that already carries
/// twelve (`capabilities` and `host_runtime` hold six each). Growth here is a
/// reviewed decision, not drift — see PROPOSAL §6.5.2/§6.10.1.
///
/// ✎ **Still 72 after the WS6 renames (#7152) folded onto this batch — one row
/// moved, the count did not.** The trusted-trigger prompt scan moved *behind*
/// the seam: it now runs in `TrustedTriggerSubmitRequest::new`, so
/// "this prompt passed the scan" is an invariant of the sealed type rather than
/// a step one submitter performs. That deletes `conversations → safety` (which
/// `reborn_dependency_boundaries.rs` now forbids by name) and adds
/// `triggers → safety` in its place — a swap, not growth. `ironclaw_safety` is
/// an I/O-free `substrates` leaf with no ironclaw dependencies, so either
/// spelling is a peer edge rather than a reach upward. See PROPOSAL
/// §6.4.2/§6.4.3.
///
/// Both halves were found by the gate rather than by reading the diff: the new
/// edge tripped the not-inventoried arm and the dead one tripped the stale-row
/// arm, on a merge that is the first tree where both sides of the swap exist.
/// The equality is what made the second half loud — under a `<=` ratchet the
/// stale row would have sat green as one entry of slack.
///
/// ✎ **72 → 71 (WS10, the `projects` → `identity` merge, 2026-08-05).** The
/// §12.10 consolidation audit's single clear merge verdict landed:
/// `ironclaw_projects` is gone and its records are `ironclaw_identity`'s
/// `projects` module, so `projects → filesystem` died and `identity →
/// filesystem` — already inventoried — absorbed it. **One** row deleted, none
/// added: the merged module's only workspace dependencies were
/// `ironclaw_host_api` and `ironclaw_filesystem`, exactly the two
/// `reborn_dependency_boundaries.rs` already pins `ironclaw_identity` to, which
/// is why the fold costs no edge. Found by this gate's stale-row arm rather than
/// by reading the diff, and recounted on the merged tree (65 layered crates,
/// 387 workspace edges, 71 same-layer) rather than derived by subtracting one.
///
/// The target is fewer, and every wave that deletes one must lower this number
/// in the same PR — the equality below refuses both growth *and* slack, so a
/// forgotten decrement is red rather than banked as headroom.
/// ✎ **2026-08-05 (WS8): 72 → 71; the projects merge in the same batch took it to 70.** The dissolution of
/// `ironclaw_first_party_extension_ports` into `ironclaw_loop_host` removed the
/// `loops` pair the two of them formed. No edge was re-plumbed and none was
/// added: the crate's five workspace dependencies were already `loop_host`'s.
const SAME_LAYER_EDGE_BASELINE: usize = 71;

/// Sanity floors for the metadata walk. A gate that scans nothing must never
/// read as success; these are deliberately far below the live values (✎ **65**
/// layered crates, **387** workspace edges, re-measured 2026-08-05 after the
/// `projects` → `identity` merge; the 67/391 this note used to quote was a
/// snapshot of `origin/main` @ `676d86ce02`) so they catch a broken walk without
/// tripping on ordinary growth or deletion.
const MIN_LAYERED_CRATES: usize = 50;
const MIN_WORKSPACE_EDGES: usize = 250;

/// Every crate's **first** declared layer, from `git log` over its
/// `Cargo.toml`. Rows are append-only and a row's origin layer is immutable —
/// editing one to match a demotion is the one way to defeat the check below,
/// and it is a visible diff in a file whose entire subject is that rule.
///
/// **Append-only is not "never deleted."** The one sanctioned removal is a
/// crate leaving the workspace, and the gate below states it rather than leaving
/// it to judgement: `vanished` fails with *"CRATE_LAYER_ORIGINS names crates the
/// workspace no longer has … Delete the rows — a stale origin row is a demotion
/// detector aimed at nothing."* A row may therefore be dropped **only** in the
/// change that deletes or merges away its crate, and only because the gate is
/// already red on it. ✎ Exercised 2026-08-05 by the `projects` → `identity`
/// merge (WS10, PROPOSAL §12.10): `ironclaw_projects` left the workspace, the
/// gate named it by that message, and its row went with it.
///
/// `ironclaw_integration_tests` predates the layer metadata (its
/// `Cargo.toml` carried no `layer` when introduced); it is recorded at its
/// current `app`, the top rung, where no demotion is representable anyway.
const CRATE_LAYER_ORIGINS: &[(&str, &str)] = &[
    ("ironclaw", "app"),
    ("ironclaw_agent_loop", "loops"),
    ("ironclaw_approvals", "kernel"),
    ("ironclaw_architecture_tests", "app"),
    ("ironclaw_attachments", "substrates"),
    ("ironclaw_auth", "substrates"),
    ("ironclaw_authorization", "kernel"),
    ("ironclaw_capabilities", "kernel"),
    ("ironclaw_common", "contracts"),
    ("ironclaw_conversations", "substrates"),
    ("ironclaw_event_projections", "substrates"),
    ("ironclaw_event_streams", "substrates"),
    ("ironclaw_event_log", "substrates"),
    ("ironclaw_extension_contracts", "contracts"),
    ("ironclaw_extension_host", "products"),
    ("ironclaw_extension_manager", "products"),
    ("ironclaw_extension_support", "loops"),
    // The one downward re-layer in this repo's history — see DOWNGRADE_PINS.
    ("ironclaw_extension_registry", "loops"),
    ("ironclaw_extractors", "substrates"),
    ("ironclaw_filesystem", "substrates"),
    // ✎ WS8, 2026-08-05: `ironclaw_first_party_extension_ports`' row is deleted
    // with the crate, per this table's own stale-row rule — a demotion detector
    // aimed at a crate that no longer exists detects nothing.
    // Promoted substrates -> loops. Moving UP narrows reach; no pin owed.
    ("ironclaw_hooks", "substrates"),
    ("ironclaw_host_api", "contracts"),
    ("ironclaw_host_ingress", "products"),
    ("ironclaw_host_runtime", "kernel"),
    ("ironclaw_libsql_runtime", "substrates"),
    ("ironclaw_llm", "substrates"),
    ("ironclaw_loop_contracts", "contracts"),
    ("ironclaw_loop_host", "loops"),
    ("ironclaw_mcp", "runtimes"),
    ("ironclaw_memory", "substrates"),
    ("ironclaw_memory_mem0", "substrates"),
    ("ironclaw_memory_native", "substrates"),
    ("ironclaw_network", "substrates"),
    ("ironclaw_observability", "substrates"),
    ("ironclaw_operator", "products"),
    ("ironclaw_outbound", "substrates"),
    ("ironclaw_processes", "runtimes"),
    ("ironclaw_assistant", "products"),
    ("ironclaw_product_contracts", "contracts"),
    ("ironclaw_prompt_envelope", "contracts"),
    ("ironclaw_composition", "app"),
    ("ironclaw_config", "substrates"),
    ("ironclaw_event_store", "substrates"),
    ("ironclaw_identity", "substrates"),
    ("ironclaw_integration_tests", "app"),
    ("ironclaw_openai_compat", "products"),
    ("ironclaw_trace_commons", "substrates"),
    ("ironclaw_resources", "kernel"),
    // Promoted kernel -> loops. Moving UP narrows reach; no pin owed.
    ("ironclaw_turn_runner", "kernel"),
    ("ironclaw_runtime_policy", "kernel"),
    ("ironclaw_safety", "substrates"),
    ("ironclaw_sandbox", "runtimes"),
    ("ironclaw_secrets", "substrates"),
    ("ironclaw_skills", "loops"),
    ("ironclaw_slack_extension", "products"),
    ("ironclaw_stress", "app"),
    ("ironclaw_telegram_extension", "products"),
    ("ironclaw_threads", "substrates"),
    ("ironclaw_triggers", "substrates"),
    ("ironclaw_web_app", "substrates"),
    ("ironclaw_web_app_extension", "products"),
    ("ironclaw_trust", "kernel"),
    ("ironclaw_turns", "kernel"),
    ("ironclaw_wasm", "runtimes"),
    ("ironclaw_wasm_limiter", "runtimes"),
    ("ironclaw_webui", "products"),
];

/// The consumer-side pin a downward re-layer owes: after the move, the set of
/// crates allowed to depend on the demoted crate is frozen, so the reach the
/// demotion *legalised* cannot be taken without a reviewed edit.
///
/// A layer ceiling would not do. `ironclaw_extension_registry` moved down precisely so
/// that `capabilities`/`host_runtime`/`mcp`/`scripts` could reach it
/// (PROPOSAL §6.8.1), so any ceiling wide enough to allow that is wide enough
/// to allow every other kernel and runtimes crate too — a pin that permits
/// what already happened and nothing else is the only one that bites.
struct DowngradePin {
    crate_name: &'static str,
    from_layer: &'static str,
    to_layer: &'static str,
    /// Where the move landed, so the pin is traceable to a change.
    demoted_in: &'static str,
    /// Every crate permitted to depend on it, frozen at the move. Enforced
    /// exactly: an absent one is a stale row, an extra one is reach taken
    /// without review.
    permitted_consumers: &'static [&'static str],
}

const DOWNGRADE_PINS: &[DowngradePin] = &[
    DowngradePin {
        crate_name: "ironclaw_extension_host",
        from_layer: "products",
        to_layer: "loops",
        demoted_in: "the WS2 flip (D-A factory port + batch-2 union emptied the \
                     product-reference ledger and trait residue to 0/0, deleting \
                     the manifest edge per the re-keyed biconditional)",
        // The flip PROPOSAL §12.1(c) calls "the sharpest edge in the whole
        // restructure": every port inversion (WS2.1/2.2/2.5/2.6 + D-A) had to
        // land first, and the port-inversion gate now pins the inverted shape.
        // Frozen at the four normal-dep consumers that existed at the flip; a fifth is a reviewed
        // decision. `webui -> extension_host` and
        // `extension_manager -> extension_host` left the same-layer inventory
        // with this demotion (products -> loops is downward); this pin is what
        // keeps that widening deliberate.
        permitted_consumers: &[
            "ironclaw",
            "ironclaw_extension_manager",
            "ironclaw_composition",
            "ironclaw_webui",
        ],
    },
    DowngradePin {
        crate_name: "ironclaw_skills",
        from_layer: "loops",
        to_layer: "substrates",
        demoted_in: "#7141 (WS4 — skills re-layer, landed in the Waves 0-4 batch)",
        permitted_consumers: &[
            "ironclaw_extension_host",
            "ironclaw_extension_manager",
            "ironclaw_extension_support",
            // ✎ WS8, 2026-08-05: `ironclaw_first_party_extension_ports` dropped
            // off here by dissolving into `ironclaw_loop_host`, which was
            // already a permitted consumer — the reach did not widen, one of
            // the two crates holding it stopped existing.
            "ironclaw_loop_host",
            "ironclaw_composition",
        ],
    },
    DowngradePin {
        crate_name: "ironclaw_extension_support",
        from_layer: "loops",
        to_layer: "runtimes",
        demoted_in: "WS3 closeout (the move that emptied LAYER_MATRIX_EXCEPTIONS)",
        // The demotion that deleted the register's last entry,
        // `host_runtime -> extension_support`. WS3's executor/adapter seam makes
        // the kernel a *designed* consumer of this crate — a tool moves here as
        // an executor and leaves its handler, manifest and registry wiring in
        // `ironclaw_host_runtime` — so a `loops` declaration contradicted the
        // design rather than describing it. `runtimes` is the least demotion
        // that legalizes a kernel consumer and creates no same-layer edge
        // (`substrates` would have hidden six of the crate's seven
        // dependencies from the matrix). Frozen at the five consumers that
        // existed at the move; a sixth is a reviewed decision, and that review
        // is the whole point of the pin, because the widening here reaches down
        // two rungs rather than one.
        // (`ironclaw` is the binary crate at `crates/app/ironclaw_cli/` —
        // the pin is keyed on package names, which is what `cargo metadata`
        // reports and what makes a row able to fire at all.)
        permitted_consumers: &[
            "ironclaw",
            "ironclaw_extension_host",
            "ironclaw_extension_manager",
            "ironclaw_host_runtime",
            "ironclaw_composition",
        ],
    },
    DowngradePin {
        crate_name: "ironclaw_extension_registry",
        from_layer: "loops",
        to_layer: "substrates",
        demoted_in: "#7094 (WS2 — Extensions family)",
        // `ironclaw_assistant` dropped off with CHECKLIST WS5's `product` narrows
        // row: `adapter_registry` was its only consumer of the registry, and it
        // moved to `ironclaw_extension_registry::host_api::product_adapter` (projection)
        // and `ironclaw_extension_contracts::product_adapter_section` (schema).
        // The edge is now forbidden outright by the crate's `BoundaryRule` in
        // `reborn_dependency_boundaries.rs`, so it cannot come back here.
        permitted_consumers: &[
            "ironclaw_capabilities",
            "ironclaw_extension_host",
            "ironclaw_extension_manager",
            "ironclaw_host_runtime",
            "ironclaw_composition",
        ],
    },
    DowngradePin {
        crate_name: "ironclaw_host_ingress",
        from_layer: "products",
        to_layer: "substrates",
        demoted_in: "#7143 (WS2 — re-layer host_ingress)",
        // The demotion this rule was written for. #7143 landed the move with no
        // consumer-side pin, leaving the crate reachable by every substrates,
        // runtimes, kernel and loops crate with nothing objecting — the matrix
        // reads the widening as an improvement. Frozen at the four consumers
        // that existed when it merged; a fifth is a reviewed decision.
        //
        // ✎ The fifth landed 2026-08-05 and is that reviewed decision, taken
        // where the rule asks for it: `ironclaw_openai_compat` takes the edge
        // for CHECKLIST WS6's OpenAI-compat eviction clause, which names
        // `product_contracts` + `host_ingress` as exactly the two crates the
        // movable residue is reachable with. The edge carries one type
        // (`ProtectedRouteMount`) into `src/mount.rs`, the crate's own
        // route-mount assembly; it is the same shape as `ironclaw_webui`'s and
        // `ironclaw_operator`'s, a transport naming the carrier it hands back
        // to composition.
        permitted_consumers: &[
            "ironclaw_extension_host",
            "ironclaw_operator",
            "ironclaw_composition",
            "ironclaw_webui",
            "ironclaw_openai_compat",
        ],
    },
];

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

fn cargo_metadata() -> Value {
    let manifest_path = workspace_root().join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata output must be JSON")
}

fn is_ironclaw_package(name: &str) -> bool {
    name == "ironclaw" || name.starts_with("ironclaw_")
}

/// Declared layer per workspace IronClaw package. Packages without the
/// metadata are omitted; `reborn_workspace_crates_declare_layers_and_follow_layer_matrix`
/// is the gate that refuses that, and duplicating its assertion here would give
/// two owners to one rule.
fn declared_layers(metadata: &Value) -> BTreeMap<String, String> {
    let mut layers = BTreeMap::new();
    for package in metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages")
    {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !is_ironclaw_package(name) {
            continue;
        }
        let Some(layer) = package
            .get("metadata")
            .and_then(|m| m.get("ironclaw"))
            .and_then(|i| i.get("layer"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        layers.insert(name.to_string(), layer.to_string());
    }
    layers
}

/// Deduplicated `(crate, dependency)` normal-dependency edges between layered
/// IronClaw packages — the same graph the layer matrix walks.
fn workspace_edges(
    metadata: &Value,
    layers: &BTreeMap<String, String>,
) -> BTreeSet<(String, String)> {
    let mut edges = BTreeSet::new();
    for package in metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages")
    {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !layers.contains_key(name) {
            continue;
        }
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            let Some(dependency_name) = dependency["name"].as_str() else {
                continue;
            };
            if !layers.contains_key(dependency_name) {
                continue;
            }
            let normal = dependency
                .get("kind")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind == "normal");
            if !normal {
                continue;
            }
            edges.insert((name.to_string(), dependency_name.to_string()));
        }
    }
    edges
}

fn same_layer_edges(
    edges: &BTreeSet<(String, String)>,
    layers: &BTreeMap<String, String>,
) -> BTreeSet<(String, String)> {
    edges
        .iter()
        .filter(|(from, to)| layers.get(from) == layers.get(to))
        .cloned()
        .collect()
}

fn ladder_index(layer: &str) -> Option<usize> {
    LAYER_LADDER.iter().position(|rung| *rung == layer)
}

/// The first missing tracking field, or `None` when the entry is fully
/// tracked. Placeholders are missing — "TBD" is not a workstream.
fn edge_tracking_defect(edge: &SameLayerEdge) -> Option<&'static str> {
    const PLACEHOLDERS: &[&str] = &["tbd", "todo", "unknown", "n/a", "na", "none", "?", "-"];
    let untracked = |value: &str| {
        let trimmed = value.trim();
        trimmed.is_empty() || PLACEHOLDERS.contains(&trimmed.to_ascii_lowercase().as_str())
    };
    [
        ("crate_name", edge.crate_name),
        ("dependency_name", edge.dependency_name),
        ("layer", edge.layer),
        ("owner", edge.owner),
        ("decided_in", edge.decided_in),
    ]
    .into_iter()
    .find_map(|(field, value)| untracked(value).then_some(field))
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

/// The default guard: every same-layer edge is inventoried, and every
/// inventoried edge is live.
#[test]
fn reborn_every_same_layer_edge_is_inventoried_and_no_entry_is_stale() {
    let metadata = cargo_metadata();
    let layers = declared_layers(&metadata);
    let edges = workspace_edges(&metadata, &layers);

    // Scanned-something guards. A metadata walk that resolves to nothing must
    // not read as an empty violation list.
    assert!(
        layers.len() >= MIN_LAYERED_CRATES,
        "only {} layered IronClaw crates resolved from cargo metadata (floor {MIN_LAYERED_CRATES}) \
         — this gate would be passing over a tree it never actually read. Repoint it rather than \
         letting it measure nothing.",
        layers.len()
    );
    assert!(
        edges.len() >= MIN_WORKSPACE_EDGES,
        "only {} workspace dependency edges resolved (floor {MIN_WORKSPACE_EDGES}) — the walk is \
         broken; a same-layer inventory over an empty graph is vacuously complete.",
        edges.len()
    );

    let unknown_layers: Vec<String> = layers
        .iter()
        .filter(|(_, layer)| layer.as_str() != "legacy" && ladder_index(layer).is_none())
        .map(|(name, layer)| format!("    {name} declares `{layer}`"))
        .collect();
    assert!(
        unknown_layers.is_empty(),
        "crates declare layers this gate's ladder does not know, so their edges would be \
         classified by a name it cannot order. Add the rung to LAYER_LADDER (and to \
         IRONCLAW_CRATE_LAYERS, its source) in the same PR:\n{}",
        unknown_layers.join("\n")
    );

    let live = same_layer_edges(&edges, &layers);
    assert!(
        !live.is_empty(),
        "no same-layer edges resolved at all. That would be the target state, but reaching it \
         retires this gate and its baseline together — an empty result is far likelier to be a \
         broken walk, so it fails rather than passes."
    );

    let inventoried: BTreeSet<(String, String)> = SAME_LAYER_EDGE_INVENTORY
        .iter()
        .map(|edge| {
            (
                edge.crate_name.to_string(),
                edge.dependency_name.to_string(),
            )
        })
        .collect();
    assert_eq!(
        inventoried.len(),
        SAME_LAYER_EDGE_INVENTORY.len(),
        "SAME_LAYER_EDGE_INVENTORY holds duplicate (crate, dependency) rows; the count ratchet \
         would then be measuring rows rather than edges"
    );

    let uninventoried: Vec<String> = live
        .difference(&inventoried)
        .map(|(from, to)| {
            format!(
                "    SameLayerEdge {{ crate_name: \"{from}\", dependency_name: \"{to}\", \
                 layer: \"{}\", owner: \"…\", decided_in: \"…\" }},",
                layers.get(from).map(String::as_str).unwrap_or("?")
            )
        })
        .collect();
    assert!(
        uninventoried.is_empty(),
        "NEW SAME-LAYER DEPENDENCY EDGE(S). The layer matrix cannot object to these — \
         `layer_allows_dependency` is reflexive, so a crate may depend on any peer in its own \
         layer with no exception entry and no gate firing (#7149). Prefer removing the coupling; \
         if the edge is intended, inventory it here AND raise SAME_LAYER_EDGE_BASELINE in the \
         same PR, both of which are reviewed decisions:\n{}",
        uninventoried.join("\n")
    );

    let stale: Vec<String> = inventoried
        .difference(&live)
        .map(|(from, to)| format!("    {from} -> {to}"))
        .collect();
    assert!(
        stale.is_empty(),
        "SAME_LAYER_EDGE_INVENTORY names edges that no longer exist. Delete the rows and lower \
         SAME_LAYER_EDGE_BASELINE in the same PR so the improvement is banked as a floor rather \
         than left as headroom for the next edge:\n{}",
        stale.join("\n")
    );

    // Each entry's `layer` field describes the real world.
    let mislabelled: Vec<String> = SAME_LAYER_EDGE_INVENTORY
        .iter()
        .filter_map(|edge| {
            let actual = layers.get(edge.crate_name)?;
            (actual != edge.layer).then(|| {
                format!(
                    "    {} -> {}: recorded layer `{}`, actual `{actual}`",
                    edge.crate_name, edge.dependency_name, edge.layer
                )
            })
        })
        .collect();
    assert!(
        mislabelled.is_empty(),
        "inventory rows record a layer the workspace no longer declares:\n{}",
        mislabelled.join("\n")
    );
}

/// Shrink-only in both directions, for the reason #7147 records: a ceiling
/// above the live list is an unclaimed budget for exactly the growth the
/// ceiling refuses.
#[test]
fn reborn_same_layer_edge_inventory_ratchets_down_only() {
    assert!(
        !SAME_LAYER_EDGE_INVENTORY.is_empty(),
        "SAME_LAYER_EDGE_INVENTORY is empty — the ratchet would pass having measured nothing"
    );
    assert!(
        SAME_LAYER_EDGE_INVENTORY.len() <= SAME_LAYER_EDGE_BASELINE,
        "same-layer edge inventory grew to {} (baseline {}): same-layer coupling is shrink-only. \
         Remove the edge rather than recording one more; if the owner has approved it, raise \
         SAME_LAYER_EDGE_BASELINE in the same PR with the rationale in the PR body.",
        SAME_LAYER_EDGE_INVENTORY.len(),
        SAME_LAYER_EDGE_BASELINE
    );
    assert!(
        SAME_LAYER_EDGE_INVENTORY.len() >= SAME_LAYER_EDGE_BASELINE,
        "same-layer edge inventory holds {} entries but SAME_LAYER_EDGE_BASELINE is {} — {} \
         entries of UNTRACKED SLACK. That headroom is a free budget for new coupling: that many \
         same-layer edges can be added with every gate green. Lower the baseline to {} in the PR \
         that deleted the edges (#7147's lesson, applied here from the start).",
        SAME_LAYER_EDGE_INVENTORY.len(),
        SAME_LAYER_EDGE_BASELINE,
        SAME_LAYER_EDGE_BASELINE.saturating_sub(SAME_LAYER_EDGE_INVENTORY.len()),
        SAME_LAYER_EDGE_INVENTORY.len()
    );
}

/// Every entry is attributable — §11.2.2's discipline, applied to this list.
#[test]
fn reborn_every_same_layer_edge_entry_is_tracked() {
    let untracked: Vec<String> = SAME_LAYER_EDGE_INVENTORY
        .iter()
        .filter_map(|edge| {
            edge_tracking_defect(edge).map(|field| {
                format!(
                    "    {} -> {}: missing `{field}`",
                    edge.crate_name, edge.dependency_name
                )
            })
        })
        .collect();
    assert!(
        untracked.is_empty(),
        "same-layer edge inventory entries without tracking metadata (every entry needs an \
         owner and the workstream that decides its disposition, so it can be retired rather \
         than accumulating):\n{}",
        untracked.join("\n")
    );
}

/// A crate re-layered **downward** must land with a consumer-side pin, and that
/// pin is enforced on every commit rather than being a note in a PR body.
#[test]
fn reborn_downward_re_layer_lands_with_its_consumer_side_pin() {
    let metadata = cargo_metadata();
    let layers = declared_layers(&metadata);
    let edges = workspace_edges(&metadata, &layers);

    assert!(
        layers.len() >= MIN_LAYERED_CRATES,
        "only {} layered crates resolved (floor {MIN_LAYERED_CRATES}) — refusing to evaluate \
         demotions against a tree this gate did not read",
        layers.len()
    );

    let origins: BTreeMap<&str, &str> = CRATE_LAYER_ORIGINS.iter().copied().collect();
    assert_eq!(
        origins.len(),
        CRATE_LAYER_ORIGINS.len(),
        "CRATE_LAYER_ORIGINS holds duplicate crate rows"
    );

    let unrecorded: Vec<&String> = layers
        .keys()
        .filter(|name| !origins.contains_key(name.as_str()))
        .collect();
    assert!(
        unrecorded.is_empty(),
        "crates with no CRATE_LAYER_ORIGINS row: {unrecorded:?}. Record the layer the crate is \
         introduced at — without it a later demotion is undetectable, which is precisely the \
         hole this gate closes."
    );
    let vanished: Vec<&str> = CRATE_LAYER_ORIGINS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !layers.contains_key(*name))
        .collect();
    assert!(
        vanished.is_empty(),
        "CRATE_LAYER_ORIGINS names crates the workspace no longer has: {vanished:?}. Delete the \
         rows — a stale origin row is a demotion detector aimed at nothing."
    );

    let bad_origin: Vec<String> = CRATE_LAYER_ORIGINS
        .iter()
        .filter(|(_, layer)| ladder_index(layer).is_none())
        .map(|(name, layer)| format!("    {name} records origin `{layer}`"))
        .collect();
    assert!(
        bad_origin.is_empty(),
        "CRATE_LAYER_ORIGINS rows naming a layer outside LAYER_LADDER — their demotion check \
         would silently never fire:\n{}",
        bad_origin.join("\n")
    );

    let pinned: BTreeMap<&str, &DowngradePin> = DOWNGRADE_PINS
        .iter()
        .map(|pin| (pin.crate_name, pin))
        .collect();
    assert_eq!(
        pinned.len(),
        DOWNGRADE_PINS.len(),
        "DOWNGRADE_PINS holds duplicate crate rows"
    );

    // 1. Every demotion has a pin.
    let mut demoted = Vec::new();
    let mut unpinned = Vec::new();
    for (name, live_layer) in &layers {
        if live_layer == "legacy" {
            continue;
        }
        let origin = origins[name.as_str()];
        let (Some(live_index), Some(origin_index)) =
            (ladder_index(live_layer), ladder_index(origin))
        else {
            continue;
        };
        if live_index >= origin_index {
            continue;
        }
        demoted.push(name.as_str());
        if !pinned.contains_key(name.as_str()) {
            unpinned.push(format!(
                "    {name}: {origin} -> {live_layer}. Every crate at {live_layer} and above may \
                 now reach it, and the layer matrix reports that widening as an improvement. Add \
                 a DowngradePin freezing its permitted consumers in the SAME PR as the move."
            ));
        }
    }
    assert!(
        unpinned.is_empty(),
        "DOWNWARD RE-LAYER WITHOUT A CONSUMER-SIDE PIN (#7149):\n{}",
        unpinned.join("\n")
    );

    // 2. Every pin describes a real, still-current demotion.
    let stale_pins: Vec<String> = DOWNGRADE_PINS
        .iter()
        .filter(|pin| !demoted.contains(&pin.crate_name))
        .map(|pin| {
            format!(
                "    {} ({} -> {}, {})",
                pin.crate_name, pin.from_layer, pin.to_layer, pin.demoted_in
            )
        })
        .collect();
    assert!(
        stale_pins.is_empty(),
        "DOWNGRADE_PINS rows for crates that are not (or no longer) demoted. A pin that describes \
         nothing constrains nothing — delete it, or fix the layers it claims:\n{}",
        stale_pins.join("\n")
    );

    // 3. Each pin's recorded from/to match the tree, so it cannot describe a
    //    move that did not happen.
    let misdescribed: Vec<String> = DOWNGRADE_PINS
        .iter()
        .filter_map(|pin| {
            let origin = origins.get(pin.crate_name)?;
            let live = layers.get(pin.crate_name)?;
            (*origin != pin.from_layer || live != pin.to_layer).then(|| {
                format!(
                    "    {}: pin says {} -> {}, tree says {origin} -> {live}",
                    pin.crate_name, pin.from_layer, pin.to_layer
                )
            })
        })
        .collect();
    assert!(
        misdescribed.is_empty(),
        "DOWNGRADE_PINS rows describing a move the tree does not show:\n{}",
        misdescribed.join("\n")
    );

    // 4. The pin bites: consumers are frozen exactly.
    let mut violations = Vec::new();
    for pin in DOWNGRADE_PINS {
        let permitted: BTreeSet<&str> = pin.permitted_consumers.iter().copied().collect();
        assert_eq!(
            permitted.len(),
            pin.permitted_consumers.len(),
            "{}'s permitted_consumers holds duplicates",
            pin.crate_name
        );
        assert!(
            !permitted.is_empty(),
            "{}'s permitted_consumers is empty — an empty allowlist reads as 'nothing may depend \
             on it', which is a stronger claim than a demotion pin should make silently. State \
             the real set.",
            pin.crate_name
        );
        let unknown: Vec<&&str> = permitted
            .iter()
            .filter(|name| !layers.contains_key(**name))
            .collect();
        assert!(
            unknown.is_empty(),
            "{}'s permitted_consumers names crates that are not layered workspace packages: \
             {unknown:?}. A row that resolves to no crate can never fire.",
            pin.crate_name
        );

        let actual: BTreeSet<&str> = edges
            .iter()
            .filter(|(_, to)| to == pin.crate_name)
            .map(|(from, _)| from.as_str())
            .collect();
        for extra in actual.difference(&permitted) {
            violations.push(format!(
                "    {extra} -> {}: reach taken after the {} -> {} demotion without review. The \
                 demotion legalised this edge by layer; the pin is what makes it a decision. If \
                 it is intended, add \"{extra}\" to permitted_consumers with the rationale.",
                pin.crate_name, pin.from_layer, pin.to_layer
            ));
        }
        for absent in permitted.difference(&actual) {
            violations.push(format!(
                "    {absent} no longer depends on {} — delete the stale permitted_consumers \
                 entry so the frozen set keeps shrinking.",
                pin.crate_name
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "DOWNGRADE_PINS consumer-set violations (#7149):\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Self-tests — the gate's own predicates, positive and negative.
// ---------------------------------------------------------------------------

#[test]
fn reborn_layer_ladder_orders_the_matrix_rungs_and_rejects_unknowns() {
    assert_eq!(ladder_index("contracts"), Some(0));
    assert!(
        ladder_index("contracts") < ladder_index("substrates"),
        "contracts must sort below substrates or every demotion check is inverted"
    );
    assert!(ladder_index("products") < ladder_index("app"));
    assert_eq!(
        ladder_index("legacy"),
        None,
        "legacy is not a rung — it may depend on anything, so `below` is undefined for it"
    );
    assert_eq!(ladder_index("not_a_layer"), None);

    // A demotion is strictly-lower, and a promotion must NOT read as one.
    let demotion = ladder_index("substrates") < ladder_index("loops");
    let promotion = ladder_index("loops") < ladder_index("substrates");
    let sideways = ladder_index("loops") < ladder_index("loops");
    assert!(demotion, "loops -> substrates must classify as downward");
    assert!(
        !promotion,
        "substrates -> loops must NOT classify as downward"
    );
    assert!(
        !sideways,
        "an unchanged layer must NOT classify as downward"
    );
}

#[test]
fn reborn_same_layer_edge_tracking_predicate_self_test() {
    let tracked = SameLayerEdge {
        crate_name: "ironclaw_example",
        dependency_name: "ironclaw_other",
        layer: "substrates",
        owner: "WS6 — Composition, app, and domain evictions",
        decided_in: "WS6",
    };
    assert_eq!(edge_tracking_defect(&tracked), None);

    assert_eq!(
        edge_tracking_defect(&SameLayerEdge {
            owner: "   ",
            ..tracked
        }),
        Some("owner"),
        "a blank owner must be reported as untracked"
    );
    assert_eq!(
        edge_tracking_defect(&SameLayerEdge {
            decided_in: "TBD",
            ..tracked
        }),
        Some("decided_in"),
        "a placeholder milestone must be reported as untracked"
    );
    assert_eq!(
        edge_tracking_defect(&SameLayerEdge {
            decided_in: "",
            ..tracked
        }),
        Some("decided_in"),
        "a blank milestone must be reported as untracked"
    );
    assert_eq!(
        edge_tracking_defect(&SameLayerEdge {
            layer: "n/a",
            ..tracked
        }),
        Some("layer"),
        "a placeholder layer must be reported as untracked"
    );
}
