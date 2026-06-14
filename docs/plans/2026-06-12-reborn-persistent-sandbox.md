# Reborn Persistent Tenant Sandbox & Agent-Built Extension Promotion

## Context

The Reborn binary's process-execution story is three-quarters designed and one-quarter
implemented:

- `ProcessBackendKind` (`crates/ironclaw_host_api/src/runtime_policy.rs:403`) is a
  vocabulary enum: `None`, `Docker`, `Srt`, `SmolVm`, `LocalHost`, `TenantSandbox`,
  `OrgDedicatedRunner`. Only `LocalHost` and `TenantSandbox` have implementations;
  `LocalInvocationServicesResolver::resolve()`
  (`crates/ironclaw_host_runtime/src/invocation_services.rs:209-225`) rejects everything
  else with `UnsupportedProcessBackend`.
- The only `SandboxCommandTransport` implementation is
  `RebornScopedSandboxCommandTransport`
  (`crates/ironclaw_host_runtime/src/sandbox_process.rs:220`): a **per-command ephemeral
  Docker container**. Every `run_command` does create → start → wait → collect logs →
  force-remove. Nothing installed inside the container survives the command that
  installed it. `npm install -g foo && foo` works only if both halves are in the same
  command string; the next tool call starts from the bare image.
- The binding is **not wired**. `RebornRuntimeProcessBinding` defaults to `None`
  (`crates/ironclaw_reborn_composition/src/input.rs:105`), and neither
  `ironclaw_reborn_cli` nor `ironclaw_reborn` ever constructs a
  `TenantSandboxProcessPort` (grep: zero references outside `ironclaw_host_runtime`
  itself and composition test fixtures). Meanwhile every hosted profile —
  `HostedSafe`, `HostedDev`, `HostedYoloTenantScoped` — resolves to
  `ProcessBackendKind::TenantSandbox`
  (`crates/ironclaw_runtime_policy/src/resolver.rs:309-334`), so a hosted deployment
  today fails composition validation
  (`RebornRuntimeProcessBindingError::MissingTenantSandboxProcessPort`) or, if policy is
  absent, simply has no process effects.
- The transport already speaks "broker": `RebornSandboxConfig` can carry a
  `RebornSandboxNetworkBroker` and `RebornSandboxSecretBroker`
  (`sandbox_process.rs:84-85`), which set `http_proxy`/`https_proxy` env vars and
  bind-mount broker unix sockets into the container. **But no host-side broker server
  exists.** `crates/ironclaw_network` provides egress validation/transport primitives,
  and `src/sandbox/proxy/` is the v1 CONNECT-proxy with allowlist + credential
  injection — neither is composed into a listener the Reborn sandbox can actually reach.
- Secrets at the boundary are in good shape and stay untouched by this plan: production
  egress only accepts `CredentialSourceStrategy::StagedObligation`
  (`crates/ironclaw_host_runtime/src/egress/credential.rs`), staged material is one-shot
  with a 5-minute TTL (`obligations.rs`), zeroized on drop, and direct
  `SecretStoreLease` is rejected outside tests.

The user-facing goal: **a hosted deployment where the agent gets a persistent
environment — shell runs freely, CLIs (node, uv, cargo-installed tools, …) can be
installed and stay installed — while the system stays secure regardless of which
sandboxing technology backs it, and software the agent builds can be promoted into
IronClaw as an extension without a privilege-escalation path.**

## Goals

1. Wire the existing Docker transport into the Reborn binary so hosted profiles work
   at all (Phase 1).
2. Make the sandbox environment persistent per scope: installed toolchains, package
   caches, and home-directory state survive across commands, runs, and host restarts
   (Phase 2).
3. Give the persistent environment **brokered** network access sufficient for package
   installation (npm/crates.io/PyPI/GitHub), with credential injection staying at the
   host boundary (Phase 3).
4. Define the promotion pipeline that turns an artifact built inside the sandbox into
   an installed IronClaw extension with no implicit trust (Phase 4).
5. Keep every security property backend-independent so `SmolVm`/`Srt`/
   `OrgDedicatedRunner` can be added later by implementing one trait, not by re-auditing
   the system.

## Non-goals

- Implementing `SmolVm`, `Srt`, or `OrgDedicatedRunner` backends (this plan makes them
  pluggable; it does not build them).
- Long-running *services* inside the sandbox (dev servers that outlive a run, exposed
  ports). Phase 2b gives processes an idle window; full background-service lifecycle is
  a follow-up that should go through `ironclaw_processes`.
- Replacing the v1 `src/sandbox/` engine-v2 per-project sandbox
  (`docs/plans/2026-04-10-engine-v2-sandbox.md`). That path serves the v1 engine; this
  plan is Reborn-native. Shared ideas (persistent workspace computer) are convergent,
  not coupled.
- Touching the staged-obligation secret pipeline. It is the part that already works.

## The backend-independent security contract

Everything below holds for Docker today and must hold verbatim for any future backend.
Security lives in the brokers and the promotion gate, **not** in container ephemerality.
A persistent environment in which the agent installs arbitrary packages is
**assumed compromised** (prompt injection, malicious transitive dependencies,
typosquatted packages). The system stays secure anyway because:

| # | Invariant | Enforced where |
|---|-----------|----------------|
| I1 | The environment has no ambient network. Its only egress is the host broker: a CONNECT-only allowlist tunnel that carries no credential machinery. Credentialed API calls happen host-side via first-party capabilities. | `RebornSandboxConfig::container_network_mode()` (`network_mode: none` unless broker requires docker networking) + Phase 3 broker server |
| I2 | Raw secret material never enters the environment. Credential use is staged-obligation injection performed host-side at the egress boundary. | `egress/credential.rs` `StagedObligation`-only production path; `RebornSandboxSecretBroker` exposes an endpoint, never values (`sandbox_process.rs` tests `secret_broker_exposes_endpoint_without_secret_material`) |
| I3 | The host↔sandbox surface is minimal and typed: the command request/response (`CommandExecutionRequest`/`CommandExecutionOutput`), the scoped workspace mount, and the broker sockets. Everything crossing host-ward is untrusted input. | `SandboxCommandTransport` trait (`process_port.rs:115`); mount validation in `sandbox_process/mounts.rs`; output capping in `collect_logs` |
| I4 | Nothing acquires privilege by virtue of originating in the sandbox. Artifacts cross the promotion gate (hash → validate → manifest → user-trust install → human capability approval) exactly like third-party registry installs. | Phase 4 pipeline |
| I5 | Persistent state is scope-isolated. No shared caches, volumes, or containers across `ResourceScope` boundaries — a poisoned npm cache in tenant A must be unreachable from tenant B. | `RebornSandboxScopeKey` derivation (`sandbox_process/scope_key.rs`) extended to volume naming (Phase 2) |
| I6 | Resource ceilings are per scope: memory, CPU, pids, disk, and command timeout. Persistence makes disk and pids first-class (an ephemeral container could not fill a disk over weeks; a persistent volume can). | `HostConfig` limits today; disk + pids added in Phase 2 |

Container hardening that already exists and **must not be relaxed** for persistence:
read-only root filesystem, `cap_drop: ALL`, `no-new-privileges`, tmpfs `/tmp`
(`sandbox_process.rs:407-422`). Phase 2 achieves persistence *around* these, not by
removing them.

## Locked design decisions

1. **Persistence = named volume, compute = disposable.** Persistent state lives in a
   per-scope Docker named volume mounted at `/home/agent`. Containers remain
   replaceable (per-command in Phase 2a, warm-with-idle-reap in Phase 2b). This is the
   shape that ports to microVMs (persistent disk + ephemeral VM) and lets the base
   image be upgraded without losing installed tools. Disposability is also the
   enforcement mechanism: timeouts and runaway processes are handled by killing the
   environment, never by in-environment process management.
   The 2a→2b transition is an internal provider swap with **no configuration
   surface** — persistence is one user-visible behavior, not a mode enum.
2. **Read-only rootfs stays; installs are user-space.** The agent installs into
   `$HOME` (`~/.local/bin`, `~/.cargo`, `~/.npm-global`, nvm-style node installs), not
   `/usr`. The base image ships common toolchains (node LTS, python3+uv, rust, git,
   build-essential) so most work needs no install at all; the home volume covers the
   rest. `apt-get install` inside the container is *not* supported — if a system
   package is missing, it belongs in the base image.
3. **Volume scope = full `RebornSandboxScopeKey`** (tenant/user/project), matching the
   existing workspace derivation. One environment per project under hosted
   multi-tenancy. Decision rationale: per-project isolates toolchain poisoning blast
   radius to one project; the scope key already encodes the identity so changing
   granularity later is a naming-policy change, not a code change.
4. **The session/lifecycle layer lives behind `SandboxCommandTransport`.** The port
   (`TenantSandboxProcessPort`), the resolver
   (`LocalInvocationServicesResolver`), the shell tool
   (`first_party_tools/shell.rs`), and composition validation all stay byte-identical.
   New backends implement `SandboxCommandTransport` (or the Phase 2b
   `SandboxEnvironmentProvider` underneath it); nothing above the trait changes.
5. **Network for the persistent environment is `NetworkMode::Allowlist` via the
   broker, never direct.** Package registries are reachable through the host broker
   with a reviewable domain list. `HostedDev`/`HostedYoloTenantScoped` already resolve
   to `Allowlist` in the policy matrix; Phase 3 makes the broker real.
6. **Only WASM artifacts are promotable to host-installed extensions.** Native
   binaries the agent builds stay usable *inside* the sandbox (they're on the home
   volume, on `PATH`, runnable via the same shell tool) but never execute on the host.
   Promoted WASM runs under the existing capability sandbox at user trust with
   deny-by-default capabilities.
7. **Capability grants for promoted extensions are human decisions.** The agent
   proposes a manifest; granting HTTP endpoints or credential mappings routes through
   the existing approval-gate machinery (`ApprovalPolicy`), the same as any
   sensitive capability lease.
8. **Configuration enters through `ironclaw_reborn_config`** and construction happens
   in module-owned factories (per the "Module-owned initialization" rule):
   `ironclaw_host_runtime` owns transport construction;
   `ironclaw_reborn_composition` owns binding assembly; `serve.rs` only calls
   factories.

## Architecture overview

```
Reborn host process                                  Per-scope sandbox (Docker today)
───────────────────                                  ────────────────────────────────
runtime policy resolver
  └─ EffectiveRuntimePolicy{process_backend:
        TenantSandbox, network_mode: Allowlist}
composition (factory.rs)
  └─ RebornRuntimeProcessBinding::TenantSandbox
       └─ TenantSandboxProcessPort
            └─ dyn SandboxCommandTransport ────────► container (hardened: ro rootfs,
                 (Phase 2: persistent-env impl)        cap_drop ALL, no-new-privs)
                                                       ├─ /workspace   ← bind mount (scoped host dir)
first_party_tools/shell.rs                             ├─ /home/agent  ← named volume (PERSISTENT)
  └─ services.process.run_command(...)                 ├─ /tmp         ← tmpfs
                                                       └─ network: none
egress broker server (Phase 3)  ◄── unix socket ─────────  http(s)_proxy → broker socket
  ├─ domain allowlist (per NetworkMode profile)
  ├─ CONNECT-only TCP splice (no MITM, no credentials)
  └─ composes ironclaw_network resolver/SSRF filtering

promotion gate (Phase 4)
  artifact in /workspace → hash → wasm validate →
  manifest (deny-default) → ExtensionRegistry at
  user trust → approval gate for capability grants
```

---

## Phase 1 — Wire the existing transport into the Reborn binary

Smallest possible slice: hosted (and opt-in local) deployments get the
already-implemented ephemeral sandbox. No new behavior in the transport.

### 1.1 Config surface (`crates/ironclaw_reborn_config`)

Add a sandbox section to the Reborn config (follow the existing section pattern in
that crate):

```rust
/// Sandbox process-backend configuration. Present iff the deployment's
/// runtime policy can resolve to ProcessBackendKind::TenantSandbox.
#[derive(Debug, Clone, Deserialize)]
pub struct RebornSandboxSettings {
    /// Root under which per-scope workspaces are created
    /// (default: <data_dir>/sandbox/workspaces).
    pub workspace_root: Option<PathBuf>,
    /// Container image (default: existing IRONCLAW_REBORN_SANDBOX_IMAGE /
    /// IRONCLAW_SANDBOX_IMAGE env fallback, then "ironclaw-worker:latest").
    pub image: Option<String>,
    pub default_timeout_secs: Option<u64>,
    pub memory_bytes: Option<u64>,
    pub cpu_shares: Option<u32>,
    /// Container user (uid:gid) and workspace sharing mode.
    pub container_user: Option<String>,
}
```

Validation: `workspace_root` must be absolute when set. Do not add broker fields yet
(Phase 3 adds them); `RebornSandboxConfig` defaults to `disable_network = true`, which
is the correct Phase 1 posture — the ephemeral sandbox is network-dark.

### 1.2 Binding factory (`crates/ironclaw_reborn_composition`)

New module `src/process_binding.rs`, owned by composition because it bridges config →
host-runtime types (mirrors how `product_auth_runtime_credentials.rs` bridges auth):

```rust
/// Build the runtime process binding the resolved policy requires.
///
/// - Policy backend != TenantSandbox → Ok(RebornRuntimeProcessBinding::None)
///   (supplying a port when the policy doesn't use it fails validation —
///   see RebornRuntimeProcessBindingError::UnexpectedTenantSandboxProcessPort).
/// - Policy backend == TenantSandbox → connect Docker, build the transport,
///   wrap in TenantSandboxProcessPort. Docker unreachable is a hard error:
///   composition must fail loudly rather than silently degrade to no
///   process effects (error-handling rule: no silent fallback on IO).
pub async fn build_runtime_process_binding(
    runtime_policy: &EffectiveRuntimePolicy,
    settings: &RebornSandboxSettings,
    data_dir: &Path,
) -> Result<RebornRuntimeProcessBinding, RebornBuildError> {
    if runtime_policy.process_backend != ProcessBackendKind::TenantSandbox {
        return Ok(RebornRuntimeProcessBinding::none());
    }
    let workspace_root = settings
        .workspace_root
        .clone()
        .unwrap_or_else(|| data_dir.join("sandbox/workspaces"));
    let mut config = RebornSandboxConfig::new(workspace_root);
    if let Some(image) = &settings.image {
        config = config.with_image(image.clone());
    }
    if let Some(timeout) = settings.default_timeout_secs {
        config = config.with_default_timeout(Duration::from_secs(timeout));
    }
    // memory/cpu/container_user analogously; add the missing
    // RebornSandboxConfig::with_memory_bytes / with_cpu_shares builders
    // (currently only defaults exist at sandbox_process.rs:45-46).
    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .map_err(|e| RebornBuildError::InvalidConfig {
            reason: format!("tenant sandbox transport unavailable: {e}"),
        })?;
    Ok(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
        transport.into_process_port(),
    )))
}
```

Required small additions in `ironclaw_host_runtime/sandbox_process.rs`:
`with_memory_bytes`, `with_cpu_shares` builders (one-liners following `with_image`).

### 1.3 Call site (`crates/ironclaw_reborn_cli/src/commands/serve.rs`)

Where `serve.rs` assembles the services input (around the
`build_reborn_services(services_input)` calls at `serve.rs:1062/1089` and the
runtime build at `serve.rs:357`): resolve policy first (already available), call
`build_runtime_process_binding`, set it on the input. The existing
`validate_for_production_policy` (`input.rs:146`) then guarantees the
policy/binding pairing at build time — this is the property that makes
misconfiguration a startup error rather than a runtime surprise. `extension.rs` and
`runtime/mod.rs` composition sites pass `RebornRuntimeProcessBinding::none()` —
lifecycle commands never execute tenant processes.

### 1.4 Tests

- Unit (`process_binding.rs`): LocalHost policy → `None` binding; TenantSandbox
  policy with unreachable Docker → `InvalidConfig` error (not a silent `None`).
- Existing composition tests already cover binding/policy mismatch
  (`error.rs:99-109`, `approval_gates.rs:96`); extend
  `local_dev_approved_shell_uses_injected_tenant_sandbox_process_port` style tests to
  the production factory path.
- Integration (Docker-gated, `#[ignore]` without daemon): serve-composition smoke —
  hosted policy + real Docker → shell tool round-trips `echo ok` with
  `"sandboxed": true`.

**Exit criteria:** a hosted-profile Reborn binary starts, `builtin.shell` executes in
a scoped container, and the same binary with Docker stopped fails at startup with a
clear reason.

---

## Phase 2 — Persistent per-scope environment

### The lifecycle seam: `SandboxEnvironmentProvider`

The lifecycle abstraction is introduced at the *start* of Phase 2, not as a 2b
afterthought — both sub-phases are providers behind it, and future backends
(SmolVM/SRT) implement it instead of `SandboxCommandTransport` directly.

New module `sandbox_process/environment.rs`:

```rust
/// Lifecycle provider beneath the command transport. Backend implementors
/// (Docker now; SmolVM/SRT later) implement this instead of
/// SandboxCommandTransport directly.
#[async_trait]
pub trait SandboxEnvironmentProvider: Send + Sync {
    /// Create-or-attach the scope's environment; idempotent.
    async fn acquire(&self, scope: &ResourceScope)
        -> Result<SandboxEnvironmentHandle, RuntimeProcessError>;
    /// Execute inside the environment (docker exec; vsock for microVMs).
    async fn exec(&self, handle: &SandboxEnvironmentHandle, req: CommandExecutionRequest)
        -> Result<CommandExecutionOutput, RuntimeProcessError>;
    /// Stop (not destroy) environments idle longer than max_idle.
    async fn reap_idle(&self, max_idle: Duration) -> Result<(), RuntimeProcessError>;
    /// Destroy environment AND persistent state (project deletion only).
    async fn remove(&self, scope: &ResourceScope) -> Result<(), RuntimeProcessError>;
}

/// Generic adapter: any environment provider is a command transport.
pub struct EnvironmentBackedCommandTransport<P: SandboxEnvironmentProvider> { /* … */ }

#[async_trait]
impl<P: SandboxEnvironmentProvider> SandboxCommandTransport
    for EnvironmentBackedCommandTransport<P>
{
    async fn run_command(&self, req: CommandExecutionRequest)
        -> Result<CommandExecutionOutput, RuntimeProcessError>
    {
        let handle = self.provider.acquire(&req.scope).await?; // create-or-attach
        self.provider.exec(&handle, req).await
    }
}
```

**Provider selection is internal to composition — there is no environment-mode
config.** 2a ships a per-command provider (each `acquire` creates a container, each
`exec` runs and removes it — current transport behavior restated as a provider,
plus the volume); 2b swaps a warm-container provider in underneath once it soaks.
Policy, config, tools, and the conformance suite see one behavior ("persistent
sandbox environment") throughout. Rationale: 2a-vs-2b is rollout sequencing, not a
user-meaningful choice; a wire-stable mode enum would be permanent (aliases,
migration tests, a doubled test matrix) for a distinction we intend to delete.

### 2a — Persistent home volume + ceilings

**Step 0 — decompose first.** `sandbox_process.rs` is at 945 lines and these
changes would push it past 1,000. Before adding anything: move the Docker
transport impl into `sandbox_process/transport.rs`, leaving `sandbox_process.rs`
as config + module wiring + re-exports (it already has `broker`/`mounts`/
`scope_key` submodules; this completes the pattern).

Then: add a per-scope named volume mounted at `/home/agent`. Installed CLIs persist
because they live in `$HOME`; container lifecycle is unchanged.

**`sandbox_process/scope_key.rs`** — extend `RebornSandboxScopeKey` with:

```rust
/// Docker named-volume identity for this scope's persistent home.
/// Same sanitized identity material as container_name_prefix(); volumes
/// and containers must never diverge on scope derivation (invariant I5).
pub fn home_volume_name(&self) -> String {
    format!("ironclaw-reborn-home-{}", self.identity_slug())
}
```

(Refactor the existing `container_name_prefix()` internals into a shared
`identity_slug()` so both call one sanitizer.)

**`sandbox_process.rs`** — config + launch changes:

```rust
pub struct RebornSandboxConfig {
    // ...existing...
    /// Mount a per-scope named volume at /home/agent and run commands with
    /// HOME=/home/agent. Off = current stateless behavior.
    persistent_home: bool,
    /// Hard ceiling for pids inside the container (always applied).
    pids_limit: i64,             // default 512
    /// Soft disk ceiling for the home volume, metered host-side.
    home_disk_limit_bytes: u64,  // default 10 GiB
}
```

In `container_launch_config`:

- `binds.push(format!("{}:/home/agent:rw", self.config.home_volume_for(scope)))` —
  Docker auto-creates named volumes on first use; no explicit create call needed.
- `host_config.pids_limit = Some(self.config.pids_limit)` (do this even for
  non-persistent mode; it's a strict improvement).
- env: `HOME=/home/agent`, and a `PATH` that prepends
  `/home/agent/.local/bin:/home/agent/.npm-global/bin:/home/agent/.cargo/bin`.
  These names are reserved like the broker env keys — extend the
  `RESERVED_BROKER_ENV_KEYS` rejection pattern (`broker.rs`) so `extra_env`
  cannot override `HOME`/`PATH`.

**Disk ceiling — metered host-side, never in-container.** Named volumes have no
native quota on the default `local` driver. The volume's contents live on the host
(`docker volume inspect` → `Mountpoint`); a periodic host-side task (tokio interval
owned by the environment provider, alongside the 2b reaper) walks the mountpoint and
records usage per scope. Once over the ceiling, the provider refuses new commands
with a typed `RuntimeProcessError::ExecutionFailed("sandbox home over disk limit …")`
instructing cleanup. Two things this deliberately avoids: (a) per-command metering
latency — walking a multi-GiB `node_modules` tree is seconds of I/O, not a cheap
pre-flight; (b) **trusting the prisoner** — an in-container `du` exec would resolve
through the agent-controlled `PATH` (which prepends `~/.local/bin`), so a shadowed
`du` defeats the ceiling. Enforcement data must never originate inside the
assumed-compromised environment. This is honest soft enforcement; hosted GA
hardening (xfs project quotas or a quota-capable volume driver) is recorded as an open
item, not silently assumed.

**Base image.** Extend the worker image (`crates/Dockerfile.sandbox` lineage) with:
node LTS + npm configured for `~/.npm-global`, python3 + uv, rust toolchain, git,
ripgrep, build-essential, `agent` user (uid 1000) with writable-volume `$HOME`.
Default `container_user` becomes `1000:1000` with
`RebornSandboxWorkspaceMode::Private` when `persistent_home` is on.

**Volume lifecycle.** Volumes are created lazily and deleted only on explicit scope
teardown (project deletion). Add `RebornScopedSandboxCommandTransport::remove_scope_environment(scope)`
for the product-level deletion flow to call. Never reap volumes on a timer — they are
the persistence.

### 2b — Warm containers with exec (latency + intra-run processes)

Motivation: per-command container create/start costs 100–500 ms and kills any
background process between commands (`npm run dev &` then `curl localhost` as two tool
calls cannot work). 2b runs one long-lived container per scope and `docker exec`s
commands into it — a second `SandboxEnvironmentProvider` impl that composition swaps
in for the 2a per-command provider.

Docker implementation notes:

- Container entrypoint: `sleep infinity` (image has no tini requirement; exec'd
  commands are individually reaped because each `docker exec` is its own process
  tree — but add `init: true` to `HostConfig` so PID 1 reaps zombies from
  backgrounded processes).
- `acquire`: inspect by deterministic name `ironclaw-reborn-env-{slug}`; if absent →
  create with the Phase 2a hardened config (same binds, volume, limits, network
  mode); if stopped → start; record `last_used` in an in-memory
  `HashMap<RebornSandboxScopeKey, Instant>` behind `RwLock` (cache only — Docker
  state is the source of truth; on host restart the map rebuilds from
  `acquire`-on-demand).
- `exec`: `create_exec` with per-command env (broker vars, `extra_env` re-validated),
  workdir resolution reusing `resolve_container_workdir`, stream capped output
  reusing `append_with_limit`.
- **Timeout = restart the container, never in-container process management.** Docker
  has no native exec-kill, and any in-container mechanism (`setsid` + `pkill` on a
  process group) is enforcement tooling running inside the assumed-compromised
  environment with an agent-controlled `PATH` — a shadowed `pkill` defeats it. On
  `timeout_secs` expiry the provider issues `docker restart` (host-side, certain):
  the whole process tree dies, the home volume persists, the next `acquire` finds the
  container running again in hundreds of milliseconds. This satisfies the
  `SandboxCommandTransport` cleanup contract (`process_port.rs:111-113`), is locked
  decision 1 applied ("compute is disposable"), and ports to microVMs as "kill the
  VM" — which a pgid trick does not. Cost: a timeout also kills unrelated background
  processes in the same environment; acceptable, since a scope that times out is
  misbehaving by definition.
- `reap_idle`: tokio interval task started by the composition factory; `docker stop`
  (not remove) idle containers. Stopped containers restart in ~hundreds of ms on next
  acquire; background processes are documented to live only within the idle window.
- Concurrency: per-scope `tokio::sync::Mutex` around acquire (two simultaneous tool
  calls must not race create); execs themselves run concurrently.

Rollout: composition constructs the per-command provider until 2b soaks, then the
warm provider — an internal swap with no config migration, no wire-stable enum, and
one conformance suite (written against `dyn SandboxEnvironmentProvider`) covering
whichever provider is live.

### Phase 2 tests

- Scope-key unit tests: volume/container/workspace names share one slug; distinct
  tenants with identical user/project strings get distinct names (the exact
  collision `sandbox_process.rs`'s module doc warns about).
- Reserved-env tests: `extra_env` cannot override `HOME`/`PATH` (extend the
  `broker_env_rejects_all_reserved_user_overrides` pattern).
- Docker-gated integration (written against `dyn SandboxEnvironmentProvider` so any
  provider — and any future backend — inherits them): (1) `npm config set prefix` +
  `npm install -g` in one command, binary runnable from a *separate* command;
  (2) state survives `reap_idle` + reacquire; (3) two scopes cannot read each
  other's home; (4) pids ceiling refuses work; host-side disk metering flips a
  scope to refusing once over the ceiling; (5) timeout restarts the environment
  and the next command still finds the home volume intact.

**Exit criteria:** a hosted agent can `npm i -g @some/cli` in one turn and use the CLI
in a later run after a host restart, inside a container that still has read-only
rootfs, no caps, no ambient network, and enforced pids/disk/memory ceilings.

---

## Phase 3 — Egress broker for the persistent environment

The transport's broker plumbing (env vars, socket binds) is built but dangling: nothing
serves the socket. This phase builds the host-side broker server and the allowlist
profiles that make package installation possible without granting open network.

### 3.1 Broker server (`crates/ironclaw_host_runtime/src/sandbox_broker/`)

**The broker is CONNECT-only.** It tunnels TLS to allowlisted hosts and does nothing
else: no plain-HTTP forwarding, no credential resolution, no dependency on the
`egress/` pipeline or the obligations store. Rationale: every supported package
registry is HTTPS, and credentialed API calls from inside the sandbox are not the
model — the agent calls credentialed APIs via the first-party `http` capability on
the host, where staged-obligation injection applies (I2). Keeping the broker
credential-free means a compromise of the broker process yields a domain-allowlisted
TCP splicer, not a secret-bearing pipeline. It also avoids building a second
HTTP-forwarding/injection proxy alongside the v1 `src/sandbox/proxy/` (architecture
rule: no duplicate dispatch pipelines) — the only shared concept left is allowlist
matching, which moves to its canonical home (3.2).

```
sandbox_broker/
├── mod.rs        # SandboxEgressBroker: per-scope unix listeners, task lifecycle
├── connect.rs    # HTTP CONNECT handling: policy check → TCP splice (no MITM)
└── policy.rs     # SandboxEgressPolicy: domain allowlist per NetworkMode profile
```

Design points:

- **Listener: one unix socket per scope**, `<data_dir>/sandbox/broker/<slug>.sock`,
  created at `SandboxEnvironmentProvider::acquire` and bind-mounted via the existing
  `with_network_broker_unix_socket` path. The listener's identity *is* the scope —
  no in-container token, nothing to steal, and `network_mode: none` is preserved on
  the container (the socket is the only hole; see existing test
  `unix_socket_network_broker_preserves_none_network_mode_and_mounts_socket`).
  Rejected alternative: a single shared socket with a per-environment scope token in
  env — adds a stealable credential and host-side token bookkeeping for no gain.
  Socket files are removed when the environment is reaped/removed.
- **CONNECT semantics**: for `CONNECT host:port`, consult
  `SandboxEgressPolicy::decide(scope, host, port)`; on allow, splice a TCP tunnel
  (TLS terminates at the destination — the broker does not MITM and cannot see or
  modify payload). Non-CONNECT requests are rejected with `405` and audited. DNS
  resolution and private-IP/SSRF filtering go through `ironclaw_network`'s
  resolver primitives (`is_private_or_loopback_ip` denial already exists at
  `resolver.rs:58`) so the sandbox cannot reach link-local/RFC1918/metadata
  endpoints even for allowed-looking names, including via DNS rebinding (connect to
  the resolved-and-checked IP, not a re-resolved name).
- **Audit**: every decision (allow/deny, scope, host, byte counts out/in — preserve
  the `network_egress_bytes` outbound-only accounting invariant) emits through the
  existing host-runtime accounting path.

### 3.2 Allowlist profiles (`policy.rs`)

Typed, reviewable-in-one-place mapping from `NetworkMode` (already in
`EffectiveRuntimePolicy`) to a base domain set:

- `NetworkMode::Brokered` (HostedSafe): deny-all default; per-tenant additions only.
- `NetworkMode::Allowlist` (HostedDev/HostedYolo): base development set —
  `registry.npmjs.org`, `crates.io`, `static.crates.io`, `index.crates.io`,
  `pypi.org`, `files.pythonhosted.org`, `github.com`, `codeload.github.com`,
  `objects.githubusercontent.com`, `raw.githubusercontent.com`,
  `deb.nodesource.com` — plus per-tenant additions via product settings (additions
  are tenant-admin approved, normal settings flow).
- Wildcard matching: **move `DomainPattern`/`DomainAllowlist` from
  `src/sandbox/proxy/allowlist.rs` into `ironclaw_network`** as their canonical home;
  the v1 proxy consumes them from there (re-export shim during migration). This is a
  prerequisite step of this phase, not an option — it is the one concept the broker
  and the v1 proxy genuinely share, and duplicating it is architecture-rule smell #4.

### 3.3 Wiring

`build_runtime_process_binding` (Phase 1) grows: when
`runtime_policy.network_mode` is `Brokered`/`Allowlist`, start the
`SandboxEgressBroker` (composition owns its task lifetime alongside the reaper) and
configure the transport with `with_network_broker_unix_socket(per_scope_socket_dir)`.
The per-scope socket creation hooks into `SandboxEnvironmentProvider::acquire`.

### 3.4 Tests

- Policy unit tests: profile → decision matrix, private-IP/metadata denial even when
  DNS for an allowed name resolves there (rebind protection via `ironclaw_network`).
- Integration (Docker-gated): container with broker can `npm install` a real small
  package; non-CONNECT requests to the broker are rejected (`405`) so plain-HTTP
  egress is impossible by construction; same container cannot
  `curl https://example.com` (denied) nor reach
  `169.254.169.254`; broker socket per scope means scope A's socket absent in scope
  B's container.

**Exit criteria:** the persistent environment installs packages from the allowlisted
registries through the broker, everything else is denied and audited, and no secret
material is reachable from inside the sandbox.

---

## Phase 4 — Promotion gate: agent-built software becomes an extension

The promotion pipeline turns "bytes the (assumed-compromised) sandbox produced" into
"installed extension" with no implicit trust. Reborn's extension surface is
`ironclaw_extensions::ExtensionRegistry`/`SharedExtensionRegistry` (composed in
`host_runtime/production.rs:64`).

### 4.1 The pipeline

```
[sandbox]  agent builds wasm32-wasip2 artifact at /workspace/build/out.wasm
              │  (host can read it via the scoped workspace dir — I3 surface)
[host]     1. size ceiling check (10 MiB default) + BLAKE3 hash
           2. structural validation: parse module, required exports, WIT
              version compatibility (reuse/port src/tools/builder/validation.rs
              logic into a reborn-reachable home — see 4.4)
           3. manifest assembly: agent-PROPOSED capabilities recorded as
              *requested*, granted set starts EMPTY (deny-by-default)
           4. registration: ExtensionRegistry entry at user trust, phase
              NeedsActivation; artifact bytes + hash persisted
           5. capability grants: each requested capability (HTTP endpoints,
              tool-invoke aliases, secret-existence checks) raises an approval
              through the existing ApprovalPolicy machinery; user approves
              individually or rejects
[runtime]  promoted extension executes under the existing WASM capability
           sandbox: endpoint allowlist, fuel/memory limits, credential
           injection host-side, no secret reads
```

Hard rules encoded in the gate, not in prompts:

- Artifact path must resolve inside the requesting scope's workspace (reuse the
  workdir validation discipline from `resolve_container_workdir` — no `..`, no
  absolute host paths).
- Non-WASM artifacts are rejected at step 2 with a message pointing at decision 6
  (native binaries stay sandbox-local).
- Trust level is pinned: there is **no parameter** by which the promotion tool can
  request verified/system trust. Elevation is a separate human/operator workflow,
  out of band.
- The granted-capability set can only grow through approvals; re-promotion of a new
  artifact version resets grants (new hash = new trust decision).

### 4.2 First-party tool (`crates/ironclaw_host_runtime/src/first_party_tools/extension_promote.rs`)

Per host_runtime guardrails: one first-party tool file per capability. Tool surface:

```jsonc
// extension_promote
{
  "artifact_path": "/workspace/build/out.wasm",
  "name": "my_tool",                  // ExtensionName::new() validated
  "display_name": "My Tool",
  "description": "…",
  // Deserializes into the EXISTING manifest types — ironclaw_extensions'
  // ExtensionManifestV2 / CapabilityDeclV2 (v2.rs:369,435) — never a bespoke
  // parallel shape. The promotion service fills provenance and pins
  // ManifestHash (installations.rs:16) from the artifact bytes.
  "requested_capabilities": [ /* CapabilityDeclV2 wire shape */ ]
}
// → { "extension": "my_tool", "artifact_hash": "blake3:…",
//     "status": "registered_pending_grants",
//     "pending_approvals": ["http:api.example.com/v1"] }
```

Dispatch follows the `shell.rs` pattern: parse/validate params, delegate to a
`PromotionService` on `InvocationServices`. **No new runtime-policy axis.** The
`backends_for` matrix (`resolver.rs:270`) stays a 6-tuple: promotion availability is
a composition concern, not a policy dimension. The factory constructs the
`PromotionService` iff (a) Reborn config enables it (`extension_promotion_enabled`,
default `false`) and (b) the approval store and extension registry are composed;
when absent, `extension_promote` returns a typed "promotion disabled in this
deployment" error. The service is another optional-but-required-when-enabled
dependency, the same shape as the process port — and it carries the same
obligation: a `validate_for_*`-style composition check (config says enabled ⇒
service present) so misconfiguration is a startup error, never a silent no-op.
This guard is mandatory, not precedent-following — it is what makes the
`Option<Arc<…>>` pattern acceptable under the architecture rules.

The promotion is itself an approval-gated action: invoking the tool raises an
approval ("Install agent-built extension my_tool (blake3:…)?") before step 4 runs,
under the same approval store the composition already wires
(`LocalDevApprovalRequestStore` / production equivalents).

### 4.3 Registry persistence

`ExtensionRegistry` entries for promoted extensions persist via the composition's
existing store backends (filesystem for local-dev/libsql, postgres for production —
follow the dual-backend rule). Persist: manifest, artifact bytes, BLAKE3 hash,
requested vs granted capability sets, provenance record
`{ scope, run_id, promoted_at, artifact_hash }`. Verify hash on every load before
instantiation (the v1 `verify_binary_integrity` pattern from
`src/tools/wasm/storage.rs`).

### 4.4 Validation code home

`src/tools/builder/validation.rs` (WASM structural checks) lives in the v1 tree,
which reborn crates must not depend on. Extract the validator into a small crate
`crates/ironclaw_wasm_validate` (pure function of bytes → report; no IO), consumed by
both the v1 builder and the reborn promotion service. This follows the established
extraction pattern (safety/skills/llm) and avoids the duplicate-pipeline smell.

### 4.5 Tests

- Gate unit tests: path escape rejected; non-wasm bytes rejected; oversize rejected;
  trust pinned at user; grants start empty; re-promotion resets grants.
- Approval-flow integration: promote → approve install → extension registered but
  HTTP capability still denied → approve endpoint grant → capability callable —
  driven through the dispatch/approval call sites, not just the helpers (testing
  rule: test through the caller).
- End-to-end (Docker-gated, the demo that proves the whole plan): agent shell-builds
  a trivial wasm tool inside the persistent sandbox (toolchain from base image),
  promotes it, user approves, agent invokes the new extension in the same session.

**Exit criteria:** the only path from "sandbox bytes" to "host execution" is the gate;
every step of it is typed, audited, approval-bearing, and yields a user-trust WASM
extension running under the existing capability sandbox.

---

## Backend portability (how this stays "any sandboxing technology")

Adding `SmolVm`/`Srt`/`OrgDedicatedRunner` later requires exactly:

1. An impl of `SandboxEnvironmentProvider` (Phase 2 trait): acquire = boot microVM
   with persistent disk attached; exec = vsock/agent command channel; reap = pause or
   shutdown; remove = destroy VM + disk.
2. A `ProcessBackendKind` arm in `LocalInvocationServicesResolver::resolve` and a
   corresponding `RebornRuntimeProcessBinding` variant + factory branch.
3. The broker socket equivalent (vsock port instead of unix socket) behind the same
   `SandboxEgressBroker`.

Invariants I1–I6 are enforced in the broker, the obligations store, the promotion
gate, and the trait contracts — none of them are Docker-specific. The conformance
test suite from Phases 2–3 (isolation, persistence, allowlist, reserved env,
timeout-restart) is written against `dyn SandboxEnvironmentProvider` so a new
backend inherits its acceptance tests.

## Threat model summary

| Threat | Mitigation |
|---|---|
| Malicious package installed in persistent env (supply chain / prompt injection) | Env is assumed compromised: no ambient network (I1), no secrets inside (I2), scope-isolated state (I5), promotion gate (I4) |
| Exfiltration of workspace data | Egress only via broker allowlist; registries are the only reachable hosts in Allowlist mode; denials audited |
| Cross-tenant contamination via shared caches | Per-scope volumes/containers/sockets; no shared mutable state (I5) |
| Privilege escalation via promoted artifact | WASM-only, user trust pinned, deny-default capabilities, human approval per grant, hash-pinned re-promotion |
| Resource exhaustion (disk fill, fork bomb, runaway build) | memory/cpu (existing), pids_limit, host-side disk metering, timeout via environment restart (disposable compute — no in-container enforcement) |
| Container escape | Unchanged hardening: ro rootfs, cap_drop ALL, no-new-privileges, tmpfs /tmp; defense-in-depth accepted as Docker-grade until microVM backend lands |
| Broker SSRF (DNS rebind to metadata/RFC1918) | `ironclaw_network` resolver + private-IP filtering in the broker connect path |
| Secret theft via broker socket | Sockets are per-scope; the broker is CONNECT-only and contains no credential machinery at all — secrets exist only in the host-side egress pipeline (staged obligations), which the sandbox cannot reach |

## Sequencing & dependencies

```
Phase 1 (wiring)            — independent, ship first
Phase 2a (volume)           — depends on 1; internal provider, no config knob
Phase 2b (warm exec)        — depends on 2a
Phase 3 (broker)            — depends on 1; parallel with 2; required before
                              "install CLIs" is real (2a without 3 = persistent
                              but network-dark env)
Phase 4 (promotion)         — depends on 1; needs 2a+3 for the e2e story;
                              ironclaw_wasm_validate extraction can start any time
```

## Open items (tracked, not blocking)

1. Hard disk quotas for named volumes (xfs project quota / quota-capable volume
   driver) — host-side mountpoint metering is the interim, explicitly logged when
   enforced.
2. Background services with lifecycles beyond the idle window — belongs to
   `ironclaw_processes`, not this plan.
3. Base-image update policy (how often, who approves new system packages).
4. Per-tenant allowlist additions UX (product settings flow, Phase 3.2 hook exists).
5. `OrgDedicatedRunner` remote-runner transport (same traits, networked).
