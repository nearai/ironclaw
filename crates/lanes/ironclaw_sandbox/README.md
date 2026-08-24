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

## Production wiring and lifecycle

The `HostedSingleTenantVolumeSandboxed` profile has a production execution
backend. Composition connects `RebornScopedSandboxCommandTransport` and installs
it behind the host runtime's user-sandbox process port. A `builtin.shell` call
therefore uses Docker Exec inside one persistent local container per
`(tenant, user)`. `RebornSandboxUserKey` supplies the stable container name and
tenant/user labels, so every thread owned by that user converges on the same
container. The host workspace is also scoped per user and mounted at
`/workspace`; container-local state survives subsequent shell calls while that
container exists.

Before an exec, the transport adopts a compatible running container, restarts a
compatible stopped container, or recycles a container whose image or security
posture no longer matches. A per-user creation gate converges concurrent first
calls on one container. Active-exec accounting prevents the idle sweeper from
stopping it until all commands finish. The sweeper stops an inactive container;
the next command adopts and restarts it.
For managed egress, idle suspension removes the proxy and its upstream network
but retains the stopped worker's private network. This keeps the proxy DNS
endpoint stable and preserves container-local state on wake. Final retention
cleanup removes the stopped container and private network together.

One IronClaw process owns a local Docker workspace root at a time. The
transport acquires a transport-local advisory owner lock on the workspace root
before container reconciliation and fails closed if another live process holds
that workspace.
The lock, not the file's metadata, grants authority; a stale lock file after a
crash grants nothing. This keeps process-local activity counters from
authorizing cleanup of another process's active container.

The idle sweeper is a narrow provider-resource cleanup loop, not a second
durable process lifecycle: it never claims runs, changes
`ironclaw_processes` run/lifecycle state, or decides whether work may execute.
`ironclaw_processes` remains the only lifecycle authority. During the current
IronClaw process lifetime, the sweeper stops inactive containers so they do not
consume Docker resources indefinitely. After a host restart, reconciliation
happens only when the next command adopts, restarts, or replaces that user's
container. Composition owns sweeper startup/shutdown through
`SandboxCommandTransport::shutdown`. If cleanup must later happen without a
new command, move that durable timer and ownership decision behind a
kernel-owned lifecycle port rather than expanding this lane's authority.

`HostedSingleTenantVolumeSandboxedRailway` remains a separate transport. It
keeps a per-user Railway sandbox and checkpointed workspace, but starts a fresh
inner worker container for each command because Railway does not preserve inner
mount namespaces across outer exec calls. It does not use the persistent local
Docker-container lifecycle above.

Sandbox deployment profiles use managed per-user egress. Worker containers
join an internal Docker network with isolated gateway mode, so the Docker host
has no bridge endpoint on that subnet. A dedicated `ironsh/iron-proxy` sidecar
joins that private network and a host-scoped shared
upstream network. Its DNS and proxy listeners bind only to the private-network
address, so other proxies on the shared network cannot use its allowlist or
attribution identity. The proxy applies the configured hostname allowlist,
rejects private-address destinations, preserves end-to-end TLS with SNI
inspection, and writes a request audit trail correlated to the capability
invocation. Before any managed proxy container is removed — idle suspension,
retention, recycling, rollback, or orphan reconciliation — its structured
request audit log is drained into a bounded per-proxy file under the
managed-egress `audit/` directory, so egress evidence survives the container.
The local runtime resolves the proxy by immutable digest and pulls
an absent public image before startup. Workers address the proxy by its stable
per-user container name; transient Docker subnet addresses do not enter the
persistent worker's compatibility stamp. Railway preview sandboxes retain a
dedicated two-network shape inside Railway's private outer sandbox. Ad hoc
transports remain fail-closed on `--network none` unless they receive an
explicit host broker or managed-egress binding.

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
- **Consumed by:** `ironclaw_composition` and `ironclaw_host_runtime` (normal);
  `ironclaw_loop_host` and `ironclaw_turn_runner` (dev-only).

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
- **Runtime HTTP boundary remains outside this lane:** scanned by
  `reborn_runtime_http_egress_has_single_network_boundary`
  (`reborn_dependency_boundaries.rs`). This invariant is separate from the
  direct process egress called out in the Step 2 limitation above.
- **Known debt:** the legacy script lane in `script.rs` still shells out
  directly with `Command::new("docker")` instead of routing through
  `SandboxCommandTransport`.

## Tests

```bash
cargo test -p ironclaw_sandbox              # unit suites; Docker cases skip without a daemon
cargo test -p ironclaw_sandbox --test user_sandbox_docker_live -- --test-threads=1
                                            # real-Docker lifecycle suite; serialized per daemon
cargo test -p ironclaw_architecture_tests   # egress scan + layer matrix
```

## See also

Dependency rules and known debt: [`AGENTS.md`](./AGENTS.md). Family boundary:
[`crates/lanes/AGENTS.md`](../AGENTS.md).
Contracts: `docs/internal/reborn/contracts/scripts.md`,
`docs/internal/reborn/contracts/processes.md`,
`docs/internal/reborn/contracts/runtime-workflows.md`,
`docs/internal/reborn/contracts/network.md`. Design record: PROPOSAL §6.6.4.
