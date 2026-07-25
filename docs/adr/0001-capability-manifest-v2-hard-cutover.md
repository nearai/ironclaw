# ADR 0001: Extension Manifest v2 hard cutover

**Status:** Accepted
**Issue:** #3537

## Context

In Reborn, extensions declare their capabilities through an Extension Manifest.
The original v1 manifest shape exposed only package metadata, a runtime, and a
small set of capability descriptions with inline input schemas and a default
permission. That shape is too small to model the properties #3537 needs:

- portable, host-defined **Capability Profiles** that providers may claim to
  implement (so native memory, Honcho, mem0, … are interchangeable);
- **source-aware trust** — a manifest may *request* first-party/system trust, but
  only a host-bundled source may ever be *granted* it;
- mediated **Host Ports** declared per operation;
- a **cold manifest / hot surface** split, where full manifests and JSON schemas
  are registry artifacts and only a compact resolved surface reaches the model.

## Decision

Reborn uses **Extension Manifest v2** (`schema_version =
"reborn.extension_manifest.v2"`) as the single manifest model in
`ironclaw_extensions`. There is **no long-lived v1/v2 dual parser**: v1 manifests
and tests migrate to v2. The crate, the `/system/extensions/<id>/manifest.toml`
root, and the CLI/registry terminology stay as "extension".

v2 adds, per the landed implementation:

- `ManifestSource::{HostBundled, InstalledLocal, RegistryInstalled}`, supplied by
  the loader/install path (never read from TOML). Only `HostBundled` may produce
  effective FirstParty/System trust or use a reserved `ironclaw.*` id; installed
  manifests may request only `untrusted`/`third_party` and use only
  `wasm`/`mcp`/`script` runtimes. Forbidden requests are **rejected**, not
  clamped.
- Provider-prefixed capability ids with `implements = ["<profile_id>", …]`
  mappings to host-defined `CapabilityProfileContract`s.
- Per-capability extension-local `input_schema_ref` / `output_schema_ref`
  (relative paths only — absolute paths, URLs, and `..` traversal are rejected),
  with `prompt_doc_ref` required only for `visibility = "model"`.
- `visibility = "model" | "host_internal" | "api"`.
- Per-operation `required_host_ports`, validated against a host-defined
  `HostPortCatalog` (unknown ports fail closed).

A host-bundled manifest's declarations are **not** authority on their own: a
`first_party`/`system` runtime takes effect only when the host also registers a
matching service/handler (see `ironclaw.memory`, whose manifest's
`service` must match the host-registered native memory provider identity).

## Consequences

- One manifest model; no parser fork to keep in sync.
- Trust is a host-policy computation over `(ManifestSource, requested_trust)`,
  not a manifest assertion.
- Memory (and any future provider) is modeled as a v2 extension implementing
  host-defined memory profiles, which is what makes per-profile binding
  (`profile_id -> extension_id`) possible — see ADR 0002 and #3537.
