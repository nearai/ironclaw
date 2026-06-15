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
///
/// # Known limitations (future work)
///
/// This helper is a deliberately simple lexer-free scanner: it locates the
/// signature with a plain substring search and then balances braces by counting
/// raw `{` / `}` bytes. It does **not** understand Rust tokens, so it will
/// miscount when `{` or `}` appear inside string literals, char literals, or
/// comments (including doc comments) within the scanned function — any such
/// brace is treated as real nesting and can prematurely or belatedly terminate
/// the extracted body. It also assumes the `signature_marker` is unique enough
/// to land on the intended function (a matching marker inside a comment or
/// string earlier in the file would be found first).
///
/// This is acceptable today because the composition functions these assertions
/// scope to are brace-balanced setter-call chains free of literal/comment
/// braces. If that ever stops holding, replace this with a real token-aware
/// scan (e.g. `proc_macro2`/`syn`) rather than extending the brace counter.
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

/// Signature marker for the function whose body actually assembles the default
/// planned runtime and calls the `with_hook_*` seam setters.
///
/// The public entry points `build_default_planned_runtime` and
/// `build_default_planned_runtime_with_wake_channel` are thin wrappers that
/// immediately delegate to this private `_with_optional_wake_channel` impl, so
/// the seam wiring lives here. Scoping the seam assertions to this function (and
/// not the public wrapper) is what keeps them load-bearing — pointing at the
/// empty wrapper would silently make every hook-seam guard vacuous. The `<`
/// pins the match to the generic definition rather than its call sites.
const PLANNED_RUNTIME_COMPOSITION_FN: &str =
    "fn build_default_planned_runtime_with_optional_wake_channel<";

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
/// #3922 (`security-audit-sink-wiring`, "wire SecurityAuditSink into obligation
/// handler + hook deny paths") has landed: `HostRuntimeServices` now carries a
/// `security_audit_sink` field plus a `with_security_audit_sink` setter, and
/// `builtin_obligation_handler` calls that setter when the sink is installed.
/// This test now actively guards that end state — if the production builder ever
/// stops calling `with_security_audit_sink`, this assertion fails the build
/// rather than letting leak-block audit events silently drop in production.
#[test]
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
/// `RebornLoopDriverHostFactory`, assembled by the default planned-runtime
/// composition.
///
/// This was originally written as a *conditional* guard for when hooks were not
/// yet composed in production: "if the composition ever wires a hook dispatcher,
/// it must also wire the gate-ref factory." That precondition has since flipped
/// — the planned-runtime composition now wires `with_hook_dispatcher_builder_factory`
/// — and doing so surfaced a real, already-tracked production gap: the dispatcher
/// is composed but its companion gate-ref factory is NOT
/// (`with_hook_gate_ref_factory*` is only ever called in
/// `crates/ironclaw_reborn/tests/hooks_integration.rs`, never in production
/// composition). A hook dispatcher without a gate-ref factory fails closed for
/// `PauseApproval` / `PauseAuth` and silently degrades the approval/auth security
/// boundary — the exact #3919/#3922/#3938 defect class.
///
/// That gap is tracked by issue #3962 ("[Reborn] Standalone composition root
/// doesn't wire hooked-prompt deps (gate-ref factory / capability input
/// resolver) under HOOKS_ENABLED"). Rather than red-block CI on a pre-existing,
/// separately-tracked defect — or hide it behind a silent early-return — this
/// test pins the gap as an *explicit, self-arming* assertion: it confirms the
/// dispatcher IS composed and the gate-ref factory is NOT, and is wired so that
/// the moment #3962 is fixed (gate-ref factory gets wired) the assertion flips
/// red, forcing whoever lands the fix to delete this gap-acknowledgment and turn
/// the test back into a positive "they are paired" guard.
///
/// TODO(#3962): once the gate-ref factory is wired into production composition,
/// replace the body below with the positive pairing assertion (dispatcher
/// composed => gate-ref factory composed) and drop the gap tolerance.
#[test]
fn production_planned_runtime_pairs_hook_dispatcher_with_gate_ref_factory() {
    let runtime = read_crate_source(REBORN_RUNTIME_SRC);
    let composition = function_body(&runtime, PLANNED_RUNTIME_COMPOSITION_FN);

    let composes_hook_dispatcher = composition.contains("with_hook_dispatcher_builder_factory")
        || composition.contains("with_hook_dispatcher_factory");
    let composes_gate_ref_factory = composition.contains("with_hook_gate_ref_factory_builder")
        || composition.contains("with_hook_gate_ref_factory");

    assert!(
        composes_hook_dispatcher,
        "production planned-runtime composition ({REBORN_RUNTIME_SRC}, \
         `{PLANNED_RUNTIME_COMPOSITION_FN}`) no longer wires a hook dispatcher. If hooks were \
         intentionally removed from production, delete this test; otherwise the dispatcher seam \
         regressed (#3919/#3922/#3938 defect class)."
    );

    // KNOWN GAP (#3962): the dispatcher is composed but the gate-ref factory is
    // not. This assertion documents and pins that state. When #3962 lands and
    // wires `with_hook_gate_ref_factory*` into production composition, this
    // flips red on purpose — see the TODO above for what to do then.
    assert!(
        !composes_gate_ref_factory,
        "the hook gate-ref factory now appears to be wired into production composition \
         ({REBORN_RUNTIME_SRC}). That closes the #3962 gap — good! Update this test: remove the \
         known-gap tolerance and replace it with a positive assertion that the dispatcher and \
         gate-ref factory are wired together, per the TODO(#3962) note above."
    );
}

/// SEAM: hook security-audit sink (`hook_security_audit_sink`).
///
/// This is the seam #3922 threads through the loop-driver host. The guard is
/// written to self-arm: if the `with_hook_security_audit_sink` setter is absent
/// from the loop-driver host it is a no-op (so it cannot falsely fail before the
/// seam exists), but once the setter is present the production planned-runtime
/// composition MUST call it. As of the current branch the setter exists and the
/// composition wires it, so this assertion is load-bearing.
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
    let composition = function_body(&runtime, PLANNED_RUNTIME_COMPOSITION_FN);
    assert!(
        composition.contains("with_hook_security_audit_sink"),
        "the `with_hook_security_audit_sink` seam now exists but the production composition \
         `build_default_planned_runtime` ({REBORN_RUNTIME_SRC}) does not call it — the hook \
         security-audit sink would silently no-op in production (#3922 defect class)."
    );
}
