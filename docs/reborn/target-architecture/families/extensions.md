# `crates/extensions/` — everything "installable package"

**Layer(s):** substrates (`ironclaw_extension_registry`), loops (`ironclaw_extension_host`, `ironclaw_extension_support`), products (`ironclaw_extension_manager`, every package crate) · **Crates:** 8 — `ironclaw_extension_registry`, `ironclaw_extension_host`, `ironclaw_extension_manager`, `ironclaw_extension_support`, `ironclaw_slack_extension`, `ironclaw_telegram_extension`, `ironclaw_memory_native`, `ironclaw_memory_mem0` · **Security posture:** the host-to-extension trust membrane — a single generic verifier is the only code permitted to mint sealed verified-inbound evidence; concrete packages parse, render, and serve their declared surfaces but can never construct trust, and only the binary may link a concrete package crate.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/extensions/
├── ironclaw_extension_registry       manifests & installation records
├── ironclaw_extension_host           generic host: verify, bind, deliver
├── ironclaw_extension_manager        product-side extension management
├── ironclaw_extension_support       shared native executors & the package inventory
└── packages/                         one self-contained directory per package
    ├── slack/                        adapter crate + manifest + assets
    ├── telegram/                     adapter crate + manifest + assets
    ├── github/                       manifest, prompts, schemas, wasm (data only)
    ├── memory-native/                provider crate: native memory backend
    ├── mem0/                         provider crate: mem0-backed memory
    └── …                             gmail, google-*, web-access, notion-mcp, …
```

Every directory under `packages/` is one installable extension, and every one of them is first-party in the shipping sense — bundled with the binary. The crate named `extension_support` is not a package: it is the shared support crate holding the package inventory and the native tool executors that serve many packages at once. A package directory carries a crate only when it must (a channel adapter or a provider implementation); otherwise it is pure data. The two memory providers are packages like any other: each declares a `[memory]` provider surface in its manifest and implements the provider-neutral contract that lives in `domains/` — exactly one of them is active per deployment, and the native one ships installed by default so memory is always available.

## Role

`crates/extensions/` holds every concern that follows from "an extension is an installable package," short of the vocabulary that concern is expressed in. An extension is the only installable product object; its surfaces are tool, channel, auth, and provider surfaces such as memory; a vendor identity is a credential-authority namespace, never a product identity; at most one channel surface exists per extension. Four host pipelines — dispatch, inbound routing, outbound delivery, and auth — are implemented exactly once, generically, and this family owns the generic implementations of routing and hosting plus the product-facing orchestration around all four. Adding a new vendor, in this design, is a package-directory addition: a manifest, an implementation of the relevant adapter contract, and one binding entry at the binary — nothing elsewhere in the family changes.

## Boundaries — what makes this family distinct

- **The four separated responsibilities, and where each lives.** *Vocabulary* — the `ChannelAdapter`/`ToolAdapter` contracts, manifest-surface descriptors, auth recipes, installation-state vocabulary, and the sealed verified-inbound-evidence constructor — lives outside this family entirely, in contracts, so that lanes, hosts, packages, product, and the manager can share one vocabulary without any of them importing a registry. *Registry and records* is `ironclaw_extension_registry` alone: manifest schema plus the durable installation store. *Generic hosting* is `ironclaw_extension_host` alone: lifecycle, binding, activation, the ingress verifier, egress transport. *Product-side management* is `ironclaw_extension_manager` alone: catalog, lifecycle commands, configuration and pairing UX. *Concrete package behavior* is `packages/*`: parsing, rendering, vendor calls, recipe data, and provider implementations behind domain contracts.
- **vs product** — extension-management UX vs conversation UX. Installing, configuring, and pairing an extension is a different surface from having a conversation with the agent; each has its own product-surface implementation, and neither reaches into the other's store to act.
- **vs lanes** — package vs mechanism, two independent axes. A package is a distribution, vendor, and configuration unit: what ships, what it may touch, how it is configured. A lane is an execution mechanism: how already-authorized code runs. A package's manifest declares which lane its code runs under; a package never depends on a lane crate, and a lane never depends on a package crate.
- **Runtime kind is loading, never taxonomy.** Every loader — native, WASM, or MCP — produces the identical binding shape from an extension's entrypoint. Nothing in the registry, the host, or the manager branches on which loader built a given extension; the choice is a manifest field, not a product category.

## What belongs here / What never belongs here

**Belongs:** manifest schema and installation/membership records; the generic lifecycle, binding, activation, and removal state machine; the generic ingress verifier and egress transports; product-facing catalog, lifecycle-command, configuration, and pairing services; concrete per-vendor parsing, rendering, and vendor calls; provider implementations of domain contracts, shipped as packages. **Never belongs:** a second dispatcher, ingress router, delivery coordinator, or auth engine defined per package — the four host pipelines are implemented exactly once, generically, and every package consumes the same one; a vendor name or protocol branch anywhere outside `packages/`.

**Package-directory self-containment.** Every package's manifest, prompts, schemas, code, and any built-artifact sources live together in one directory. A package's assets never live in one crate while its code lives in another.

**The package-to-crate rule.** A package earns its own crate only if it implements a channel adapter or a provider surface — linked exclusively by the binary, never by a generic crate — or carries a heavy or isolated native dependency that would otherwise leak into every consumer of a shared package crate. Every other package is a directory of manifest and asset data with no crate of its own; where a package needs native, non-WASM tool logic, that logic lives as a module inside the shared `extension_support` crate rather than as a crate on its own.

**The bundled set, concretely.** `packages/` ships with four crate-bearing packages — `slack/` and `telegram/` (channel adapters) and `memory-native/` and `mem0/` (the two `[memory]` providers) — and the data-only directories `github/`, `gmail/`, `google-drive/` and its `google-*` siblings (one directory per extension; they share the `google` credential authority but are separate product objects), `web-access/`, `notion-mcp/`, and `nearai-mcp/`. A data-only package is its manifest plus prompts, schemas, and any committed WASM artifacts with their build sources beside them; its tools execute under whichever lane the manifest declares (WASM for github and the gsuite tools' portable half, MCP for notion-mcp and nearai-mcp), and any native executor it needs lives as a module in `extension_support`, registered against the package's manifest identity — never as a crate of its own.

## Dependency direction

`ironclaw_extension_host` consumes ports defined in `product_contracts` and `extension_contracts` — it never depends on `ironclaw_assistant` directly, and never names a concrete product type. `ironclaw_extension_manager` depends on `product_contracts`, `extension_contracts`, `ironclaw_extension_registry`, and `ironclaw_extension_host`, calling their authority rather than reimplementing it; nothing else in this family depends on the manager. Every channel-package crate depends on `extension_contracts` alone. A provider-package crate additionally depends on the domain contract it implements (`ironclaw_memory`) and the substrates its backend genuinely needs — never on a host, kernel, or product crate. `ironclaw_extension_registry` depends only on a storage substrate and contracts vocabulary.

## Security & authority

A generic verifier executes each channel's manifest-declared recipe — a signature scheme, the secret handle to check the signature against, and the exact shape the signature takes — identically for every vendor, and only once that check passes does it mint the sealed verified-inbound evidence that everything downstream trusts. A channel adapter's inbound handler parses only: it can misinterpret content, but it can never forge the fact that a request passed that check. Outbound, delivery is mediated the same way in reverse: a package's adapter renders and calls the vendor only through the host's egress path, with a credential injected by the host at send time — the adapter never holds a raw, storable secret of its own.

## Crates

### `ironclaw_extension_registry`

- **Purpose:** manifest schema and the durable registry of what is installed, by whom, and with which credentials bound.
- **Owns:** the wire manifest schema and an internal normal form, plus a resolved-and-digested form the rest of the system reads instead of re-parsing declarative data; the in-memory catalog; the durable installation, membership, and credential-binding records.
- **Never contains:** execution of any kind; a secret; a trust decision; a vendor name.
- **Public surface:** the registry and the installation-record store, read by the generic host and by the product-management crate.
- **Depends on:** a storage substrate and contracts (`host_api`, `extension_contracts`).
- **Never depends on:** `ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel crate.
- **Security & authority role:** the installation-lifecycle record authority — it records what is installed; it never decides whether an effect is allowed.
- **Why a separate crate:** a record authority with a genuine persistence obligation and a manifest grammar many crates read; keeping it apart from the generic host keeps a stateful, compare-and-swap-mutated store out of a crate whose other job is verifying inbound trust.

### `ironclaw_extension_host`

- **Purpose:** the generic host — lifecycle writer, loaders, binding and activation, the vendor-blind ingress verifier, and egress transports. No concrete vendor name, protocol route, or behavior branch belongs here.
- **Owns:** the extension lifecycle writer and the active-installation snapshot; the loader trio — native, WASM, MCP — and binding checks; activation and removal transactions; the ingress router and the manifest-recipe verifier that mints sealed verified-inbound evidence; egress transports; the generic channel-identity, connection, configuration, and pairing mechanisms every vendor shares.
- **Never contains:** a concrete vendor name, protocol route, or behavior branch; product-facing catalog, credential-view, or administrator UX — that is the product-management crate's job; a route framework of its own — any exposed route is owned and mounted by the product-facing transport crate.
- **Public surface:** the generic hosting operations — install, activate, bind, remove — and the ingress-verification path, consumed by the product-management crate and, indirectly through the router, by every package.
- **Depends on:** `ironclaw_extension_registry`; the kernel crates it hosts activity for — `ironclaw_capabilities`, `ironclaw_authorization`, `ironclaw_processes`, `ironclaw_resources`, `ironclaw_trust`, `ironclaw_turns`; and contracts (`extension_contracts`, `product_contracts` for the ports it implements). Secret material it never touches directly — credentials are staged and injected by the kernel's mediated egress path.
- **Never depends on:** `ironclaw_assistant`. Every product-facing operation it exposes is defined as a port in a contracts crate and implemented here against that port, never against a concrete product type.
- **Security & authority role:** the raw-request-to-verified-inbound membrane, and the sole writer of installation-lifecycle state transitions.
- **Why a separate crate:** the generic hosting machinery carries a real trust job — verification, binding, activation — that must stay free of any vendor or product-specific branch; a crate boundary is what makes "no concrete product name here" a fact that can be checked, not merely a convention.

### `ironclaw_extension_manager`

- **Purpose:** the product face of extensions — everything a user or operator does to discover, install, configure, and pair an extension.
- **Owns:** the available-extension catalog and its import path; lifecycle commands and capabilities, exposed through their own product-surface implementation; the channel-configuration product service; pairing-workflow orchestration; credential views; administrator, operator, and skill-activation capability handlers.
- **Never contains:** lifecycle authority — it always calls the generic host, never mutates an installation record on its own; ingress verification; a vendor name outside recipe data.
- **Public surface:** a product-surface implementation scoped to extension management, plus the product-facing services — channel configuration, account-connection status, lifecycle commands — that contracts define as ports.
- **Depends on:** `product_contracts`, `extension_contracts`, `ironclaw_extension_registry`, `ironclaw_extension_host`.
- **Never depends on:** the conversation-facing product crate; any package crate directly; `ironclaw_extension_registry`'s store except through `ironclaw_extension_host`'s authority-bearing operations.
- **Security & authority role:** none of its own — a product-UX layer that calls the generic host's authority-bearing operations rather than duplicating them.
- **Why a separate crate:** a coherent product sub-owner with its own surface, distinct from the conversation-facing product crate and from the generic host whose authority it only ever calls.

### `packages/` — directory rules

Every installable extension's manifest, prompts, schemas, code, and any built-artifact sources live together in its own directory under `packages/` — one directory per extension, uniformly, whether or not it carries a crate. A package earns its own crate only when it implements a channel adapter or a provider surface — linked exclusively by the binary, never by any generic crate — or when it carries a heavy or isolated native dependency that would otherwise leak into every consumer of a shared crate. Every other package is a directory of manifest and asset data with no crate of its own; where a package needs native, non-WASM tool logic, that logic lives as a module in the shared `extension_support` crate, registered against the package's manifest identity. Packages whose tools compile to a portable component artifact keep their build sources beside their manifest, entirely self-contained and independent of the workspace's own build graph: the artifact ships with the package, and the source that produced it travels with it, never housed as a separate concern elsewhere.

### `ironclaw_extension_support` (`crates/extensions/ironclaw_extension_support/`)

- **Purpose:** the shared support crate for the bundled packages — the package inventory and the native tool executors that serve many packages at once. It is not itself a package: every installable extension lives in its own directory under `packages/`.
- **Owns:** the package inventory (which package directories ship, with which trust-effect declarations); native tool executors — general-purpose file, text, and search tooling, groupware integrations, web access; the generic builtin tool implementations — file, shell, http, time, memory, trigger management, skill management and installation, and telemetry submission — available to any loop by capability grant, not by extension identity.
- **Never contains:** a type only a loop-hosting crate should hold; this crate is invoked only through the same capability-dispatch path any tool uses.
- **Public surface:** tool-adapter implementations for its bundled tools, consumed through the generic dispatch path.
- **Depends on:** `ironclaw_auth`, `ironclaw_extractors`, a storage substrate, `ironclaw_observability`, `ironclaw_safety`, `ironclaw_skills`, `extension_contracts`; the domains its bundled tools need — memory, traces, triggers — by declared charter.
- **Never depends on:** `ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or `ironclaw_loop_host` — a package is content, not a host.
- **Security & authority role:** none — first-party status raises a policy ceiling, it never grants permission; every tool call still crosses the same authorization and approval stages as any other capability invocation.
- **Why a separate crate:** it is the one place vendor names and a heavy, varied native-tool dependency surface are sanctioned; keeping it apart from the generic host stops a vendor name or a native dependency from ever leaking into vendor-blind code.

### `ironclaw_slack_extension` (`packages/slack/`)

- **Purpose:** the protocol-only channel adapter for Slack.
- **Owns:** payload parsing, message rendering, delivery, and preference-target encoding for Slack; the bot-token manifest handle.
- **Never contains:** signature verification — the generic verifier owns that, driven by this package's manifest recipe; the OAuth flow — declared as recipe data; setup or configuration UX; delivery semantics or retry policy.
- **Public surface:** an implementation of the channel-adapter contract — activation, cleanup, inbound parsing, and delivery.
- **Depends on:** `extension_contracts` only.
- **Never depends on:** `ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel crate.
- **Security & authority role:** none — pure parsing and rendering. It cannot construct verified-inbound evidence; it can misrender a message, but it cannot forge the fact that a request passed its signature check.
- **Why a separate crate:** it implements a channel adapter, and channel-adapter packages are linked exclusively by the binary — a boundary only a crate, not a module, can enforce.

### `ironclaw_telegram_extension` (`packages/telegram/`)

- **Purpose:** the protocol-only channel adapter for Telegram.
- **Owns:** payload parsing, message rendering, delivery, and preference-target encoding for Telegram.
- **Never contains:** signature verification, the OAuth flow, setup UX, delivery semantics, or retry policy — the same exclusions as every other channel package.
- **Public surface:** an implementation of the channel-adapter contract, identical in shape to every other channel package.
- **Depends on:** `extension_contracts` only.
- **Never depends on:** `ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel crate.
- **Security & authority role:** none — the same parse-only posture as every channel package.
- **Why a separate crate:** it implements a channel adapter, subject to the same binary-only linkage boundary as every other channel package.

### `ironclaw_memory_native` (`packages/memory-native/`)

- **Purpose:** the bundled memory provider — the filesystem-native implementation of the provider-neutral memory contract, shipped and installed by default so memory is always available.
- **Owns:** the package manifest declaring its `[memory]` provider surface; the `MemoryService` implementation and the backend abstraction it is built from; filesystem and in-memory repositories; full-text indexing and search; the prompt-write-safety enforcement engine that implements the vocabulary the neutral contract defines; wiring for the contract's shared conformance suite.
- **Never contains:** the neutral `MemoryService` vocabulary itself — that stays in `domains/`; a second backend; virtual-path or mount authority, which stays in the filesystem substrate.
- **Public surface:** an implementation of `ironclaw_memory::MemoryService`, bound only by the binary; no additional public ports.
- **Depends on:** `ironclaw_memory`, `ironclaw_filesystem`, `ironclaw_safety`, `ironclaw_host_api`, `extension_contracts`.
- **Never depends on:** the mem0 package; any HTTP client; `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel or product crate.
- **Security & authority role:** none — a record and search backend; the safety engine it hosts enforces contract vocabulary the kernel consumes, but the enforcement call itself grants no authority.
- **Why a separate crate:** it implements a provider surface with a real native backend — indexing weight and a filesystem cone that must not leak into consumers of the shared support crate — and provider packages, like channel packages, are linked exclusively by the binary.

### `ironclaw_memory_mem0` (`packages/mem0/`)

- **Purpose:** the alternative memory provider, backed by an external mem0 service — installed per deployment in place of the native provider.
- **Owns:** the package manifest declaring its `[memory]` provider surface; the mapping from IronClaw's memory operations onto the external service's REST surface; a hardened transport seam — bounded timeout, redirects disabled, target URL validated before any request leaves the process; configuration and error vocabulary for the external integration.
- **Never contains:** the neutral `MemoryService` vocabulary; filesystem-backend logic; any naming of this provider outside its own package directory and the assembly layer that installs it.
- **Public surface:** an implementation of `ironclaw_memory::MemoryService`; a mock-transport seam that keeps the mapping unit-testable without live network.
- **Depends on:** `ironclaw_memory`, `ironclaw_host_api`, `extension_contracts`.
- **Never depends on:** the native memory package; anything outside its own narrow HTTP cone; `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel or product crate.
- **Security & authority role:** none directly; carries the target-validation obligation for its one HTTP egress path.
- **Why a separate crate:** an isolated external HTTP dependency cone, and the second independent implementation that keeps the memory contract's conformance suite honest.

## Family AGENTS.md requirements

`crates/extensions/AGENTS.md` must carry, verbatim or by direct reference:

- **The unified extension model recap** — an extension is the only installable product object; its surfaces are tool, channel, auth, and provider surfaces such as memory; a vendor identity is a credential authority, never a product identity; runtime kind is a loading mechanism, never taxonomy.
- **The package-directory self-containment rule** and **the package-to-crate rule**, restated verbatim from this document's "What belongs here" section, so "does this need a crate?" is answered the same way every time a vendor is added.
- **The four-responsibility lookup table** — vocabulary (contracts, outside this family), registry and records (`ironclaw_extension_registry`), generic hosting (`ironclaw_extension_host`), product-side management (`ironclaw_extension_manager`), concrete packages (`packages/*`) — so "where does this code go" is a lookup, not a re-derivation.
- **The vendor-name rule** — a vendor identifier, protocol branch, or vendor-specific behavior is legal only under `packages/*`; nowhere else in this family, and nowhere outside it.
- **The family's dependency direction, restated as a check** — the generic host consumes ports from contracts and never depends on `ironclaw_assistant`; the manager calls the registry's and the host's authority rather than reimplementing it; a channel package depends on `extension_contracts` alone; a provider package depends only on the domain contract it implements and the substrates its backend genuinely needs; and only the binary links a concrete package crate.
