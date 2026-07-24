# Target Relational Model

**Status:** Proposed architecture, not a frozen schema

**Date:** 2026-07-24

**Purpose:** Show which relationships the database should eventually understand
without replacing Reborn's typed domain ownership or universal dispatch seam.

The entities below are conceptual. Names, columns, and boundaries must be
validated against each owning crate before a migration is written.

Adopting normalized domain adapters would deliberately revise the accepted
"one entry type" persistence decision and the current repository rule that new
domain persistence uses `RootFilesystem`. This document is not implementation
permission: the ADR, storage contracts, architecture tests, and repository
guidance must be updated together before the first normalized Reborn adapter
lands.

## Target storage planes

The target is a hybrid model with four physical planes:

| Plane | Use it for | Primary guarantees |
| --- | --- | --- |
| Relational control plane | identity, membership, lifecycle, settings, credential metadata, runtime coordination | foreign keys, uniqueness, checks, transactions, typed indexes |
| Blob/object plane | compiled extensions, package assets, project files, large outputs, opaque checkpoint payloads | content integrity, streaming, size isolation, retention |
| Append plane | domain events, audit evidence, delivery history, resource deltas | ordering, immutability, cursors |
| Derived/search plane | memory chunks, embeddings, full-text indexes, event projections | rebuildability and query performance |

`RootFilesystem` can continue to route the blob, generic-record, and append
planes. Domain repository adapters should route relational entities to typed
tables while preserving the same public domain interfaces.

## Relational placement rules

Put a value in a typed column when it participates in identity, scope,
uniqueness, joins, lifecycle, authorization, retention, or routine filtering.
Use JSON only for an evolving provider payload or genuinely unstructured
metadata. Do not put a foreign key solely inside JSON.

A conventional mutable entity should normally carry:

```text
id
tenant_id
owner_user_id or another explicit owner
status
created_at
updated_at
version
removed_at or archived_at when history must remain
```

The exact columns vary by domain. `version` is an optimistic concurrency value,
not a substitute for a transaction when several rows form one invariant.

Large immutable bytes should be content-addressed in the blob plane and
referenced by digest/key. The relational row owns metadata and lifecycle; the
blob store owns bytes. References to an object store or external provider are
logical references and cannot use a physical SQL foreign key.

## Identity and access

```mermaid
erDiagram
    TENANTS {
        string id PK
        string status
        datetime created_at
        datetime updated_at
    }

    USERS {
        string id PK
        string tenant_id FK
        string status
        string display_name
        datetime removed_at
        datetime created_at
        datetime updated_at
    }

    EXTERNAL_IDENTITIES {
        string id PK
        string tenant_id FK
        string user_id FK
        string surface
        string provider
        string provider_instance
        string subject
        datetime created_at
        datetime last_seen_at
    }

    VERIFIED_EMAILS {
        string tenant_id PK, FK
        string normalized_email PK
        string user_id FK
        datetime verified_at
    }

    AUTH_SESSIONS {
        string id PK
        string tenant_id FK
        string user_id FK
        string status
        datetime expires_at
        datetime revoked_at
        datetime created_at
    }

    API_TOKENS {
        string id PK
        string tenant_id FK
        string user_id FK
        string secret_digest
        string status
        datetime expires_at
        datetime revoked_at
        datetime created_at
    }

    TENANTS ||--o{ USERS : contains
    TENANTS ||--o{ EXTERNAL_IDENTITIES : scopes
    USERS ||--o{ EXTERNAL_IDENTITIES : resolves
    USERS ||--o{ VERIFIED_EMAILS : verifies
    USERS ||--o{ AUTH_SESSIONS : owns
    USERS ||--o{ API_TOKENS : owns
```

Recommended constraints include:

- unique external identity on
  `(tenant_id, surface, provider, provider_instance, subject)`;
- unique verified email on `(tenant_id, normalized_email)`;
- user, identity, session, and token tenant IDs must agree;
- status values use closed database checks or database enums;
- bearer material is never stored in plaintext.

The current signed WebUI session is stateless apart from an in-process bounded
revocation denylist. `AUTH_SESSIONS` is therefore a target choice if durable
logout, multi-instance revocation, or operator session inspection is required,
not a description of the present implementation.

## Extensions and user installation state

```mermaid
erDiagram
    EXTENSION_PACKAGES {
        string id PK
        string extension_key
        string version
        string manifest_digest
        string asset_object_key
        string source
        datetime created_at
    }

    EXTENSION_INSTALLATIONS {
        string id PK
        string tenant_id
        string package_id FK
        string status
        datetime activated_at
        datetime removed_at
        datetime created_at
        datetime updated_at
        int row_version
    }

    USER_EXTENSION_MEMBERSHIPS {
        string id PK
        string installation_id FK
        string user_id
        string status
        datetime installed_at
        datetime disabled_at
        datetime removed_at
        datetime updated_at
        int row_version
    }

    EXTENSION_CREDENTIAL_BINDINGS {
        string id PK
        string membership_id FK
        string credential_account_id
        string requirement_key
        string status
        datetime removed_at
        datetime created_at
        datetime updated_at
    }

    EXTENSION_ADMIN_CONFIGS {
        string id PK
        string tenant_id
        string extension_key
        string status
        int current_revision
        datetime removed_at
        datetime created_at
        datetime updated_at
    }

    EXTENSION_ADMIN_CONFIG_REVISIONS {
        string config_id PK, FK
        int revision PK
        string schema_version
        json config_payload
        string changed_by_user_id
        datetime created_at
    }

    EXTENSION_PACKAGES ||--o{ EXTENSION_INSTALLATIONS : instantiates
    EXTENSION_INSTALLATIONS ||--o{ USER_EXTENSION_MEMBERSHIPS : grants
    USER_EXTENSION_MEMBERSHIPS ||--o{ EXTENSION_CREDENTIAL_BINDINGS : binds
    EXTENSION_ADMIN_CONFIGS ||--o{ EXTENSION_ADMIN_CONFIG_REVISIONS : versions
```

This replaces the highest-pressure current shape: installation membership is
embedded in serialized installation data, while compiled package bytes,
manifests, and mutable lifecycle state share one virtual namespace.

Important target rules:

- package rows are immutable and unique by extension/version/content digest;
- package bytes live in the blob plane, not in a lifecycle row;
- a tenant installation points to exactly one package version at a time;
- user membership is a join entity, not a set embedded in an installation blob;
- disabling/removing a membership updates status/timestamps rather than
  destroying its history by default;
- credential bindings reference credential metadata, never secret material;
- evolving admin configuration may remain JSON, but identity, scope, status,
  revision, and authorship are typed columns;
- revision rows make configuration history explicit instead of requiring the
  current blob to carry its own audit history.

`USER_EXTENSION_MEMBERSHIPS` should have a unique constraint on
`(installation_id, user_id)`. The surrogate ID gives credential bindings a
simple foreign key while the compound uniqueness still expresses membership
identity.

## Secrets, credentials, and settings

```mermaid
erDiagram
    SECRET_RECORDS {
        string id PK
        string tenant_id
        string owner_user_id
        string agent_id
        string project_id
        string name
        string status
        string current_version_id FK
        datetime removed_at
        datetime created_at
        datetime updated_at
    }

    SECRET_VERSIONS {
        string id PK
        string secret_id FK
        bytes ciphertext
        string key_version
        datetime created_at
    }

    SECRET_LEASES {
        string id PK
        string secret_id FK
        string invocation_id
        string status
        datetime expires_at
        datetime consumed_at
        datetime created_at
    }

    CREDENTIAL_ACCOUNTS {
        string id PK
        string tenant_id
        string owner_user_id
        string provider
        string provider_subject
        string status
        json provider_metadata
        datetime removed_at
        datetime created_at
        datetime updated_at
    }

    CREDENTIAL_ACCOUNT_SECRETS {
        string credential_account_id PK, FK
        string purpose PK
        string secret_id FK
        datetime created_at
    }

    CREDENTIAL_SESSIONS {
        string id PK
        string credential_account_id FK
        string invocation_id
        string status
        datetime expires_at
        datetime revoked_at
        datetime created_at
    }

    SETTINGS {
        string id PK
        string tenant_id
        string owner_user_id
        string namespace
        string key
        string status
        int current_revision
        datetime removed_at
        datetime created_at
        datetime updated_at
    }

    SETTING_REVISIONS {
        string setting_id PK, FK
        int revision PK
        json value
        string changed_by_user_id
        datetime created_at
    }

    SECRET_RECORDS ||--o{ SECRET_VERSIONS : versions
    SECRET_RECORDS ||--o{ SECRET_LEASES : leases
    CREDENTIAL_ACCOUNTS ||--o{ CREDENTIAL_ACCOUNT_SECRETS : references
    SECRET_RECORDS ||--o{ CREDENTIAL_ACCOUNT_SECRETS : supplies
    CREDENTIAL_ACCOUNTS ||--o{ CREDENTIAL_SESSIONS : issues
    SETTINGS ||--o{ SETTING_REVISIONS : versions
```

Secret metadata and encrypted material can be separated further if a KMS/HSM
owns ciphertext. The invariant is that user/provider/account relationships are
queryable without decrypting secrets, while secret values remain inaccessible
to generic listing, logging, or model-facing paths.

Product-auth flow and interaction records are short-lived coordination state.
They may use a relational flow table with explicit expiry and uniqueness or a
generic record plane if their lifecycle is strictly bounded. Durable credential
accounts and runtime credential sessions should converge on one canonical
account identity model rather than retaining competing account records.

Settings must also preserve the boundary among:

- bootstrap configuration in `config.toml`;
- provider catalog definitions in `providers.json`;
- mutable persisted settings;
- encrypted credentials and provider session material.

Moving everything into one `settings` table would recreate the same ambiguity
in a different form.

## Conversation and runtime state

```mermaid
erDiagram
    THREADS {
        string id PK
        string tenant_id
        string owner_user_id
        string project_id
        string status
        datetime created_at
        datetime updated_at
    }

    MESSAGES {
        string id PK
        string thread_id FK
        string actor_kind
        string content_ref
        datetime created_at
    }

    TURNS {
        string id PK
        string thread_id FK
        string status
        string idempotency_key
        datetime created_at
        datetime updated_at
    }

    TURN_RUNS {
        string id PK
        string turn_id FK
        string status
        string active_gate_ref FK
        int attempt
        datetime lease_expires_at
        datetime started_at
        datetime finished_at
        int row_version
    }

    GATE_RECORDS {
        string gate_ref PK
        string run_id FK
        string kind
        string safe_summary
        string activity_id
        json payload
        datetime created_at
    }

    CHECKPOINT_REFS {
        string id PK
        string run_id FK
        string object_key
        string digest
        datetime created_at
    }

    PROCESSES {
        string id PK
        string run_id FK
        string status
        string result_ref
        datetime started_at
        datetime finished_at
        int row_version
    }

    APPROVALS {
        string id PK
        string run_id FK
        string gate_ref FK
        string status
        string capability_id
        datetime expires_at
        datetime resolved_at
        int row_version
    }

    RESOURCE_RESERVATIONS {
        string id PK
        string run_id FK
        string gate_ref FK
        string resource_kind
        string status
        int reserved_amount
        int consumed_amount
        datetime expires_at
    }

    AUTH_GATE_FLOWS {
        string id PK
        string gate_ref FK
        string status
        datetime expires_at
        datetime completed_at
    }

    DEPENDENT_RUN_WAITS {
        string gate_ref PK, FK
        string dependent_run_id FK
        string result_ref
        datetime completed_at
    }

    EXTERNAL_TOOL_CALLS {
        string gate_ref PK, FK
        string call_id
        string status
        string output_ref
        datetime resolved_at
    }

    OUTBOUND_ATTEMPTS {
        string id PK
        string run_id FK
        string destination_kind
        string status
        string provider_evidence_ref
        datetime attempted_at
    }

    EVENT_STREAMS {
        string id PK
        string tenant_id
        string aggregate_kind
        string aggregate_id
        int next_sequence
    }

    EVENT_RECORDS {
        string stream_id PK, FK
        int sequence PK
        string event_kind
        json payload
        datetime occurred_at
    }

    THREADS ||--o{ MESSAGES : contains
    THREADS ||--o{ TURNS : receives
    TURNS ||--o{ TURN_RUNS : attempts
    TURN_RUNS ||--o{ GATE_RECORDS : encounters
    GATE_RECORDS ||--o| APPROVALS : may_require
    GATE_RECORDS ||--o| AUTH_GATE_FLOWS : may_require
    GATE_RECORDS ||--o| RESOURCE_RESERVATIONS : may_require
    GATE_RECORDS ||--o| DEPENDENT_RUN_WAITS : may_require
    GATE_RECORDS ||--o| EXTERNAL_TOOL_CALLS : may_require
    TURN_RUNS ||--o{ CHECKPOINT_REFS : checkpoints
    TURN_RUNS ||--o{ PROCESSES : launches
    TURN_RUNS ||--o{ APPROVALS : waits_for
    TURN_RUNS ||--o{ RESOURCE_RESERVATIONS : reserves
    TURN_RUNS ||--o{ OUTBOUND_ATTEMPTS : emits
    EVENT_STREAMS ||--o{ EVENT_RECORDS : appends
```

Message bodies, process outputs, and checkpoint payloads may be blob references
rather than inline values. Small message text can remain inline if size,
encryption, and retention constraints support it.

This diagram does not collapse conversation, turn coordination, process
lifecycle, approval, resource, or outbound semantics into one repository.
Each owning crate still defines its allowed operations. The diagram only makes
the cross-domain identifiers and physical relationships explicit.

### Blocked gates

Blocked state has two distinct authorities:

- `TURN_RUNS.status` plus `active_gate_ref` is the canonical answer to whether a
  run is currently parked;
- `GATE_RECORDS` is the write-once, retained, model-visible explanation and
  resume payload keyed by the opaque gate reference.

`GATE_RECORDS` deliberately has no mutable lifecycle status. Approval, auth,
resource, dependent-run, and external-tool modules own their respective
resolution state. A run may encounter multiple gates over its history, while
`active_gate_ref` points to at most one current gate.

The turn module continues to own the park/resume logic. The relational adapter
should make a gate transition atomic:

1. insert the immutable gate record;
2. insert or update the gate-kind-specific coordination record;
3. set the run's blocked status and `active_gate_ref` using the expected row
   version;
4. append or enqueue the blocked lifecycle event.

Resume checks actor ownership, expected blocked status, gate reference, and row
version immediately before resolving the kind-specific record and clearing the
active gate. The historical gate record remains. Database checks can require a
blocked status to have an active gate and a non-blocked status not to have one;
the typed turn transition enforces that the status variant matches the
referenced gate kind.

## Derived and search data

Memory chunks, embeddings, full-text search indexes, event projections, and
operator dashboards are derived data. Their schema should carry a source
version/digest and be safely rebuildable from canonical documents or events.

Do not make a search index the only record of:

- a memory document;
- an extension installation;
- a credential binding;
- a message;
- a completed external side effect.

Provider-issued or durable host evidence for side effects belongs in canonical
event/outbound records, even if a projection makes that evidence easier to
query.

## Preserving filesystem and directory behavior

The virtual filesystem remains a useful product and runtime interface. The
target changes which layer is authoritative, not whether callers can navigate
paths.

There are three mount patterns:

| Mount pattern | Canonical physical store | Filesystem behavior |
| --- | --- | --- |
| Native file/blob mount | local filesystem, object store, or DB-backed bytes | `get`, `put`, `delete`, `list`, and `stat` operate on real file-shaped entries |
| Relational projection mount | normalized domain tables | `get`, `list`, `stat`, and bounded `query` synthesize directory entries and JSON views from rows |
| Generic record mount | `root_filesystem_entries` | existing record/CAS behavior for explicitly retained schemaless domains and migration compatibility |

For example:

```text
/system/extensions/github/manifest.toml
  -> native package/blob content

/system/extensions/.installations/installations/{installation_id}.json
  -> read-only projection assembled from extension_installations,
     user_extension_memberships, and extension_packages

/gate-records/{gate_ref}.json
  -> read-only projection of one gate_records row
```

`CompositeRootFilesystem` and `MountView` can keep longest-prefix routing,
virtual directory semantics, and tenant/user scope enforcement. A relational
projection adapter may satisfy the read-oriented subset of `RootFilesystem` for
its mounted path. Generic `put` and `delete` on that projection must be rejected
unless they validate and invoke the owning typed domain operation; directly
editing synthesized JSON would bypass foreign keys, lifecycle transitions,
audit, and authorization.

The relational row ID is authoritative. Projection paths are deterministic,
reversible encodings of typed IDs, not independent identities. Directory
listing is a query over typed scope and parent columns, not a scan whose only
understanding of hierarchy comes from string prefixes.

This preserves current runtime-facing paths while separating two concerns:

```mermaid
flowchart LR
    caller["Runtime or operator filesystem caller"]
    mount["MountView and CompositeRootFilesystem"]
    files["Native file/blob backend"]
    projection["Relational projection adapter"]
    repository["Owning typed domain module"]
    tables["Normalized domain tables"]

    caller --> mount
    mount --> files
    mount --> projection
    projection --> repository
    repository --> tables
```

Normal product mutations call the typed domain module directly. The projection
adapter is primarily a compatibility, inspection, import/export, and
runtime-readable view. This keeps the module interface deep: callers do not
need to know which physical plane stores a path, while relational invariants
remain inside the owning module and database adapter.

## What happens to `root_filesystem_entries`

The table should remain available, but its role narrows:

1. blob-like DB-backed entries for embedded deployments;
2. opaque, self-contained, low-volume records;
3. compatibility records during staged migrations;
4. projections or import/export views;
5. backend-neutral test implementations.

Each normalized domain needs a compatibility decision:

- dual-read with one authoritative writer;
- backfill and cut over;
- projection from normalized tables into the virtual filesystem;
- or continued generic storage with an explicit reason.

Dual-write without an explicit authority and repair rule is not an acceptable
steady state.

## Transaction and deletion principles

Schema design should make lifecycle visible even though detailed mutations are
out of scope here:

- use status plus lifecycle timestamps for user-visible installation,
  membership, account, and configuration history;
- use immutable revision/version rows for values whose history matters;
- use hard deletion or cryptographic erasure when policy requires actual secret
  destruction;
- place rows that must change atomically in one database transaction boundary;
- use an outbox/event record when a database transaction must coordinate with a
  blob store or external provider;
- never depend on an untyped JSON field to implement a cascade or uniqueness
  invariant.

## Compatibility boundary

PostgreSQL and libSQL remain required production/embedded targets unless a
future contract explicitly changes that rule. Proposed types and constraints
must use a shared subset or have tested dialect adapters. Domain-specific
repositories should hide those differences from callers.
