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

Production process execution now crosses this crate when either explicit
sandbox profile is selected. The local profile constructs the Docker transport;
the Railway preview profile constructs the Railway transport. Every other
profile is rejected if given a sandbox binding, and both sandbox profiles fail
closed without one. Existing non-execution paths remain:

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

`system.process_sandbox.run` remains a separate plan-driven capability and is
not newly wired by this slice. The production path here is `builtin.shell`
through `SandboxCommandTransport`; the script-runtime lane remains separate.

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
- **The Docker fail-closed switch is lane-specific — follow-up to issue #7081.**
  `tests/integration/support/docker_gate.rs` says
  `IRONCLAW_REQUIRE_DOCKER_TESTS=1` is what turns a missing daemon or image from
  a visible skip into a hard failure. The `sandbox-docker-tests` CI job now sets
  the switch and provisions the worker image, so that lane fails closed. Other
  jobs and ad-hoc local runs leave the switch unset and continue to print a
  visible skip when Docker or the worker image is unavailable. The wiring can
  be audited with:
  `git grep IRONCLAW_REQUIRE_DOCKER_TESTS -- '*.yml' '*.yaml' '*.sh' '*.toml' '*.py' '*.json' '.env*'`.
  The implementation history has two parts:
  - ✅ **Done (2026-08-03, #7065):** `tests/docker_security.rs` used to bypass
    the gate entirely — it open-coded its own `docker version` check and three
    `return`s, so it would have stayed fail-open even once something set the
    variable. It now takes both preconditions from `docker_gate`
    (`docker_available` + `docker_image_available`) and skips with a visible
    `SKIP:` line. With the variable unset — i.e. everywhere today — this is a
    no-op: the daemon-down path already reached the image check and skipped
    there.
  - ✅ **Done in the sandbox PR1 lane:** `sandbox-docker-tests` sets
    `IRONCLAW_REQUIRE_DOCKER_TESTS=1` and builds the exact worker image before
    running the Docker-backed profile and full-turn tests. Tests outside that
    lane remain intentionally optional unless an operator sets the switch.
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
