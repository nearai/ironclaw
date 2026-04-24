# IronClaw Reborn — OS-Like Architecture Design

**Status:** Draft for review — local only, do not push  
**Date:** 2026-04-24  
**Authors:** Firat + Illia + pi

---

## 1. Purpose

Define a cleaner architecture for the IronClaw reboot where the host behaves more like a small operating system and most product behavior lives outside the kernel.

This design is based on:

- the earlier reboot direction: small core, language-agnostic extensibility, and strong boundaries
- the newer architecture sketch from Firat + Illia
- the desire to stop treating the agent loop, gateway, UI, and product logic as kernel internals

This document intentionally shifts the design away from a large “smart kernel” and toward a smaller host runtime with explicit system services.

---

## 2. Design goals

1. **Kernel stays small**  
   The kernel should mostly wire together system services and expose stable contracts.

2. **Everything useful is externalized**  
   Agent loops, gateway, TUI, and other product behavior should be extensions or first-party modules outside the kernel.

3. **Filesystem becomes the universal persistence surface**  
   State, config, threads, extension assets, and mounted backends should all be accessed through a filesystem abstraction.

4. **Processes are first-class runtime units**  
   Extensions are packages. Processes are live running instances. Threads are durable logical work records. These are different things.

5. **Security and multi-tenancy should not glue the system together**  
   Auth, network, sandboxing, secret access, and scoping should be system services, not extension-owned ad hoc logic.

6. **Language-agnostic execution remains a hard requirement**  
   Extension authors and agents must not be cornered into Rust or WASM-only implementation paths.

7. **Boundaries must be enforceable**  
   The design should be protected by narrow APIs, dependency direction, contract tests, and forbidden-import checks, not by team memory alone.

---

## 3. Architecture laws

These laws should be copied into any implementation plan and crate-level docs.

1. **ExtensionManager knows what can run.**  
   It discovers, validates, activates, and deactivates extension packages.

2. **ProcessManager knows what is running.**  
   It owns dispatch, spawn, process lifecycle, sandbox execution, and the process table.

3. **Filesystem knows what is durable.**  
   Durable config, state, threads, artifacts, and mounts are exposed through filesystem semantics.

4. **Kernel wires; it does not become a product runtime.**  
   Kernel composition should stay logic-light and product-agnostic.

5. **Extensions own product behavior.**  
   Agent loops, gateway, TUI, domain workflows, and integrations should live outside the kernel.

6. **Secrets are never ordinary config.**  
   Config may reference secret handles; raw secret material stays behind the auth/secret service.

7. **Outbound network is mediated.**  
   Extensions should not invent direct network paths that bypass `ironclaw_network` policy and audit hooks.

8. **Extension, Process, and Thread are distinct.**  
   Extension = packaged provider. Process = live instance. Thread = durable logical work record.

9. **Realtime events are not the audit log.**  
   The bus is for live updates and orchestration. Durable audit/history is written to filesystem-backed state.

10. **First-party privilege must be explicit.**  
    `agent_loop`, `gateway`, and `tui` can be privileged extensions, but privilege must be represented in manifests and policy, not hidden in special-case code.

11. **Every new abstraction must state what it does not own.**  
    This is the main defense against recreating the current blob with better names.

---

## 4. Top-level shape: OS/service model, not a forced 3-box model

This design should not be presented as a canonical 3-box architecture. The 3-box framing was useful while deciding what not to put in the kernel, but the Miro architecture is more concrete: it is a small host plus system-service crates plus extension userland.

```text
                 extensions/
+--------------------------------------------------------------+
| agent_loop_tools | agent_loop_codeact | gateway | tui | ...  |
| first-party and third-party executable userland              |
+-----------------------------↑--------------------------------+
                              |
                              | narrow host API:
                              | capabilities, config, mounts,
                              | dispatch/spawn, events, fs/auth/network
                              |
+-----------------------------|--------------------------------+
|                      ironclaw_kernel                         |
|--------------------------------------------------------------|
| host composition | boot | scope wiring | event bus wiring     |
| ExtensionManager initially lives here if kept narrow         |
+-----------------------------↑--------------------------------+
                              |
                              | composes system-service crates
                              |
        +---------------------+---------------------+
        |                     |                     |
+-------|------+      +-------|------+      +-------|------+
| filesystem  |      | processes    |      | auth         |
| mounts      |      | dispatch     |      | identity     |
| durable API |      | spawn        |      | secret refs  |
+-------↑------+      +-------↑------+      +-------↑------+
        |                     |                     |
        |              +------|-------+             |
        |              | network      |             |
        |              | egress/API   |             |
        |              +------↑-------+             |
        |                     |                     |
+-------|---------------------|---------------------|------+
|        mounted durable state + mediated external world       |
| /system | /users | /projects | /memory | remote APIs | ... |
+--------------------------------------------------------------+
```

The important architectural unit is not “box 1/2/3”. The important unit is the service boundary:

- **extensions/** = executable userland and product behavior
- **ironclaw_kernel** = composition, boot, scope wiring, and event bus wiring
- **ironclaw_filesystem** = durable path/mount API
- **ironclaw_processes** = live execution, dispatch/spawn, process lifecycle, sandbox execution
- **ironclaw_auth** = identity, credentials, secret handles, short-lived injection
- **ironclaw_network** = mediated outbound network
- **mounted state / external world** = storage and external effects behind service boundaries

This is closer to an OS design than an application-layer control-plane design. The kernel host should act like a small OS compositor around explicit services, not like a smart runtime that owns product behavior.

---

## 5. Core concepts

### 5.1 Extension

An **Extension** is a packaged capability provider.

It owns:

- manifest
- capability declarations
- executable entrypoints
- its own config/state/cache folders through the filesystem

It does **not** imply a running process.

### 5.2 Process

A **Process** is a live runtime instance of an extension or system task.

It owns:

- runtime identity (`ProcessId`)
- lifecycle state
- sandbox execution context
- scoped mounts, capabilities, and permissions for that run

A process may be:

- ephemeral request/response work
- a warm worker used for repeated dispatches
- long-running background work
- an interactive loop such as an agent loop

### 5.3 Thread

A **Thread** is a durable logical work record, not a process.

It owns:

- steps
- history
- artifacts
- progress and outcomes

Threads should live in filesystem-backed durable state, usually under mounted project or system paths.

A thread can outlive any one process.

### 5.4 Capability

A **Capability** is the dispatchable surface exposed by an extension.

It should contain:

- dispatch name
- parameter schema
- description
- permission scheme / gating policy hints
- declared filesystem, auth, network, and sandbox needs

Capabilities are discovered from extensions and registered by the kernel host.

### 5.5 Mount

A **Mount** is how durable state enters the system namespace.

Examples:

- local disk mount
- database-backed mount
- remote drive mount
- generated in-memory mount

The system should interact with these through the filesystem abstraction, not ad hoc storage APIs.

---

## 6. Crate layout

The proposed main crates are:

```text
crates/
  ironclaw_filesystem
  ironclaw_processes
  ironclaw_auth
  ironclaw_network
  ironclaw_kernel
```

`ExtensionManager` should stay explicit. It can begin as a narrow subsystem inside `ironclaw_kernel`, but it should become `crates/ironclaw_extensions` if extension discovery, manifests, lifecycle, or package management grow beyond simple host wiring.

### 6.1 `crates/ironclaw_filesystem`

This crate replaces the old `Workspace` abstraction.

#### Owns

- `Filesystem` trait
- path-oriented read/write/list/stat operations
- mounting local, DB-backed, and remote stores into a unified namespace
- extension folders, user folders, project folders, memory folders
- durable storage of threads, settings, and artifacts via mounted paths

#### V1 API

- `read(path)`
- `write(path, content)`
- `list(path)`
- `stat(path)`
- `mount(path, backend)`

#### Does not own

- search/indexing
- transactions beyond backend-local atomic writes
- subscriptions/reactivity
- auth policy
- secret material
- thread orchestration

If rich querying or indexing becomes necessary, add a separate indexing/query service on top of the filesystem instead of putting those semantics inside the filesystem trait.

### 6.2 `crates/ironclaw_processes`

This crate owns live execution.

#### Owns

- capability runtime representation
- `dispatch(name, params)` for request/response execution
- `spawn(name, params)` for background or long-lived execution
- `ProcessId`
- process table: `HashMap<ProcessId, Process>`
- process lifecycle state
- process-scoped capability, user, tenant, project, and workspace context
- sandbox execution mechanism
- process lifecycle event emission

#### Does not own

- extension discovery
- extension manifest validation as the source of truth
- thread persistence
- global auth policy
- global network policy
- routing/repair/reflection logic
- durable event storage

#### Recommended API

```text
dispatch(name, params, scope) -> Result<Value>
spawn(name, params, scope) -> Result<ProcessId>
status(process_id) -> Result<ProcessStatus>
await(process_id) -> Result<ProcessExit>
kill(process_id) -> Result<()>
subscribe(process_id) -> EventStream
```

`dispatch` and `spawn` should stay separate:

- **dispatch** = execute and return a result
- **spawn** = start tracked background/interactive work and return `ProcessId`

Internally, `dispatch` may use an ephemeral process or a warm worker pool. That is an implementation detail of `ironclaw_processes` and should not leak into extension APIs.

### 6.3 `crates/ironclaw_auth`

This crate handles authentication and credential management.

#### Owns

- identity to external services
- user or service credentials
- token flows / OAuth helpers
- secret handles and secret resolution
- short-lived secret injection for process execution
- revocation and rotation hooks

#### Does not own

- every authorization decision in the system
- custom network routing
- sandbox execution
- product-specific setup UI

This crate should be about credential and identity plumbing, not every policy decision.

### 6.4 `crates/ironclaw_network`

This crate is the network mediation boundary.

#### Owns

- outbound network API
- allowlists or mediated egress policy hooks
- shared transport behavior for extensions and system services
- optional proxying / request shaping / audit hooks

#### Does not own

- extension capability routing
- auth token storage
- sandbox execution
- product-specific API clients unless they are generic adapters

This crate should be the place where network effects are mediated, not just a bag of HTTP helpers.

### 6.5 `crates/ironclaw_kernel`

This crate composes the system.

#### Owns

- wiring between extension manager, filesystem, processes, auth, and network
- stable system contracts shared across services
- user/tenant/project scope wiring
- high-level host startup
- event bus composition

#### Must not become

- the new god crate
- the place where product behavior silently accumulates
- a second agent runtime full of business logic
- a policy dumping ground

The kernel should be composition-heavy and logic-light.

---

## 7. Dependency direction

The intended dependency direction is:

```text
extensions -> host interface/contracts
ironclaw_kernel -> filesystem + processes + auth + network + extension manager
ironclaw_processes -> filesystem contracts + auth/network/sandbox interfaces as needed
ironclaw_auth -> filesystem contracts
ironclaw_network -> auth handles only when explicitly injected
ironclaw_filesystem -> no other system-service crates
```

Hard rules:

- `ironclaw_filesystem` must not depend on product extensions.
- `ironclaw_processes` must not depend on extension discovery internals.
- Extensions must not import kernel internals directly.
- First-party extensions must use the same host API shape as third-party extensions, with explicit privilege levels.

---

## 8. Extension manager

The architecture sketch puts `Extension => ExtensionManager` at the top. That is correct and should stay explicit.

### Owns

- extension discovery under `/system/extensions/...`
- manifest loading and validation
- capability extraction from extensions
- activation/deactivation
- registration into the process runtime
- extension-owned config/state/cache folder setup
- manifest-level trust class and compatibility checks

### Should not own

- sandbox policy
- auth policy
- network policy
- long-lived process table
- routing decisions
- thread persistence

A clean separation is:

- **ExtensionManager** knows what can run
- **ProcessManager** knows what is running

---

## 9. Extension host API

Extensions should receive a narrow host API instead of direct access to internal crates.

Suggested host API shape:

```text
host.dispatch(name, params)
host.spawn(name, params)
host.fs.read(path)
host.fs.write(path, content)
host.fs.list(path)
host.events.publish(event)
host.events.subscribe(scope)
host.auth.resolve(handle)
host.network.request(spec)
host.threads.append_step(thread_id, step)
```

This API should be scoped by user/tenant/project/process context.

The goal is to keep first-party extensions from becoming “extensions in name only.” If `agent_loop` needs privileged access, it should receive privileged host API permissions through manifest policy, not direct imports into kernel internals.

---

## 10. Event model

The current sketch places the event bus near the process manager. That is partially right, but the bus should not be a purely process-owned subsystem.

### 10.1 Split realtime events from durable audit/history

Use two concepts:

1. **Realtime event bus**
   - ephemeral
   - useful for UI, gateway, progress, and orchestration
   - not the source of truth

2. **Durable audit/history**
   - written into filesystem-backed state
   - used for replay, audit, debugging, and learning
   - can be derived from or fed by realtime events, but is not identical to them

### 10.2 Event classes

Define event classes early:

- **runtime events** — process started/stopped/output, sandbox events
- **domain events** — thread step added, mission created, workflow progress
- **audit events** — secret accessed, approval granted/denied, network call made
- **extension lifecycle events** — installed, activated, disabled, upgraded

### 10.3 Ownership

- `ironclaw_processes` is a major producer of runtime events.
- `ironclaw_kernel` composes the shared realtime event bus and event contracts.
- durable audit/history is stored through `ironclaw_filesystem` under a scoped namespace.

---

## 11. Where the processes live

Processes live in `ironclaw_processes`.

That crate should own the runtime process table and lifecycle.

Processes do **not** live in:

- `ExtensionManager`
- filesystem state
- thread history

Clean separation:

- **Extension** = packaged capability provider
- **Process** = live running instance
- **Thread** = durable logical work record

These three should not be collapsed into one abstraction.

---

## 12. Agent loop placement

The new sketch moves the agent loop into `extensions/agent_loop/`. That is a strong move and should be preserved.

Recommended structure:

```text
extensions/
  agent_loop_tools/
  agent_loop_codeact/
  gateway/
  tui/
```

This means:

- the kernel is not the agent loop
- the gateway is not the kernel
- the TUI is not the kernel
- multiple agent loop strategies can coexist

The agent loop should be a first-party privileged extension that uses:

- filesystem state
- process dispatch/spawn
- auth/network services
- capability registration
- thread append APIs

It should write durable thread/step state into the mounted filesystem, not hide that state inside its own runtime memory.

---

## 13. Filesystem-based configuration

The design proposes killing the current config system and moving to filesystem-based config, with each extension managing its own config in its own folder.

This is directionally correct, but it needs structure.

### 13.1 Extension folder model

```text
/system/extensions/<extension>/
  manifest.toml
  config/
  state/
  cache/
  bin/        # optional executable assets
```

### 13.2 Folder semantics

- **config** = durable user-controlled config
- **state** = durable extension-owned state
- **cache** = disposable generated state
- **manifest** = activation/capability metadata
- **bin** = executable assets if the extension ships local binaries/scripts

### 13.3 Schema and migration rule

Each extension config should have:

- schema version
- validation entrypoint or schema file
- migration path for config/state upgrades

Do not let “filesystem config” become 50 untyped mini-config systems.

### 13.4 Secret rule

Do **not** silently store raw secrets in ordinary config files.

Config may reference:

- secret handles
- auth identifiers
- external account IDs

Raw secret material should be mediated by `ironclaw_auth`.

---

## 14. Security, network, sandboxing, secrets, and tenancy

The OS-like architecture still needs hard boundaries, but they should be system services, not kernel bloat.

### 14.1 Sandboxing

Sandboxing belongs under `ironclaw_processes` as execution mechanism.

That crate should know:

- how to run isolated code
- how to apply a sandbox profile
- how to terminate and clean up isolated processes

It should not be the place where every policy decision is made.

### 14.2 Secrets

Secrets should be managed by `ironclaw_auth`, not scattered through config files or extension-owned storage.

Recommended model:

- config references secret handles
- `ironclaw_auth` resolves and injects secrets at process runtime
- raw secret material is not treated as normal filesystem config
- secret use emits durable audit events

### 14.3 Network

All outbound network should flow through `ironclaw_network`, not arbitrary extension bypass paths.

### 14.4 Tenant and user scoping

User, tenant, and workspace scope should be wired by the kernel and enforced by system services, then passed into process execution.

Every runtime-scoped operation should carry enough scope to prevent accidental cross-user or cross-tenant leakage:

- user
- tenant or account when applicable
- project/workspace
- process id
- extension id

This keeps extensions from inventing their own scoping rules.

---

## 15. Capabilities and permission schemes

The sketch suggests capability metadata such as:

- list of dispatch names
- dispatch arguments and descriptions
- text file references
- permission scheme such as:
  - `GoingAsk`
  - `Depend on arguments`
  - `Approved`

This is promising, as long as it remains declarative.

Capability metadata should describe:

- what the capability does
- what parameters it takes
- what permission profile it suggests
- what paths or mounts it intends to touch
- what network/auth services it expects
- whether it requires foreground dispatch or background spawn
- whether it can stream updates

The kernel and system services should enforce policy; capability declarations should not be trusted as enforcement by themselves.

---

## 16. Recommended filesystem namespace

A namespace like this is a good starting point:

```text
/system/
  extensions/
  auth/
  settings/
  capabilities/
  events/

/users/
  <user>/
    memory/
    auth/
    settings/

/projects/
  <project>/
    threads/
    artifacts/
    settings/
    events/

/memory/
  ... mounted backend if needed ...
```

This keeps durable state visible and inspectable while still allowing different backends underneath via mounts.

---

## 17. Boundary enforcement and CI guardrails

The architecture should be enforced mechanically.

### 17.1 Dependency checks

Add checks that prevent forbidden imports, for example:

- `ironclaw_filesystem` cannot depend on product extensions
- `ironclaw_processes` cannot depend on extension discovery internals
- extensions cannot import host internals directly
- outbound HTTP helpers outside `ironclaw_network` are banned or flagged
- raw secret file reads outside `ironclaw_auth` are banned or flagged

### 17.2 Contract tests

Add tests for:

- extension manifest validation
- extension config schema validation
- dispatch vs spawn behavior
- process lifecycle cancellation
- sandbox profile enforcement
- network mediation
- secret handle resolution and redaction
- tenant/user scope propagation

### 17.3 Architecture docs as ratchets

Each core crate should include a short crate-level doc section:

- owns
- does not own
- allowed dependencies
- forbidden dependencies

This prevents future contributors and agents from guessing.

---

## 18. Risks and mitigations

### 18.1 `ironclaw_processes` becoming the new blob

Risk: it absorbs capabilities, orchestration, sandboxing, routing, events, and policy.

Mitigations:

- crate-level “does not own” section
- no extension discovery imports
- no global auth/network policy
- no thread persistence semantics
- narrow dispatch/spawn/process lifecycle API

### 18.2 `ironclaw_kernel` becoming misc glue

Risk: “puts it all together” becomes “everything complicated goes here.”

Mitigations:

- kernel stays composition-only
- smart logic moves to explicit services or extensions
- no product workflows in kernel

### 18.3 filesystem abstraction growing too wide too fast

Risk: `ironclaw_filesystem` becomes POSIX + SQL + object store + search + reactive indexer.

Mitigations:

- V1 API limited to read/write/list/stat/mount
- indexing/querying lives in separate services
- watch/subscription semantics postponed until needed

### 18.4 secrets leaking into extension config

Risk: filesystem config convenience causes raw secrets to land in config folders.

Mitigations:

- config supports secret handles only
- raw secrets mediated by `ironclaw_auth`
- redaction tests and audit events required

### 18.5 auth/network bypasses

Risk: extensions directly open sockets or read credentials.

Mitigations:

- mediated host API only
- forbidden-import/forbidden-call checks
- sandbox profiles that can deny direct network

### 18.6 first-party extensions become special-case internals

Risk: `agent_loop`, `gateway`, and `tui` become extensions only on paper.

Mitigations:

- explicit trust classes in manifests
- same host API shape for first-party and third-party extensions
- privileged APIs are declared, scoped, and audited

### 18.7 event semantics diverge

Risk: UI events, audit logs, process events, and thread history become separate half-overlapping systems.

Mitigations:

- define event classes early
- separate realtime bus from durable audit/history
- store durable history through filesystem namespace

---

## 19. V1 implementation constraints

V1 should be intentionally narrow.

Choose exactly:

- one extension protocol: stdio JSON or equivalent simple process protocol
- one sandbox backend
- one local filesystem backend
- one event bus format
- one durable audit/history format
- one first-party agent loop extension
- one gateway extension
- one TUI extension

Do not build:

- full extension marketplace
- multiple sandbox backends
- every filesystem backend
- hot reload if it muddies lifecycle
- multiple competing config formats
- automatic repair/evolution mechanics

The goal of V1 is to prove the OS-like shape, not recreate all current IronClaw behavior.

---

## 20. V1 implementation order

1. **`ironclaw_filesystem`**
   - define the `Filesystem` trait
   - implement local mount
   - define mount table and minimal namespace

2. **ExtensionManager**
   - manifest format
   - extension discovery under `/system/extensions`
   - capability extraction
   - config/state/cache folder contract

3. **`ironclaw_processes`**
   - `dispatch`
   - `spawn`
   - `ProcessId`
   - process table
   - simple sandbox profile hook
   - process lifecycle events

4. **event model**
   - realtime bus
   - durable audit/history path
   - runtime/domain/audit event classes

5. **`ironclaw_kernel` composition**
   - wire filesystem + extension manager + process manager
   - wire auth/network service handles
   - wire event bus

6. **first-party extensions**
   - `extensions/agent_loop_tools`
   - `extensions/gateway`
   - `extensions/tui`

7. **`ironclaw_auth` and `ironclaw_network` mediation**
   - make auth and network explicit services
   - move extension runtime off implicit access paths

This sequencing preserves the OS-like shape early instead of reintroducing product logic too soon.

---

## 21. Final recommendation

This revised architecture is stronger than both the earlier “smart kernel” direction and the forced 3-box framing.

It keeps the most valuable properties:

- small kernel host
- explicit system-service crates instead of a vague middle box
- first-party extensions for agent loop, gateway, and TUI
- filesystem as the primary persistence surface
- clear separation between extension, process, and thread
- enforceable architecture boundaries

The main discipline required is this:

- **ExtensionManager** knows what can run
- **ProcessManager** knows what is running
- **Filesystem** knows what is durable
- **Kernel** wires the system together
- **Auth** owns credential and secret mediation
- **Network** owns outbound network mediation
- **Extensions** own product behavior

If those boundaries are enforced by APIs and tests, this design has a real chance to stay simpler and less brittle than the current IronClaw architecture.
