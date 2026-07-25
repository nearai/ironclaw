//! Origin→gate matrix architecture ratchet (§5.2.1/§5.2.7/§10 of the Reborn
//! authorization design; S5 of #6396).
//!
//! S2–S4 turned the per-origin gate matrix into declarative capability data
//! (`OriginGateMatrix`), populated it on every PRODUCTION descriptor, and folded
//! it into `authorize()` as a two-tier gate. This ratchet locks in the invariants
//! that keep that matrix honest. It changes NO production behavior — it only fails
//! a build that would silently weaken the origin→gate contract.
//!
//! ## What each assertion protects
//!
//! 1. **The Ungated-for-LoopRun allowlist is pinned to its reviewed seed.**
//!    `UNGATED_LOOP_RUN_CAPABILITIES` (`ironclaw_host_api`) is the exact set of
//!    builtins the model (`LoopRun` origin) may invoke with NO approval gate. It
//!    is the security-critical list: an entry here is a capability whose effects
//!    reviewers judged safe to run ungated. This test freezes the current 17 ids
//!    (checked-in `EXPECTED_UNGATED_SEED`) so any addition or removal is a
//!    reviewed diff — the same "frozen list only changes under review" property
//!    `reborn_capability_dto_collapse_ratchet` gives its DTO set. An addition
//!    ungates a capability for the model and MUST be a deliberate, reviewed change.
//!
//! 2. **No hand-authored extension TOML ungates a capability off-allowlist, and
//!    every declared capability carries a well-formed matrix.** The first-party
//!    extension manifests are the hand-authored source where a one-word typo
//!    (`loop_run = "ungated"` on a networked, credentialed capability) would
//!    silently remove its approval gate. This test parses every manifest asset and
//!    asserts, for every declared capability:
//!    - it carries an `origin_gate_matrix` with a `loop_run` policy — the
//!      TOML-source half of "every production descriptor declares `Some(..)`"
//!      (§5.2.1 invariant 1; the derived-descriptor half is enumerated elsewhere,
//!      see below);
//!    - that matrix never uses `consent_sufficient` on the `loop_run`/`automation`
//!      columns — `ConsentSufficient` is Product-only per §5.2.1 (the §11.7
//!      per-descriptor self-consistency check, applied to the TOML source);
//!    - any `ungated` `loop_run` has its id in the reviewed allowlist.
//!
//!    The scan covers both the first-party extension assets and the
//!    host-bundled always-on package assets (`ironclaw_host_runtime/assets`,
//!    the `ironclaw.memory` manifests). Today the only `ungated` TOML
//!    capabilities are the allowlisted read-only memory tools
//!    (`ironclaw.memory.read`/`search`/`tree`); every installable extension
//!    capability remains gated.
//!
//! ## Behavior-preserving grandfathered seed (deviates from §10 "starts empty")
//!
//! The design doc's §10 imagines the Ungated allowlist STARTING EMPTY and growing
//! only under review. S3 instead SEEDED it with the 17 builtins already ungated
//! under today's `AskDestructive` effect gate (their effects are a subset of
//! `{read_filesystem, dispatch_capability}` or are approval-exempt) — a deliberate
//! behavior-preservation choice so folding the matrix into `authorize()` (S4)
//! changed no user-visible gating. A future tightening slice REMOVES entries (each
//! a reviewed diff this ratchet forces), walking the seed toward the §10 empty
//! ideal. This test therefore pins the seed as-is: shrinking it is expected and
//! welcome (update `EXPECTED_UNGATED_SEED` in the same PR); silently growing it is
//! the security regression to catch.
//!
//! ## Where invariant 1's REAL (derived-descriptor) enumeration lives
//!
//! "Every PRODUCTION kernel descriptor declares `Some(origin_gate_matrix)`" is
//! enforced authoritatively where descriptors are actually built from the
//! manifest→descriptor path — not by a Rust text scan here. `ironclaw_architecture`
//! is a leaf test crate and must not take a production dependency on the
//! composition/host_runtime crates that assemble descriptors, so it guards the two
//! invariants a leaf crate CAN own soundly: the pinned allowlist (imported from
//! its owner) and the hand-authored TOML source scan. The derived-descriptor
//! enumeration tests, which fail on a `None` descriptor and pin its well-formed
//! matrix, are:
//!   - builtins package — `ironclaw_host_runtime` →
//!     `builtin_first_party_package_declares_expected_capabilities`;
//!   - extension-lifecycle caps — `ironclaw_reborn_composition` →
//!     `extension_lifecycle_capabilities_declare_behavior_neutral_origin_gate_matrix`;
//!   - bundled extensions — `ironclaw_reborn_composition` →
//!     `bundled_extension_capabilities_carry_behavior_neutral_origin_gate_matrix`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ironclaw_host_api::{
    CapabilityId, OriginGateMatrix, OriginGatePolicy, UNGATED_LOOP_RUN_CAPABILITIES,
};

/// The reviewed S5 seed of builtins the model may invoke UNGATED (§5.2.1/§10).
/// Any drift from this list is a reviewed diff — see the module header. Adding an
/// id ungates a capability for the model with no approval gate; removing one
/// tightens toward the §10 empty ideal.
const EXPECTED_UNGATED_SEED: &[&str] = &[
    "builtin.echo",
    "builtin.time",
    "builtin.json",
    "builtin.trace_commons.status",
    "builtin.trace_commons.credits",
    "builtin.trace_commons.onboard",
    "builtin.profile_set",
    // Reviewed rename, not an addition: the memory tools moved from the builtin
    // package (`builtin.memory_*`) to the always-on `ironclaw.memory` package
    // (#3537) with the same read-only effect posture; `ironclaw.memory.write`
    // stays off the list (gated, arbitrary-path write).
    "ironclaw.memory.search",
    "ironclaw.memory.read",
    "ironclaw.memory.tree",
    "builtin.read_file",
    "builtin.list_dir",
    "builtin.glob",
    "builtin.grep",
    "builtin.skill_list",
    "builtin.trigger_list",
    "builtin.extension_search",
];

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/ironclaw_architecture; up two to the workspace.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}

fn first_party_assets_dir() -> PathBuf {
    workspace_root().join("crates/ironclaw_first_party_extensions/assets")
}

/// Host-bundled always-on packages (the `ironclaw.memory` native/mem0
/// manifests) are hand-authored TOML exactly like the first-party extension
/// assets, so the same matrix scan applies. Missing coverage here let the
/// memory manifests ship with no `origin_gate_matrix`, which the S4 fold
/// fail-closes to Forbidden for every origin-stamped dispatch.
fn host_runtime_assets_dir() -> PathBuf {
    workspace_root().join("crates/ironclaw_host_runtime/assets")
}

fn collect_manifest_tomls(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_manifest_tomls(&path, out);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("manifest.toml") {
            out.push(path);
        }
    }
}

/// Recursively collect every capability table from a parsed manifest. Handles
/// v2 capability arrays, v3 top-level `[[tools]]`, and the v3 `[mcp]`
/// connection template that supplies policy to dynamically discovered tools.
fn collect_capability_tables<'a>(value: &'a toml::Value, out: &mut Vec<&'a toml::Table>) {
    match value {
        toml::Value::Table(table) => {
            for key in ["capabilities", "tools"] {
                if let Some(toml::Value::Array(items)) = table.get(key) {
                    for item in items {
                        if let toml::Value::Table(capability) = item {
                            out.push(capability);
                        }
                    }
                }
            }
            if let Some(toml::Value::Table(mcp)) = table.get("mcp") {
                out.push(mcp);
            }
            for nested in table.values() {
                collect_capability_tables(nested, out);
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                collect_capability_tables(item, out);
            }
        }
        _ => {}
    }
}

/// Invariant 1 (of the S5 ratchet): the Ungated allowlist is pinned to the
/// reviewed 17-id seed, deduplicated, and every id well-formed. See the header.
#[test]
fn ungated_loop_run_allowlist_is_pinned_to_reviewed_seed() {
    let actual: BTreeSet<&str> = UNGATED_LOOP_RUN_CAPABILITIES.iter().copied().collect();
    assert_eq!(
        actual.len(),
        UNGATED_LOOP_RUN_CAPABILITIES.len(),
        "duplicate id in UNGATED_LOOP_RUN_CAPABILITIES"
    );
    let expected: BTreeSet<&str> = EXPECTED_UNGATED_SEED.iter().copied().collect();
    assert_eq!(
        expected.len(),
        EXPECTED_UNGATED_SEED.len(),
        "duplicate id in EXPECTED_UNGATED_SEED (fix the test's frozen list)"
    );

    let added: Vec<&&str> = actual.difference(&expected).collect();
    let removed: Vec<&&str> = expected.difference(&actual).collect();
    assert!(
        added.is_empty() && removed.is_empty(),
        "UNGATED_LOOP_RUN_CAPABILITIES drifted from the reviewed S5 seed. An ADDITION \
         ungates a capability for the model (LoopRun) with NO approval gate — a security \
         change requiring explicit review. A REMOVAL is welcome (tightening toward the §10 \
         empty ideal) but must update EXPECTED_UNGATED_SEED in the SAME PR so the change is \
         reviewed. added={added:?} removed={removed:?}"
    );

    for id in UNGATED_LOOP_RUN_CAPABILITIES {
        CapabilityId::new(*id)
            .unwrap_or_else(|_| panic!("allowlist id {id} must be a well-formed capability id"));
    }
    assert_eq!(
        EXPECTED_UNGATED_SEED.len(),
        17,
        "the reviewed S5 seed is 17 ids; a size change is a reviewed diff"
    );
}

/// Invariant 2/3 (of the S5 ratchet): every hand-authored extension-manifest
/// capability declares a well-formed `origin_gate_matrix` — present, no
/// Product-only `consent_sufficient` on the loop_run/automation columns, and no
/// off-allowlist `ungated` loop_run. See the header.
#[test]
fn extension_toml_capabilities_declare_wellformed_origin_gate_matrix() {
    let mut manifests = Vec::new();
    collect_manifest_tomls(&first_party_assets_dir(), &mut manifests);
    assert!(
        manifests.len() >= 12,
        "expected at least the 12 first-party extension manifest assets, found {} — did the \
         assets directory move? ({})",
        manifests.len(),
        first_party_assets_dir().display()
    );
    collect_manifest_tomls(&host_runtime_assets_dir(), &mut manifests);
    for required in ["memory_native/manifest.toml", "memory_mem0/manifest.toml"] {
        assert!(
            manifests
                .iter()
                .any(|path| path.ends_with(Path::new(required))),
            "expected the host-bundled {required} under {} — a missing or moved memory manifest \
             would silently drop it from this origin-gate scan",
            host_runtime_assets_dir().display()
        );
    }

    let allowlist: BTreeSet<&str> = UNGATED_LOOP_RUN_CAPABILITIES.iter().copied().collect();
    let mut violations: Vec<String> = Vec::new();
    let mut checked_capabilities = 0usize;

    for path in &manifests {
        let raw = fs::read_to_string(path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let value: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
        let mut capabilities = Vec::new();
        collect_capability_tables(&value, &mut capabilities);

        for capability in capabilities {
            checked_capabilities += 1;
            let id = capability
                .get("id")
                .and_then(toml::Value::as_str)
                .unwrap_or("<capability with no id>");

            let Some(matrix_value) = capability.get("origin_gate_matrix") else {
                violations.push(format!(
                    "{} :: {id}: capability declares no origin_gate_matrix (§5.2.1 invariant 1)",
                    path.display()
                ));
                continue;
            };

            let matrix: OriginGateMatrix = match matrix_value.clone().try_into() {
                Ok(matrix) => matrix,
                Err(err) => {
                    violations.push(format!(
                        "{} :: {id}: invalid origin_gate_matrix structure or policy: {err}",
                        path.display()
                    ));
                    continue;
                }
            };

            if matrix.loop_run == OriginGatePolicy::ConsentSufficient {
                violations.push(format!(
                    "{id}: consent_sufficient is Product-only (§5.2.1) — not valid on the \
                     loop_run column"
                ));
            }
            if matrix.automation == OriginGatePolicy::ConsentSufficient {
                violations.push(format!(
                    "{id}: consent_sufficient is Product-only (§5.2.1) — not valid on the \
                     automation column"
                ));
            }

            if matrix.loop_run == OriginGatePolicy::Ungated && !allowlist.contains(id) {
                violations.push(format!(
                    "{id}: loop_run = \"ungated\" but id is NOT in the reviewed \
                     UNGATED_LOOP_RUN_CAPABILITIES allowlist — a hand-authored typo would \
                     silently remove this capability's approval gate for the model"
                ));
            }
        }
    }

    assert!(
        checked_capabilities > 0,
        "found no capability tables in the manifest assets — the TOML walker is broken"
    );
    assert!(
        violations.is_empty(),
        "hand-authored extension-manifest origin_gate_matrix violations (§5.2.1):\n{}",
        violations.join("\n")
    );
}

/// Self-test for the TOML walker across both v2 forms and both v3 policy
/// declaration forms.
#[test]
fn toml_walker_finds_all_manifest_capability_shapes() {
    let sample = r#"
schema_version = "reborn.extension_manifest.v2"
id = "sample"

[[capabilities]]
id = "sample.top_level"
origin_gate_matrix = { loop_run = "ungated", product = "forbidden", automation = "forbidden" }

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "sample.nested"

[[tools]]
id = "sample.v3_tool"

[mcp]
server = "https://mcp.example.com"
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
"#;
    let value: toml::Value = toml::from_str(sample).expect("sample manifest parses");
    let mut capabilities = Vec::new();
    collect_capability_tables(&value, &mut capabilities);

    let ids: BTreeSet<&str> = capabilities
        .iter()
        .filter_map(|cap| cap.get("id").and_then(toml::Value::as_str))
        .collect();
    assert!(
        ids.contains("sample.top_level")
            && ids.contains("sample.nested")
            && ids.contains("sample.v3_tool"),
        "walker must reach v2 and v3 capability-array schemas, found: {ids:?}"
    );
    assert!(
        capabilities.iter().any(|cap| cap.get("server").is_some()),
        "walker must reach the v3 MCP connection template"
    );

    // And the off-allowlist `ungated` in the fixture is visible to the check.
    let top_level = capabilities
        .iter()
        .find(|cap| cap.get("id").and_then(toml::Value::as_str) == Some("sample.top_level"))
        .expect("top-level capability present");
    let matrix = match top_level.get("origin_gate_matrix") {
        Some(toml::Value::Table(matrix)) => matrix,
        _ => panic!("fixture capability must carry an origin_gate_matrix table"),
    };
    assert_eq!(
        matrix.get("loop_run").and_then(toml::Value::as_str),
        Some("ungated"),
        "the walker must surface the raw loop_run policy for the allowlist check"
    );
}
