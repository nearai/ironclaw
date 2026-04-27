# Reborn Product Manager Architecture Guide

**Date:** 2026-04-27
**Status:** PM-facing guide, not an implementation contract
**Audience:** product managers, design partners, engineering leads planning product features

---

## 1. Purpose

This guide explains how to follow Reborn architecture changes from a product perspective.

Use it to answer:

```text
Where does a feature sit?
Which architecture areas does it touch?
Which team/reviewer should care?
What must be true for the feature to be safe, observable, and shippable?
```

This is not a new kernel layer, Rust crate, or service boundary. It is a planning map that translates product features into the Reborn architecture.

---

## 2. The mental model

Reborn separates product behavior from safety enforcement.

```text
Product feature / user promise
  -> channel or UI surface
  -> product workflow and agent loop behavior
  -> kernel-mediated capability/memory/process/provider calls
  -> runtime/extension/memory/network/secret/resource/event substrates
```

Plain-language layers:

| Layer | PM meaning | Engineering owner/boundary |
| --- | --- | --- |
| Product feature | What the user can do and how success feels. | Product/design + owning engineering area. |
| Channel / UI | Where the user starts and receives the result. | Web, CLI, Slack, Telegram, etc. adapters. |
| Product workflow | The sequence of decisions, retries, states, and UX messages. | Userland product code or reference loop, not kernel. |
| Agent loop | How the assistant reasons, selects tools, asks follow-ups, and summarizes. | Replaceable userland loop on the kernel surface. |
| Kernel | The safety perimeter: authorization, approvals, leases, mounts, secrets, network, resources, redaction, durable audit/events. | Kernel-facing crates such as `ironclaw_host_api`, `ironclaw_capabilities`, `ironclaw_authorization`, `ironclaw_host_runtime`, etc. |
| Extensions/capabilities | What actions the assistant can request. | `ironclaw_extensions`, runtime lanes, capability descriptors. |
| Memory/workspace | What the assistant can remember, search, and use as context. | `ironclaw_memory`, filesystem/repository backends, prompt-safety policy. |
| Events/audit | What support, QA, agents, and users can replay or inspect. | `ironclaw_events`, projections, SSE/WebSocket later. |

The kernel is not the product brain. It makes product workflows safe.

### User flow at a glance

```text
USER
  |
  | asks / clicks / approves / installs / uploads
  v
CHANNEL OR UI
  |  Web, CLI, Slack, Telegram, API
  |  "Who is the user and where should the reply go?"
  v
PRODUCT WORKFLOW
  |  User promise, UX states, retries, success/failure copy
  |  Examples: answer chat, run task, remember preference, install extension
  v
AGENT LOOP OR WORKFLOW LOGIC
  |  Decide what to say, ask, remember, or request as a capability
  |  Loop behavior can change without changing kernel guarantees
  v
KERNEL SAFETY SURFACE
  |  authorize -> approve/auth if needed -> apply obligations
  |  mounts, secrets, network policy, resources, redaction, audit/events
  v
CAPABILITY / MEMORY / PROCESS / PROVIDER
  |  Extension runtime, memory backend, long-running process, external API
  v
RESULT + EVENTS
  |  User-visible reply/status/result
  |  Redacted event/audit trail for support, QA, replay, and debugging
  v
USER / SUPPORT / QA
```

### High-level component interaction

```text
+------------------+       +----------------------+       +------------------+
| User / Customer  | ----> | Channel or UI        | ----> | Product workflow |
|                  | <---- | Web/CLI/Slack/etc.   | <---- | UX + orchestration|
+------------------+       +----------------------+       +---------+--------+
                                                                  |
                                                                  v
                                                        +-------------------+
                                                        | Agent loop /      |
                                                        | workflow logic    |
                                                        +---------+---------+
                                                                  |
                                  requested capability / memory / provider call
                                                                  |
                                                                  v
+--------------------------------------------------------------------------------+
| KERNEL SAFETY SURFACE                                                          |
| CapabilityHost | Authorization | Approvals/leases | Run-state | Redaction      |
| Scoped mounts  | Secret leases | Network policy   | Resources | Audit/events   |
+---------+----------------+--------------------+--------------------+-----------+
          |                |                    |                    |
          v                v                    v                    v
+----------------+  +----------------+  +----------------+  +--------------------+
| Extensions /   |  | Memory /        |  | Process /      |  | Provider / network |
| capabilities   |  | workspace       |  | runtime lanes   |  | clients            |
| WASM/Script/MCP|  | docs/search     |  | bg work/results |  | APIs via policy    |
+-------+--------+  +-------+--------+  +-------+--------+  +---------+----------+
        |                   |                   |                     |
        +-------------------+-------------------+---------------------+
                            |
                            v
                  +---------------------+
                  | Durable events /    |
                  | audit / projections |
                  +----------+----------+
                             |
                             v
                  Support, QA, replay, user-facing status
```

Read this diagram top-down for user experience and left/right across the bottom for architecture ownership. Product managers usually own the top three boxes: user promise, channel/UI experience, and product workflow. Engineering/kernel reviewers own the safety surface and every privileged dependency below it.

### Example: “Run a task for me”

```text
User asks: "Book this meeting and tell me when done"
  |
  v
Channel/UI records the request and thread
  |
  v
Product workflow decides the UX:
  - show task started
  - ask approval if calendar write is risky
  - show progress while background work runs
  - show final result or failure reason
  |
  v
Agent loop chooses capability calls:
  - calendar.search_availability
  - calendar.create_event
  - maybe email.send_summary
  |
  v
Kernel safety checks:
  - is this user/agent/project allowed to call these capabilities?
  - does it need approval or auth repair?
  - which Gmail/calendar account is selected?
  - what secrets/network/resource budget are allowed?
  - what must be redacted in output/events?
  |
  v
Extensions/providers do the work through mediated runtimes/network
  |
  v
Process/events/reporting return status to user and support tools
```

---

## 3. Feature mapping template

For every feature, write this before deciding where code belongs:

```text
Feature name:
User promise:
User entry point/channel:
Primary product workflow:
Agent loop behavior needed:
Capabilities/extensions needed:
Memory reads/writes needed:
Secrets/accounts needed:
Network/provider calls needed:
Approval/auth gates:
Background process or long-running work:
Events/support visibility needed:
Kernel guarantees relied on:
Known non-goals:
```

If the feature needs a privileged effect, identify the kernel-mediated surface:

```text
CapabilityHost
ProcessHost
ScopedFilesystem / memory services
Secret lease / credential account metadata
ironclaw_network provider client
event/audit append log
resource governor
approval resolver / exact invocation lease
```

If no mediated surface exists, that is an architecture contract request, not just feature work.

---

## 4. Common product features and where they sit

| Feature / user promise | Primary place it sits | Kernel-mediated dependencies | Notes for PMs |
| --- | --- | --- | --- |
| “Answer my chat message” | Channel + turn coordination + active loop | memory reads, provider call, events, optional capabilities | Behavior bugs usually belong to the loop; leaks/unauthorized actions belong to kernel. |
| “Run this tool/task for me” | Capability request from loop or UI | `CapabilityHost`, authorization, approvals, runtime adapter, events/resources | Every side effect should be an explicit capability call, not hidden text. |
| “Keep working in the background” | Product workflow + process APIs | `CapabilityHost::spawn_json`, `ProcessHost`, process events/results/resources | Product owns status UX; kernel/process services own lifecycle authority. |
| “Remember this preference” | Memory product workflow | scoped memory write, prompt-safety policy, events | Memory write safety matters if the content affects future prompts. |
| “Use my Gmail/work account” | Extension/account selection UX | credential account metadata, secret leases, network policy | Credential accounts are metadata; raw tokens stay behind secret lease boundaries. |
| “Install this extension” | Extension lifecycle workflow | extension registry, trust policy, grants, settings, secrets | Install does not equal authority. Grants/trust still matter. |
| “Approve this risky action” | Approval UX + control plane | approval request store, exact invocation lease, audit | Approval should be replay-safe and specific to the invocation. |
| “Search/use my memory” | Memory search/profile workflow | memory repository/search services, scope policy, events | Identity/system-prompt docs are primary-scope only. |
| “Call an external API/provider” | Capability/provider workflow | `ironclaw_network`, secrets/credentials, resources, audit | All host/provider HTTP goes through network boundary. |
| “Customize the agent loop” | Loop/userland package | trust policy, grants, kernel facade | Custom loops are allowed, but they do not bypass kernel guarantees. |

---

## 5. How to tell whether a bug is kernel or product behavior

Use this triage split:

| Question | If yes | Example |
| --- | --- | --- |
| Did the system perform an unauthorized side effect? | Kernel/control-plane bug. | Tool ran without grant or approval. |
| Did it leak secrets, host paths, raw hidden input, or private memory? | Kernel/redaction/memory policy bug. | Secret appeared in event or model output. |
| Did it ignore one-active-run/thread or reuse an approval incorrectly? | Kernel coordination bug. | Same approval reused for different input. |
| Did the assistant choose the wrong tool, phrase something badly, or claim success too early? | Loop/product behavior bug. | Agent said task was done before checking result. |
| Was the feature hard to debug after the fact? | Event/product observability gap. | No replayable event path for a failed workflow. |

This matters because fixes go to different places. Kernel bugs require contract/regression coverage. Loop/product behavior bugs can be fixed by changing the active loop, product workflow, prompt strategy, or UX.

---

## 6. Where product work usually lands

| Product change | Usually lands in | Usually should not require |
| --- | --- | --- |
| New user-facing workflow | Product workflow / turn coordination / channel UI | New kernel authority model, unless new privileged operation is needed. |
| New tool integration | Extension package, capability descriptor, runtime adapter config | Direct dispatcher calls or bespoke auth path. |
| New agent behavior | Reference loop or loop package | Kernel changes, unless a new safety invariant is needed. |
| New memory behavior | `ironclaw_memory` service facade or product workflow using memory | Filesystem owning semantic memory/search rules. |
| New external provider | Provider adapter through `ironclaw_network` + secrets/resources | Raw HTTP from product code. |
| New approval UX | Product/channel UX + approval resolver | Reusable broad approvals in V1. |
| New observability surface | Event projection / product read model | Raw unredacted runtime logs. |
| New secret/account UX | Credential account metadata + secret lease flow | Credential account records containing raw tokens. |

---

## 7. Product planning checklist

Before a feature is ready for implementation planning, answer:

1. What user promise does this feature make?
2. Which channel/UI starts it?
3. Is this loop behavior, extension capability, memory behavior, provider behavior, or kernel-mediated authority?
4. What data scopes are involved: tenant, user, agent, project, thread, invocation?
5. Does it need approvals, auth repair, secrets, external network calls, or resource limits?
6. What events must support/QA/users be able to replay?
7. What is the smallest safe fallback if part of the substrate is missing?
8. What is explicitly out of scope for the first version?

If answers mention authorization, secret material, network policy, filesystem authority, approval leases, redaction, or resource quotas, include a kernel/security reviewer.

---

## 8. Relationship to the contract docs

Use this guide for product planning. Use the contract docs for implementation boundaries:

```text
Architecture map: docs/reborn/2026-04-25-current-architecture-map.md
Kernel boundary: docs/reborn/contracts/kernel-boundary.md
Capability flow: docs/reborn/contracts/capabilities.md
Memory: docs/reborn/contracts/memory.md
Events/replay: docs/reborn/contracts/events-projections.md
Secrets: docs/reborn/contracts/secrets.md
Network: docs/reborn/contracts/network.md
Extensions: docs/reborn/contracts/extensions.md
Turns/loops: docs/reborn/contracts/turns-agent-loop.md
```
