# Reborn Contract — Kernel Boundary

**Status:** Contract-freeze draft
**Date:** 2026-04-27
**Depends on:** [`host-api.md`](host-api.md), [`capabilities.md`](capabilities.md), [`capability-access.md`](capability-access.md), [`run-state.md`](run-state.md), [`approvals.md`](approvals.md), [`filesystem.md`](filesystem.md), [`network.md`](network.md), [`secrets.md`](secrets.md), [`resources.md`](resources.md), [`events-projections.md`](events-projections.md)

---

## 1. Purpose

The Reborn kernel is the security perimeter. It is defined by what it mediates and secures, not by how much product behavior it performs.

Terminology:

```text
kernel                 architectural security boundary
ironclaw_host_runtime  current concrete composition crate for kernel-facing services/adapters
```

There is no requirement to create an `ironclaw_kernel` crate. The contract is about the boundary: privileged operations must cross kernel-mediated ports, regardless of which concrete crate wires them.

---

## 2. Kernel responsibilities

The kernel mediates operations that can affect authority, isolation, durable control-plane state, or sensitive data.

Kernel-mediated operations include:

```text
capability invocation/resume/spawn
capability authorization and grant matching
trust-class policy evaluation
obligation preparation/completion/abort
approval request and exact-invocation lease coordination
run-state coordination for active/blocked/completed/failed work
one-active-run-per-thread coordination
filesystem mount/scoped path authority
network policy evaluation and hardened host/provider HTTP egress
secret metadata lookup, lease issuance, one-shot consumption, and injection handoff
resource reservation, reconciliation, release, and quota enforcement
redaction and leak-detection obligations
prompt-injection write-safety policy for kernel-injected prompt files
durable audit/event append substrate and scoped replay cursors
process lifecycle/result/output authority surfaces
```

Any privileged operation a loop, channel, extension, or service needs must appear behind one of these mediated surfaces or be added through an explicit contract-change request. There is no private back door for shipped loops or first-party code.

---

## 3. Kernel non-responsibilities

The kernel must stay small. Product behavior that does not need to mediate authority must live in userland.

Userland responsibilities include:

```text
agent-loop strategy and model/provider heuristics
prompt assembly strategy over authorized memory reads
routine engines and mission orchestration
skill selection and planning policy
channel-specific user experience
profile presentation and summarization strategy
reference loop implementations such as lightweight loop or CodeAct loop
provider-specific behavior above the network/secrets/resource boundaries
```

Userland code may be shipped by the project, installed as an extension, or configured by deployment. In every case it runs through kernel-mediated surfaces and receives only the grants, mounts, leases, policies, and resource budget allowed by host policy.

---

## 4. Loop model

Loop diversity is an expected feature:

```text
lightweight loop
CodeAct loop
model-specific loop
provider-specific loop
deployment-specific loop
community/user-installed loop
```

A loop is not trusted because the project shipped it. A shipped loop may receive a higher trust ceiling by host policy, but it still needs explicit grants and must use `CapabilityHost`, scoped memory/filesystem APIs, provider/network clients, process APIs, and event/audit surfaces. The kernel secures the loop environment; it does not rely on the loop to preserve kernel invariants.

Reference loop docs such as [`turns-agent-loop.md`](turns-agent-loop.md), [`agent-loop-protocol.md`](agent-loop-protocol.md), and [`lightweight-agent-loop.md`](lightweight-agent-loop.md) describe one loop family. They are not the only allowed loop shape and must not be read as kernel responsibilities unless they explicitly identify a kernel-mediated invariant.

---

## 5. Trust-class policy

`TrustClass` is an authority ceiling, not a permission grant and not a bypass.

Minimum policy rules:

- trust class is assigned by host policy, signed/bundled package metadata, or admin configuration;
- user-installed packages cannot self-declare `TrustClass::FirstParty` or `TrustClass::System`;
- `TrustClass::FirstParty` and `TrustClass::System` do not grant authority by themselves;
- every privileged effect still requires explicit grants, scoped mounts, leases, resource budget, and obligation handling;
- trust-class downgrade or revocation must make affected grants unusable before new side effects;
- extension or loop upgrade retains grants only when package identity, signer/source policy, trust class, and requested authority are still valid;
- upgraded code that requests expanded authority requires renewed approval or admin policy;
- `TrustClass::System` remains host-owned and is not a matchable grantee for ordinary extension manifests.

The policy engine that enforces these rules is load-bearing for loop replacement and extension lifecycle. It belongs in the contract set before user-installed privileged loops are productized.

---

## 6. Turn and prompt split

Turn coordination and loop behavior are separate.

Kernel-mediated turn invariants:

```text
one active run per thread
scope-consistent turn/run state
approval/auth/resource waits recorded as structured blocked states
cancellation and process authority routed through process/capability services
redacted durable progress/audit events
```

Userland loop behavior:

```text
which model/provider to call
how to assemble prompt context from authorized memory reads
which tools/capabilities to request
how to plan, retry, summarize, or ask follow-up questions
how to checkpoint loop-local strategy state through mediated storage
```

Prompt-injection write safety is kernel-mediated policy because it protects future execution context. Prompt assembly is loop/userland strategy over authorized memory reads. A reference loop may ship a default prompt assembler, but the assembler is not the kernel.

---

## 7. QA and incident triage

Triage every behavior bug with this split:

```text
Did the kernel fail to enforce an authority/security/coordination guarantee?
  -> kernel bug; fix in mediated contract/implementation and add caller-level regression coverage.

Did the loop make a poor behavioral choice while staying inside its grants?
  -> loop implementation bug; fix or replace the loop without changing kernel guarantees.
```

Examples of loop bugs include poor tool choice, weak temporal reasoning, premature success claims, or unhelpful instructions. Examples of kernel bugs include unauthorized capability execution, leaked secrets/host paths, ignored approval leases, unscoped memory reads, unbounded network egress, or broken one-active-run-per-thread coordination.

---

## 8. Acceptance tests

Kernel-boundary implementation tasks must include caller-level tests that prove:

- loops cannot bypass `CapabilityHost` for privileged effects;
- trust class alone does not grant authority;
- user-installed code cannot self-promote to `FirstParty`/`System`;
- upgrade with expanded requested authority requires re-approval/admin policy;
- one-active-run-per-thread blocks concurrent work before model/tool side effects;
- prompt-injected file writes are scanned or fail closed, while prompt assembly remains replaceable;
- redacted audit/event records distinguish kernel failures from loop behavior failures;
- tenant/user/project/agent scope flows through kernel-mediated calls.
