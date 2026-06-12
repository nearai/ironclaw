//! Composition-seam contract: the canonical production composition must
//! populate every security-relevant `Option<Arc<dyn ...>>` seam.
//!
//! # The defect class this kills
//!
//! A capability is wired as an `Option<Arc<dyn Trait>>` field with a `with_*`
//! setter, but the canonical production composition path
//! (`builtin_obligation_handler` / `build_default_planned_runtime` /
//! `build_reborn_runtime`) forgets to call the setter — so the security feature
//! silently no-ops in production while every unit test (which constructs the
//! handler/host directly and calls the setter) keeps passing. Only a reviewer
//! reading the composition function by hand catches it.
//!
//! This recurred three times in one development cycle (#3919, #3922,
//! #3938/#3573). This test converts "a reviewer must notice the missing setter"
//! into a failing build.
//!
//! # Why source-text, not runtime construction
//!
//! `ironclaw_architecture` is deliberately dependency-light: its only dev-dep is
//! `serde_json`, and its sibling tests (`reborn_composition_boundaries.rs`,
//! `reborn_dependency_boundaries.rs`) assert structural invariants over crate
//! source text and `cargo metadata` rather than constructing runtimes. That is
//! intentional — the crate is a fast structural-invariant guard that must not
//! pull the entire composition graph (tokio, libsql, tempfile, the substrate
//! crates) into its dependency closure.
//!
//! The security seams this test guards are *private internal fields* of the
//! constructed runtime (`BuiltinObligationHandler::security_audit_sink`, the
//! hook seams on `RebornLoopDriverHostFactory`). They are consumed internally
//! and never retained on the public `RebornRuntime` surface, so a black-box
//! "construct the runtime, then read the `Option`" assertion is impossible
//! without adding inspection accessors that would leak composition internals
//! and fight the documented facade shape of `ironclaw_reborn_composition`.
//!
//! So we assert the next-best, still-load-bearing invariant: **the `with_*`
//! setter for each security seam must literally appear in the production
//! composition function that builds that component.** This catches the exact
//! recurring failure mode (setter exists, production forgot to call it). The
//! known limitation is that a rename of the setter or the production builder
//! could evade the textual match; that is an acceptable tradeoff for keeping
//! the guard in the dependency-free architecture crate, and is documented on
//! each assertion.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates")
        .to_path_buf()
}

fn read_crate_source(relative: &str) -> String {
    let path = workspace_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("composition source {relative} readable: {error}"))
}

/// Extract the body of a `fn <name>` from `source`, returning the text from the
/// signature up to the matching closing brace. Used to scope assertions to the
/// production composition function rather than the whole file (so a setter call
/// that only appears in a `#[cfg(test)]` unit test does not satisfy a
/// production-path assertion).
fn function_body<'a>(source: &'a str, signature_marker: &str) -> &'a str {
    let start = source.find(signature_marker).unwrap_or_else(|| {
        panic!("expected to find `{signature_marker}` in composition source");
    });
    let after_sig = &source[start..];
    let open = after_sig
        .find('{')
        .unwrap_or_else(|| panic!("function `{signature_marker}` has no body brace"));
    let bytes = after_sig.as_bytes();
    let mut depth = 0usize;
    let mut end = open;
    for (idx, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = idx + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    &after_sig[..end]
}

const OBLIGATIONS_SRC: &str = "crates/ironclaw_host_runtime/src/obligations.rs";
const SERVICES_SRC: &str = "crates/ironclaw_host_runtime/src/services.rs";
const REBORN_RUNTIME_SRC: &str = "crates/ironclaw_reborn/src/runtime.rs";
const LOOP_DRIVER_HOST_SRC: &str = "crates/ironclaw_reborn/src/loop_driver_host.rs";

/// Sanity check: the security seams this test references must still exist as
/// `Option<Arc<dyn ...>>` fields with `with_*` setters. If a setter is renamed
/// or removed, this fails loudly here rather than silently making the
/// production-wiring assertions vacuous.
#[test]
fn security_seam_setters_exist() {
    let obligations = read_crate_source(OBLIGATIONS_SRC);
    assert!(
        obligations.contains("security_audit_sink: Option<Arc<dyn SecurityAuditSink>>"),
        "BuiltinObligationHandler must keep the leak-detector `security_audit_sink` \
         seam; if it was renamed, update {OBLIGATIONS_SRC} assertions in this test"
    );
    assert!(
        obligations.contains("pub fn with_security_audit_sink("),
        "BuiltinObligationHandler must keep the `with_security_audit_sink` setter; \
         if it was renamed, update this test"
    );

    let host = read_crate_source(LOOP_DRIVER_HOST_SRC);
    assert!(
        host.contains(
            "hook_gate_ref_factory: Option<Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>>"
        ),
        "RebornLoopDriverHostFactory must keep the `hook_gate_ref_factory` seam"
    );
    assert!(
        host.contains("pub fn with_hook_gate_ref_factory_builder<F>")
            && host.contains("pub fn with_hook_dispatcher_builder_factory<F>"),
        "RebornLoopDriverHostFactory must keep the hook gate-ref / dispatcher \
         builder-factory setters"
    );
}

/// SEAM: obligation / leak-detector `SecurityAuditSink`.
///
/// `BuiltinObligationHandler::complete_dispatch` records a payload-free
/// `SecurityAuditEvent` when leak redaction blocks output — but only if a
/// `security_audit_sink` was installed. The canonical production path that
/// constructs the handler is `HostRuntimeServices::builtin_obligation_handler`
/// (reached from `build_reborn_runtime` ->
/// `services.host_runtime_for_production` -> `build_host_runtime`).
///
/// If that builder never calls `with_security_audit_sink`, leak-detector blocks
/// in production are never audited and the seam silently no-ops — exactly the
/// #3919/#3922/#3938 defect class.
///
/// `#[ignore]` UNTIL #3922 MERGES. As of `reborn-integration` this assertion
/// FAILS, correctly: `HostRuntimeServices` has no `security_audit_sink` field,
/// and `builtin_obligation_handler` wires the (distinct) obligation `AuditSink`
/// via `with_audit_sink_dyn` but never the `SecurityAuditSink` — so leak-block
/// audit events are dropped in production. The open PR #3922
/// (`security-audit-sink-wiring`, "wire SecurityAuditSink into obligation
/// handler + hook deny paths") is the fix: it adds a `security_audit_sink` field
/// plus a `with_security_audit_sink` setter to `HostRuntimeServices` and calls
/// that setter inside `builtin_obligation_handler`. This test asserts that exact
/// end state.
/// It is `#[ignore]`d only so it does not red-block CI before #3922 lands;
/// remove the `#[ignore]` (and this note) the moment #3922 merges, at which
/// point the textual assertion goes green and guards against regression.
/// Verified against the #3922 diff: the `with_security_audit_sink` call lands
/// inside the same `fn builtin_obligation_handler` body this test scopes to.
#[test]
#[ignore = "asserts the #3922 end state; un-ignore once #3922 (security-audit-sink-wiring) merges into reborn-integration"]
fn production_obligation_handler_wires_security_audit_sink() {
    let services = read_crate_source(SERVICES_SRC);
    let builder = function_body(&services, "fn builtin_obligation_handler(");
    assert!(
        builder.contains("with_security_audit_sink"),
        "the canonical production obligation-handler builder \
         (`HostRuntimeServices::builtin_obligation_handler` in {SERVICES_SRC}) must call \
         `with_security_audit_sink(...)`, otherwise the leak-detector security-audit seam \
         silently no-ops in production. This is the recurring #3919/#3922/#3938 defect \
         class: the seam exists with a setter, but the production composition forgot to \
         call it. Wiring the obligation `AuditSink` via `with_audit_sink_dyn` is NOT the \
         same seam."
    );
}

/// SEAM: hook gate-ref factory + hook dispatcher builder factory on
/// `RebornLoopDriverHostFactory`, assembled by `build_default_planned_runtime`.
///
/// These seams are *conditionally* required: the production hooks feature is
/// being landed incrementally and is not yet composed into the production
/// runtime path on `reborn-integration` (the `with_hook_*` setters are
/// currently only exercised by `crates/ironclaw_reborn/tests/hooks_integration.rs`).
/// Asserting they are populated *today* would be a false positive — the feature
/// is legitimately not composed yet, which is different from "forgotten setter".
///
/// The guard we CAN assert without a false positive: **if** the production
/// composition `build_default_planned_runtime` ever starts wiring hooks (i.e.
/// references a `HookDispatcher` / hook-dispatcher builder), **then** it must
/// also wire the gate-ref factory. A hook dispatcher without a gate-ref factory
/// is fail-closed for `PauseApproval` / `PauseAuth` and silently degrades the
/// approval/auth security boundary — the same "feature enabled but seam
/// forgotten" failure mode, one layer up.
#[test]
fn production_planned_runtime_pairs_hook_dispatcher_with_gate_ref_factory() {
    let runtime = read_crate_source(REBORN_RUNTIME_SRC);
    let composition = function_body(&runtime, "pub fn build_default_planned_runtime<");

    let composes_hook_dispatcher = composition.contains("with_hook_dispatcher_builder_factory")
        || composition.contains("with_hook_dispatcher_factory");

    if !composes_hook_dispatcher {
        // Hooks not yet composed into the production planned runtime. This is
        // the legitimate `reborn-integration` state, not a forgotten setter.
        // Nothing to assert until the feature is composed; the
        // `security_seam_setters_exist` test guards the setters' continued
        // existence in the meantime.
        return;
    }

    assert!(
        composition.contains("with_hook_gate_ref_factory_builder")
            || composition.contains("with_hook_gate_ref_factory"),
        "`build_default_planned_runtime` in {REBORN_RUNTIME_SRC} composes a hook dispatcher \
         but does not wire a hook gate-ref factory. Without it, hook `PauseApproval` / \
         `PauseAuth` decisions fail closed and the approval/auth security boundary silently \
         degrades. When hooks are enabled in production, the gate-ref factory seam must be \
         populated (#3919/#3922/#3938 defect class)."
    );
}

/// SEAM: hook security-audit sink (`hook_security_audit_sink`).
///
/// This is the seam the in-flight #3922 fix threads through
/// `default_loop_composition`. As of `reborn-integration` the symbol does not
/// exist yet (the fix is unmerged). We assert the END STATE conditionally: once
/// the `with_hook_security_audit_sink` setter exists in the loop-driver host,
/// the production planned-runtime composition must call it. Until then this is a
/// no-op so the guard does not falsely fail before #3922 lands; after #3922
/// merges it becomes load-bearing automatically.
#[test]
fn production_planned_runtime_wires_hook_security_audit_sink_once_available() {
    let host = read_crate_source(LOOP_DRIVER_HOST_SRC);
    let setter_exists = host.contains("with_hook_security_audit_sink");
    if !setter_exists {
        // #3922 not merged yet: the `hook_security_audit_sink` seam does not
        // exist on this branch. Nothing to guard until it lands.
        return;
    }

    let runtime = read_crate_source(REBORN_RUNTIME_SRC);
    let composition = function_body(&runtime, "pub fn build_default_planned_runtime<");
    assert!(
        composition.contains("with_hook_security_audit_sink"),
        "the `with_hook_security_audit_sink` seam now exists but the production composition \
         `build_default_planned_runtime` ({REBORN_RUNTIME_SRC}) does not call it — the hook \
         security-audit sink would silently no-op in production (#3922 defect class)."
    );
}
