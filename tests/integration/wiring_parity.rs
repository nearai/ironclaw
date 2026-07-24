//! W5-WIRING-PARITY (issue #5637): the harness's `DefaultPlannedRuntimeParts`
//! construction (`support/group.rs`'s `into_group`) stays field-Some/None-
//! identical to production's local-dev construction
//! (`ironclaw_reborn_composition::runtime::build_reborn_runtime`), modulo a
//! named allowlist of deliberate test-double substitutions — so a new
//! port/field lands loud instead of silently drifting. Zero production-crate
//! edits: the mechanism is entirely test-side.
//!
//! **Scope, by design**: this compares Some/None SHAPE only, never field
//! VALUES; only the local-dev/local-dev-yolo production profile (never
//! `Production`/`HostedSingleTenant*`); never `DefaultPlannedRuntimeConfig`'s
//! inner fields. Known accepted gap: it also cannot detect a silent
//! value-preserving rewiring of an *existing* field in production — only
//! added/removed fields and harness regressions on already-tracked fields.
//!
//! A companion, unrelated check lives in the same file: every
//! `harness/profiles/*.rs` domain's `capability_ids` — read from that
//! domain's REAL `ToolsProfile`/harness constructor, never a hand-copied
//! table — is a subset of production's real capability surface
//! (`builtin_first_party_package()` + the github/bundled-extension
//! manifest-derived id sets), modulo a skip-list of deliberately synthetic
//! local-dev-only ids. See `production_capability_surface()`'s doc for one
//! known, deliberately-excluded gap (the extension-lifecycle ids) that is
//! visibility-blocked rather than papered over.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::collections::HashSet;

use ironclaw_host_api::CapabilityId;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::harness::HarnessResult;
use reborn_support::harness::options::ToolsProfile;
use reborn_support::planned_runtime_parts_shape::DefaultPlannedRuntimePartsShape;

// ---------------------------------------------------------------------------
// Part 1: DefaultPlannedRuntimeParts Some/None shape parity
// ---------------------------------------------------------------------------

/// Hand-derived from `crates/ironclaw_reborn_composition/src/runtime.rs`
/// lines 3365-3459 (`build_reborn_runtime`, `LocalDev`/`LocalDevYolo`
/// profile, `local_runtime: Some(..)`). NOT computed from running code —
/// RE-DERIVE by re-reading that literal whenever it changes. (Verified
/// against this range 2026-07-04.) The smoke build below at least proves the
/// referenced literal still exists and the profile still builds.
///
/// **`model_budget_accountant` scope**: this constant (and the harness sides
/// it's compared against) models the NO-LLM local-dev build / the harness's
/// no-real-LLM scripted `TraceLlm` path — both leave `llm_cost_table` `None`,
/// so `model_budget_accountant` is `None` too. A production deployment with
/// a resolved LLM policy and a real model cost table is a different,
/// wider scope where that field is `Some` — see the field-level comment
/// below for the exact code path.
const EXPECTED_PRODUCTION_SHAPE: DefaultPlannedRuntimePartsShape =
    DefaultPlannedRuntimePartsShape {
        model_route_resolver: false, // :3406 hardcoded None
        cancellation_factory: false, // :3407 hardcoded None
        skill_context_source: true,  // :2917-2929 local_dev_filesystem_skill_context_source
        attachment_read_port: true,  // :3372-3376 local_runtime.map(ProjectScopedAttachmentReader)
        gate_record_store: true, // local_runtime.map(gate_record_store) — always Some when local_runtime present
        input_queue: false,      // hardcoded None
        model_policy_guard: false, // hardcoded None
        // :3027-3073 — scope: this constant models the NO-LLM local-dev
        // shape. When `model_gateway_override` is set (the harness's
        // scripted `TraceLlm` path, and any test build), `llm_cost_table` is
        // forced `None` unconditionally by the override arm (runtime.rs
        // ~2973-2977) regardless of profile, so the harness's scripted-LLM
        // shape matches this `false` too — it's genuinely like-for-like, not
        // an LLM-vs-no-LLM mismatch papered over. A REAL production
        // deployment with a resolved LLM policy (no override) and a known
        // model cost table sets `model_budget_accountant: Some(..)` instead
        // (the `(_, Some(cost_table)) => Some(accountant)` arm) — this
        // constant does NOT hold for that case; it only pins the
        // local-dev/harness scope this file compares.
        model_budget_accountant: false,
        safety_context: false,                  // hardcoded None
        hook_security_audit_sink: true, // runtime.rs:3780 always Some(TracingSecurityAuditSink)
        turn_event_sink: true,          // runtime.rs:3781 always Some(turn_event_sink)
        hook_dispatcher_builder_factory: false, // :3230-3258 None, HooksActivationConfig defaults OFF
        communication_context_provider: true,   // :3337-3357 Some whenever local_runtime present
        scheduler_wake_wiring: false, // :2847-2857 None outside Production/MigrationDryRun
    };

/// Deliberate test-double substitutions: `(field, reason)`. Every other field
/// must match `EXPECTED_PRODUCTION_SHAPE` exactly, or the parity assertion
/// fails naming the field. Re-derived by reading both `group.rs`'s
/// `into_group` (harness) and `runtime.rs`'s `build_reborn_runtime`
/// (production) literals in full.
const ALLOWED_DIVERGENCES: &[(&str, &str)] = &[
    (
        "skill_context_source",
        "harness: None outside skill_activation_tools() groups (recorder.rs:56-63); \
         production: always Some via local_dev_filesystem_skill_context_source (runtime.rs:2917-2929)",
    ),
    (
        "attachment_read_port",
        "harness: None outside attachment_tools() groups (recorder.rs:67-74); \
         production: always Some via local_runtime.map(ProjectScopedAttachmentReader) (runtime.rs:3372-3376)",
    ),
    (
        "gate_record_store",
        "harness: Some only for groups with approval parts (the only ones that raise a durable \
         auth gate to render credential_requirements from); production: always Some via \
         local_runtime.map(gate_record_store). A harness group without approval parts never raises \
         a resumable auth gate, so the None is behavior-neutral for the executor read.",
    ),
    (
        "turn_event_sink",
        "harness: None unless .with_turn_event_sink() was called (group.rs); \
         production: always Some, no config gate (runtime.rs:3453)",
    ),
    (
        "communication_context_provider",
        "harness: None unless .communication_context_provider() was called (group.rs); \
         production: always Some whenever local_runtime is present (runtime.rs:3337-3357)",
    ),
];

/// Overwrite `field` on `shape` with `from`'s value for that SAME field — an
/// exhaustive match on real struct-field assignments (not a string
/// comparison), so a field rename/removal makes the corresponding arm fail to
/// compile instead of letting a stale allowlist entry silently survive.
fn mask(
    mut shape: DefaultPlannedRuntimePartsShape,
    field: &str,
    from: DefaultPlannedRuntimePartsShape,
) -> DefaultPlannedRuntimePartsShape {
    match field {
        "model_route_resolver" => shape.model_route_resolver = from.model_route_resolver,
        "cancellation_factory" => shape.cancellation_factory = from.cancellation_factory,
        "skill_context_source" => shape.skill_context_source = from.skill_context_source,
        "attachment_read_port" => shape.attachment_read_port = from.attachment_read_port,
        "gate_record_store" => shape.gate_record_store = from.gate_record_store,
        "input_queue" => shape.input_queue = from.input_queue,
        "model_policy_guard" => shape.model_policy_guard = from.model_policy_guard,
        "model_budget_accountant" => shape.model_budget_accountant = from.model_budget_accountant,
        "safety_context" => shape.safety_context = from.safety_context,
        "hook_security_audit_sink" => {
            shape.hook_security_audit_sink = from.hook_security_audit_sink
        }
        "turn_event_sink" => shape.turn_event_sink = from.turn_event_sink,
        "hook_dispatcher_builder_factory" => {
            shape.hook_dispatcher_builder_factory = from.hook_dispatcher_builder_factory
        }
        "communication_context_provider" => {
            shape.communication_context_provider = from.communication_context_provider
        }
        "scheduler_wake_wiring" => shape.scheduler_wake_wiring = from.scheduler_wake_wiring,
        other => panic!(
            "ALLOWED_DIVERGENCES references unknown field {other:?} — update this match and \
             DefaultPlannedRuntimePartsShape together"
        ),
    }
    shape
}

/// Apply every `ALLOWED_DIVERGENCES` row to `harness_shape`, then assert the
/// masked result matches `EXPECTED_PRODUCTION_SHAPE` exactly. Any un-allowed
/// field that still differs fails `assert_eq!` with a real `Debug` diff
/// naming the field.
fn assert_planned_runtime_parts_shape_parity(
    harness_shape: DefaultPlannedRuntimePartsShape,
    context: &str,
) {
    let mut masked = harness_shape;
    for (field, _reason) in ALLOWED_DIVERGENCES {
        masked = mask(masked, field, EXPECTED_PRODUCTION_SHAPE);
    }
    assert_eq!(
        masked, EXPECTED_PRODUCTION_SHAPE,
        "{context}: DefaultPlannedRuntimeParts Some/None shape diverges from production's \
         local-dev build outside ALLOWED_DIVERGENCES — either wire the field to match \
         production, or add a named allowlist row with a reason"
    );
}

#[tokio::test]
async fn test_default_planned_runtime_parts_shape_matches_production() {
    let harness = RebornIntegrationHarness::test_default()
        .build()
        .await
        .expect("test_default() harness builds");
    assert_planned_runtime_parts_shape_parity(
        harness.planned_runtime_parts_shape(),
        "RebornIntegrationHarness::test_default()",
    );
}

#[tokio::test]
async fn builtin_tools_planned_runtime_parts_shape_matches_production() {
    let group = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("builtin_tools() group builds");
    assert_planned_runtime_parts_shape_parity(
        group.planned_runtime_parts_shape(),
        "RebornIntegrationGroup::builtin_tools()",
    );
}

/// Smoke build backing `EXPECTED_PRODUCTION_SHAPE`'s doc comment: proves the
/// referenced `LocalDev` literal still exists and the profile still builds.
/// Mirrors `crates/ironclaw_reborn_composition/tests/runtime.rs:132-146`. The
/// constant's `bool` values still come from the hand-read above, NOT from
/// introspecting this built `RebornRuntime` (there is no production-side
/// accessor to do so without a production-crate change — the known accepted
/// gap noted in the module doc).
#[tokio::test]
async fn local_dev_profile_still_builds() {
    let root = tempfile::tempdir().expect("tempdir");
    let policy = ironclaw_reborn_composition::local_dev_runtime_policy()
        .expect("local-dev runtime policy resolves");
    let input = ironclaw_reborn_composition::local_dev_build_input(
        "wiring-parity-smoke-owner",
        root.path().join("local-dev"),
    )
    .with_runtime_policy(policy);
    let runtime = ironclaw_reborn_composition::build_reborn_runtime(
        ironclaw_reborn_composition::RebornRuntimeInput::from_build_input(input),
    )
    .await
    .expect(
        "local-dev profile builds — EXPECTED_PRODUCTION_SHAPE's referenced literal still exists",
    );
    runtime.shutdown().await.expect("shutdown");
}

// ---------------------------------------------------------------------------
// Part 2: capability-id subset (companion check, unrelated to the shape
// mechanism above — shares this file only because both assert something
// about the harness's construction code never drifting from production).
// ---------------------------------------------------------------------------

/// Ids deliberately excluded from the subset check: local-dev-only synthetic
/// capabilities (never part of `builtin_first_party_package()` or the
/// extension/github id lists) or a fully dynamic runtime parameter with no
/// static id to check. `(domain, reason)` — same naming discipline as
/// `ALLOWED_DIVERGENCES`.
const SYNTHETIC_CAPABILITY_SKIP_LIST: &[(&str, &str)] = &[
    (
        "mock_mcp",
        "mock_mcp_tools(mcp_url, provider_id, capability_id) takes capability_id as a runtime \
         string parameter (harness/profiles/mock_mcp.rs:70) — no static id to check",
    ),
    (
        "project",
        "PROJECT_CREATE_CAPABILITY_ID (harness/profiles/project.rs) is a local-dev synthetic \
         capability (E-PROJ, ironclaw_reborn_composition::test_support), not part of \
         builtin_first_party_package()",
    ),
    (
        "outbound",
        "OUTBOUND_DELIVERY_TARGETS_LIST/TARGET_SET_CAPABILITY_ID (harness/profiles/outbound.rs) \
         are local-dev synthetic capabilities (C-SYNTH outbound, \
         ironclaw_reborn_composition::test_support), not part of builtin_first_party_package()",
    ),
    (
        "skill",
        "skill_activation_tools_profile()'s SKILL_ACTIVATE_CAPABILITY_ID \
         (harness/profiles/skill.rs) is a local-dev synthetic capability (E-SKILL, \
         ironclaw_reborn_composition::test_support), not part of builtin_first_party_package(); \
         skill_management_tools_profile()'s ids in the same file ARE checked below",
    ),
    (
        "extension",
        "extension_visibility_probe_tools_profile()'s visprobe.* ids \
         (harness/profiles/extension.rs) belong to a synthetic fixture package published \
         only via test-support publish, never part of any production manifest; \
         extension_lifecycle_tools_profile()'s ids in the same file ARE checked below",
    ),
];

/// The production capability surface: `builtin_first_party_package()`'s
/// declared capabilities, unioned with two independently production-derived
/// sources — the github extension's real manifest-derived ids
/// (`github_support::capability_ids()`) and every OTHER bundled extension's
/// real manifest-derived ids
/// (`extension_surface::bundled_extension_manifest_capability_ids()`) — both
/// parse the actual `manifest.toml` assets under
/// `crates/ironclaw_first_party_extensions/assets/`, so they are themselves
/// production truth, not a second test-only source.
///
/// **Deliberately NOT unioned**: `extension_surface::EXTENSION_LIFECYCLE_CAPABILITY_IDS`
/// (the three `builtin.extension_search`/`_install`/`_remove` ids).
/// Their real values are defined in a production crate
/// (`ironclaw_reborn_composition::extension_lifecycle_capabilities::EXTENSION_LIFECYCLE_CAPABILITY_IDS`),
/// but as a `pub(crate)` constant with no public accessor — visibility-blocked
/// from this test crate short of a `crates/` change, which is out of scope
/// here. Unioning the test-support copy of this list back in would recreate
/// exactly the tautology this restructure removes (RHS built from the same
/// hand-transcribed list LHS grants from). STOP/report, not paper over: see
/// `profile_capability_ids_by_domain()`'s "extension" row, which excludes
/// these 3 ids from the check it runs for the same reason, and the PR that
/// introduced this restructure (W5-WIRING-PARITY finding 1) for the tracked
/// follow-up (export the list, or add a public accessor, from
/// `ironclaw_reborn_composition`).
fn production_capability_surface() -> HashSet<String> {
    let mut surface: HashSet<String> = ironclaw_host_runtime::builtin_first_party_package()
        .expect("builtin first-party package parses")
        .manifest
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect();
    surface.extend(
        reborn_support::github::capability_ids()
            .expect("github extension manifest parses")
            .iter()
            .map(|id| id.as_str().to_string()),
    );
    surface.extend(
        reborn_support::extension_surface::bundled_extension_manifest_capability_ids()
            .expect("bundled extension manifests parse")
            .iter()
            .map(|id| id.as_str().to_string()),
    );
    surface
}

/// One row per `harness/profiles/*.rs` domain (16 files total — see the
/// module doc). Each `Vec<String>` is read from the domain's REAL
/// constructor — a `ToolsProfile`'s `capability_ids` field for the domains
/// that have one, or a small pure accessor added alongside the bespoke
/// harness-literal constructors that don't
/// (`core_builtin_tools_capability_ids()`, `qa_smoke_tools_capability_ids()`,
/// `web_access_tools_capability_ids()` — each shared with, not duplicated
/// from, the literal the harness builder itself uses) — never a
/// hand-transcribed second copy. A renamed/removed production capability
/// constant fails this file to compile (the profile constructors import the
/// same named constants) rather than silently drifting. Domains on
/// `SYNTHETIC_CAPABILITY_SKIP_LIST` contribute an empty (trivially passing)
/// list, documented via that table instead of here.
fn profile_capability_ids_by_domain() -> HarnessResult<Vec<(&'static str, Vec<String>)>> {
    use reborn_support::harness::profiles;

    fn ids_of(profile: ToolsProfile) -> Vec<String> {
        profile
            .capability_ids
            .into_iter()
            .map(CapabilityId::into_string)
            .collect()
    }

    Ok(vec![
        // attachment_tools_profile(): no first-party capability dispatch at all
        // (harness/profiles/attachment.rs) — trivially empty.
        (
            "attachment",
            ids_of(profiles::attachment::attachment_tools_profile()?),
        ),
        // coding_read_tools_profile() (harness/profiles/coding_read.rs).
        (
            "coding_read",
            ids_of(profiles::coding_read::coding_read_tools_profile()?),
        ),
        // core_builtin_tools_capability_ids() — the SAME accessor
        // core_builtin_tools_from_runtime() calls to build its harness
        // (harness/profiles/core_builtin.rs).
        (
            "core_builtin",
            profiles::core_builtin::core_builtin_tools_capability_ids()?
                .into_iter()
                .map(CapabilityId::into_string)
                .collect(),
        ),
        // extension_lifecycle_tools_profile() (harness/profiles/extension.rs)
        // grants EXTENSION_LIFECYCLE_CAPABILITY_IDS plus the real GitHub
        // package ids and the other manifest-derived bundled extension ids.
        // The lifecycle ids are filtered out BY NAME (order-independent):
        // their real production values are visibility-blocked (see
        // `production_capability_surface()`'s doc), so checking them against a
        // surface that correctly omits them would report unrelated drift.
        // Every remaining manifest-derived capability id is checked.
        ("extension", {
            let capability_ids =
                profiles::extension::extension_lifecycle_tools_profile()?.capability_ids;
            capability_ids
                .into_iter()
                .map(CapabilityId::into_string)
                .filter(|id| {
                    !reborn_support::extension_surface::EXTENSION_LIFECYCLE_CAPABILITY_IDS
                        .contains(&id.as_str())
                })
                .collect()
        }),
        // Union of all three file-domain constructors' ids
        // (harness/profiles/file.rs).
        (
            "file",
            [
                profiles::file::file_tools_profile()?.capability_ids,
                profiles::file::file_tools_requiring_approval_profile()?.capability_ids,
                profiles::file::write_only_profile()?.capability_ids,
            ]
            .into_iter()
            .flatten()
            .map(CapabilityId::into_string)
            .collect(),
        ),
        // file_and_github_auth_tools_profile() (harness/profiles/github.rs).
        // github_issue_tools()/github_issue_tools_auth_required() use
        // github_support::capability_ids() instead, which is itself
        // production-manifest-derived (see production_capability_surface())
        // and so isn't re-checked here.
        (
            "github",
            ids_of(profiles::github::file_and_github_auth_tools_profile()?),
        ),
        // mock_mcp: SYNTHETIC_CAPABILITY_SKIP_LIST.
        ("mock_mcp", vec![]),
        // outbound_target_tools_profile(): SYNTHETIC_CAPABILITY_SKIP_LIST.
        ("outbound", vec![]),
        // process_tools_profile() (harness/profiles/process.rs).
        (
            "process",
            ids_of(profiles::process::process_tools_profile()?),
        ),
        // profile_tools_profile() (harness/profiles/profile.rs).
        (
            "profile",
            ids_of(profiles::profile::profile_tools_profile()?),
        ),
        // project_tools_profile()/project_tools_with_fault_injection_profile():
        // SYNTHETIC_CAPABILITY_SKIP_LIST.
        ("project", vec![]),
        // qa_smoke_tools_capability_ids() — the SAME accessor qa_smoke_tools()
        // calls to build its harness (harness/profiles/qa_smoke.rs).
        (
            "qa_smoke",
            profiles::qa_smoke::qa_smoke_tools_capability_ids()?
                .into_iter()
                .map(CapabilityId::into_string)
                .collect(),
        ),
        // skill_management_tools_profile() only (harness/profiles/skill.rs);
        // skill_activation_tools_profile()'s id is on
        // SYNTHETIC_CAPABILITY_SKIP_LIST.
        (
            "skill",
            ids_of(profiles::skill::skill_management_tools_profile()?),
        ),
        // trace_commons_tools_profile() (harness/profiles/trace_commons.rs).
        (
            "trace_commons",
            ids_of(profiles::trace_commons::trace_commons_tools_profile()?),
        ),
        // trigger_management_tools_profile() (harness/profiles/trigger.rs).
        (
            "trigger",
            ids_of(profiles::trigger::trigger_management_tools_profile()?),
        ),
        // web_access_tools_capability_ids() — the SAME accessor
        // web_access_tools() calls to build its harness
        // (harness/profiles/web_access.rs).
        (
            "web_access",
            profiles::web_access::web_access_tools_capability_ids()?
                .into_iter()
                .map(CapabilityId::into_string)
                .collect(),
        ),
    ])
}

/// Plain `#[test]`, not `#[tokio::test]`: every `profile_capability_ids_by_domain()`
/// row is built from a pure, sync constructor (no I/O beyond reading the
/// github manifest asset off disk), so no async runtime is needed.
#[test]
fn harness_profile_capability_ids_are_a_production_subset() {
    let surface = production_capability_surface();
    let rows = profile_capability_ids_by_domain().expect("profile constructors build");
    assert_eq!(
        rows.len(),
        16,
        "expected exactly the 16 harness/profiles/*.rs domains named in the module doc"
    );
    for (domain, ids) in &rows {
        for id in ids {
            assert!(
                surface.contains(id),
                "harness/profiles/{domain}.rs capability id {id:?} is not in the production \
                 capability surface (builtin_first_party_package() + manifest-derived \
                 extension id lists) — either it's a real drift, or it belongs on \
                 SYNTHETIC_CAPABILITY_SKIP_LIST with a reason"
            );
        }
    }
    // Every skip-listed domain must still be accounted for above (as an
    // empty/partial row), so a skip-list entry can't silently stop being
    // checked for the ids it DOES declare (e.g. `skill`).
    let checked_domains: HashSet<&str> = rows.iter().map(|(domain, _)| *domain).collect();
    for (domain, _reason) in SYNTHETIC_CAPABILITY_SKIP_LIST {
        assert!(
            checked_domains.contains(domain),
            "SYNTHETIC_CAPABILITY_SKIP_LIST references unknown domain {domain:?} — it must also \
             appear (with its non-synthetic ids, if any) in profile_capability_ids_by_domain()"
        );
    }
}

/// Falsification for the restructured subset check (W5-WIRING-PARITY finding
/// 1): temporarily narrow a COPY of the production surface so it's MISSING
/// an id the "profile" domain genuinely grants (per its real
/// `profile_tools_profile()` constructor, asserted below, not assumed), then
/// confirm the check would fail on that row — proving the new shape (real
/// constructor LHS vs. manifest-derived RHS) still catches drift instead of
/// two hand-lists trivially agreeing with each other. This is a permanent
/// regression test, not a one-off manual check: a future accidental
/// reversion of `production_capability_surface()` back to a tautological
/// harness-support-list union would show up as this test failing (the
/// narrowed surface would then already be missing real ids for unrelated
/// reasons, or the removed id would resurface via a re-added hand list).
#[test]
fn harness_profile_capability_ids_subset_falsifies_on_narrowed_surface() {
    let mut surface = production_capability_surface();
    let removed = ironclaw_host_runtime::PROFILE_SET_CAPABILITY_ID;
    assert!(
        surface.remove(removed),
        "expected {removed:?} to be present in the real production surface before narrowing it"
    );
    let rows = profile_capability_ids_by_domain().expect("profile constructors build");
    let profile_domain_ids = rows
        .iter()
        .find(|(domain, _)| *domain == "profile")
        .map(|(_, ids)| ids.clone())
        .expect("\"profile\" row present");
    assert!(
        profile_domain_ids.iter().any(|id| id == removed),
        "\"profile\" row must actually grant {removed:?} for this falsification to be meaningful"
    );
    assert!(
        !surface.contains(removed),
        "narrowed surface must not contain {removed:?}"
    );
    // `PROFILE_SET_CAPABILITY_ID` is granted by more than one domain
    // (`profile`, and `core_builtin`/`qa_smoke`'s bespoke literals), so
    // narrowing the surface by this one id can legitimately fail more than
    // one row — assert `profile` is AMONG the domains the check catches,
    // rather than assuming it's the only or first one.
    let failing_domains: Vec<&str> = rows
        .iter()
        .filter(|(_, ids)| ids.iter().any(|id| !surface.contains(id)))
        .map(|(domain, _)| *domain)
        .collect();
    assert!(
        failing_domains.contains(&"profile"),
        "narrowing the production surface by {removed:?} (which the \"profile\" domain actually \
         grants) should have made the subset check fail on the \"profile\" row — it didn't \
         (failing domains: {failing_domains:?}), so the restructured check is not actually \
         verifying anything"
    );
}
