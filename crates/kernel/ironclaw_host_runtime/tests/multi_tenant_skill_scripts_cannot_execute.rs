//! A skill's scripts must not be executable under multi-tenant hosting — pinned, not assumed.
//!
//! #6745 lets a skill ship `scripts/*.py` alongside its SKILL.md, which is what makes a learned
//! skill reusable rather than a prose description. @serrrfirat's question on the epic was the
//! right one: *"What if it's a malicious script and we run it on host and ggs."*
//!
//! The answer today is that a multi-tenant agent has nothing to run it WITH. `builtin.shell` is
//! the only process-port-backed builtin, and it is removed from the capability package outright
//! when the resolved process backend cannot execute — so the script sits inert in the skill
//! store with no tool able to invoke it.
//!
//! That guarantee is currently a chain of three inferences: `HostedMultiTenant` →
//! `RuntimeProfile::SecureDefault` → `ProcessBackendKind::None` → shell removed. Every link is
//! correct and none is asserted anywhere, so a future change to any one of them silently enables
//! script execution for every tenant. This test asserts the property directly.
//!
//! **`TenantSandbox` is deliberately treated as execution-capable** and is NOT covered by this
//! test, because it is @henrypark133's sandbox work: `crates/ironclaw_process_sandbox` is a
//! complete Docker backend (`--cap-drop ALL`, `no-new-privileges`, `readonly_rootfs`,
//! `--network none`, non-root uid) that currently has no non-test caller. When that lands,
//! multi-tenant execution becomes safe *because it is sandboxed*, and the first assertion here
//! should be revisited rather than deleted. Until then, composition refuses to build a policy
//! requesting `TenantSandbox` without a port, which the second assertion pins.

use ironclaw_host_api::runtime_policy::ProcessBackendKind;
use ironclaw_host_runtime::builtin_first_party_package_for_process_backend;

/// The capability a skill's `scripts/*.py` would need in order to run.
const SHELL_CAPABILITY: &str = "builtin.shell";

fn capability_ids(backend: ProcessBackendKind) -> Vec<String> {
    let package = builtin_first_party_package_for_process_backend(backend)
        .expect("the builtin first-party package must build for every backend");
    package
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

#[test]
fn a_backend_that_cannot_execute_has_no_shell_so_skill_scripts_are_inert() {
    // `None` is what `HostedMultiTenant` resolves to today, via `RuntimeProfile::SecureDefault`.
    // Named as one backend rather than looped over a one-element list: the moment a second
    // non-executing backend exists (`TenantSandbox` is the expected one), it needs its OWN case
    // with its own message, not a silent extra iteration here.
    let backend = ProcessBackendKind::None;
    let ids = capability_ids(backend);
    assert!(
        !ids.iter().any(|id| id == SHELL_CAPABILITY),
        "{backend:?} exposes {SHELL_CAPABILITY}, so a skill's scripts/*.py become executable \
         under multi-tenant hosting. Script-bearing skills are disabled for multi-tenant \
         execution until the tenant sandbox lands; if this is now intentional, the sandbox \
         must be wired first."
    );
    assert!(
        !ids.is_empty(),
        "removing process-backed builtins must not empty the package"
    );
}

#[test]
fn execution_capable_backends_keep_shell_so_this_test_is_not_vacuous() {
    // Without this, the assertion above would pass just as happily if `builtin.shell` had been
    // renamed or removed everywhere -- proving nothing about the multi-tenant case.
    //
    // `TenantSandbox` appears here on purpose: it is execution-capable BY DESIGN, and safe only
    // once the sandbox process port is wired. That is the boundary this pair of tests draws.
    for backend in [
        ProcessBackendKind::LocalHost,
        ProcessBackendKind::Docker,
        ProcessBackendKind::TenantSandbox,
    ] {
        let ids = capability_ids(backend);
        assert!(
            ids.iter().any(|id| id == SHELL_CAPABILITY),
            "{backend:?} is execution-capable but does not expose {SHELL_CAPABILITY}; the \
             multi-tenant assertion in this file would then be vacuous"
        );
    }
}
