# `crates/extensions/` — everything "installable package"

**Layer(s):** substrates (`ironclaw_extension_registry`, both memory providers) · runtimes (`ironclaw_extension_support`) · loops (`ironclaw_extension_host`) · products (`ironclaw_extension_manager`, the channel packages) · **Crates:** 10 (re-derive: `ls -d crates/extensions/*/` for the family crates, `ls -d crates/extensions/packages/*/` for the 15 packages, of which 5 carry crates) · **May depend on:** downward only, per crate — the registry reaches contracts + `ironclaw_filesystem`; the host reaches kernel, domains, and loop; packages reach contracts (+ the domain contract a provider implements) · **Depended on by:** the binary (`ironclaw_cli`) and `ironclaw_composition`; the registry additionally by kernel/lanes/loop/events crates that read manifest vocabulary; `ironclaw_webui` holds one sanctioned edge onto the host (pairing).

## What this family is

Every concern that follows from "an extension is an installable package", short
of the vocabulary that concern is expressed in (that lives in `contracts/`):
manifests and the durable installation records, the generic lifecycle/binding
host with its ingress verifier and egress transports, the product-side
management surface, the shared native executors, and `packages/` — one
self-contained directory per installable package. Adding a vendor is a
package-directory addition plus one binding entry at the binary; nothing
generic changes.

## The unified extension model (read this before touching anything here)

This is the most misunderstood area of the codebase. The model, in five rules:

1. **The top-level product object is always an *extension*.** A *channel* is
   not a sibling product type — it is one **capability surface** an extension's
   manifest declares, exactly like tools and auth. The surface vocabulary is
   `CapabilitySurfaceKind` (`tool` / `channel` / `auth`, + reserved kinds) in
   `crates/contracts/ironclaw_extension_contracts/src/surface.rs`. At most one
   channel surface exists per extension.
2. **Runtime is implementation, never taxonomy.** `[runtime] kind =
   "first_party" | "wasm"` (or an `[mcp]` section for hosted-MCP extensions) says
   how the extension's code *loads*. Every loader produces the identical binding
   shape (`ExtensionBindings`, `crates/extensions/ironclaw_extension_host/src/entrypoint.rs`);
   nothing in the registry, host, or manager branches on which loader built an
   extension.
3. **Two identities, never conflated.** `ExtensionId` (`slack`, `gmail`,
   `github`) is the product/installed identity. `VendorId` (`slack`, `google`,
   `github`) is the credential-authority namespace — the manifest field is
   `vendor = "…"`, and one vendor may back many extensions: six manifests here
   declare `[auth.google]` (gmail + google-calendar/docs/drive/sheets/slides;
   count them with `rg -l '^\[auth\.google\]' crates/extensions/packages/*/manifest.toml`).
   Both newtypes live in `ironclaw_host_api::ids`. (Older docs call `VendorId`
   "`ProviderId`" — the stored id strings are unchanged, and some struct fields
   still read `provider`.)
4. **Surfaces are derived, never stored as a parallel taxonomy.** The manifest
   (`schema_version = "reborn.extension_manifest.v3"`) compiles once per install
   into a resolved, digested record; production projection reads the resolved
   record, never re-parsed TOML.
5. **The retired vocabulary stays dead.** There is no separate channel
   registry, no `slack_bot`/`slack_personal` split, and no extension `kind`
   wire string. `crates/app/ironclaw_architecture_tests/tests/reborn_retired_taxonomy.rs`
   pins that vocabulary at zero occurrences across Reborn `.rs`, frontend
   `.ts`/`.js`, and `.toml` sources — if a change trips it, a deleted model is
   being reintroduced, not a style rule.

**The four-responsibility lookup** (where does this code go?):

| Concern | Home |
|---|---|
| Surface/adapter *vocabulary*: `ChannelAdapter`, `ToolAdapter`, `CapabilitySurfaceKind`, the exported conformance suite | `crates/contracts/ironclaw_extension_contracts` — outside this family, so every layer shares one vocabulary |
| Manifest schema + durable installation/membership/credential-binding/definition records | `ironclaw_extension_registry` |
| Generic hosting: lifecycle authority, loaders, activation, the vendor-blind ingress verifier, egress, hosted-MCP registration | `ironclaw_extension_host` |
| Product face: catalog UX, lifecycle commands/capabilities, credential views, the extension hub | `ironclaw_extension_manager` |
| Concrete vendor behavior: parsing, rendering, vendor calls, recipe data, provider implementations | `packages/*` (native executors for data-only packages: `ironclaw_extension_support`) |

## The crates

| Crate | Charter (one line) | Go here when |
|---|---|---|
| [`ironclaw_extension_registry`](./ironclaw_extension_registry) | Manifest schema (v3 wire / v2 internal / resolved+digest) and the durable installation, membership, credential-binding, and registered-definition records | you change what a manifest can say or what is durably recorded about installs |
| [`ironclaw_extension_host`](./ironclaw_extension_host) | The generic host: lifecycle writer + active snapshot, loaders (native/WASM/MCP), activation/removal transactions, the manifest-recipe ingress verifier, egress transports, channel identity/pairing/config service cores, hosted-MCP registration pipeline | you change how *any* extension is installed, verified, bound, activated, or delivered — never for one vendor |
| [`ironclaw_extension_manager`](./ironclaw_extension_manager) | The product face: lifecycle commands/capabilities, the lifecycle product service, admin/operator capability handlers, credential views, the extension hub | you change what a user or operator sees or does to manage extensions |
| [`ironclaw_extension_support`](./ironclaw_extension_support) | Shared support for the bundled packages: the `PACKAGES` inventory and the native tool *executors* (gsuite, web-access, coding, skills) — not itself a package | you add native, non-WASM tool logic for a data-only package |
| [`packages/slack`](./packages/slack) (`ironclaw_slack_extension`) | Protocol-only Slack `ChannelAdapter`: payload parsing, mrkdwn rendering, delivery | Slack-shaped bytes only |
| [`packages/telegram`](./packages/telegram) (`ironclaw_telegram_extension`) | Protocol-only Telegram `ChannelAdapter`: Bot API normalization + rendering | Telegram-shaped bytes only |
| [`packages/memory-native`](./packages/memory-native) (`ironclaw_memory_native`) | The default `[memory]` provider: filesystem-backed `MemoryService` implementation | the bundled memory backend's behavior |
| [`packages/mem0`](./packages/mem0) (`ironclaw_memory_mem0`) | The alternative `[memory]` provider over an external mem0 REST service | the mem0 mapping or its hardened transport |

## `packages/` — the directory rules

Every installable extension is one self-contained directory under `packages/`,
whether or not it carries a crate. Two rules from the family spec
(`docs/reborn/target-architecture/families/extensions.md`, "What belongs here"),
restated because "does this need a crate?" must be answered the same way every
time:

> **Package-directory self-containment.** Every package's manifest, prompts,
> schemas, code, and any built-artifact sources live together in one directory.
> A package's assets never live in one crate while its code lives in another.
>
> **The package-to-crate rule.** A package earns its own crate only if it
> implements a channel adapter or a provider surface — linked exclusively by
> the binary, never by a generic crate — or carries a heavy or isolated native
> dependency. Every other package is a directory of manifest and asset data
> with no crate of its own; where a package needs native, non-WASM tool logic,
> that logic lives as a module inside the shared `extension_support` crate,
> registered against the package's manifest identity.

**Anatomy.** `manifest.toml` always; `prompts/` and `schemas/` for the
model-visible copy and tool schemas; `Cargo.toml` + `src/` + `tests/` only in
the four crate-bearing packages; `wasm/` + `wasm-src/` only where tools compile
to a WASM guest. The **artifact boundary**: committed `wasm/*.wasm` guests are
built out-of-band (`./scripts/build-wasm-extensions.sh --first-party` — the
`wasm-src/` guest crates are excluded from the workspace build graph), and
`scripts/ci/check-wasm-artifact-freshness.py` keys each committed artifact to a
digest of its `wasm-src/` tree (recorded in `scripts/ci/wasm-src-digests.toml`).
Editing `wasm-src/` without rebuilding and re-recording fails CI.

**The catalog** (measured from the manifests; re-derive membership with
`ls -d crates/extensions/packages/*/`):

| Package | Extension id | Surfaces (tools / channel / auth) | Vendor | Runtime | Code |
|---|---|---|---|---|---|
| `github/` | `github` | 49 tools | `github` | wasm | data-only |
| `gmail/` | `gmail` | 6 tools | `google` | first_party | data-only (executor: `extension_support::gsuite`) |
| `google-calendar/` | `google-calendar` | 9 tools | `google` | first_party | data-only (executor: `extension_support::gsuite`) |
| `google-docs/` | `google-docs` | 11 tools | `google` | wasm | data-only |
| `google-drive/` | `google-drive` | 12 tools | `google` | wasm | data-only |
| `google-sheets/` | `google-sheets` | 11 tools | `google` | wasm | data-only |
| `google-slides/` | `google-slides` | 14 tools | `google` | wasm | data-only |
| `mem0/` | `mem0.local.memory` | 5 memory tools + `[memory]` provider | — | first_party | crate `ironclaw_memory_mem0` |
| `memory-native/` | `ironclaw.memory` | 5 memory tools + `[memory]` provider | — | first_party | crate `ironclaw_memory_native` |
| `nearai-mcp/` | `nearai` | `[mcp]` hosted server + 1 pinned tool | `nearai` (api_key) | mcp | data-only |
| `notion-mcp/` | `notion` | `[mcp]` hosted server (tools discovered) | `notion` | mcp | data-only |
| `slack/` | `slack` | 8 tools + channel | `slack` | wasm (tools) + first-party adapter | crate `ironclaw_slack_extension` + `wasm/` |
| `telegram/` | `telegram` | channel only (no tools, no auth recipe — deployment credentials via `[admin_configuration]`) | — | first_party | crate `ironclaw_telegram_extension` |
| `web-push/` | `web-push` | channel only (outbound-only browser push; no ingress, no pairing — VAPID key material auto-seeded under `[admin_configuration]`) | — | first_party | crate `ironclaw_web_push_extension` |
| `web-access/` | `web-access` | 2 tools | — | first_party | data-only (executor: `extension_support::web_access`) |

Data-only packages ship through the `PACKAGES` inventory in
`crates/extensions/ironclaw_extension_support/src/packages/mod.rs` (a module per
package embeds the directory via `include_str!`/`include_bytes!`). One embed
module is deliberately *not* in `PACKAGES`: `nearai`, whose `[mcp].server` is
patched by the host from operator LLM-admin configuration — read that module's
header before "fixing" the omission. The two memory providers are crates the
binary links; exactly one `[memory]` provider is active per deployment, and
`memory-native` ships installed by default.

## What never belongs here

- **Host authority in a package.** A package never verifies a signature, mints
  or handles verified-inbound evidence, stores or reads a raw credential,
  decides delivery/retry semantics, or writes installation state. The host
  executes each channel's manifest-declared verification recipe generically and
  injects credentials at send time on mediated egress; an adapter parses and
  renders — it can misrender a message, it cannot forge the fact that a request
  passed verification.
- **A second dispatcher, ingress router, delivery coordinator, or auth engine.**
  The four host pipelines exist exactly once, generically. Auth has no adapter
  trait at all — the engine in `crates/domains/ironclaw_auth` runs manifest
  recipes; a new vendor adds recipe *data*, not flow code.
- **A vendor name, protocol branch, or vendor-specific behavior outside
  `packages/*`** (`ironclaw_extension_support` is the one sanctioned,
  scan-exempt vendor-name home for native executors). Generic code naming a
  vendor trips `reborn_extension_specificity.rs`.
- **Reaching a domain store directly.** Product/manager code calls the host's
  authority-bearing operations and the typed capability contracts; it never
  mutates installation, credential, or thread stores around them.
- **Conversation UX** (→ `crates/product/ironclaw_assistant`), **lane
  mechanics** (→ `crates/lanes/`), **assembly** (→ `crates/app/ironclaw_composition`),
  and **surface vocabulary** (→ `crates/contracts/ironclaw_extension_contracts`).

## The rules, and what enforces them

All of the gates below run inside `cargo test -p ironclaw_architecture_tests`
unless noted; each is named so you can run it alone.

| Rule | Gate |
|---|---|
| Retired taxonomy stays at zero | `crates/app/ironclaw_architecture_tests/tests/reborn_retired_taxonomy.rs` |
| No vendor name in generic code; concrete extension crates link only from the binary and tests | `crates/app/ironclaw_architecture_tests/tests/reborn_extension_specificity.rs` (incl. `concrete_extension_crates_link_only_from_the_binary_and_tests`) |
| The manager calls the host; the host never depends on the manager (any dependency kind, dev included); the manager's `ironclaw_assistant` residue is frozen shrink-only | `crates/app/ironclaw_architecture_tests/tests/reborn_extension_manager_split.rs` |
| A product-side port the host implements is declared in `ironclaw_product_contracts`; the host's residual product-symbol list is shrink-only | `crates/app/ironclaw_architecture_tests/tests/reborn_extension_host_port_inversion.rs` |
| Hosted-MCP registration vocabulary never leaks out of `src/hosted_mcp_*` (host + registry scopes) | `crates/app/ironclaw_architecture_tests/tests/reborn_registration_pipeline_boundary.rs` |
| Per-crate forbidden edges (e.g. `extension_support` may name neither `ironclaw_host_runtime` nor `ironclaw_extension_registry`) | `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs` |
| `extension_support`'s `runtimes` demotion cannot silently widen its consumer set | `DowngradePin` in `crates/app/ironclaw_architecture_tests/tests/reborn_same_layer_edge_inventory.rs` |
| Extension-vocabulary types live in contracts, not re-derived locally | `crates/app/ironclaw_architecture_tests/tests/reborn_extension_contract_location_scan.rs` |
| Telegram keeps dependency-set parity with Slack | `crates/app/ironclaw_architecture_tests/tests/telegram_extension_gates.rs` |
| Committed WASM artifacts match their `wasm-src/` source | `python3 scripts/ci/check-wasm-artifact-freshness.py` |
| Adapter behavior conforms to the contract | `ironclaw_extension_contracts::test_support::conformance`, run by `cargo test -p ironclaw_slack_extension` / `-p ironclaw_telegram_extension` (`crates/extensions/packages/slack/tests/channel_conformance.rs` and the telegram twin) |

**Dependency direction, restated as a check:** the host consumes ports from
`product_contracts`/`extension_contracts` and never depends on
`ironclaw_assistant` (residue ratcheted by the port-inversion gate); the manager
calls the registry's and the host's authority rather than reimplementing it; a
channel package depends on contracts-tier crates only (`host_api`,
`extension_contracts`, `product_contracts`, + `ironclaw_attachments`) — never on
`ironclaw_assistant`, the registry, or the host; a provider package depends on
the domain contract it implements (`ironclaw_memory`) plus the substrates its
backend genuinely needs; and only the binary links a concrete package crate.
Verify any edge with `cargo metadata`, not by reading this file.

## Crossing out of this family

- **Up to `contracts/`** (`ironclaw_extension_contracts`, `ironclaw_host_api`)
  when a type must be shared by lanes, hosts, packages, and product — the
  adapter traits, surface kinds, ids, and sealed evidence live there.
- **Down from `app/`**: `ironclaw_composition` wires the generic seams
  (`RebornHostBindings::with_channel_extension_bindings`,
  `crates/app/ironclaw_composition/src/input.rs`) and never names a concrete
  package; the binary (`crates/app/ironclaw_cli`) is the only crate that links
  package crates.
- **Sideways to `product/`**: `ironclaw_webui` renders extension UX through the
  manager's product services and holds one documented edge onto the host (the
  pairing service core); conversation behavior belongs to
  `ironclaw_assistant`, not here.
- **Domains** (`ironclaw_auth`, `ironclaw_memory`, …) own the operations
  packages plug into: the auth engine runs recipes, the memory contract is
  implemented by the provider packages.

## Sources

`docs/reborn/target-architecture/families/extensions.md` (the family spec —
charter, per-crate dispositions, security posture) · PROPOSAL §6.8.1–§6.8.4
(dispositions + amendments), §8.1–§8.2 (layer/forbidden-edge matrix, vendor
rule) · `docs/reborn/extension-runtime/overview.md` (manifest/adapters/flows) ·
`docs/reborn/guidance-conventions.md` (this file's shape) · the
`reborn-extension-surfaces` skill (authoring walkthrough). Where this file and
the design record disagree, the code and its gates win — file a dated
correction both places.
