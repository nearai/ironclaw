# `crates/lanes/` — execution mechanisms

**Layer(s):** `runtimes` · **Crates:** 4 — `ironclaw_wasm`, `ironclaw_wasm_limiter`, `ironclaw_mcp`, `ironclaw_sandbox` · **Security posture:** deny-by-default execution mechanisms that receive only sealed, already-authorized work and host-mediated services; a lane can fail closed but can never authorize itself more than the kernel already granted, and it never holds an ambient network, secret, or filesystem handle of its own.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/lanes/
├── ironclaw_wasm              WASM component lane
├── ironclaw_wasm_limiter      shared wasmtime resource limiter
├── ironclaw_mcp               MCP lane over host-mediated HTTP
└── ironclaw_sandbox           sandboxed process lane
```

## Role

`crates/lanes/` is how an already-authorized invocation runs. Every lane receives work only after it has crossed the kernel membrane carrying a sealed `Authorized` witness already bound to exactly one `RuntimeLane` — a closed, fixed set of variants (`FirstParty`, `Wasm`, `Mcp`, `Process`) selected once by runtime-policy planning and never re-derived inside the lane itself. A lane's job is narrow by design: load or connect to the already-selected execution mechanism, run the request under mediated services — scoped-down mounts, one-shot staged secrets, policy-scoped egress — and hand back a normalized outcome or a bounded, host-visible failure class. A lane never decides whether work is allowed; that decision is made and sealed before the lane ever sees the request.

## Boundaries — what makes this family distinct

- **vs `kernel/`:** the kernel decides — trust ceiling, default-deny grant matching, exact-invocation approval, obligation preparation, and the sealed `Authorized` witness are all kernel work. `lanes/` executes what the kernel already decided and can only refuse to run — an unconfigured lane fails closed — never grant itself additional authority. This is the family's authorized-before-execution contract, stated once for the whole family: no lane may run before authorization arrives sealed, and no lane may widen what it was authorized for once it does.
- **vs `extensions/`:** extensions own what an installable package *is* — its manifest, registry entry, and installation record, plus vendor-specific adapter behavior. `lanes/` owns *how* code of a given kind actually executes — a WASM store, an MCP JSON-RPC client, a sandboxed process — regardless of which package asked for it; a lane has no notion of "Slack" or "Discord," only "WASM component" or "MCP server." Extensions name the mechanism a package wants; lanes are the mechanism itself, shared across every package that names it.
- **vs `loop/`:** the loop tier hosts replaceable agent strategy — the userland code deciding what capability to invoke next — behind typed loop ports, and is never trusted with authority on its own. `lanes/` executes already-decided, already-authorized invocations; a lane never sees why a request was made, only "run this, under these mediated services, and report back." Loop chooses; lanes run what was chosen, only after the kernel — never the loop — has authorized it.

## What belongs here / What never belongs here

**Belongs:** runtime loading, isolation, and metering for exactly the closed set of `RuntimeLane` variants; normalized outcomes and bounded, host-visible failure classes; the vendor SDKs a lane's own protocol genuinely requires — never a vendor SDK for something the lane merely dispatches to.

**Never belongs here:** authorization, approval, or trust-policy logic of any kind; ambient network or secret access — every credential and every egress path arrives by injection from the kernel's mediated-services layer, never as a lane-owned client or store; product behavior or presentation; a second, parallel lifecycle or supervisor — process lifecycle authority belongs to the kernel; vendor or product names — a lane executes "WASM" or "a sandboxed process," never "the GitHub tool" or "Slack."

## Dependency direction

Every lane depends on the neutral authority vocabulary crate and, where it touches extension-declared surface data, the neutral extension-surface vocabulary crate — never the extension registry crate itself. The WASM lane additionally depends on its sibling resource-limiter crate. Mediated services — secrets, network, filesystem, resources — arrive by injection from the kernel's host-runtime layer at construction time; a lane never adds a secrets, network, or filesystem crate as a dependency of its own. The resource-limiter crate is also consumed by the loop family's hook engine, which is legal because the loop tier sits above the lane tier in the dependency ladder. Nothing in the kernel is a normal dependency of any lane; the relationship inverts — the kernel's host-runtime layer depends on the lanes and selects among them through a closed lane executor.

## Security & authority

Lanes are where an authorized invocation turns into a lane call, and a lane call turns into safe evidence. The kernel's closed lane executor selects a lane by the sealed witness — an unconfigured lane always fails closed — hands the lane scoped-down mounts and one-shot staged secrets, and the lane executes under those grants alone. On return, the kernel's mediated-execution layer — never the lane — applies redaction, output-limit, and leak checks before anything becomes model-visible or durable evidence. The WASM lane makes "mediated services only" mechanical rather than aspirational: every host-import capability — HTTP, workspace, secrets, tools, clock — has a deny-by-default implementation, so a WASM component gets exactly the host capabilities composition explicitly wires and nothing by omission. The sandboxed-process lane carries the same guarantee for OS-process execution: the only real containment for a spawned process is the sandbox it runs in, a served multi-user deployment must never resolve to an unsandboxed host-process backend, and a missing execution backend must degrade to "no shell," never to a silently unsandboxed one.

## Crates

### `ironclaw_wasm`

- **Purpose:** the WASM component execution lane — load, compile, validate, meter, and execute an already-selected WASM component under deny-by-default host imports.
- **Owns:** component loading, compilation, and validation; fresh-store-per-call instantiation; fuel, epoch, memory, table, and instance limits; the host-import adapter surface (HTTP, workspace, secrets, tool invocation, clock) and its deny-by-default trait family; the domain-free WASM/WASI sandbox primitives shared with other runtime hosts; the canonical component-model interface definitions this lane executes against.
- **Never contains:** decisions about which tools or channels are exposed to the model; authorization, approval, trust, or dispatch-routing logic; direct production HTTP or secret retrieval outside the injected host-import seam.
- **Public surface:** the host-import trait family — `WasmHostHttp`, `WasmHostWorkspace`, `WasmHostSecrets`, `WasmHostTools`, `WasmHostClock` — each with a deny-by-default implementation; the generated component bindings over the canonical interface definition.
- **Depends on:** the neutral authority vocabulary crate; the neutral extension-surface vocabulary crate; the resource-limiter crate.
- **Never depends on:** the extension registry crate; any storage, secrets, or network crate directly; any product or kernel crate.
- **Security & authority role:** the family's clearest mediated-services example — every host capability is deny-by-default and must be explicitly wired by composition; fresh store per call plus fuel/epoch/memory ceilings bound a hostile component's blast radius before any host-import decision even matters.
- **Why a separate crate:** the WASM runtime's dependency cone and the genuine trust boundary of executing untrusted, model-selected component code both justify isolating this lane on their own — no other crate in the workspace needs a WASM engine, and this is the only one permitted to hold one.

### `ironclaw_wasm_limiter`

- **Purpose:** the resource-limiter implementation shared by every WASM host in the workspace, so the tool lane and the hook engine cannot silently diverge on limits.
- **Owns:** a single resource-limiter type tracking memory usage against a ceiling, plus table, instance, and memory-count limits, implementing the WASM runtime's resource-limiter interface.
- **Never contains:** anything host-specific — store setup, component loading, or bindings; those stay with each consumer.
- **Public surface:** the resource-limiter type and its construction and usage-accessor methods.
- **Depends on:** nothing internal.
- **Never depends on:** any other crate in the workspace — this crate's entire reason to exist is holding zero internal dependencies while sitting between two consumers that must not depend on each other.
- **Security & authority role:** enforces one half of the WASM trust boundary — resource ceilings — identically for every WASM host in the workspace, closing the door on two hosts silently diverging on limits.
- **Why a separate crate:** it is a single behavior shared by two otherwise-unrelated hosts, a tool lane and a hook engine; giving it its own crate makes the shared dependency an explicit, tooling-visible edge rather than a duplicated implementation neither host owns.

### `ironclaw_mcp`

- **Purpose:** the MCP execution lane — adapts manifest-declared MCP tools into capabilities over host-mediated HTTP only, with no ambient filesystem, secret, or network authority of its own.
- **Owns:** the MCP JSON-RPC adapter and its protocol-version handling; HTTP egress planning for MCP calls; discovered-tool-to-capability translation.
- **Never contains:** any direct, non-host-mediated networking — every outbound call is planned and executed through the injected HTTP-egress port, never a lane-owned client.
- **Public surface:** the MCP runtime and its configuration type; the HTTP client/egress-planner pair that composition wires with a concrete host-mediated transport.
- **Depends on:** the neutral authority vocabulary crate, including its resource-estimate and usage vocabulary; the neutral extension-surface vocabulary crate.
- **Never depends on:** the extension registry crate; the resource-governor crate directly — the governor arrives kernel-injected, not as a compiled dependency; any direct HTTP client crate.
- **Security & authority role:** proves the "host-mediated HTTP only" invariant in code — every outbound MCP call routes through an injected egress port, never a lane-owned client, so the lane cannot originate a connection the kernel has not mediated.
- **Why a separate crate:** it is a distinct protocol lane with its own discovery and JSON-RPC surface, and the per-lane external rule — a WASM engine only in the WASM lane, container machinery only in the sandbox lane — stays statable only while each lane is its own crate.

### `ironclaw_sandbox`

- **Purpose:** the sandboxed-process execution lane — a typed plan contract for what an OS-process invocation is allowed to do, and a container-backed execution backend that runs a validated plan behind the kernel's process-transport seam.
- **Owns:** the plan contract itself — a typed, validated description of an install phase and a credentialed-run phase, each with its own scoped mounts, network policy, and credential bindings, so a caller can never smuggle raw container flags, raw host paths, or raw secret material through plan input; the container-backed transport that executes a validated plan, including per-tenant container identity, a per-tenant certificate authority for egress interception whose root key never leaves memory and is never returned to a caller, a credential-firewall obligation-staging point that lets an invocation consume only the credential it was already entitled to, and the network/secret brokering that keeps a container's egress path host-mediated.
- **Never contains:** ambient credentials of any kind — every credential reaches a container only through the staged, one-shot obligation seam; direct process spawning outside the transport seam — every command this lane runs is a validated plan executed through the container backend, never a raw host-process invocation; the transport *port* itself, which is contracts vocabulary (`host_api`) this lane implements and the kernel consumes — never owns.
- **Public surface:** the plan and validated-plan types (`SandboxProcessPlan`/`ValidatedSandboxProcessPlan`, with typed install-plan, command-plan, mount, network-plan, and credential-binding sub-vocabulary); the container-backed implementation of the `SandboxCommandTransport` port — contracts vocabulary this lane implements and the kernel consumes.
- **Depends on:** the neutral authority vocabulary crate; the neutral extension-surface vocabulary crate, where a plan carries manifest-declared command metadata.
- **Never depends on:** the extension registry crate; any crate above the runtime tier.
- **Security & authority role:** carries the family's most detailed containment story. The only real containment for a spawned OS process is the sandbox it runs in — a virtual, scoped filesystem view does not contain a subprocess, only a real container boundary does. The per-tenant certificate authority's root key never touches disk and is never serialized to a caller; only its public trust anchor ever reaches a container. The credential firewall is a staging chokepoint by design, not a bypassable trait: a caller stages what an invocation is entitled to, and the consumer only ever sees a yes/no answer. A served, multi-user deployment must never resolve to an unsandboxed host-process backend, and a missing container backend must degrade to no shell at all, never to a silently unsandboxed one.
- **Why a separate crate:** the container and certificate-authority dependency cone is a genuinely different trust environment than the rest of the kernel service graph, and isolating it here keeps that cone — and the elevated review scrutiny it deserves — out of every other kernel-adjacent crate's build.

## Family AGENTS.md requirements

The family root's `AGENTS.md` states the lane contract as the governing law of the family: a lane accepts only a canonical, already-authorized invocation; it uses mediated services exclusively, never an ambient client or store; it returns a normalized outcome or a bounded failure class; it never runs a parallel or independent lifecycle. It also states the closed-lane-set rule: `RuntimeLane` is a closed, exhaustively-matched set of variants, and adding a lane is a contract change reviewed as one — never a registry entry a lane crate can add on its own. It states the family's dependency direction as a check: a lane depends only on the neutral authority and extension-surface vocabulary crates — plus the shared resource limiter, for WASM hosts — never on the extension registry, a substrate, or anything above; the kernel's host-runtime layer depends on the lanes, never the reverse. Every crate in the family ships both an `AGENTS.md` and a `CLAUDE.md` restating its own slice of that law, including which mediated service it receives by injection and which vendor SDK, if any, its own protocol genuinely requires.
