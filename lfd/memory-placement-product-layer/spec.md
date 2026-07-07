# Spec: memory-placement-product-layer — product/provider memory boundary

Sources: `docs/reborn/contracts/storage-placement.md`,
`docs/reborn/contracts/memory.md`,
`docs/reborn/contracts/memory-profiles.md`, `crates/ironclaw_memory/`,
`crates/ironclaw_memory_native/`, `crates/ironclaw_product_workflow/src/reborn_services.rs`,
and the retention/versioning themes from `lfd/_briefs/long-term-memory.md`.

## 1. Boundary model

`ironclaw_memory` owns provider-neutral operation shapes and the
`MemoryService` trait. Product-facing memory code depends on that contract,
not on native implementation details. `ironclaw_memory_native` is the default
provider because it is the first concrete implementation, not because product
code is allowed to special-case it.

Required operation families through the boundary:

- write;
- read;
- search;
- tree/list;
- profile set;
- prompt-context retrieval;
- version/retention queries needed to prove old content remains reachable.

The native provider may use filesystem/storage mechanics internally, but memory
path grammar, metadata inheritance, versioning, search behavior, prompt context,
and layer rules are memory semantics owned by the memory contract/provider. Raw
filesystem, control-plane, or prompt-assembly paths must not become hidden
sources of truth for memory.

## 2. Host and admin mediation

Before a provider operation runs, host/admin policy decides whether the
selected provider is allowed, denied, or constrained. Backend capability
declarations are support declarations, not authority grants.

Required decisions:

- `allow`: the provider operation runs through `MemoryService`;
- `deny`: fail closed before provider invocation, with a gate/audit record;
- `constrain.read_only`: reads/search/prompt-context may run, writes fail
  closed before provider invocation;
- `constrain.no_embeddings`: lexical or non-embedding reads may run, embedding
  search fails closed or uses an explicitly host-approved fallback;
- scope constraints: tenant/user/project/agent scope is checked by host policy
  before provider dispatch.

Audit, auth, sandboxing, storage, streams, and network remain host mediated.
Embedding/network egress must go through host network services. Providers do
not open raw egress channels or emit unsanitized host paths.

## 3. Fake-provider parity fence

The LFD runner must use a pinned fake memory provider from shared support,
outside the writable lane profile. The profile may select this provider by
provider id, but it must not contain the fake provider implementation. Product
code must not branch on provider identity. The same product-layer operation
against native and fake providers must produce structurally equivalent
operation result shape, audit/gate shape, and policy outcome.

The fake provider is intentionally not native-like internally. It should record
provider-trait calls and return deterministic result shapes from a simple
in-memory backing store. The scorer never compares fake internals to native
internals; it compares product-facing shape and host-mediated side effects.

## 4. Retention and versioning parity

This lane scores retention/versioning only as boundary parity. Write-side
correctness belongs to the write-pipeline lane, but a product/provider refactor
must not drop inherited memory guarantees:

- overwrite/update creates a new version when previous non-empty content
  changes;
- old versions remain retrievable;
- retention/version records survive provider swapping;
- no delete/truncate event appears as a shortcut for replacement;
- version attribution stays tied to invocation scope.

## 5. LFD profile input contract

Every case uses profile `memory_placement`. `setup.profile_extra` is owned by
the profile and has this shape:

```json
{
  "scenario": "short scenario class",
  "case_kind": "positive | failure_denial",
  "provider": {
    "default": "native",
    "active": "native | fake_echo | fake_counter | fake_shadow | fake_holdout",
    "compare_with": "native",
    "fake_provider_pinned_in_support": true
  },
  "policy": {
    "decision": "allow | deny | constrain",
    "constraints": ["read_only", "no_embeddings", "quota_one_write", "scope_project"],
    "denial_reason": "stable reason when denied"
  },
  "operation": {
    "family": "write | read | search | tree | profile_set | prompt_context | version_check",
    "target": "memory path or logical target",
    "query": "search/context query when applicable",
    "content": "write content when applicable",
    "append": false
  },
  "seed": {
    "tenant_id": "T...",
    "user_id": "U...",
    "project_id": "project name",
    "docs": [{"path": "notes/x.md", "content": "visible seed content"}],
    "versions": [{"path": "notes/x.md", "content": "previous content"}],
    "retention_records": [{"path": "notes/x.md", "version": 1}]
  },
  "contract_markers": ["generic strings expected in state/events/contracts"]
}
```

Cases may add fields under these objects as needed. Unknown top-level fields
outside `profile_extra` are invalid by the pinned runner schema.

## 6. State-query contract

The `memory_placement` profile must implement these state query kinds. Results
are normalized JSON read from real persisted state, recorder events, static
dependency probes, and provider call recorders. They are not copied from case
inputs.

### `memory_provider_boundary`

Params: `{"operation_ref": "...", "provider": "native|fake_*"}`.

Result:

```json
{
  "provider_id": "native",
  "provider_trait_invoked": true,
  "provider_trait_invocations": {"write": 1, "read": 0, "search": 0, "tree": 0, "profile_set": 0, "retrieve_context": 0},
  "native_backend_touched": true,
  "product_native_internal_calls": 0,
  "result_shape": "write_result | read_result | search_result | tree_result | profile_set_result | prompt_context_result",
  "status": "ok | denied | constrained"
}
```

### `memory_provider_parity`

Params: `{"operation_ref": "...", "baseline_provider": "native", "candidate_provider": "fake_*"}`.

Result:

```json
{
  "same_result_shape": true,
  "same_policy_shape": true,
  "same_audit_shape": true,
  "provider_identity_leaked": false,
  "native_only_branch": false
}
```

### `memory_policy_decision`

Params: `{"operation_ref": "..."}`.

Result:

```json
{
  "decision": "allowed | denied | constrained",
  "failure_closed": true,
  "constraints_enforced": ["read_only"],
  "denial_reason": "provider_denied | read_only | no_embeddings | scope_project | quota_exceeded",
  "provider_invoked_after_denial": false
}
```

### `memory_host_mediation`

Params: `{"operation_ref": "..."}`.

Result:

```json
{
  "audit_event_kinds": ["memory.document_written"],
  "host_network_used": true,
  "provider_direct_egress": 0,
  "raw_host_path_events": 0,
  "stream_sanitized": true,
  "storage_tier": "memory_provider_repository"
}
```

### `memory_storage_placement`

Params: `{"operation_ref": "..."}`.

Result:

```json
{
  "storage_tier": "memory_provider_repository",
  "wrong_storage_tier": false,
  "control_plane_writes": 0,
  "prompt_only_writes": 0,
  "memory_repository_writes": 1
}
```

### `memory_retention_versions`

Params: `{"path": "notes/x.md"}`.

Result:

```json
{
  "version_count": 2,
  "old_version_retrievable": true,
  "retention_record_count": 2,
  "version_record_dropped": false,
  "delete_events": 0
}
```

### `memory_dependency_boundary`

Params: `{"surface": "product|composition|host_runtime"}`.

Result:

```json
{
  "product_depends_on_native": false,
  "native_reexport_count": 0,
  "provider_specific_branches": 0,
  "fake_provider_in_support": true
}
```

### `memory_prompt_boundary`

Params: `{"operation_ref": "..."}`.

Result:

```json
{
  "prompt_context_built": true,
  "memory_write_from_prompt_assembly": false,
  "assembled_context_leaked_provider_identity": false,
  "snippet_refs_hashed": true
}
```

## 7. Eval composition

Dev set: 30 cases. Eleven are denial/failure-direction cases. Every case has
at least one state query, and the sealed contracts require state, gate, event,
or egress observations rather than reply text alone.

Holdout set: 12 off-repo cases with distinct entities, dates, providers, and
one additional sub-scenario class: provider fallback after a denied primary.
Holdout answers use a different canary from dev answers.

## 8. Non-goals

- Building a new memory implementation.
- Scoring retrieval quality beyond provider-boundary parity.
- Scoring write-pipeline artifact classification.
- Adding a live external memory provider.
- Moving memory semantics into filesystem, prompts, or generic storage
  primitives.

## 9. Risk and rollback notes

This work touches auth, storage, network, audit, and persistent memory. Treat
policy bypass, raw provider egress, wrong-tier writes, and retention loss as
stop-the-run issues. Rollback should restore the previous native default path
while preserving the provider-neutral contract crate and tests that prove no
data was lost.
