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
//! `harness/profiles/*.rs` domain's `capability_ids` is a subset of
//! production's real capability surface (`builtin_first_party_package()` +
//! the extension/github id lists), modulo a skip-list of deliberately
//! synthetic local-dev-only ids.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::collections::HashSet;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
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
const EXPECTED_PRODUCTION_SHAPE: DefaultPlannedRuntimePartsShape =
    DefaultPlannedRuntimePartsShape {
        model_route_resolver: false,            // :3406 hardcoded None
        cancellation_factory: false,            // :3407 hardcoded None
        skill_context_source: true, // :2917-2929 local_dev_filesystem_skill_context_source
        attachment_read_port: true, // :3372-3376 local_runtime.map(ProjectScopedAttachmentReader)
        input_queue: false,         // hardcoded None
        model_policy_guard: false,  // hardcoded None
        model_budget_accountant: false, // :3027-3073 None absent a resolved LLM cost table
        safety_context: false,      // hardcoded None
        hook_security_audit_sink: true, // :3452 always Some(TracingSecurityAuditSink)
        turn_event_sink: true,      // :3453 always Some(turn_event_sink)
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
        "turn_event_sink",
        "harness: None unless .with_turn_event_sink() was called (group.rs); \
         production: always Some, no config gate (runtime.rs:3453)",
    ),
    (
        "communication_context_provider",
        "harness: None unless .communication_context_provider() was called (group.rs); \
         production: always Some whenever local_runtime is present (runtime.rs:3337-3357)",
    ),
    (
        "hook_security_audit_sink",
        "harness: always None (group.rs:704) — no RecordingSecurityAuditSink double exists \
         yet (standing gap, tracked by nearai/ironclaw#5640, not a substitution); \
         production: always Some(TracingSecurityAuditSink) (runtime.rs:3452)",
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
    let input = ironclaw_reborn_composition::RebornBuildInput::local_dev(
        "wiring-parity-smoke-owner",
        root.path().join("local-dev"),
    )
    .with_runtime_policy(policy);
    let runtime = ironclaw_reborn_composition::build_reborn_runtime(
        ironclaw_reborn_composition::RebornRuntimeInput::from_services(input),
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
];

/// The production capability surface: `builtin_first_party_package()`'s
/// declared capabilities, unioned with the extension-lifecycle/bundled-
/// extension id lists and the github extension's real manifest-derived ids
/// (`github_support::capability_ids()` — parses the actual production asset
/// at `crates/ironclaw_first_party_extensions/assets/github/manifest.toml`,
/// so it is itself production truth, not a second test-only source).
fn production_capability_surface() -> HashSet<String> {
    let mut surface: HashSet<String> = ironclaw_host_runtime::builtin_first_party_package()
        .expect("builtin first-party package parses")
        .manifest
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect();
    surface.extend(
        reborn_support::extension_surface::EXTENSION_LIFECYCLE_CAPABILITY_IDS
            .iter()
            .map(|id| id.to_string()),
    );
    surface.extend(
        reborn_support::extension_surface::BUNDLED_EXTENSION_CAPABILITY_IDS
            .iter()
            .map(|id| id.to_string()),
    );
    surface.extend(
        reborn_support::github::capability_ids()
            .expect("github extension manifest parses")
            .iter()
            .map(|id| id.as_str().to_string()),
    );
    surface
}

/// One row per `harness/profiles/*.rs` domain (16 files total — see the
/// module doc). The `Vec<&str>` is hand-transcribed from each file's
/// `capability_ids` literal(s), built from the SAME named production
/// constants those files import (not re-typed string literals), so a renamed
/// constant fails this file to compile rather than silently drifting. Files
/// on `SYNTHETIC_CAPABILITY_SKIP_LIST` contribute an empty (trivially
/// passing) list, documented via that table instead of here.
fn profile_capability_ids_by_domain() -> Vec<(&'static str, Vec<&'static str>)> {
    use ironclaw_first_party_extensions::{
        WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID,
    };
    use ironclaw_host_runtime::{
        APPLY_PATCH_CAPABILITY_ID, ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID,
        HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID,
        MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
        MEMORY_WRITE_CAPABILITY_ID, PROFILE_SET_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
        SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID,
        SKILL_REMOVE_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID, TIME_CAPABILITY_ID,
        TRACE_COMMONS_CREDITS_CAPABILITY_ID, TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID, TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID,
        TRACE_COMMONS_STATUS_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID,
        TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID,
        TRIGGER_RESUME_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
    };

    vec![
        // attachment_tools_profile(): no first-party capability dispatch at all
        // (harness/profiles/attachment.rs) — trivially empty.
        ("attachment", vec![]),
        // coding_read_tools_profile() (harness/profiles/coding_read.rs:15-18).
        (
            "coding_read",
            vec![
                LIST_DIR_CAPABILITY_ID,
                GLOB_CAPABILITY_ID,
                GREP_CAPABILITY_ID,
            ],
        ),
        // core_builtin_tools_from_runtime() (harness/profiles/core_builtin.rs:177-193).
        (
            "core_builtin",
            vec![
                TIME_CAPABILITY_ID,
                JSON_CAPABILITY_ID,
                HTTP_CAPABILITY_ID,
                HTTP_SAVE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                PROFILE_SET_CAPABILITY_ID,
                READ_FILE_CAPABILITY_ID,
                APPLY_PATCH_CAPABILITY_ID,
                SHELL_CAPABILITY_ID,
            ],
        ),
        // extension_lifecycle_tools_profile() (harness/profiles/extension.rs:21-22)
        // unions exactly these two production id lists — trivially a subset of
        // the surface (which unions the SAME two lists); listed for the 16/16
        // count, not because it exercises anything new.
        (
            "extension",
            [
                reborn_support::extension_surface::EXTENSION_LIFECYCLE_CAPABILITY_IDS,
                reborn_support::extension_surface::BUNDLED_EXTENSION_CAPABILITY_IDS,
            ]
            .concat(),
        ),
        // file_tools_profile()/file_tools_requiring_approval_profile()/write_only_profile()
        // (harness/profiles/file.rs) — union of all three constructors' ids.
        (
            "file",
            vec![WRITE_FILE_CAPABILITY_ID, READ_FILE_CAPABILITY_ID],
        ),
        // file_and_github_auth_tools_profile() (harness/profiles/github.rs:62-66).
        // github_issue_tools()/github_issue_tools_auth_required() use
        // github_support::capability_ids() instead, which is itself
        // production-manifest-derived (see production_capability_surface()) and
        // so isn't re-checked here.
        (
            "github",
            vec![
                WRITE_FILE_CAPABILITY_ID,
                READ_FILE_CAPABILITY_ID,
                "github.get_repo",
            ],
        ),
        // mock_mcp: SYNTHETIC_CAPABILITY_SKIP_LIST.
        ("mock_mcp", vec![]),
        // outbound_target_tools_profile(): SYNTHETIC_CAPABILITY_SKIP_LIST.
        ("outbound", vec![]),
        // process_tools_profile() (harness/profiles/process.rs:14-17).
        (
            "process",
            vec![
                ECHO_CAPABILITY_ID,
                SHELL_CAPABILITY_ID,
                SPAWN_SUBAGENT_CAPABILITY_ID,
            ],
        ),
        // profile_tools_profile() (harness/profiles/profile.rs:18).
        ("profile", vec![PROFILE_SET_CAPABILITY_ID]),
        // project_tools_profile()/project_tools_with_fault_injection_profile():
        // SYNTHETIC_CAPABILITY_SKIP_LIST.
        ("project", vec![]),
        // qa_smoke_tools() (harness/profiles/qa_smoke.rs:66-91).
        (
            "qa_smoke",
            vec![
                ECHO_CAPABILITY_ID,
                TIME_CAPABILITY_ID,
                JSON_CAPABILITY_ID,
                HTTP_CAPABILITY_ID,
                HTTP_SAVE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                READ_FILE_CAPABILITY_ID,
                WRITE_FILE_CAPABILITY_ID,
                LIST_DIR_CAPABILITY_ID,
                GLOB_CAPABILITY_ID,
                GREP_CAPABILITY_ID,
                APPLY_PATCH_CAPABILITY_ID,
                SHELL_CAPABILITY_ID,
                SPAWN_SUBAGENT_CAPABILITY_ID,
                SKILL_LIST_CAPABILITY_ID,
                SKILL_INSTALL_CAPABILITY_ID,
                SKILL_REMOVE_CAPABILITY_ID,
                TRIGGER_CREATE_CAPABILITY_ID,
                TRIGGER_LIST_CAPABILITY_ID,
                TRIGGER_PAUSE_CAPABILITY_ID,
                TRIGGER_RESUME_CAPABILITY_ID,
                TRIGGER_REMOVE_CAPABILITY_ID,
            ],
        ),
        // skill_management_tools_profile() only (harness/profiles/skill.rs:17-20);
        // skill_activation_tools_profile()'s id is on
        // SYNTHETIC_CAPABILITY_SKIP_LIST.
        (
            "skill",
            vec![
                SKILL_LIST_CAPABILITY_ID,
                SKILL_INSTALL_CAPABILITY_ID,
                SKILL_REMOVE_CAPABILITY_ID,
            ],
        ),
        // trace_commons_tools_profile() (harness/profiles/trace_commons.rs:15-20).
        (
            "trace_commons",
            vec![
                TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
                TRACE_COMMONS_STATUS_CAPABILITY_ID,
                TRACE_COMMONS_CREDITS_CAPABILITY_ID,
                TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID,
                TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
            ],
        ),
        // trigger_management_tools_profile() (harness/profiles/trigger.rs:14-19).
        (
            "trigger",
            vec![
                TRIGGER_CREATE_CAPABILITY_ID,
                TRIGGER_LIST_CAPABILITY_ID,
                TRIGGER_PAUSE_CAPABILITY_ID,
                TRIGGER_RESUME_CAPABILITY_ID,
                TRIGGER_REMOVE_CAPABILITY_ID,
            ],
        ),
        // web_access_tools() (harness/profiles/web_access.rs:47-50).
        (
            "web_access",
            vec![WEB_SEARCH_CAPABILITY_ID, WEB_GET_CONTENT_CAPABILITY_ID],
        ),
    ]
}

/// Plain `#[test]`, not `#[tokio::test]`: pure data (production constants +
/// hand-transcribed literals), no runtime built.
#[test]
fn harness_profile_capability_ids_are_a_production_subset() {
    let surface = production_capability_surface();
    let rows = profile_capability_ids_by_domain();
    assert_eq!(
        rows.len(),
        16,
        "expected exactly the 16 harness/profiles/*.rs domains named in the module doc"
    );
    for (domain, ids) in &rows {
        for id in ids {
            assert!(
                surface.contains(*id),
                "harness/profiles/{domain}.rs capability id {id:?} is not in the production \
                 capability surface (builtin_first_party_package() + extension/github id \
                 lists) — either it's a real drift, or it belongs on \
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
