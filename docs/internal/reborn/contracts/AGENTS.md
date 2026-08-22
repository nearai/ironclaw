# Agent Map — `docs/internal/reborn/contracts/`

## Purpose

This directory is the **source of truth for Reborn boundary contracts** consumed by `ironclaw_host_api`, `ironclaw_capabilities`, and `ironclaw_extension_registry`.

Files here are not implementation. They describe the vocabulary, validation rules, and capability profile contracts that Rust code in those crates must keep aligned with.

## What lives here

One `.md` contract per cross-crate domain, plus `schemas/<domain>/*.json` for
the domains whose profiles need JSON Schema refs (currently `schemas/memory/`).
Enumerate the current set with:

    ls docs/internal/reborn/contracts/*.md

`_contract-freeze-index.md` is the single frozen decision index across
all of them — start there for "who owns X" before reading an individual
contract.

## Authority rules

- These docs are authoritative when they disagree with code. If you find a mismatch, fix the code or open a contract-change request — do not silently update the doc to match drift.
- `CapabilityProfileId`, `CapabilityProfileOperationId`, `HostPortId` follow the validation rules in `host-api.md`. They are lowercase dotted names. Profile/operation ids end in `vN`. Host port ids start with `host.` and have at least three segments.
- Schema refs are extension-local relative paths. Absolute paths, URLs, `..` traversal, backslashes, NUL/control characters, colons, and any non-`[A-Za-z0-9._\-/]` character are rejected by `CapabilityProfileSchemaRef`.
- A `HostPortCatalog` is a **validation allowlist**, not a runtime implementation registry. Concrete host-port adapters live in host/runtime service crates and are constructed after authorization and obligation preparation, never from a manifest.

## When you change something here

- Update the matching Rust contract types in `crates/contracts/ironclaw_host_api/src/` and adjust callers/tests in the same branch.
- If you change a schema ref name in `memory-profiles.md`, move/rename the JSON file under `schemas/memory/` in the same commit. The `memory_profile_schema_refs_exist_on_disk` test in `ironclaw_capabilities` guards against drift but only catches *missing* files, not stale ones.
- If you add a new profile, list it explicitly in `memory-profiles.md` and add its schema refs under `schemas/<domain>/`. Mark anything that depends on unbuilt host ports (embeddings, vector search) under a "Deferred" section.
- If you change validation behavior (e.g. relax/tighten allowed characters), update both `host-api.md` and the matching validator in `host_port.rs` / `capability_profile.rs`. Both must move together; one without the other is a silent contract drift.

## Hot / cold split

- Full manifests, full JSON schemas, and full profile contracts are **cold**: stored under `docs/internal/reborn/contracts/` and reachable from the registry, never serialized into model context.
- The Hot Capability Surface (added in a later slice) is what the model sees: compact tool name, short prompt doc, selected operation ids.
- Any new artifact in this directory must be safe to keep cold. Do not put per-turn data, secrets, or user content here.

## Out of scope for this directory

- Runtime adapters, dispatch wiring, persistence schemas, sandbox plumbing.
- Per-tenant or per-user configuration (those live in `~/.ironclaw/` config or DB tables).
- Provider-specific (third-party) profile claims — those ship inside the extension's own manifest, not under `docs/internal/reborn/contracts/`.
