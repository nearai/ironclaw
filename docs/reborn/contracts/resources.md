# Reborn Resources Contract

**Status:** Draft implementation contract
**Date:** 2026-04-24
**Depends on:** `docs/reborn/contracts/host-api.md`, `crates/ironclaw_host_api`

---

## 1. Purpose

`ironclaw_resources` is the host-level governor for cost, quota, and scarce runtime capacity.

It answers one question before costed or quota-limited work starts:

```text
Can this tenant/user/project/agent/mission/thread/invocation reserve enough resources to run?
```

The resource governor is not a billing provider, payment system, LLM provider, sandbox runtime, policy engine, approval UI, or database migration layer. It owns the shared reservation protocol and ledgers that every runtime lane must use.

---

## 2. Core invariant

No costed or quota-limited work should execute in hosted/multi-tenant mode without an active reservation.

```text
reserve(scope, estimate) -> execute -> reconcile(actual)
                         \-> release(unused)
```

Applies to:

- LLM calls
- WASM capability invocations when quota-limited
- Docker-backed script runner jobs
- MCP calls
- spawned/background capability processes
- mission ticks
- routines/heartbeats/background jobs
- artifact/export work when limited by bytes/disk

---

## 3. Scope cascade

Resource scope comes from `ironclaw_host_api::ResourceScope`.

Canonical cascade:

```text
tenant/org -> user -> project -> mission -> thread -> invocation
```

Limits may be attached at any durable account level:

```text
tenant
user
project
mission
thread
```

The invocation is the reservation identity for one unit of work; it is not usually the account that stores long-lived budget limits.

Reservation must check all applicable ancestors. If any ancestor would be oversubscribed, reservation is denied.

Example:

```text
tenant limit:  $10
project A used/reserved: $7
project B request:       $4
result: deny at tenant, even if project B has no local limit
```

---

## 4. Resource dimensions

V1 dimensions:

| Dimension | Meaning |
|---|---|
| `usd` | primary ledgered spend |
| `input_tokens` | LLM/input token budget |
| `output_tokens` | LLM/output token budget |
| `wall_clock_ms` | total runtime duration cap |
| `output_bytes` | stdout/stderr/result/artifact output cap |
| `network_egress_bytes` | outbound byte budget when measured |
| `process_count` | process/subprocess cap |
| `concurrency_slots` | active in-flight work slots |

CPU, memory, disk, and network enforcement may initially live in sandbox profiles, but their limits should still be represented in resource records when available.

---

## 5. Reservation semantics

A reservation is a hold against every account in the scope cascade.

Rules:

1. Reservation uses estimated resource demand.
2. Missing estimate dimensions count as zero for that dimension.
3. Reservation succeeds only if all applicable account limits can absorb the estimate plus current active reservations plus reconciled usage.
4. Reservation returns a unique `ResourceReservationId`; obligation handoff may request a specific reservation id through `reserve_with_id(scope, estimate, reservation_id)` so authorization obligations, dispatch receipts, and runtime reconciliation refer to the same durable id.
5. Active reservations count against limits until reconciled or released.
6. Reconcile replaces reserved estimate with actual usage and closes the reservation.
7. Release removes the reservation without recording spend.
8. Unknown reservations fail closed.
9. Double reconcile/release fails closed.
10. Denials must say which account and resource dimension blocked the request.

---

## 6. Concurrency

Concurrency is modeled as a resource dimension, not as a separate ad hoc lock.

```text
ResourceEstimate { concurrency_slots: Some(1), ... }
```

Rules:

- active reservations consume concurrency slots
- reconciliation releases concurrency slots while recording actual usage
- release frees concurrency slots without recording spend
- concurrent reservations must be serialized by the governor so two callers cannot oversubscribe the same account

V1 in-memory implementation can use a mutex. Persistent/fleet implementations should use database transactions or advisory locks.

---

## 7. Reconciliation semantics

`reconcile(reservation_id, actual_usage)` records actual consumption.

Rules:

- actual usage is added to every account in the original scope cascade
- the original estimated hold is removed
- the reservation becomes closed
- reconciliation is idempotency-sensitive: repeating it is an error unless a future API adds explicit idempotency keys
- if actual usage exceeds estimate, record actual usage; runtime/sandbox layers are responsible for enforcing hard limits during execution

---

## 8. Release semantics

`release(reservation_id)` is for work that never ran or was canceled before spend should be recorded.

Rules:

- active reservation hold is removed
- no usage is recorded
- reservation becomes closed
- releasing an unknown or closed reservation is an error

---

## 9. Denial semantics

Denials should be structured.

Minimum data:

```rust
ResourceDenied {
    account: ResourceAccount,
    dimension: ResourceDimension,
    limit: ResourceAmount,
    current_usage: ResourceAmount,
    active_reserved: ResourceAmount,
    requested: ResourceAmount,
}
```

This supports user messaging, approval requests, and audit/provenance without parsing strings.

---

## 10. Process resource lifecycle

Spawned/background capability processes can own reservations through `ironclaw_processes::ResourceManagedProcessStore`:

```text
ProcessStart.estimated_resources
  -> reject caller-supplied ProcessStart.resource_reservation_id
  -> ResourceGovernor::reserve(scope, estimate)
  -> ProcessRecord.resource_reservation_id = Some(id)
  -> process runs
  -> complete: reconcile(id, configured_completion_usage)
  -> fail/kill: release(id)
```

`ResourceManagedProcessStore` owns reservation creation and cleanup for processes. Reservation IDs are authority-bearing: callers cannot inject them through `ProcessStart`, and process state transitions only reconcile/release reservations the wrapper created for that process.

The wrapper reserves before process records are created, releases the reservation if the underlying store rejects `start`, and verifies the resulting process record preserved the reservation ID. Resource denial therefore prevents process persistence. When a background process already owns a reservation, the process executor dispatch path uses a default runtime estimate to avoid double-reserving the same process estimate. Completion reconciliation currently uses configured/default usage because `ProcessExecutionResult` does not yet report measured usage.

---

## 11. Audit and provenance

Every reservation lifecycle should be auditable:

```text
reservation_requested
reservation_granted
reservation_denied
reservation_reconciled
reservation_released
```

Audit records should include:

- correlation ID
- invocation ID
- tenant/user/project/agent/mission/thread scope
- estimate or actual usage summary
- denial dimension and account when denied
- reservation ID when granted/reconciled/released

`ironclaw_resources` may emit audit/event records later; the first crate can expose structured receipts that callers can turn into audit envelopes.

---

## 12. Initial Rust API sketch

```rust
pub struct ResourceLimits { /* optional max per dimension */ }

pub enum ResourceAccount {
    Tenant { tenant_id: TenantId },
    User { tenant_id: TenantId, user_id: UserId },
    Project { tenant_id: TenantId, user_id: UserId, project_id: ProjectId },
    Agent { tenant_id: TenantId, user_id: UserId, agent_id: AgentId },
    Mission { /* tenant/user/project/agent/mission */ },
    Thread { /* tenant/user/project/agent/mission/thread */ },
}

pub trait ResourceGovernor {
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits);
    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError>;
    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError>;
    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError>;
    fn release(&self, reservation_id: ResourceReservationId) -> Result<ResourceReceipt, ResourceError>;
}
```

The V1 crate should start with an in-memory implementation for contract tests. Persistence, PostgreSQL/libSQL backends, and distributed locks can come later.

---

## 12. Minimum TDD coverage

Local contract tests should prove:

- reservation succeeds when budget is available
- reservation denies when USD would exceed limit
- reservation denies when runtime quota would exceed limit even with zero USD
- active reservations consume concurrency slots
- concurrent reservations cannot oversubscribe a scope
- release frees reserved-but-unused capacity and records no spend
- reconcile records actual usage and releases active reservation
- unknown reservation cannot be reconciled or released
- double reconcile/release fails closed
- tenant/user/project/agent hierarchy checks ancestors, not only leaf scope

---

## 13. Non-goals

Do not add in `ironclaw_resources` V1:

- payment processing
- subscription plans
- provider-specific LLM price catalogs
- database schema/migrations
- Docker/WASM/MCP execution
- approval UI
- auth/secret storage
- network clients
- product workflows
- retry/stuck-loop heuristics except as future input signals


---

## Contract freeze addendum — V1 quota coverage (2026-04-25)

V1 resource reservation/enforcement covers:

```text
runtime/process execution
network egress estimates and reconciled usage
embedding/provider calls
artifact/storage quotas
```

Memory indexing/search may initially report usage without hard reservation, but embedding provider calls and artifact writes must participate in resource accounting.

Every reservation should carry tenant/user/project/agent/invocation/process scope where relevant. Reservation release/reconcile must be owned by the service that owns the side effect lifecycle.
