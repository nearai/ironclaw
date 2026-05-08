# Reborn Contract — Settings and Configuration

**Status:** Contract-freeze draft
**Date:** 2026-04-25
**Depends on:** [`storage-placement.md`](storage-placement.md), [`secrets.md`](secrets.md), [`extensions.md`](extensions.md), [`filesystem.md`](filesystem.md)

---

## 1. Purpose

Settings/configuration is structured control-plane state, not arbitrary memory.

Production Reborn source of truth is:

```text
typed settings repositories
```

Optional file-shaped projections may exist under:

```text
/system/settings
/system/extensions
/system/skills
```

but those projections are not the canonical storage schema unless a domain contract explicitly says so.

---

## 2. Configuration layers

Keep these layers distinct:

| Layer | Purpose | Source of truth |
| --- | --- | --- |
| Bootstrap config | process startup, DB URL, initial provider hints, deployment mode | environment/files controlled by operator |
| DB-backed settings | user/admin/runtime product settings | typed settings repository |
| Secrets | credentials and secret material | typed encrypted secret repository |
| Extension config | extension-specific validated config | typed extension config repository |
| File projections | diagnostics/editing/import/export view | generated projection over typed state |

Rules:

- secrets are never stored in settings values;
- settings may reference `SecretHandle`, never `SecretMaterial`;
- bootstrap config does not silently override DB-backed runtime settings after setup unless precedence says so;
- post-secret LLM/provider resolution must re-resolve after secrets become available.

---

## 3. Scope model

Settings may be scoped by:

```text
system/global
tenant
user
project
agent
extension
skill
```

Every setting contract must state:

1. allowed scopes;
2. default scope;
3. inheritance/override order;
4. admin-only vs user-writable;
5. whether project/agent overrides are allowed.

Recommended precedence for runtime settings:

```text
explicit invocation override
agent/project setting
user setting
tenant setting
system default
bootstrap fallback
```

A setting write must record the actor scope for audit/provenance.

---

## 4. Typed repository contract

A settings repository should expose operations like:

```rust
get(scope, key)
set(scope, key, value, actor)
delete(scope, key, actor)
list(scope, prefix)
resolve(scope_chain, key)
watch/emit change event
```

Rules:

- keys are validated path-adjacent identifiers;
- values are JSON values validated by schema when known;
- unknown keys are allowed only in namespaces explicitly marked extensible;
- writes are audited as metadata/redacted values;
- values containing secret-looking material should be rejected or require explicit secret migration.

---

## 5. Schema validation

Known settings keys should have JSON Schema definitions.

Examples:

```text
llm_backend
selected_model
llm_custom_providers
tool_permissions.shell
workspace_search
embedding_provider
extension enabled/config state
```

Rules:

- schema validation happens before persistence;
- validation reports all relevant errors where practical;
- schema errors must be stable and user-actionable;
- schema definitions are versioned with the owning domain;
- projection write-back uses the same schemas.

---

## 6. File projection contract

Typed settings may be projected under:

```text
/system/settings/{scope}/{key}.json
/system/extensions/{extension}/config.json
/system/skills/{skill}/manifest.json
```

Projection modes:

| Mode | Meaning |
| --- | --- |
| read-only | file reads reflect typed repository; writes denied |
| validated write-back | file writes validate schema and call typed repository |
| export-only | generated snapshot, not mounted for writes |

Each projection must state its mode.

Rules:

- projection paths never bypass typed repository authorization;
- projection writes must call the typed repository rather than mutate projection storage directly;
- projections do not expose secret material;
- projections include enough metadata/provenance to debug source scope without leaking private values.

---

## 7. Extension and skill config

Extension/skill config source of truth is typed state owned by extension/skill services.

Minimum lifecycle states:

```text
discovered
installed
authentication_required
authenticated
configured
active
disabled
removed
upgrade_required
failed
```

Config writes must validate against the extension/skill declared schema when present.

An extension may own private state, config, and cache roots, but it must not store cross-extension or global settings outside its namespace.

---

## 8. Events/audit

Settings changes emit redacted events/audit records:

```text
settings.changed
settings.deleted
settings.validation_failed
extension.configured
skill.configured
```

Rules:

- event payloads include key, scope, actor, version/revision, and redacted summary;
- full old/new values are not emitted by default;
- secret handles may be emitted as handles, not material;
- projection write-back records both projection path and typed key.

---

## 9. Required acceptance tests

- key validation rejects path traversal/control characters;
- schema validation rejects malformed values before persistence;
- precedence resolution chooses the nearest allowed scope;
- tenant/user/project/agent isolation;
- projection read reflects typed repository;
- projection write-back validates and updates typed repository;
- projection cannot bypass authorization;
- secret-looking values are rejected or converted to handles according to policy;
- settings change emits redacted event/audit metadata;
- PostgreSQL/libSQL repository parity where settings are DB-backed.
