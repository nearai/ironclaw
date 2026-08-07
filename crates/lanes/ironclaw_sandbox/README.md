# ironclaw_sandbox

The sandboxed-process execution lane: a typed plan contract for what an OS
process invocation may do, and the container-backed machinery that executes a
validated plan behind the kernel's `SandboxCommandTransport` seam — Docker
connect, network broker and allowlist, credential firewall, container identity,
the per-tenant sandbox CA, mounts, and scope/user keys. The
`bollard`/`rcgen`/`libc` cone lives here and nowhere else in the workspace;
keeping it (and the elevated review scrutiny it deserves) out of the kernel is
the point of the crate.

- **Family / layer:** `lanes` / `runtimes` · **Package:** `ironclaw_sandbox` ·
  **Manifest:** `crates/lanes/ironclaw_sandbox/Cargo.toml`
- **Use this when:** an OS process must run — as a typed, validated
  `SandboxProcessPlan`, inside a real container boundary (a virtual scoped
  filesystem view does not contain a subprocess).
- **Don't use this when:** you're deciding whether the process may run →
  kernel; you need process-lifecycle supervision → `ironclaw_processes`;
  you're tempted to pass raw Docker flags, raw host paths, or raw secret
  material → the plan contract exists to make that impossible.

## Public surface

- Plan contract: `SandboxProcessPlan` / `ValidatedSandboxProcessPlan`, with
  typed install-plan, command-plan, mount, network-plan, and
  credential-binding sub-vocabulary (`plan`, `validation`). Install and
  credentialed-run phases stay separate.
- Execution machinery: `RebornScopedSandboxCommandTransport` (implements the
  `ironclaw_host_api::process::SandboxCommandTransport` port — contracts
  vocabulary this lane implements and the kernel consumes),
  `RebornSandboxConfig`, the broker/firewall/CA/identity types
  (`sandbox_process`).
- Script lane: `ScriptRuntime`, `ScriptExecutor`/`ScriptBackend`,
  `DockerScriptBackend`, `ScriptRuntimeHttpAdapter`, normalized
  request/result/error types (`script`).

**Wiring status:** three production paths cross this crate (plan validation on
the kernel spawn path, the process-executor routing check, the saved-output
scope digest) but there is **no production execution backend** today —
`with_script_runtime` and `RebornScopedSandboxCommandTransport::new` have zero
production callers. Read `CLAUDE.md`'s "Wiring status" before deleting
anything as dead code.

## Depends on / consumed by

- **Depends on (workspace, normal):** `ironclaw_host_api`,
  `ironclaw_extension_contracts`, `ironclaw_common` — plus, measured today,
  three substrate crates the WS3 merge carried in: `ironclaw_network`,
  `ironclaw_safety`, `ironclaw_secrets`. The lanes family target is
  injection-only for mediated services; these edges are live, mechanically
  legal, and recorded as a deviation in
  [`crates/lanes/AGENTS.md`](../AGENTS.md). `ironclaw_resources` is
  **dev-only** (#7067). External: `bollard`, `rcgen`, `libc`, and friends —
  declared by this crate and no other.
- **Consumed by (measured 2026-08-05):** `ironclaw_host_runtime` (normal);
  `ironclaw_loop_host` and `ironclaw_turn_runner` dev-only.

## Invariants

- **Typed plans only:** no raw container flags, host paths, environment
  inheritance, or secret material through plan input (`plan`/`validation`).
- **Credential firewall as a staging chokepoint:** an invocation consumes only
  the credential it was already entitled to; consumers see yes/no, never
  material. The per-tenant CA root key never touches disk and is never
  serialized to a caller.
- **Fail closed on missing containment:** a served multi-user deployment must
  never resolve to an unsandboxed host-process backend; a missing backend
  degrades to "no shell", never to a silently unsandboxed one.
- **No lane-owned networking:** scanned by
  `reborn_runtime_http_egress_has_single_network_boundary`
  (`reborn_dependency_boundaries.rs`).
- **Known debt, not invariant yet:** `script.rs` still shells out directly
  (`Command::new("docker")`) instead of routing through
  `SandboxCommandTransport`, and the `IRONCLAW_REQUIRE_DOCKER_TESTS` fail-closed
  switch is armed by nothing (#7081) — both carried in `CLAUDE.md`.

## Tests

```bash
cargo test -p ironclaw_sandbox              # real-Docker suites skip without a daemon (#7081)
cargo test -p ironclaw_architecture_tests   # egress scan + layer matrix
```

## See also

Working rules, wiring status, and known debt: [`CLAUDE.md`](./CLAUDE.md)
(canonical). Family boundary: [`crates/lanes/AGENTS.md`](../AGENTS.md).
Contracts: `docs/reborn/contracts/scripts.md`,
`docs/reborn/contracts/processes.md`,
`docs/reborn/contracts/runtime-workflows.md`,
`docs/reborn/contracts/network.md`. Design record: PROPOSAL §6.6.4.
