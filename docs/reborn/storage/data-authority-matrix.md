# Reborn Data Authority Matrix

**Status:** Descriptive inventory with proposed placement

**Snapshot date:** 2026-07-24

**Scope:** Canonical and semi-persistent user, tenant, runtime, security, and
operator data. Detailed mutation behavior and migration sequencing are out of
scope.

This matrix names the owner before the store. A path or SQL table is a physical
mechanism; the owning domain defines the record grammar, invariants, and allowed
operations.

## Authority labels

| Label | Meaning |
| --- | --- |
| Canonical | The authoritative durable representation for current Reborn behavior |
| Blob | Authoritative opaque/file-shaped bytes with metadata owned elsewhere or in the entry |
| Append | Ordered durable history rather than a mutable entity |
| Derived | Rebuildable index, chunk, projection, or cache |
| Ephemeral | Process-local or expiry-bounded coordination state |
| Bootstrap | Host configuration loaded before user-domain persistence is available |
| Legacy | Schema lineage or compatibility surface; not automatically a current Reborn authority |

## Identity, users, and access

| Data family | Owning module | Current authority and shape | Scope / sensitivity | Relational pressure | Recommended target |
| --- | --- | --- | --- | --- | --- |
| Canonical user profile | `ironclaw_reborn_identity` | Canonical JSON records under `/tenant-shared/reborn-identity/users`; generic filesystem record plane | Tenant/user; PII | High: lifecycle, enumeration, references from many domains | `users` relational table |
| External login identity | `ironclaw_reborn_identity` | Canonical JSON keyed by tenant, surface, provider, instance, and subject | Tenant/user; authentication identity | Very high: compound uniqueness and user FK | `external_identities` relational table |
| Verified email link | `ironclaw_reborn_identity` | Canonical JSON secondary-index records keyed by tenant and normalized email | Tenant/user; PII | Very high: cross-provider uniqueness | `verified_emails` relational table |
| User delete tombstone | `ironclaw_reborn_identity` | Temporary canonical record during a delete cascade | User; security-sensitive coordination | Medium: exclusion and expiry semantics | Typed tombstone/status row or transaction-local coordination |
| WebUI bearer session | `ironclaw_webui` | Signed token is the record; bounded process-local revocation denylist | Tenant/user; bearer credential | High if multi-instance logout/audit is required | Durable `auth_sessions` and revocations; otherwise document stateless limitation |
| WebUI pending login flow and session ticket | `ironclaw_webui` | Bounded process-local stores around the login exchange | Tenant/user; authentication-sensitive | Medium if serving multiple instances | Keep explicitly ephemeral for single-instance mode or use expiry-indexed coordination rows |
| Legacy users and API tokens | legacy database layer | `users` and `api_tokens` migration tables | User; authentication material | Already relational | Classify per compatibility consumer; do not treat as Reborn authority by existence alone |
| Channel identity and pairing lineage | legacy DB plus Reborn product/identity services | Legacy `channel_identities` and `pairing_requests`; current Reborn flows use typed product/identity boundaries | Tenant/user; external identity | High: uniqueness and lifecycle | Converge on typed external identity and channel connection tables |

## Extensions, skills, and project data

| Data family | Owning module | Current authority and shape | Scope / sensitivity | Relational pressure | Recommended target |
| --- | --- | --- | --- | --- | --- |
| Extension package bytes | `ironclaw_extensions` | Blob/file-shaped content under `/system/extensions/{extension}`; DB-backed in production, local filesystem in selected local-dev mounts | System/tenant; executable code | Low for bytes, high for metadata | Content-addressed blob/object plane |
| Extension manifest metadata | `ironclaw_extensions` | Manifest/package records under `/system/extensions/.installations/manifests` and package tree | System/tenant | High: version, digest, source, compatibility | `extension_packages` relational metadata plus blob ref |
| Extension installation | `ironclaw_extensions` | Serialized installation record under `/system/extensions/.installations/installations` | Tenant/system | High: version choice and lifecycle | `extension_installations` relational table |
| User extension membership | `ironclaw_extensions` | User membership embedded as a set inside installation data | Tenant/user | Very high: user join, soft removal, queries | `user_extension_memberships` join table |
| Extension credential binding | product auth, secrets, extensions | Binding/account references distributed across product-auth and installation lifecycle records | Tenant/user; security-sensitive metadata | Very high: account/package/membership relationships | Typed binding table referencing account metadata only |
| Extension admin configuration | composition/product extension admin | Revisioned JSON record family under `/extension-admin-configuration/groups` | Tenant/admin; may contain sensitive configuration but not secret values | High for status, scope, revision, author; payload may evolve | Relational config header and immutable JSON revisions |
| Skill package content and registry state | skills owner plus filesystem substrate | `/system/skills` and per-user `/skills` mounts; file-shaped packages/records | System/tenant/user; executable instructions | Mixed | Blob/package plane plus relational registry/install metadata when lifecycle joins emerge |
| Project catalog and memberships | `ironclaw_projects` | Canonical records under `/tenant-shared/reborn-projects` | Tenant/user/project | High: membership, roles, lifecycle | `projects` and `project_memberships` relational tables |
| Project/workspace files | project/workspace owners | `/projects` and `/workspace` file-shaped roots | Tenant/user/project; user content | Low for bytes | Blob/object/local filesystem with typed metadata as needed |
| Runtime artifact bytes | artifact/process owners | File-shaped `/artifacts` content or process output refs | Tenant/user/run; generated content | Low for bytes, moderate for retention metadata | Blob/object plane plus relational artifact metadata when referenced across domains |

## Secrets, credentials, authentication, and settings

| Data family | Owning module | Current authority and shape | Scope / sensitivity | Relational pressure | Recommended target |
| --- | --- | --- | --- | --- | --- |
| Secret material and metadata | `ironclaw_secrets` | Encrypted canonical JSON records under scoped `/secrets/.../secrets` paths | Tenant/user/agent/project; highest sensitivity | High for metadata and versions; bytes must remain opaque | Relational metadata/version rows plus encrypted blob/KMS material |
| One-shot secret lease | `ironclaw_secrets` | Canonical records under `/secrets/.../secret-leases`, using CAS | Tenant/user/invocation; highest sensitivity | High: expiry, consume-once, FK to secret | `secret_leases` relational table with atomic claim/consume |
| Runtime credential account | `ironclaw_secrets` | Canonical encrypted records under `/secrets/.../credential-accounts` | Tenant/user/agent/project; security-sensitive | High: owner/provider/status and secret refs | Canonical `credential_accounts` table |
| Runtime credential session | `ironclaw_secrets` | Canonical encrypted records under `/secrets/.../credential-sessions` | Tenant/user/invocation; bearer capability | High: expiry/revocation/account FK | `credential_sessions` relational table |
| Product-auth flow and interaction | `ironclaw_auth` contract, composition adapter | Expiry-bounded JSON under `/secrets/.../product-auth/{surface}/flows` and `interactions` | Tenant/user/surface/session; OAuth-sensitive | Medium/high: uniqueness, expiry, claimed state | Typed short-lived tables or explicitly bounded generic records |
| Product-auth credential account | `ironclaw_auth` contract, composition adapter | JSON account records under `/secrets/.../product-auth/{surface}/accounts` | Tenant/user/provider; security-sensitive metadata | Very high: overlaps runtime account model | Converge on one canonical credential account identity and use scoped grants/views |
| Provider access/refresh material | secrets and LLM provider owners | Secret handles, encrypted records, and provider-specific local session JSON coexist | User/host; highest sensitivity | High because multiple stores can claim freshness | Secret/KMS authority with explicit import/export compatibility adapters |
| System settings | settings owner through composition | Generic record plane mounted at `/system/settings` | System/tenant/user depending setting | High when scope, revision, or audit matters | Typed settings header plus revision rows |
| LLM provider catalog | `ironclaw_reborn_config` and `ironclaw_llm` | Bootstrap file `$IRONCLAW_REBORN_HOME/providers.json` plus compiled defaults | Host/tenant; configuration | Moderate: catalog is file-shaped, selection is relational | Keep catalog bootstrap file; persist user selection separately |
| Default LLM selection | Reborn config/composition | `config.toml` bootstrap fields, with admin service coordinating updates | Host/tenant; configuration | High if per-user/per-tenant overrides exist | Explicit scoped setting, with bootstrap precedence documented |
| NEAR AI login session | `ironclaw_llm` | Local `session.json`, mode-restricted; compatibility may also use legacy settings | Host/user; bearer credential | High: freshness and single authority | Secret store/KMS authority; local file only as explicit compatibility cache |
| OpenAI Codex login session | `ironclaw_llm` | Local `openai_codex_session.json`, mode-restricted | Host/user; bearer credential | High: freshness and revocation | Secret store/KMS authority or explicit host-local authority |

The two credential-account record families are the clearest current authority
ambiguity: product-auth owns user-facing account setup and the runtime broker
owns credential issuance. They need one canonical account identity even if the
two APIs remain separate.

## Conversations, turns, and execution

| Data family | Owning module | Current authority and shape | Scope / sensitivity | Relational pressure | Recommended target |
| --- | --- | --- | --- | --- | --- |
| Thread | `ironclaw_threads` | Typed JSON records under scoped `/threads` | Tenant/user/thread; user content metadata | High: lifecycle and joins | `threads` relational table |
| Message/transcript item | `ironclaw_threads` | Typed records, summary artifacts, and idempotency records under `/threads` | Tenant/user/thread; user content | High for order/idempotency; body may be large | Relational message envelope plus inline or blob content |
| Conversation binding | `ironclaw_conversations` | Singleton typed state under `/conversations` | Tenant/user/surface/thread | High: unique binding and thread FK | Relational conversation bindings |
| OpenAI-compatible public ref and idempotency mapping | `ironclaw_reborn_openai_compat` | Canonical records under `/engine/openai_compat/refs` through `RootFilesystem` | Tenant/user/API caller; opaque public IDs | Very high: actor-scoped uniqueness and lookup | Relational public-ref and idempotency tables |
| Turn and run state | `ironclaw_turns` | Row-store records under `/turns/rows/v1`, CAS snapshots/deltas, events, and leases | Tenant/user/thread/run; control plane | Very high: transitions, claims, idempotency | `turns`, `turn_runs`, leases, and transactionally related rows |
| Loop checkpoint payload | `ironclaw_turns` | Opaque records under `/checkpoint-state`; public state stores refs | Tenant/user/run; potentially sensitive model/runtime state | Low for payload, high for metadata | Blob/object payload plus relational checkpoint ref |
| Process lifecycle | `ironclaw_processes` | Typed records, results, and output refs under `/processes` | Tenant/user/run/process | High for lifecycle; outputs are blob-like | Relational process rows plus blob result/output refs |
| Run state | `ironclaw_run_state` | Typed invocation records under `/run-state` | Tenant/user/run | High: transitions and idempotency | Relational run state or merge with canonical turn-run model |
| Approval request | `ironclaw_run_state` / approval service | Typed records under `/approvals` | Tenant/user/run; authorization-sensitive | Very high: expiry, single resolution, actor | Relational approvals table |
| Blocked run state and active gate link | `ironclaw_turns` | Run status, gate reference, blocked activity, and credential requirements are persisted in turn-run state | Tenant/user/run; recoverability-critical control plane | Very high: status/ref consistency and atomic park/resume | `turn_runs.status` plus `active_gate_ref`, updated through typed turn transitions |
| Gate record | `ironclaw_run_state` | Write-once model-visible record under `/gate-records`; no mutable lifecycle status | Tenant/user/run; redacted but security-sensitive | High: run FK, kind, retained history, typed payload refs | Immutable `gate_records` relational table plus read-only filesystem projection |
| Replay payload | replay owner | Opaque record under `/replay-payloads` | Tenant/user/run; may contain sensitive input | Low for payload relationships | Blob/generic payload referenced by relational gate or run metadata |
| Authorization lease | `ironclaw_authorization` | Typed records under `/authorization` using scoped filesystem/CAS | Tenant/user/run/capability; security-sensitive | Very high: expiry, scope, consume/fence | Relational leases with typed scope columns |
| Resource snapshot/reservation/delta | `ironclaw_resources` | Snapshot and journal records under `/resources` | Tenant/user/run; metering | Very high for reservation transaction; deltas append | Relational reservations/snapshots plus append delta log |
| Outbound policy/subscription/preferences | `ironclaw_outbound` | Typed records under `/outbound` with indexed scope projection | Tenant/user/destination; may contain PII | Very high: filtering, uniqueness, cursor state | Relational policy/subscription/preference tables |
| Outbound delivery attempt/evidence | `ironclaw_outbound` | Delivery and handoff records under `/outbound` | Tenant/user/run; external side-effect evidence | Very high: idempotency, status, provider evidence | Relational attempts plus append evidence/events |

## Events, memory, triggers, hooks, and observability

| Data family | Owning module | Current authority and shape | Scope / sensitivity | Relational pressure | Recommended target |
| --- | --- | --- | --- | --- | --- |
| Durable domain event/audit record | events and Reborn event-store owners | Append-capable `/events` backend using root filesystem event/sequence tables | Tenant/aggregate; may contain redacted user data | Append/query pressure, not mutable-entity pressure | Dedicated append streams with typed envelope and JSON payload |
| Event projection | `ironclaw_event_projections` | Derived read models | Depends on projection | Query-specific | Dedicated rebuildable projection tables |
| Memory document | `ironclaw_memory` contract and `ironclaw_memory_native` | Canonical filesystem memory documents on specialized `/memory` mount | Tenant/user/agent/project; user content | Moderate: version and scope; body document-like | Document/blob authority with typed metadata |
| Memory chunks, FTS, embeddings | memory provider/indexer | Specialized filesystem/database capabilities; legacy `memory_*` SQL lineage also exists | Same as document; derived sensitive content | Search-heavy | Rebuildable derived/search tables keyed to source version |
| Trigger definition | `ironclaw_triggers` | Dedicated `trigger_records` SQL table | Tenant/user/trigger; control plane | High | Keep dedicated relational table |
| Trigger run history | `ironclaw_triggers` | Dedicated `trigger_run_history` SQL table | Tenant/user/trigger/run | High / append-like | Keep relational history or append event projection |
| Hook predicate invocation/value | `ironclaw_hooks` | Dedicated hook predicate SQL migrations for PostgreSQL and libSQL | Tenant/hook/invocation; control plane | High | Keep dedicated relational tables |
| Trace contribution policy/queue/records | `ironclaw_reborn_traces` | Host-local files under `trace_contributions/`, including scope-hashed subtrees | Host/user scope; potentially sensitive observability data | Low for product relations; high for retention/security | Keep separate operational store with explicit policy and cleanup |
| Legacy conversations, jobs, routines, WASM tools, secrets, memory, usage logs | legacy database layer | Dedicated migration table families from V1 onward | Mixed; several are sensitive | Varies | Inventory live readers/writers, then retain, migrate, or retire explicitly |

## Summary by target plane

| Normalize first | Keep blob/file-shaped | Keep append/derived | Keep generic only with an explicit reason |
| --- | --- | --- | --- |
| users and identity links | extension package assets | domain/audit events | expiry-bounded auth flow records |
| extension installations and user memberships | project/workspace files | trigger/delivery history | opaque replay/checkpoint payloads |
| credential accounts, secret metadata, and leases | large message/process output bodies | memory chunks and embeddings | low-volume self-contained metadata |
| settings headers and revisions | encrypted secret material when held by KMS/blob backend | event projections | compatibility records during migration |
| threads, turns, runs, approvals, and authorization leases | provider catalog bootstrap file | resource delta journals | embedded/test deployments where portability wins |
| outbound policies, subscriptions, and attempts | host-local provider sessions only when explicitly authoritative | operator read models | |

## Authority rules for future designs

1. Every durable data family names exactly one canonical writer.
2. A projection or compatibility copy names its source version and repair path.
3. A local file and database row must not both be silently authoritative.
4. Domain IDs, scope, status, lifecycle timestamps, and relationship keys are
   typed columns when the data is relational.
5. Large or encrypted bytes may live outside SQL, but their metadata and
   ownership remain queryable without reading the bytes.
6. Generic filesystem records remain behind typed repositories; callers do not
   infer domain behavior from paths.
7. Legacy tables are not deleted or adopted based solely on schema inspection.
   Live readers, writers, migration compatibility, and rollback requirements
   must be proven first.

## Evidence pointers

The principal live-code and migration sources used for this snapshot are:

- [`ironclaw_reborn_composition/src/factory.rs`](../../../crates/ironclaw_reborn_composition/src/factory.rs)
  for production and local-development mounts;
- [`ironclaw_reborn_composition/src/lib.rs`](../../../crates/ironclaw_reborn_composition/src/lib.rs)
  for per-invocation alias rewriting;
- [`ironclaw_filesystem/src/record.rs`](../../../crates/ironclaw_filesystem/src/record.rs)
  for opaque bodies and indexed projections;
- [`ironclaw_reborn_identity/src/identity_store/paths.rs`](../../../crates/ironclaw_reborn_identity/src/identity_store/paths.rs)
  for the Reborn identity record families;
- [`ironclaw_extensions/src/installations.rs`](../../../crates/ironclaw_extensions/src/installations.rs)
  for installation records and embedded user membership;
- [`ironclaw_secrets/src/secret_store.rs`](../../../crates/ironclaw_secrets/src/secret_store.rs)
  for secret, lease, account, and credential-session paths;
- [`ironclaw_reborn_composition/src/product_auth/durable/paths.rs`](../../../crates/ironclaw_reborn_composition/src/product_auth/durable/paths.rs)
  for product-auth flows, interactions, and account records;
- [`ironclaw_webui/src/signed_session_login.rs`](../../../crates/ironclaw_webui/src/signed_session_login.rs)
  for signed WebUI sessions and the process-local revocation denylist;
- [`ironclaw_turns/src/turn_state_row_store`](../../../crates/ironclaw_turns/src/turn_state_row_store)
  and the other owning domain crates for runtime record families;
- [`migrations/`](../../../migrations/) for universal-root and legacy SQL
  lineage.
