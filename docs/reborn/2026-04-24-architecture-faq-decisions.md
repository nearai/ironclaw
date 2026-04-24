# IronClaw Reborn — Architecture FAQ and Decision Log

**Status:** Draft for review — local only until merged  
**Date:** 2026-04-24  
**Participants:** Firat, Illia, pi  
**Related docs:**

- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-self-contained-crate-roadmap.md`
- `docs/reborn/2026-04-24-existing-code-reuse-map.md`
- `docs/reborn/2026-04-24-host-api-invariants-and-authorization.md`

---

## 1. Purpose

Record the reasoning and decisions behind the IronClaw Reborn architecture discussion.

This document is intentionally written as a FAQ/provenance log. It captures not only the current shape, but also the tradeoffs considered and the reasons behind the decisions.

---

## 2. What architecture are we choosing?

We are choosing an **OS-like host architecture**, not a forced 3-box application architecture.

The shape is:

```text
extensions/ userland
  -> ironclaw_kernel host composition
  -> system-service crates
  -> mounted durable filesystem state + mediated external world
```

The key system-service crates are:

```text
crates/ironclaw_host_api
crates/ironclaw_extensions
crates/ironclaw_filesystem
crates/ironclaw_resources
crates/ironclaw_wasm
crates/ironclaw_mcp
crates/ironclaw_scripts
crates/ironclaw_processes
crates/ironclaw_auth
crates/ironclaw_network
crates/ironclaw_kernel
```

First-party products such as agent loop, gateway, and TUI should live under `extensions/` rather than inside the kernel.

---

## 3. Why not keep the earlier 3-box model?

The earlier 3-box model was useful while brainstorming because it separated:

1. kernel
2. control/runtime
3. products/extensions

But once secrets, sandboxing, multi-tenancy, channels, filesystem persistence, and process management were introduced, the middle box became too vague.

The Miro/OS-like model is better because it names concrete system services:

- filesystem
- processes
- auth
- network
- extension manager
- kernel composition

This reduces the risk of a giant undefined “middle layer” becoming the new monolith.

**Decision:** Do not present the architecture primarily as 3 boxes. Present it as an OS-like host with explicit system-service crates and extension userland.

---

## 4. What does the kernel own?

`ironclaw_kernel` owns composition and wiring.

It wires together:

- extension manager
- filesystem
- resource/budget governor
- WASM/MCP/script runtime lanes
- process manager
- auth
- network
- event bus
- user/tenant/project scope

It should not own:

- agent loop behavior
- gateway behavior
- TUI behavior
- product workflows
- routing heuristics
- repair/self-learning logic
- extension business logic

**Decision:** Kernel is composition-heavy and logic-light. If product behavior starts accumulating in `ironclaw_kernel`, the architecture is failing.

---

## 5. Where does `ExtensionManager` live?

`ExtensionManager` should live in `crates/ironclaw_extensions` from day one.

It owns:

- extension discovery
- manifest loading and validation
- capability extraction
- activation/deactivation
- extension folder setup
- registration into the process runtime

It does not own:

- the process table
- sandbox policy
- auth policy
- network policy
- routing decisions
- thread persistence

`ironclaw_kernel` composes this crate; it should not parse manifests or own extension discovery directly.

**Decision:** Keep `ExtensionManager` explicit and narrow in `crates/ironclaw_extensions`. It knows what can run; it does not know what is currently running.

---

## 6. Where do processes live?

Processes live in `crates/ironclaw_processes`.

This crate owns:

- `ProcessId`
- `Process`
- process table
- `dispatch`
- `spawn`
- process lifecycle
- process-scoped sandbox execution

Processes do not live in:

- `ExtensionManager`
- filesystem state
- thread history

**Decision:** Keep three concepts separate:

```text
Extension = packaged capability provider
Process   = live runtime instance
Thread    = durable logical work record
```

Collapsing these concepts is a major architecture risk.

---

## 7. Why split `dispatch` and `spawn`?

`dispatch` and `spawn` have different semantics.

### `dispatch`

- request/response
- execute and return a result
- may use an ephemeral process or warm worker internally
- caller does not manage a `ProcessId`

### `spawn`

- background, long-running, streaming, or interactive work
- returns `ProcessId`
- caller can check status, subscribe, await, stop, or kill

**Decision:** Keep both APIs. Do not overload one process API to mean both immediate execution and background lifecycle management.

---

## 8. Where does sandboxing live?

Sandboxing is part of `ironclaw_processes` because it is execution mechanism.

`ironclaw_processes` should know:

- how to start isolated execution
- how to apply sandbox profiles
- how to kill and clean up isolated processes
- how to enforce time/memory/output/process limits

It should not own all policy decisions about what should be allowed.

**Decision:** Sandbox mechanism lives under `ironclaw_processes`. Policy inputs come from host scope, auth/network/filesystem constraints, and extension manifest permissions.

---

## 9. Can sandboxing scale to 10k users?

Not if every action gets a fresh heavy container.

Sandboxing must be tiered:

```text
Tier 0: host-mediated services, no child sandbox
Tier 1: stateless micro-script workers
Tier 2: project-scoped warm runtimes
Tier 3: strong per-job isolation for risky work
```

Most users should be idle most of the time. Active work must be quota-controlled and scheduled.

**Decision:** Use profiles, pools, quotas, admission control, and idle cleanup. Do not design around one container per tool call.

---

## 10. Should each project have a sandbox?

A project-scoped sandbox/runtime is useful, but it is not the whole sandboxing story.

Pros:

- good developer ergonomics
- reusable project helpers
- cached dependencies
- closer to pi-mono scripting magic
- better self-learning/self-repair workflow

Cons:

- multi-user permission complexity
- secret leakage risk
- environment drift
- resource/zombie risk
- weaker isolation than per-job strong sandboxing

**Decision:** Adopt project sandboxes as a Tier 2 execution profile: **project-scoped runtime with per-invocation authority**.

The sandbox may be project-scoped, but each invocation must still carry user/principal authority, secret leases, network policy, and audit identity.

---

## 11. Where do secrets live?

Secrets are managed by `crates/ironclaw_auth`.

Config may reference secret handles, but raw secret material should not live in extension config folders.

Good:

```toml
github_token = "secret://user/firat/github_token"
```

Bad:

```toml
github_token = "ghp_raw_token"
```

`ironclaw_auth` owns:

- identity to external services
- token flows / OAuth helpers
- secret handles
- secret resolution
- short-lived injection into process execution
- revocation/rotation hooks

**Decision:** Secrets are not ordinary filesystem config. They are mediated auth resources referenced by handles.

---

## 12. Where does networking live?

Outbound network mediation lives in `crates/ironclaw_network`.

This crate should own:

- outbound network API
- allowlists / egress policy hooks
- shared transport
- proxying/request shaping if needed
- audit hooks

It should not become a random bag of HTTP helpers.

**Decision:** Extensions should not invent direct network paths that bypass `ironclaw_network`.

---

## 13. What replaces the current config system?

Filesystem-based config.

Each extension gets a structured folder:

```text
/system/extensions/<extension>/
  manifest.toml
  config/
  state/
  cache/
  bin/
```

Semantics:

- `manifest.toml` = metadata, compatibility, capabilities, trust
- `config/` = durable user-controlled config
- `state/` = durable extension-owned state
- `cache/` = disposable generated state
- `bin/` = executable assets if shipped locally

**Decision:** Kill the current config system over time and replace it with filesystem-backed extension-local config/state/cache. Require schema versioning and migration paths.

---

## 14. Where does the filesystem fit?

`crates/ironclaw_filesystem` replaces the old Workspace abstraction.

It provides a mountable filesystem trait and namespace for:

- system extension folders
- user state
- project state
- threads
- artifacts
- memory
- config

Project, memory, engine, and system extension state are first-class roots. V1 should explicitly preserve roots/aliases like:

```text
/engine/
/project              # scoped alias to active project
/projects/<project>/
/memory/
/system/extensions/<extension>/
```

Extension folders can contain their own workflow and capability assets:

```text
/system/extensions/<extension>/SKILL.md
/system/extensions/<extension>/skills/
/system/extensions/<extension>/scripts/
/system/extensions/<extension>/wasm/
/system/extensions/<extension>/capabilities.json
/system/extensions/<extension>/config/
/system/extensions/<extension>/state/
```

V1 API should stay small:

- `read`
- `write`
- `list`
- `stat`
- `mount`

**Decision:** Filesystem is the persistence surface, not a universal query engine. Search, indexing, and rich subscriptions should be separate services on top if needed. Keep project and memory roots explicit rather than hiding them in ad hoc stores.

---

## 15. Where do resource budgets live?

Resource budgeting is a host-level system service, not agent-loop logic and not kernel magic. It should live in `crates/ironclaw_resources` or equivalent.

The scope hierarchy includes tenant/org from day one, even if local V1 maps tenant to user:

```text
tenant/org -> user -> project -> mission -> thread -> sub-thread/invocation
```

V1 budget is a broader resource governor, not only USD/tokens. USD remains the primary ledgered budget for LLM spend, but the same service owns or coordinates limits for:

- tokens
- wall-clock
- concurrency
- output bytes
- process count
- sandbox CPU, memory, disk, and network quotas

Costed work uses reservation and reconciliation:

```text
reserve(scope, estimate) -> execute -> reconcile(actual) / release()
```

Every LLM call, WASM capability invocation, MCP call, script run, mission tick, heartbeat, routine, and background job should go through this path when costed or quota-limited. Budget increases and denials are explicit audited events.

**Decision:** Add a first-class resource/budget governor with tenant/org in the cascade. V1 budgets cover USD/tokens plus runtime quotas, with CPU/memory/disk/network initially enforced through sandbox profiles.

---

## 16. Where does the event bus live?

The event bus is composed by `ironclaw_kernel` and produced by multiple system services.

`ironclaw_processes` is a major event producer, but it should not own the whole event system.

Events should be split into:

- realtime events for UI/progress/orchestration
- durable audit/history written to filesystem-backed state

Event classes:

- runtime events
- domain events
- audit events
- extension lifecycle events

**Decision:** Realtime bus is not the durable audit log. Keep these concepts separate from the beginning.

---

## 17. Where does the agent loop live?

The agent loop lives in `extensions/`, not in kernel.

Recommended layout:

```text
extensions/agent_loop_tools/
extensions/agent_loop_codeact/
```

`agent_loop_tools` is the reliable default.  
`agent_loop_codeact` is optional/experimental.

Agent loop should:

- use filesystem state
- call `dispatch`/`spawn`
- use auth/network services through the host API
- append durable thread/step state

**Decision:** Agent loop is a first-party privileged extension, not kernel. Multiple loop implementations can coexist.

---

## 18. How can the agent loop stay stateless?

Agent loop should be **process-stateless**, not logically stateless.

Authoritative state lives in the filesystem:

```text
/projects/<project>/threads/<thread_id>/
  thread.toml
  steps/
  messages/
  artifacts/
  summaries/
  locks/
```

On each invocation, the agent loop:

1. receives a scoped thread reference
2. loads bounded context from filesystem
3. acquires a lock/lease
4. runs the loop
5. appends steps/messages/artifacts
6. releases the lock/lease

**Decision:** In-memory agent loop state is disposable. Durable thread state lives in filesystem-backed paths.

---

## 19. What happens if a gateway drops and reconnects?

Gateways/channels are transport adapters, not sources of truth.

They should store durable cursors/inbox/outbox state in the filesystem:

```text
/users/<user>/channels/<channel_id>/
  inbound/
  outbound/
  cursor.json
```

On reconnect, the gateway:

1. reloads cursor
2. reads missed outbound messages
3. resubscribes to realtime events
4. resumes delivery
5. acknowledges delivered messages

**Decision:** Realtime connections are disposable. Channel delivery state is durable.

---

## 20. What are channels?

Channels are extensions.

Examples:

```text
extensions/gateway/
extensions/tui/
extensions/telegram/
extensions/slack/
extensions/email/
```

Channels should:

- normalize inbound transport messages
- write/call into conversation ingestion
- deliver outbound messages from outbox/event streams
- manage transport-specific cursors and acknowledgements

Channels should not own:

- threads
- agent loop execution
- model reasoning
- custom secret storage

**Decision:** Channels are transport adapters.

---

## 21. How do channels interact with the agent loop?

Recommended flow:

```text
channel extension
  -> conversation extension
  -> agent_loop extension
  -> filesystem-backed thread/outbox
  -> channel extension
```

Add a first-party privileged extension:

```text
extensions/conversation/
```

It owns:

- normalized inbound schema
- channel-to-thread routing
- default agent loop selection
- inbox/outbox semantics

**Decision:** Do not couple channels directly to `agent_loop`. Use `conversation.ingest` as the routing boundary.

---

## 22. Where do skills live?

Skills are filesystem-backed cognitive artifacts.

Suggested locations:

```text
/system/skills/
/users/<user>/skills/
/projects/<project>/skills/
/system/extensions/<extension>/skills/
```

A skill can include:

```text
skill.toml
SKILL.md
examples/
templates/
optional checks/evals/
```

Skills teach the agent:

- workflow
- judgment
- tool/capability choice
- validation steps
- output shape

Skills do not directly execute privileged behavior.

**Decision:** Skills are not kernel, not processes, and not capabilities by themselves. They guide agent loops over real capabilities.

---

## 23. What are the V1 runtime/capability lanes?

V1 should use three lanes:

```text
WASM + MCP + Script Runner
```

### WASM

Stable installable capabilities.

Use for:

- reusable tools
- weaker-model reliability
- hosted/multi-tenant safety
- shareable/installable capabilities

### MCP

Compatibility and ecosystem lane.

Use for:

- existing MCP servers
- stdio or remote MCP integrations
- tools users already depend on

MCP tools are adapted into IronClaw capabilities and still go through IronClaw policy, scope, approval, and audit.

### Script runner

Dynamic creativity lane.

Use for:

- Python/bash/JS snippets
- project-local helpers
- self-repair experiments
- pi-mono-style model-written scripting

**Decision:** V1 capability/runtime lanes are WASM, MCP, and script runner. Generic arbitrary process extensions are deferred as a public extension model.

---

## 24. Why not just skills and scripts?

Skills plus scripts are powerful with frontier models, but they make the system model-dependent. Smaller or cheaper models need stable capabilities.

The split is:

```text
strong model -> can use script runner for missing glue
weak model   -> should prefer WASM/MCP/stable capabilities
```

Scripts are discovery and creativity. WASM/MCP capabilities are stabilization and reliability.

**Decision:** Do not rely on dynamic scripting as the only path. Use scripting for exploration and helper generation; promote stable workflows into WASM/MCP capabilities when reliability matters.

---

## 25. What hot reload do we support?

Hot reload is supported only at safe boundaries in V1.

V1 can support:

- reload skills
- reload config
- reload extension manifests
- reload WASM modules for future invocations
- reconnect or restart MCP servers
- restart script runner workers

V1 should not support:

- live in-flight process migration
- mutating an active agent loop mid-turn
- changing schemas while calls are in flight
- gateway connection-preserving upgrade
- project sandbox migration while scripts are running

**Decision:** V1 supports reload-at-safe-boundaries, not arbitrary live mutation.

---

## 26. What happens to missions?

Missions are not kernel.

The mission engine should be a first-party extension:

```text
extensions/missions/
```

Mission definitions are filesystem data:

```text
/projects/<project>/missions/*.toml
/users/<user>/missions/*.toml
/system/missions/*.toml
```

A mission definition describes durable intent, triggers, and the target action. When triggered, the missions extension uses dispatch/spawn to run agent loops, scripts, or capabilities.

Example:

```toml
id = "daily-pr-review"
enabled = true
trigger = "cron:0 9 * * *"
agent_loop = "agent_loop_tools"
skill = "github-pr-review"
```

**Decision:** Mission engine is a first-party extension. Mission definitions are durable filesystem records.

---

## 27. Should CodeAct/Monty be foundational?

No.

Monty is useful as an optional constrained execution backend, but it should not drive architecture.

Reasons:

- brittle compatibility with model-written Python
- embedded interpreter failures are host-adjacent
- unclear multi-tenant isolation guarantees at scale
- poor fit for real API integrations and auth-sensitive work

**Decision:** Default to regular tool/capability loop. Keep CodeAct as optional/experimental.

Recommended first-party loop split:

```text
extensions/agent_loop_tools/      # default, reliable
extensions/agent_loop_codeact/    # optional, experimental
```

---

## 28. Do skills replace first-party tools/extensions?

No.

Skills are cognition/workflow. Capabilities are executable power.

For important integrations such as GitHub, Slack, Telegram, browser automation, or auth-sensitive workflows, use real first-party capability extensions.

Example:

```text
github extension:
  github.get_pr
  github.list_comments
  github.get_checks
  github.post_review

github skill:
  how to review a PR
  how to categorize feedback
  when to run tests
  how to summarize findings
```

**Decision:** Do not rely on “skill + raw HTTP” for important recurring product integrations. Build first-party capability extensions where reliability, auth, pagination, rate limits, or security matter.

---

## 29. How do self-learning and self-repair work?

They are first-party extensions, not kernel magic.

Recommended extensions:

```text
extensions/reflection/
extensions/repair/
extensions/evals/
extensions/skills/
```

Flow:

```text
agent_loop / processes / channels
  -> emit events + write thread steps
  -> reflection reads traces
  -> reflection writes findings/lessons
  -> repair proposes changes
  -> evals validates candidates
  -> approved changes update skills/extensions/config
```

Repair ladder:

1. skill update
2. routing/config update
3. script/helper generation
4. extension patch
5. host/kernel change only with high bar

**Decision:** Self-learning and self-repair operate through normal extension/system-service boundaries.

---

## 30. How do we get pi-mono-style scripting safely?

Use sandboxed real scripting, not Monty as the primary answer.

Add a script runner capability, likely as a first-party extension or process profile:

```text
script.run(language, source, inputs, mounts, network_policy, timeout, memory)
```

Support:

- Python
- bash
- Node later

Execution must use:

- explicit inputs
- scoped mounts
- mediated network
- no raw host env
- timeout/memory/output limits
- artifact directory

**Decision:** For pi-mono-like scripting magic, prefer sandboxed real Python/bash/Node via `ironclaw_processes` over Monty as the default.

---

## 31. Can Monty scale to 10k users?

Only for tiny pure snippets should Monty be considered; do not bet the architecture on it.

Monty may be useful as:

- Tier 0.5 / Tier 1 constrained backend
- pure transformations
- no filesystem/network/secrets
- local or trusted snippets

It should not be used for:

- default CodeAct
- important integrations
- hosted multi-tenant script execution with secrets/network/files
- anything requiring CPython semantics

**Decision:** Monty can remain optional, but real sandboxed scripting profiles should be the main path.

---

## 32. What is the recommended execution tier model?

Use tiered execution:

```text
Tier 0: host-mediated operations
  filesystem/network/auth APIs, no script runtime

Tier 1: stateless micro-script
  tiny Python/bash/Node, no secrets/network by default

Tier 2: project-scoped runtime
  persistent project sandbox, scoped mounts, reusable helpers/deps

Tier 3: strong per-job isolation
  container/microVM for risky or untrusted work
```

**Decision:** Project sandboxes are useful Tier 2 runtimes, but not the whole sandboxing solution.

---

## 33. What decisions are still open?

Open implementation details:

1. exact WASM host ABI/import surface
2. exact MCP adapter shape and tool-to-capability mapping
3. exact script runner sandbox profiles
4. exact filesystem backend interface and mount semantics
5. initial sandbox backend
6. initial event bus implementation
7. initial durable audit/history file format
8. first GitHub capability scope across WASM/MCP/skills/script runner
9. exact project sandbox lifecycle policy
10. exact mission trigger format and scheduler semantics
11. exact resource ledger schema, thresholds, and rollout modes

These should be resolved during implementation planning, not by expanding the kernel.

---

## 34. Summary of major decisions

| Topic | Decision |
|---|---|
| Top-level model | OS-like host with system-service crates, not forced 3-box |
| Kernel | Composition/wiring, not product runtime |
| Host API | Shared authority-bearing contracts and invariants |
| ExtensionManager | Knows what can run |
| ProcessManager | Knows what is running |
| Filesystem | Durable mount/persistence surface with explicit `/engine`, `/projects`, `/memory`, and `/system/extensions` roots |
| Resources/Budgets | Host-level resource governor; tenant/org included in cascade; USD primary plus runtime quotas |
| Config | Filesystem-based, extension-local, schema-versioned |
| Secrets | Handles in config, raw material mediated by auth |
| Network | Mediated by `ironclaw_network` |
| Sandboxing | Mechanism under `ironclaw_processes`, tiered profiles |
| Runtime lanes | V1 uses WASM + MCP + script runner |
| Hot reload | Safe-boundary reload only, no live in-flight migration |
| Agent loop | First-party extension, not kernel |
| Channels | Transport extensions |
| Conversation | First-party routing extension between channels and agent loop |
| Skills | Filesystem-backed cognitive artifacts |
| CodeAct | Optional/experimental, not foundation |
| Monty | Optional constrained backend, not default scripting path |
| Scripting | Prefer sandboxed real Python/bash/Node |
| Missions | Mission engine is an extension; mission definitions are filesystem data |
| Self-repair | Reflection/repair/evals extensions, not kernel magic |
| Scaling | Use pools, quotas, project runtimes, and tiered isolation |
