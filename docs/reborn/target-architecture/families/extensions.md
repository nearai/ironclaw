# `crates/extensions/` — everything "installable package"

**Layer(s):** substrates (`ironclaw_extensions`), loops (`ironclaw_extension_host`, the first-party package), products (`ironclaw_extension_manager`, every channel package) · **Crates:** 6 — `ironclaw_extensions`, `ironclaw_extension_host`, `ironclaw_extension_manager`, `ironclaw_first_party_extensions`, `ironclaw_slack_extension`, `ironclaw_telegram_extension` · **Security posture:** the host-to-extension trust membrane — a single generic verifier is the only code permitted to mint sealed confirmed-inbound evidence; concrete packages parse and render but can never construct trust, and only the binary may link a concrete package crate.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

## Role

`crates/extensions/` holds every concern that follows from "an extension is an installable package," short of the vocabulary that concern is expressed in. An extension is the only installable product object; its surfaces are tool, channel, and auth; a vendor identity is a credential-authority namespace, never a product identity; at most one channel surface exists per extension. Four host pipelines — dispatch, inbound routing, outbound delivery, and auth — are implemented exactly once, generically, and this family owns the generic implementations of routing and hosting plus the product-facing orchestration around all four. Adding a new vendor, in this design, is a package-directory addition: a manifest, an implementation of the relevant adapter contract, and one binding entry at the binary — nothing elsewhere in the family changes.

## Boundaries — what makes this family distinct

- **The four separated responsibilities, and where each lives.** *Vocabulary* — the `ChannelAdapter`/`ToolAdapter` contracts, manifest-surface descriptors, auth recipes, installation-state vocabulary, and the sealed confirmed-inbound-evidence constructor — lives outside this family entirely, in contracts, so that lanes, hosts, packages, product, and the manager can share one vocabulary without any of them importing a registry. *Registry and records* is `ironclaw_extensions` alone: manifest schema plus the durable installation store. *Generic hosting* is `ironclaw_extension_host` alone: lifecycle, binding, activation, the ingress verifier, egress transport. *Product-side management* is `ironclaw_extension_manager` alone: catalog, lifecycle commands, configuration and pairing UX. *Concrete package behavior* is `packages/*`: parsing, rendering, vendor calls, recipe data.
- **vs product** — extension-management UX vs conversation UX. Installing, configuring, and pairing an extension is a different surface from having a conversation with the agent; each has its own product-surface implementation, and neither reaches into the other's store to act.
- **vs lanes** — package vs mechanism, two independent axes. A package is a distribution, vendor, and configuration unit: what ships, what it may touch, how it is configured. A lane is an execution mechanism: how already-authorized code runs. A package's manifest declares which lane its code runs under; a package never depends on a lane crate, and a lane never depends on a package crate.
- **Runtime kind is loading, never taxonomy.** Every loader — native, WASM, or MCP — produces the identical binding shape from an extension's entrypoint. Nothing in the registry, the host, or the manager branches on which loader built a given extension; the choice is a manifest field, not a product category.

## What belongs here / What never belongs here

**Belongs:** manifest schema and installation/membership records; the generic lifecycle, binding, activation, and removal state machine; the generic ingress verifier and egress transports; product-facing catalog, lifecycle-command, configuration, and pairing services; concrete per-vendor parsing, rendering, and vendor calls. **Never belongs:** a second dispatcher, ingress router, delivery coordinator, or auth engine defined per package — the four host pipelines are implemented exactly once, generically, and every package consumes the same one; a vendor name or protocol branch anywhere outside `packages/`.

**Package-directory self-containment.** Every package's manifest, prompts, schemas, code, and any built-artifact sources live together in one directory. A package's assets never live in one crate while its code lives in another.

**The package-to-crate rule.** A package earns its own crate only if it implements a channel adapter — linked exclusively by the binary, never by a generic crate — or carries a heavy or isolated native dependency that would otherwise leak into every consumer of a shared package crate. Every other package is a directory of manifest and asset data with no crate of its own; where a package needs native, non-WASM tool logic, that logic lives as a module inside the first-party package rather than as a crate on its own.

## Dependency direction

`ironclaw_extension_host` consumes ports defined in `product_contracts` and `extension_contracts` — it never depends on `ironclaw_product` directly, and never names a concrete product type. `ironclaw_extension_manager` depends on `product_contracts`, `extension_contracts`, `ironclaw_extensions`, and `ironclaw_extension_host`, calling their authority rather than reimplementing it; nothing else in this family depends on the manager. Every package crate depends on `extension_contracts` alone. `ironclaw_extensions` depends only on a storage substrate and contracts vocabulary.

## Security & authority

A generic verifier executes each channel's manifest-declared recipe — a signature scheme, the secret handle to check the signature against, and the exact shape the signature takes — identically for every vendor, and only once that check passes does it mint the sealed confirmed-inbound evidence that everything downstream trusts. A channel adapter's inbound handler parses only: it can misinterpret content, but it can never forge the fact that a request passed that check. Outbound, delivery is mediated the same way in reverse: a package's adapter renders and calls the vendor only through the host's egress path, with a credential injected by the host at send time — the adapter never holds a raw, storable secret of its own.

## Crates

### `ironclaw_extensions`

- **Purpose:** manifest schema and the durable registry of what is installed, by whom, and with which credentials bound.
- **Owns:** the wire manifest schema and an internal normal form, plus a resolved-and-digested form the rest of the system reads instead of re-parsing declarative data; the in-memory catalog; the durable installation, membership, and credential-binding records.
- **Never contains:** execution of any kind; a secret; a trust decision; a vendor name.
- **Public surface:** the registry and the installation-record store, read by the generic host and by the product-management crate.
- **Depends on:** a storage substrate and contracts (`host_api`, `extension_contracts`).
- **Never depends on:** `ironclaw_product`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel crate.
- **Security & authority role:** the installation-lifecycle record authority — it records what is installed; it never decides whether an effect is allowed.
- **Why a separate crate:** a record authority with a genuine persistence obligation and a manifest grammar many crates read; keeping it apart from the generic host keeps a stateful, compare-and-swap-mutated store out of a crate whose other job is verifying inbound trust.

### `ironclaw_extension_host`

- **Purpose:** the generic host — lifecycle writer, loaders, binding and activation, the vendor-blind ingress verifier, and egress transports. No concrete vendor name, protocol route, or behavior branch belongs here.
- **Owns:** the extension lifecycle writer and the active-installation snapshot; the loader trio — native, WASM, MCP — and binding checks; activation and removal transactions; the ingress router and the manifest-recipe verifier that mints sealed confirmed-inbound evidence; egress transports; the generic channel-identity, connection, configuration, and pairing mechanisms every vendor shares.
- **Never contains:** a concrete vendor name, protocol route, or behavior branch; product-facing catalog, credential-view, or administrator UX — that is the product-management crate's job; a route framework of its own — any exposed route is owned and mounted by the product-facing transport crate.
- **Public surface:** the generic hosting operations — install, activate, bind, remove — and the ingress-verification path, consumed by the product-management crate and, indirectly through the router, by every package.
- **Depends on:** `ironclaw_extensions`; the kernel crates it hosts activity for — `ironclaw_capabilities`, `ironclaw_authorization`, `ironclaw_processes`, `ironclaw_resources`, `ironclaw_secrets`, `ironclaw_trust`, `ironclaw_turns`; `ironclaw_loop_host`; and contracts (`extension_contracts`, `product_contracts` for the ports it implements).
- **Never depends on:** `ironclaw_product`. Every product-facing operation it exposes is defined as a port in a contracts crate and implemented here against that port, never against a concrete product type.
- **Security & authority role:** the raw-request-to-confirmed-inbound membrane, and the sole writer of installation-lifecycle state transitions.
- **Why a separate crate:** the generic hosting machinery carries a real trust job — verification, binding, activation — that must stay free of any vendor or product-specific branch; a crate boundary is what makes "no concrete product name here" a fact that can be checked, not merely a convention.

### `ironclaw_extension_manager`

- **Purpose:** the product face of extensions — everything a user or operator does to discover, install, configure, and pair an extension.
- **Owns:** the available-extension catalog and its import path; lifecycle commands and capabilities, exposed through their own product-surface implementation; the channel-configuration product service; pairing-workflow orchestration; credential views; administrator, operator, and skill-activation capability handlers.
- **Never contains:** lifecycle authority — it always calls the generic host, never mutates an installation record on its own; ingress verification; a vendor name outside recipe data.
- **Public surface:** a product-surface implementation scoped to extension management, plus the product-facing services — channel configuration, account-connection status, lifecycle commands — that contracts define as ports.
- **Depends on:** `product_contracts`, `extension_contracts`, `ironclaw_extensions`, `ironclaw_extension_host`.
- **Never depends on:** the conversation-facing product crate; any package crate directly; `ironclaw_extensions`' store except through `ironclaw_extension_host`'s authority-bearing operations.
- **Security & authority role:** none of its own — a product-UX layer that calls the generic host's authority-bearing operations rather than duplicating them.
- **Why a separate crate:** a coherent product sub-owner with its own surface, distinct from the conversation-facing product crate and from the generic host whose authority it only ever calls.

### `packages/` — directory rules

Every installable extension's manifest, prompts, schemas, code, and any built-artifact sources live together in one package directory. A package earns its own crate only when it implements a channel adapter — linked exclusively by the binary, never by any generic crate — or when it carries a heavy or isolated native dependency that would otherwise leak into every consumer of a shared crate. Every other package is a directory of manifest and asset data with no code of its own; where a package needs native, non-WASM tool logic, that logic lives as a module inside the first-party package rather than as a crate of its own. Packages whose tools compile to a portable component artifact keep their build sources beside their manifest, entirely self-contained and independent of the workspace's own build graph: the artifact ships with the package, and the source that produced it travels with it, never housed as a separate concern elsewhere.

### `ironclaw_first_party_extensions` (`packages/first_party/`)

- **Purpose:** the sanctioned home for vendor-named, first-party package content — the package inventory, every non-channel package's assets, native tool executors, and the generic first-party tool implementations available to any deployment.
- **Owns:** the package inventory; the asset tree for every first-party package that is not a channel; native tool executors — general-purpose file, text, and search tooling, groupware integrations, web access; the generic builtin tool implementations — file, shell, http, time, memory, trigger management, skill management and installation, and telemetry submission — available to any loop by capability grant, not by extension identity.
- **Never contains:** a type only a loop-hosting crate should hold; this package is invoked only through the same capability-dispatch path any tool uses.
- **Public surface:** tool-adapter implementations for its bundled tools, consumed through the generic dispatch path.
- **Depends on:** `ironclaw_auth`, `ironclaw_extractors`, a storage substrate, `ironclaw_observability`, `ironclaw_safety`, `ironclaw_skills`, `extension_contracts`; the domains its bundled tools need — memory, traces, triggers — by declared charter.
- **Never depends on:** `ironclaw_product`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or `ironclaw_loop_host` — a package is content, not a host.
- **Security & authority role:** none — first-party status raises a policy ceiling, it never grants permission; every tool call still crosses the same authorization and approval stages as any other capability invocation.
- **Why a separate crate:** it is the one place vendor names and a heavy, varied native-tool dependency surface are sanctioned; keeping it apart from the generic host stops a vendor name or a native dependency from ever leaking into vendor-blind code.

### `ironclaw_slack_extension` (`packages/slack/`)

- **Purpose:** the protocol-only channel adapter for Slack.
- **Owns:** payload parsing, message rendering, delivery, and preference-target encoding for Slack; the bot-token manifest handle.
- **Never contains:** signature verification — the generic verifier owns that, driven by this package's manifest recipe; the OAuth flow — declared as recipe data; setup or configuration UX; delivery semantics or retry policy.
- **Public surface:** an implementation of the channel-adapter contract — activation, cleanup, inbound parsing, and delivery.
- **Depends on:** `extension_contracts` only.
- **Never depends on:** `ironclaw_product`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel crate.
- **Security & authority role:** none — pure parsing and rendering. It cannot construct confirmed-inbound evidence; it can misrender a message, but it cannot forge the fact that a request passed its signature check.
- **Why a separate crate:** it implements a channel adapter, and channel-adapter packages are linked exclusively by the binary — a boundary only a crate, not a module, can enforce.

### `ironclaw_telegram_extension` (`packages/telegram/`)

- **Purpose:** the protocol-only channel adapter for Telegram.
- **Owns:** payload parsing, message rendering, delivery, and preference-target encoding for Telegram.
- **Never contains:** signature verification, the OAuth flow, setup UX, delivery semantics, or retry policy — the same exclusions as every other channel package.
- **Public surface:** an implementation of the channel-adapter contract, identical in shape to every other channel package.
- **Depends on:** `extension_contracts` only.
- **Never depends on:** `ironclaw_product`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any kernel crate.
- **Security & authority role:** none — the same parse-only posture as every channel package.
- **Why a separate crate:** it implements a channel adapter, subject to the same binary-only linkage boundary as every other channel package.

## Family AGENTS.md requirements

`crates/extensions/AGENTS.md` must carry, verbatim or by direct reference:

- **The unified extension model recap** — an extension is the only installable product object; its surfaces are tool, channel, and auth; a vendor identity is a credential authority, never a product identity; runtime kind is a loading mechanism, never taxonomy.
- **The package-directory self-containment rule** and **the package-to-crate rule**, restated verbatim from this document's "What belongs here" section, so "does this need a crate?" is answered the same way every time a vendor is added.
- **The four-responsibility lookup table** — vocabulary (contracts, outside this family), registry and records (`ironclaw_extensions`), generic hosting (`ironclaw_extension_host`), product-side management (`ironclaw_extension_manager`), concrete packages (`packages/*`) — so "where does this code go" is a lookup, not a re-derivation.
- **The vendor-name rule** — a vendor identifier, protocol branch, or vendor-specific behavior is legal only under `packages/*`; nowhere else in this family, and nowhere outside it.
