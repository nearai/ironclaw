# IronClaw Reborn capability access contract

**Date:** 2026-04-25
**Status:** Draft contract
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/extensions.md`, `docs/reborn/contracts/kernel-dispatch.md`

---

## 1. Purpose

Reborn should not recreate the old universal `ToolRegistry` with a new name.

Capability access is split across separate responsibilities:

```text
ExtensionRegistry / CapabilityCatalog -> what exists
CapabilityAccessManager              -> what is visible and callable in this scope
RuntimeDispatcher                     -> which runtime lane executes it
Runtime lane crates                   -> how it executes
Host API policy/approvals/resources   -> whether the action is authorized and budgeted
```

---

## 2. Key invariant

```text
visible capability surface != action authorization
```

A capability can be shown to the model because it is useful in the current context, but every action still requires action-time authorization.

This prevents stale prompts, race conditions, extension activation changes, or malicious model output from bypassing policy.

---

## 3. Responsibility split

| Component | Owns | Must not own |
| --- | --- | --- |
| `ExtensionRegistry` | validated packages and declared capability descriptors | execution, grants, prompt assembly |
| `CapabilityCatalog` | normalized descriptor lookup and model-visible capability metadata | action-time authorization, runtime lane execution |
| `CapabilityAccessManager` | visible capability snapshots, scope filtering, grants/policy inputs, action-time checks | manifest parsing, runtime execution, resource reconciliation |
| `RuntimeDispatcher` | runtime kind validation and handoff to configured runtime lane | authorization policy, prompt visibility, manifest discovery |
| Runtime lane crates | lane-specific prepare/invoke/cleanup mechanics | capability discovery, cross-runtime policy |
| `ApprovalManager` | structured approval requests and reusable approval scopes | deciding what exists or executing capabilities |
| `ResourceGovernor` | reserve/reconcile/release protocol and ledgers | capability visibility or model prompting |

---

## 4. Visible capability snapshot

A visible capability snapshot is built on the warm path before an LLM call.

Inputs may include:

- execution scope
- active project/thread
- extension activation state
- user/org policy
- grants
- skill/instruction context
- model/tool token budget
- capability trust class

Snapshot output includes only model-visible information:

- name/id
- description
- parameter schema or simplified schema
- safety notes if needed

Snapshot output must not include:

- raw secrets
- raw host paths
- hidden policy details that would help bypass controls
- capabilities outside the caller's scope

---

## 5. Action-time authorization

Before execution, every call is normalized into an `Action` and authorized with current state:

```text
ExecutionContext + Action + Grants + Approvals + Policy + ResourceState -> Decision
```

Rules:

- missing capability -> deny
- missing grant -> deny or require approval according to policy
- stale visible snapshot -> does not authorize execution
- approval for one action/path does not authorize broader actions/paths
- resource reservation must happen before costed work
- most restrictive decision wins

---

## 6. Extension activation changes

When extension activation changes:

```text
ExtensionRegistry changes
-> CapabilityCatalog refreshes descriptors
-> CapabilityAccessManager invalidates visible snapshots
-> InstructionBundleAssembler rebuilds only if model-visible text changed
```

Running calls are not automatically granted new authority. In-flight behavior must be determined by the action-time authorization state and safe-boundary reload rules.

---

## 7. Relationship to runtime dispatch

`RuntimeDispatcher` assumes the capability is declared and receives a dispatch request. It still performs fail-closed consistency checks:

- capability exists
- provider package exists
- descriptor runtime matches package runtime
- selected backend is configured

It does not replace `CapabilityAccessManager`. In the V1 stack, dispatcher tests intentionally prove runtime routing, not complete authorization policy.

---

## 8. Contract tests to add later

When the access manager exists, add tests for:

- visible but unauthorized capability cannot execute
- extension activation invalidates visible snapshot
- stale snapshot does not grant action authority
- reusable approval scope is exact or policy-defined, never heuristic
- runtime dispatch cannot bypass access manager in host call path
- hidden capabilities are omitted from instruction bundles
