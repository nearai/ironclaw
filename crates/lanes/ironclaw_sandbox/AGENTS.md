# ironclaw_sandbox guardrails

The sandboxed-process lane. Merged in WS3 from `ironclaw_process_sandbox`
(plan contract), `ironclaw_host_runtime::sandbox_process` (Docker / broker /
credential-firewall / CA), and `ironclaw_scripts` (script lane + Docker
execution path). PROPOSAL §6.6.4.

## Why one crate

The `bollard`/`rcgen`/`libc` cone lives here and **nowhere else in the
workspace** — keeping it out of the kernel is the point. `ironclaw_host_runtime`
no longer declares any of the three.

## Wiring status — read before assuming this is dead code

**Three** production call paths cross this crate today, and none of them is
execution. Only the first is plan validation — do not delete the other two as
dead code on the strength of a "plan validation only" reading:

- `ironclaw_host_runtime::production::host_runtime_spawn_input_for_capability`
  parses and validates `SandboxProcessPlan` → `ValidatedSandboxProcessPlan` on
  the spawn path, rejecting bad plans as model-visible tool errors, and
  `services::process_executor` routes such requests away from the dispatch
  executor.
- `ironclaw_loop_host` compares against
  `ironclaw_host_api::capability::PROCESS_SANDBOX_CAPABILITY_ID`.
- `ironclaw_host_runtime::process_output` uses `RebornSandboxScopeKey` to derive
  the scoped saved-output directory — a production path through this crate's
  scope-key digest.

There is still **no production execution backend** for
`system.process_sandbox.run`: the Docker/CA machinery and the script lane have
no production constructor (`with_script_runtime` and
`RebornScopedSandboxCommandTransport::new` are called only from tests). The
`#[allow(dead_code)] // consumed by W6` markers are accurate.

## Ownership

- `plan` — typed `SandboxProcessPlan` / `ValidatedSandboxProcessPlan`. Accept
  only typed plan input: no raw Docker flags, raw host paths, host environment
  inheritance, or raw secret material from plan JSON. Keep install and
  credentialed-run phases separate: install may declare scoped tool/cache state
  with no secrets; credentialed run declares brokered secrets and read-only
  tool/cache state.
- `sandbox_process` — the execution machinery behind
  `ironclaw_host_api::process::SandboxCommandTransport`: Docker connect, network
  broker and allowlist, credential firewall, container identity, the sandbox CA,
  mounts, scope/user keys, shell limits, activity registry.
- `script` — `ScriptRuntime`, the `ScriptExecutor`/`ScriptBackend` traits,
  `DockerScriptBackend`, `ScriptRuntimeHttpAdapter`, and the normalized
  request/result/error types.

## Do not move in here

- Ambient credentials. The credential-firewall design stays: secret values live
  behind broker/lease seams and redaction helpers, and never appear in plan
  JSON, validation errors, debug output, or logs.
- Dispatcher composition. This crate must not expose `RuntimeAdapter`-shaped
  surface or depend on `ironclaw_capabilities` — script/MCP dispatch adapters
  are host-runtime-private composition (pinned by
  `reborn_dependency_boundaries.rs`).
- Manual credentials, direct provider HTTP, or duplicated
  dispatcher/process/resource policy.
- Docker mount-root or executor configuration: that belongs with whatever crate
  eventually wires a real backend.

## Known debt

- **Direct process spawning.** `script.rs` still shells out with
  `std::process::Command` (`Command::new("docker")`) rather than going through
  `SandboxCommandTransport`. CHECKLIST WS3 calls for routing all process
  spawning through the transport seam; the merge colocated the two halves that
  makes that possible but did **not** perform the rewiring, because that is a
  behavior change and this was a move.
- **The Docker fail-closed switch is wired to nothing — issue #7081
  (pre-existing, inherited with the move).** `tests/support/docker_gate.rs` says
  `IRONCLAW_REQUIRE_DOCKER_TESTS=1` is what turns a missing daemon or image from
  a silent skip into a hard failure, and that "CI sets this". **Nothing sets
  it.** Stated as the search that checks it: no workflow, script, env file or
  manifest mentions the name at all —
  `git grep IRONCLAW_REQUIRE_DOCKER_TESTS -- '*.yml' '*.yaml' '*.sh' '*.toml' '*.py' '*.json' '.env*'`
  is empty in this tree **and** on `main` — and the sole code reference is a
  **read**, `std::env::var("IRONCLAW_REQUIRE_DOCKER_TESTS")` at
  `docker_gate.rs:23`. Every other occurrence (here, `docker_security.rs`,
  `attribution_tests.rs`, the rest of `docker_gate.rs`) is a doc comment or a
  panic message. So every real-Docker test in this crate skips-and-
  passes everywhere, which is the exact gap the gate's own comment says let
  sandbox security bugs ship unnoticed. Guardrail-claim-vs-reality, the #6945
  class. The fix has two halves and **only one of them is here**:
  - ✅ **Done (2026-08-03, #7065):** `tests/docker_security.rs` used to bypass
    the gate entirely — it open-coded its own `docker version` check and three
    `return`s, so it would have stayed fail-open even once something set the
    variable. It now takes both preconditions from `docker_gate`
    (`docker_available` + `docker_image_available`) and skips with a visible
    `SKIP:` line. With the variable unset — i.e. everywhere today — this is a
    no-op: the daemon-down path already reached the image check and skipped
    there.
  - ❌ **Open (#7081):** nothing sets `IRONCLAW_REQUIRE_DOCKER_TESTS=1`, so the
    switch is still inert and the whole family still skips-and-passes. Arming it
    is a CI-behavior change — it hard-fails any lane lacking a daemon or the
    `ironclaw-worker` image — and needs a runner that is guaranteed to have
    both. Deliberately not made inside a move PR. Note the required Rust e2e
    lane (`scripts/reborn-e2e-rust.sh`) runs `docker_security` as of WS3, which
    is strictly more coverage than before (it was in no lane); it will assert
    rather than skip the moment #7081 lands.
- **No budget authority (#7067, 2026-08-04).** The lane takes
  `ironclaw_host_api::resource::RuntimeResourceBudget` — reserve / reconcile /
  release, and nothing else — never `ResourceGovernor`. The kernel implements
  the port over its governor (`ironclaw_resources::GovernorRuntimeBudget`), so
  the lane cannot set limits, read account state, or name an account, and the
  `runtimes → kernel` layer-matrix exception this dependency used to need is
  **deleted, not waived**. `ironclaw_resources` remains a **dev**-dependency
  only: the lane suites drive the port over the real governor so a denial they
  assert on is one the kernel actually produced. Do not re-add it under
  `[dependencies]`, and do not widen the port — a lane that needs more budget
  surface is a design question for the kernel, not a lane change.

## Validation

- Fast local check: `cargo test -p ironclaw_sandbox`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- Contracts: `docs/reborn/contracts/scripts.md`,
  `docs/reborn/contracts/runtime-workflows.md`,
  `docs/reborn/contracts/network.md`

## See also

[`README.md`](./README.md) — orientation: public surface, measured edges
(including the three substrate deps carried in by the WS3 merge), tests.
[`../AGENTS.md`](../AGENTS.md) — the `lanes/` family boundary and its gates.
