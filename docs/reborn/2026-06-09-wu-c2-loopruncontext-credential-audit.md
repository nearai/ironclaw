# WU-C2: LoopRunContext Credential Audit

**Date:** 2026-06-09
**Workstream:** WU-C2 — Durable Gate-Resolution Store
**Outcome:** Option (a) — zero sensitive fields; compile-time lint guard added.

---

## Purpose

`LoopRunContext` is serialized to JSON and stored verbatim in the
`subagent_gate_awaited_children.parent_run_context_json` column by the durable
gate-resolution backends.  This audit verifies that no credential material
(API keys, bearer tokens, passwords, OAuth grants, or other secrets) can reach
the database through that field.

---

## Struct: `LoopRunContext`

**Crate:** `ironclaw_turns`
**Module:** `crates/ironclaw_turns/src/run_profile/host.rs`

| Field | Type | Classification | Rationale |
|---|---|---|---|
| `scope` | `TurnScope` | **Non-sensitive** | See TurnScope table below |
| `actor` | `Option<TurnActor>` | **Non-sensitive** | `TurnActor { user_id: UserId }` — opaque scoped identifier, not a credential |
| `accepted_message_ref` | `Option<AcceptedMessageRef>` | **Non-sensitive** | `bounded_ref!` newtype wrapping an opaque message-reference string |
| `thread_id` | `ThreadId` | **Non-sensitive** | Opaque scoped identifier (`string_id!` newtype) |
| `turn_id` | `TurnId` | **Non-sensitive** | Opaque UUID-based identifier |
| `run_id` | `TurnRunId` | **Non-sensitive** | Opaque UUID-based identifier |
| `resolved_run_profile` | `ResolvedRunProfile` | **Non-sensitive** | See ResolvedRunProfile table below |
| `resolved_model_route` | `Option<LoopModelRouteSnapshot>` | **Non-sensitive** | See LoopModelRouteSnapshot table below |
| `loop_driver_id` | `LoopDriverId` | **Non-sensitive** | Opaque string ID |
| `loop_driver_version` | `RunProfileVersion` | **Non-sensitive** | Integer version |
| `checkpoint_schema_id` | `CheckpointSchemaId` | **Non-sensitive** | Opaque string ID |
| `checkpoint_schema_version` | `RunProfileVersion` | **Non-sensitive** | Integer version |

---

## Nested Type: `TurnScope`

**Module:** `crates/ironclaw_turns/src/scope.rs`

| Field | Type | Classification | Rationale |
|---|---|---|---|
| `tenant_id` | `TenantId` | **Non-sensitive** | Opaque scoped identifier (`string_id!` newtype) |
| `agent_id` | `Option<AgentId>` | **Non-sensitive** | Opaque scoped identifier |
| `project_id` | `Option<ProjectId>` | **Non-sensitive** | Opaque scoped identifier |
| `thread_id` | `ThreadId` | **Non-sensitive** | Opaque scoped identifier |
| `thread_owner` | `TurnThreadOwner` | **Non-sensitive** | Enum: `ActorFallback`, `ExplicitUser { owner_user_id: UserId }`, or `Ownerless` — user identity, not a credential |

---

## Nested Type: `TurnActor`

**Module:** `crates/ironclaw_turns/src/scope.rs`

| Field | Type | Classification | Rationale |
|---|---|---|---|
| `user_id` | `UserId` | **Non-sensitive** | Opaque scoped identifier, not an authentication token |

---

## Nested Type: `ResolvedRunProfile`

**Module:** `crates/ironclaw_turns/src/run_profile/snapshot.rs`

| Field | Type | Classification | Rationale |
|---|---|---|---|
| `run_class_id` | `RunClassId` | **Non-sensitive** | Opaque string ID |
| `profile_id` | `RunProfileId` | **Non-sensitive** | Opaque string ID |
| `profile_version` | `RunProfileVersion` | **Non-sensitive** | Integer version |
| `loop_driver` | `AgentLoopDriverDescriptor` | **Non-sensitive** | Driver ID + version + schema ID/version |
| `checkpoint_schema_id` | `CheckpointSchemaId` | **Non-sensitive** | Opaque string ID |
| `checkpoint_schema_version` | `RunProfileVersion` | **Non-sensitive** | Integer version |
| `model_profile_id` | `ModelProfileId` | **Non-sensitive** | Opaque string ID |
| `capability_surface_profile_id` | `CapabilitySurfaceProfileId` | **Non-sensitive** | Opaque string ID |
| `context_profile_id` | `ContextProfileId` | **Non-sensitive** | Opaque string ID |
| `steering_policy` | `SteeringPolicy` | **Non-sensitive** | Boolean flags |
| `cancellation_policy` | `CancellationPolicy` | **Non-sensitive** | Boolean flags |
| `checkpoint_policy` | `CheckpointPolicy` | **Non-sensitive** | Boolean flags + `max_checkpoint_bytes: u64` |
| `resource_budget_policy` | `ResourceBudgetPolicy` | **Non-sensitive** | Tier enum + integer limits |
| `personal_context_policy` | `PersonalContextPolicy` | **Non-sensitive** | Enum: `Excluded` / `Allowed` |
| `runtime_constraints` | `RuntimeProfileConstraints` | **Non-sensitive** | Boolean flags |
| `runner_pool_id` | `Option<RunnerPoolId>` | **Non-sensitive** | Opaque string ID |
| `scheduling_class` | `SchedulingClass` | **Non-sensitive** | Opaque string ID |
| `concurrency_class` | `ConcurrencyClass` | **Non-sensitive** | Opaque string ID |
| `resolution_fingerprint` | `RunProfileFingerprint` | **Non-sensitive** | Opaque string fingerprint |
| `provenance` | `RedactedRunProfileProvenance` | **Non-sensitive** | See RedactedRunProfileProvenance table below |

---

## Nested Type: `RedactedRunProfileProvenance`

**Module:** `crates/ironclaw_turns/src/run_profile/policy.rs`

The `Redacted` prefix signals this type was explicitly designed to strip
sensitive data from raw provenance before serialization.

| Field | Type | Classification | Rationale |
|---|---|---|---|
| `sources` | `Vec<RedactedRunProfileSource>` | **Non-sensitive** | Layer enum + source ref + human-readable summary string |
| `effective_privileges` | `Vec<PrivilegedRunProfileDimension>` | **Non-sensitive** | Enum of dimension identifiers |

`RedactedRunProfileSource.summary` is a human-readable description of the
profile source — not credential material.

---

## Nested Type: `LoopModelRouteSnapshot`

**Module:** `crates/ironclaw_turns/src/run_profile/host.rs`

| Field | Type | Classification | Rationale |
|---|---|---|---|
| `provider_id` | `String` | **Non-sensitive** | Validated by `validate_model_route_component_value` with `reject_sensitive_model_route_markers`; forbidden tokens include `api_key`, `access_token`, `secret`, `password`, `bearer`, and `sk-` prefixes |
| `model_id` | `String` | **Non-sensitive** | Same validation |
| `config_version` | `String` | **Non-sensitive** | Same validation |
| `auth_version` | `String` | **Non-sensitive** | Version token (e.g. `"v1"`, `"2024-01"`). Same validation. Despite the name, this is a configuration version token, not an authentication credential. Its allowed character set (`a-z0-9_-.:`), maximum length (128 bytes), and `reject_sensitive_model_route_markers` validation collectively prevent any raw secret from fitting |

---

## Summary

**All 12 top-level fields of `LoopRunContext` are non-sensitive.** No field
contains or wraps a credential. The deepest risk was `auth_version` on
`LoopModelRouteSnapshot`, which carries a version token but is actively
protected by `reject_sensitive_model_route_markers` — an existing production
validator that rejects `api_key`, `access_token`, `secret`, `password`,
`bearer`, and `sk-` prefixed values.

---

## Compile-Time Guard

**Outcome: Option (a)** — zero sensitive fields found. A sealed marker trait
`CredentialFree` has been added to `ironclaw_turns`:

- **Trait definition:** `crates/ironclaw_turns/src/run_profile/host.rs`
- **Export:** `ironclaw_turns::run_profile::CredentialFree`
- **Implementation:** `impl CredentialFree for LoopRunContext {}`

The trait is sealed (`private_credential_free::Sealed`), so no downstream crate
can implement it arbitrarily.  The doc comment on the trait specifies the
review gate: any PR that adds a field to `LoopRunContext` or a reachable nested
type must re-verify this checklist and update this document.

Future write-site adapters that convert `AwaitedChildSetRecord` →
`AwaitedChildRecord` should include a `where T: CredentialFree` bound on the
`parent_run_context` serialization helper to enforce the invariant at the call
site.

---

## Review Gate

Any PR modifying `LoopRunContext` or a type reachable from it must:

1. Add the new field/type to this table with a classification and rationale.
2. If classification is "Sensitive" or "Unclear": implement write-site stripping
   and add a unit test proving the field is absent in the serialized output
   before updating `impl CredentialFree for LoopRunContext`.
3. If all fields remain non-sensitive: confirm `impl CredentialFree for
   LoopRunContext {}` is still correct and update this document.
