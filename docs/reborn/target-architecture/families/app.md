# `crates/app/` — assembly & enforcement

**Layer(s):** `app` · **Crates:** 4 — `ironclaw_composition`, `ironclaw_cli` (binary `ironclaw`), `ironclaw_config`, `ironclaw_architecture_tests` · **Security posture:** holds no standing authority beyond deployment-mode selection — never policy content — and fail-closed readiness gating. Its one deliberate privileged act is narrow and named: the binary is the only crate in the workspace permitted to name a concrete extension package, and it alone implements the token-minting port the assembly crate defines but does not satisfy.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/app/
├── ironclaw_composition          the assembly root: selection & wiring
├── ironclaw_cli                  the binary `ironclaw` & its binding tables
├── ironclaw_config               boot contract & config.toml schema
└── ironclaw_architecture_tests   mechanical enforcement tests

tools/                       stress harness & excluded helpers
```

## Role

`app/` is the assembly root, the shipped artifact, the boot-configuration leaf, and the enforcement suite — four crates whose only shared trait is that nothing else in the workspace may depend on any of them. `ironclaw_composition` wires every owning crate from every other family into a running deployment; `ironclaw_cli` produces the binary named `ironclaw`, the thing an operator actually runs, and the only place a concrete extension package is named; `ironclaw_config` is the boot-time configuration schema the rest of this family reads; `ironclaw_architecture_tests` is the enforcement suite that fails the build whenever a crate's dependency graph or public surface drifts from the declared model.

This is deliberately the one family whose crates are permitted to see the entire workspace. Every other family document in this series names a bounded, explicit set of crates it may depend on; the assembly root's own dependency set is simply everything, because its job is to construct every other family's owners, not to own a domain of its own. Seeing everything is not license to become anything: the family's charter is wiring, never behavior, and the boundary section below exists to keep that distinction sharp.

Composition selects a deployment's shape through a small set of closed choices — which runtime substrate a deployment assembles, which storage shape backs it, which profile and mode apply — rather than through predicate logic scattered across its own code. A reviewer can enumerate every deployment shape by reading those choices; the moment a decision requires more than selecting between named values, it belongs to a lower-layer crate instead.

## Boundaries — what makes this family distinct

Against every other family, the same asymmetry holds: they are bounded, `app` is not — its dependency reach is the whole workspace, an exception no other family shares. But seeing everything is not owning everything, and the family's charter is exactly the discipline of never crossing that line:

- against `contracts/`: contracts define neutral vocabulary; `app` defines nothing neutral — it selects and constructs concrete implementations of vocabulary owned elsewhere.
- against `substrates/` and `domains/`: those families own mechanism and record grammar; `app` never owns a record shape of its own.
- against `kernel/`: kernel makes authority decisions; `app` selects *which* policy applies but never authors policy *content* — deployment-mode and profile selection are data the assembly root picks a point on, never a computation of what a policy permits.
- against `lanes/` and `loop/`: those own execution mechanics; `app` only registers which implementation a deployment uses.
- against `extensions/`: the registry, host, and management crates own extension lifecycle; the assembly root's only extension-shaped privilege is a binding table of opaque adapter handles, and even that table is built entirely in the binary, never in the assembly crate itself.
- against `product/`: product owns conversation UX and delivery semantics; `app` owns neither.

**The binary is the only crate naming concrete extension packages.** The CLI alone imports a concrete channel-adapter crate and builds its binding; the assembly crate receives every binding as an opaque, already-constructed handle and is designed so it can never itself name a concrete extension package — its own charter states this as an invariant, not an aspiration.

**"Composition wires owners, never becomes one."** This is the family's one-sentence charter: if a module inside the assembly crate computes a policy decision, renders a prompt, or owns a domain record shape, it does not belong there, however convenient that placement might seem. Every "owns" list below is deliberately narrow for exactly this reason.

## What belongs here / What never belongs here

**Belongs:** deployment configuration as data — axes a deployment selects a point on, never predicates a lower crate re-derives; binding tables that pair typed identities with already-constructed adapter handles; storage backend selection; owner-factory invocation, calling into every family's own construction entry point and never reimplementing what it builds; readiness computation over already-constructed handles; service-graph handles, never the services' internal logic; background-task lifecycle management, never the logic those tasks run; the command surface, boot wiring, and platform-service mechanics that make up a shipped binary; the boot-configuration schema and its seeding and validation; and the enforcement suite itself.

**Never belongs:** any domain behavior — a record shape, a state machine, a redaction rule, all belong to the family that owns the concept; policy *content*, as distinct from policy *selection*; prompt content of any kind; vendor flows beyond the two license exceptions this family shares with the product family — the CLI's first-party registrar wiring for a small set of vendor integrations is the binary's equivalent of the extension-binding table, not a general vendor allowance; and HTTP route handler logic — mounting a prebuilt router behind a carrier is fine, writing the handler is not.

## Dependency direction

Composition sees everything; nothing depends on `app`. Every crate in every other family may be constructed by the assembly root, and none of them may import it back — the assembly root sits at the top of the dependency ladder by design, and no lower crate may reach up into it. Internally, the family's own edges are asymmetric: the assembly crate depends on essentially every owning crate in the workspace in order to construct them; the binary depends on the assembly crate, the boot-configuration crate, and — uniquely within this family — the concrete extension-package and product-surface crates it links directly, because it alone is permitted to. `ironclaw_config` has zero workspace dependencies: it is a pure leaf, consumed only by the assembly crate and the binary, and depending on nothing beyond its own schema and validation logic — a property the family enforces as an invariant rather than a coincidence, and one that holds regardless of which family a crate that needs a boot-time value sits in: any such value reaches that crate as construction input from the assembly root, never as a direct dependency on `ironclaw_config` itself. `ironclaw_architecture_tests` depends at build time on nothing in the workspace beyond a small, explicitly dev-only vocabulary import; its enforcement mechanism inspects the workspace's declared structure and source text rather than linking the crates it polices, so a crate can fail its own boundary check without the checker itself becoming part of the thing being checked.

## Security & authority

Fail-closed readiness is the assembly root's one true authority-adjacent act: a deployment's readiness computation either confirms every required handle is present and correctly shaped, or it blocks — production and any data-migration profile must fail closed on a missing or under-specified handle, never default to a permissive shape. This is a gate on whether an assembled deployment is safe to serve traffic, never a gate on what an individual request may do; that decision belongs entirely to the kernel family, several layers below.

Deployment policy is data, never content: the assembly root selects a deployment mode and a runtime profile as plain values, and the kernel family resolves what those values actually permit. The security property this buys is auditability — a reviewer can enumerate every deployment shape by reading a small set of enums, rather than tracing predicate logic scattered through the assembly crate.

Trusted binding tables live only in the binary. Two narrow, real authority acts belong there and nowhere else in the family: the table pairing each concrete extension package with its adapter implementation, which the assembly root receives only as an opaque, pre-built handle; and the implementation of the admin API token-minting port the assembly crate defines but deliberately does not satisfy itself, so the authority to mint an administrative token has exactly one implementation, in exactly one place, in the artifact an operator actually runs.

The enforcement suite is the mechanism, not a participant, in every claim this document and its sibling make. Every boundary rule named in this family and in the product family's own document is only as real as a contract test that pins it, and every one of those tests lives inside the enforcement crate — nowhere else in the workspace is permitted to define one.

## Crates

### `ironclaw_composition`

- **Purpose:** the assembly root — deployment selection and dependency wiring, exclusively.
- **Owns:** deployment configuration as data; the host-binding input structures a binary supplies bindings through; storage-backend selection; owner-factory invocation across every family; readiness computation; service-graph handles, exposed as methods rather than raw internals; background-task lifecycle management; and wiring the memory provider where the kernel consumes its contract — the concrete provider is linked and constructed by the binary, like every package crate; composition receives the handle and never names an implementation.
- **Never contains:** approval or authorization policy content; trigger-firing logic beyond starting and stopping a poller's lifecycle; conversation, automation-panel, or admin-user behavior; prompt content of any kind; HTTP route handler logic beyond mounting a prebuilt carrier; or any domain record shape.
- **Public surface:** the host-binding input structures, the service-graph handle type exposing product, auth, and readiness as methods, and the token-minting port it defines for the binary to satisfy.
- **Depends on:** every owning crate in every other family, as the one designed exception to the family's own boundary rule.
- **Never depends on:** nothing in the workspace is off-limits by the layer model, but the assembly root must never itself be depended on by anything it constructs.
- **Security & authority role:** fail-closed readiness gating and deployment-shape selection; the assembly site whose handles the kernel family's mediated services are built from, without itself constructing the kernel membrane.
- **Why a separate crate:** the assembly root is, by definition, the one crate allowed to see everything — no lower crate could hold this role without breaking the layer model it depends on to exist.

### `ironclaw_cli`

- **Purpose:** the shipped binary — command surface, serve wiring, the binding tables that link concrete extension packages, first-party registrars, credential-visibility policy, and the administrative token minter. The binary this crate produces is named `ironclaw`.
- **Owns:** the command surface; the serve-loop sequence that assembles a deployment, obtains a product surface, and starts the web gateway; the binding table pairing each concrete channel-adapter package with a typed extension identity; a small set of first-party registrars for vendor integrations that ship natively rather than as installed packages; credential-visibility policy; and the administrative token-minting implementation.
- **Never contains:** domain behavior of any kind — every command is a thin caller into the assembly root or a family crate, never a reimplementation of what it calls.
- **Public surface:** none consumed elsewhere in the workspace — it is a shipped binary, the leaf of every dependency chain in the system.
- **Depends on:** `ironclaw_composition`, `ironclaw_config`, `ironclaw_operator`, and — uniquely in this family — the concrete extension-package and product-surface crates it links directly.
- **Never depends on:** nothing is forbidden by the layer model, but it must never construct a wiring path the assembly crate could instead be asked to build.
- **Security & authority role:** the sole binary-linking anchor for concrete extension packages, and the sole implementer of the administrative token-minting authority — both single-implementor by design, both findable in exactly one place each.
- **Why a separate crate:** it is the shipped artifact — a binary target, not a library — and the entire discipline of "only the binary names a concrete extension" depends on there being exactly one binary crate to hold that privilege.

### `ironclaw_config`

- **Purpose:** boot-time configuration contracts for the standalone binary — home, profile, and boot resolution, the configuration-file schema, seeding, budget defaults, and inline-secret rejection.
- **Owns:** the configuration-file schema; home, profile, and boot resolution; configuration seeding; budget environment defaults; and inline-secret rejection at parse time.
- **Never contains:** vendor-specific configuration sections or vendor parse logic — those are package-owned schema and data, surfaced through a generic administrative-configuration model rather than hardcoded into the boot schema; or any runtime wiring — this crate defines a schema and validates it, and never itself constructs the deployment the schema describes.
- **Public surface:** the configuration-file schema and its resolved boot-configuration types.
- **Depends on:** nothing in the workspace.
- **Never depends on:** every other crate in the workspace, without exception — the zero-dependency guarantee is the crate's entire reason to be its own compilation unit rather than a module inside the assembly crate.
- **Security & authority role:** inline-secret rejection at parse time — a fail-closed check that a raw secret value typed directly into the configuration file is refused rather than silently accepted, the one place this crate makes anything resembling a security decision.
- **Why a separate crate:** the boot contract with a machine-enforced no-dependency rule — a guarantee only meaningful as a separately compiled, separately reviewed unit.

### `ironclaw_architecture_tests`

- **Purpose:** the workspace's architecture-contract test suite — the mechanism that fails the build whenever a crate's dependency graph or public surface drifts from the declared layer and family model.
- **Owns:** every architecture-contract test in the workspace, including the layer ladder itself and the boundary rules every other family's document describes; family-and-layer consistency checks; contract-purity allowlists for the neutral vocabulary tier; a rule pinning where trusted-evidence constructors may be called from; a persistence-idiom rule bounding which crates may speak a database driver directly; a rule against reaching into another crate's assets by a relative path; and conformance suites for every family with more than one backend or provider implementation.
- **Never contains:** any production code, any runtime behavior, or any type another crate imports for a non-test purpose.
- **Public surface:** none — nothing else in the workspace imports it.
- **Depends on:** a small, explicitly dev-only vocabulary import needed to pin one allowlist against its owning crate's real definition rather than a copy.
- **Never depends on:** any normal, non-dev dependency on anything in the workspace — a production dependency here would make the crate's own zero-production-dependency claim false the moment it existed.
- **Security & authority role:** it is the enforcement mechanism for every security-relevant claim this document and its sibling make; a rule that is not a test here is not a rule.
- **Why a separate crate:** test-only isolation by definition — folding these tests into any crate they police would let that crate's own compilation succeed or fail on its own boundary tests, defeating the purpose of an independent check.

### tools/ and the workspace root

A small set of packages sits outside the enforced family tree, each isolated for a distinct reason.

A diagnostic load-generation harness exercises the storage and provider layers directly, at a scale and with a raw driver access no production crate is permitted — this is by design: it exists to find bottlenecks, not to ship, and it is excluded from the default build so that building the product never requires building it.

An excluded helper binary decodes a legacy voice-note codec into a standard audio format; it is isolated as its own build entirely, with its own dependency resolution, because its one native dependency requires a C toolchain the main build must never require.

A fuzz-testing surface exercises the parsing paths most exposed to untrusted input, built and run in isolation from the main workspace so that fuzzing infrastructure never becomes part of the product's own dependency graph.

The workspace root hosts a dedicated integration-test package, `ironclaw_reborn_integration_tests`, with no library or binary target of its own — a natural home for the tests that exercise interactions across every family at once, distinct from any single crate's own test suite because no single crate is the right owner for a test that spans five of them.

## Family AGENTS.md requirements

The family's root guidance states, once: the assembly root's charter — wires owners, never becomes one — restated as the operative test for any change to what the assembly crate contains; the rule that only the binary names a concrete extension package; the boot-configuration crate's zero-dependency guarantee, stated as non-negotiable; and the enforcement crate's role as mechanism rather than participant — a change that makes a boundary test easier to pass, rather than the underlying design more correct, is exactly the failure mode this family's guidance exists to name.
