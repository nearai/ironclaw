# `crates/substrates/` — privileged mechanism substrates

**Layer(s):** substrates · **Crates:** 7 — `ironclaw_filesystem`, `ironclaw_documents`, `ironclaw_libsql_runtime`, `ironclaw_secrets`, `ironclaw_network`, `ironclaw_safety`, `ironclaw_observability` · **Security posture:** each crate is a mediated mechanism invoked on behalf of an already-decided effect; none makes an authority decision itself — containment, custody, admission, policy enforcement, detection, and bounded document transformation only, fail-closed by local invariant, never by ambient trust.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/substrates/
├── ironclaw_filesystem        storage fabric: mounts, containment, CAS
├── ironclaw_documents         bounded, structure-preserving document transforms
├── ironclaw_libsql_runtime    libSQL connection admission: one read pool, one write lane
├── ironclaw_secrets           secret custody & one-shot leases
├── ironclaw_network           egress policy & hardened transport
├── ironclaw_safety            scanning & redaction primitives
└── ironclaw_observability     latency-trace macros
```

## Role

Substrate holds the durable, reusable mechanisms the kernel mediates: storage fabric, bounded document transformation, database-connection admission, secret storage, network policy and transport, safety scanning, and cross-cutting tracing. A crate belongs here exactly when it is a backend-generic mechanism with a real containment story and a fail-closed local invariant; it does not belong here if it makes an authority decision, which is kernel's job, or if it owns domain record grammar, which is domains' job. Seven crates satisfy that test, and each is isolated from the others by a genuine dependency cone — a document parser and renderer, database driver stack, connection-pool implementation, operating-system keychain integration, HTTP client and DNS resolver, or pattern-matching engine — never by categorization for its own sake.

## Boundaries — what makes this family distinct

- **vs `contracts/`:** contracts is vocabulary with zero I/O; substrate is the mechanism that vocabulary describes. A substrate crate takes contracts types as input and does real work — resolves a path, leases a secret, opens a socket; a contracts crate never does work at all.
- **vs `domains/`:** domains own record grammar and durable service identity — a trigger record, a thread transcript, a memory item; substrate owns the storage, policy, and safety mechanism domains are built on top of. If a crate answers "what is true about this entity," it is domains; if it answers "how do bytes, secrets, network calls, or redaction actually happen," it is substrate — domains call the filesystem fabric, they never reimplement it.
- **vs `kernel/`:** kernel decides whether an effect is authorized; substrate performs the effect once kernel has decided. Filesystem, secrets, and network never authorize — each enforces only its own local invariant, mount containment, one-shot lease consumption, private-address denial — the grant to reach them at all comes from the kernel's capability membrane.
- **vs `events/`:** both sit at the same layer, but the contract shape differs. Events owns canonical redacted evidence and durable log traits, never a mutable resource with a lease or a mount; substrate — filesystem specifically — is the fabric the durable event backend is built on. Filesystem is the foundation; events is one specific append-only consumer of a slice of it.
- **vs `lanes/`:** lanes never depend on substrate crates directly — mediated services arrive by injection, never by a live handle. A lane receives a bounded mount, a staged one-shot secret, or a policy-scoped egress handle from the kernel; it never holds its own filesystem, secrets, or network handle. This is the sharpest boundary in the dependency model — it is the operational meaning of "an already-authorized invocation."

## What belongs here / What never belongs here

**Belongs here:**
- A backend-generic mechanism with a real driver, operating-system integration, or pattern-matching cone that would burden every consumer if it were inlined.
- Containment and compare-and-swap primitives.
- Connection admission: bounded pools and single-writer lanes whose invariant only holds if exactly one of them exists per resource.
- One-shot lease and consume primitives.
- Hardened egress transport and address-policy enforcement.
- Pattern-based detection, validation, and redaction.
- Zero-cost-when-off cross-cutting instrumentation.

**Never belongs here:**
- Any authority decision — a substrate crate that started deciding who may call it would duplicate the kernel's job with none of its fail-closed guarantees.
- Domain record schemas or service identity — those are built on top of this family, never inside it.
- Product or vendor behavior, branching, or naming — every crate in this family is vendor-blind by construction.
- Ambient credentials or ambient network reachability handed to an unmediated caller — every credential or egress path here is scoped, leased, or policy-checked per call, never a standing client.
- A placeholder or demo backend running as though it were a real security boundary.

## Dependency direction

- **Depends on:** at most `ironclaw_host_api` and `ironclaw_observability`, plus three sanctioned family-internal edges — secrets builds on the storage fabric, filesystem depends on safety for a single sensitive-path classification, and filesystem depends on the libSQL runtime for connection admission. Several crates depend on nothing internal at all; the libSQL runtime is the strictest of them, holding no workspace dependency in either direction it could take.
- **Never depends on:** any other substrate crate beyond the three edges above — a lattice of substrate-on-substrate dependencies would recreate the driver-cone leakage this family exists to prevent. No crate in this family depends on anything above substrates.
- **Depended on by:** kernel is the primary consumer — the kernel's service graph mediates filesystem, secrets, and network for everything above it, so most upper-tier access to this family is indirect. Domains hold direct filesystem access, since backend-neutral persistence is the point of the storage-placement rule. The durable event backend is built on filesystem. Composition selects backends and constructs the concrete implementations. A lane crate never holds a direct dependency on a substrate crate — mediated services arrive by injection. Every other family's substrate access is charter-governed: a crate depends on exactly the substrate its charter names — storage for record owners, safety for scanners — and nothing more; secrets is the tightest, reachable directly only from the kernel's staging path and the auth engine.
- **Inversions:** the filesystem trait is itself a dependency-inversion target — every crate above substrate holds a handle against the trait, never against a concrete backend, so a backend change never touches a domain crate.

## Security & authority

Every crate in this family executes a kernel-mediated responsibility without deciding, itself, who may invoke it. Filesystem enforces path containment once handed a mount view. The libSQL runtime enforces single-writer admission once handed a database, and refuses to serve a target it cannot prove it was opened for. Secrets enforces one-shot consumption once handed a lease. Network enforces policy once handed a network policy. Safety enforces detection and redaction rules that are data, not authority. The one crate authorized to reach secrets directly, with no kernel mediation in between, is the auth engine — a domains-family crate — because it owns the token-custody flows — OAuth handshakes, refresh, session issuance — that need lease access on every request; every other consumer reaches secrets through a mediated port instead.

## Crates

### `ironclaw_filesystem`

- **Purpose:** the universal storage-dispatch fabric — a root filesystem trait, scoped and mount-checked access above it, a mount catalog, a compare-and-swap floor, and the concrete backends that implement durable storage.
- **Owns:**
  - the root filesystem trait — read, write, list, stat, append, and transactional operations over a virtual path space, with backend capability negotiation.
  - scoped access — a caller-facing wrapper that resolves a caller's mount view before any operation reaches the trait.
  - the mount catalog — composite routing across multiple backends by path placement.
  - the compare-and-swap floor — a bounded-retry, timeout-guarded conditional update primitive every durable record type is built on.
  - record and index vocabulary — versioned entries, content types, index specifications — and the disk, in-memory, and durable SQL-backed implementations of the trait.
- **Never contains:** domain DTOs or policy, transport-layer security policy, or backend-selection decisions, which belong to composition. Never ships a placeholder backend as though it were production-safe.
- **Public surface:** the root filesystem trait and the scoped wrapper above it. Multiple production backends implement the trait; every domain crate above this family holds a handle against the trait, never against a concrete backend.
- **Depends on:** `ironclaw_host_api`, `ironclaw_observability`, `ironclaw_libsql_runtime` for libSQL connection admission, and `ironclaw_safety` for a single sensitive-path classification used when redacting a path for display.
- **Never depends on:** anything above substrates.
- **Security & authority role:** path containment and mount authority are kernel-listed responsibilities, executed here on every call; this crate is also the sole point of isolation for the durable-storage driver cone, so nothing above it needs to compile against a database client directly.
- **Why a separate crate:** one contract, many production backends, and a driver cone wide enough that no crate should acquire it by accident — isolating it here is what lets the rest of the workspace stay backend-agnostic.

### `ironclaw_documents`

- **Purpose:** bounded, structure-preserving OOXML reads and typed transforms for DOCX, XLSX, and PPTX, plus deterministic HTML-subset PDF generation.
- **Owns:** addressable document views, typed format-specific edits, bounded OOXML package parsing, copy-through serialization for untargeted parts, and the deterministic local PDF renderer.
- **Never contains:** filesystem access, path selection, authorization, approvals, product workflow, vendor behavior, or ambient network and credential access.
- **Public surface:** `read_document` and `edit_document`, the format-specific `docx`, `xlsx`, and `pptx` views and edit types, `html_to_pdf`, and `DocumentError`.
- **Depends on:** contracts-free parsing and rendering libraries only; it remains vendor-blind and has no dependency on another workspace crate.
- **Never depends on:** filesystem or any crate above substrates; mediated callers supply bounded bytes and own all authority decisions.
- **Security & authority role:** no authority decision. It fails closed on malformed, ambiguous, duplicate, or oversized packages through the bounded package limits in `ooxml.rs`, and copies every untargeted package part through unchanged.
- **Why a separate crate:** document transformation has a distinct parser/rendering dependency cone and a loss-avoidance invariant shared by host-runtime and integration callers without granting those libraries filesystem authority.

### `ironclaw_libsql_runtime`

- **Purpose:** the shared libSQL connection-admission runtime — one bounded reader pool and exactly one writer lane per physical database, so that every adapter writing the same file queues behind the same admission point instead of forming a writer group of its own.
- **Owns:**
  - the runtime handle for one database, and the two pools beneath it: a bounded reader pool whose connections are opened read-only, and a single-slot writer pool that is the only way to obtain a writable connection.
  - typed read and write connection leases — each exposes only the operations its lane permits, and neither hands out the underlying connection.
  - admission behavior: bounded checkout with a deadline, connect retry and backoff, connection recycling, and rejection of a reentrant writer acquisition.
  - target provenance — the runtime records what it was opened for and can prove it, so a caller cannot hand a prebuilt handle to a store that believes it is talking to a different database.
  - a redacted, typed failure vocabulary that distinguishes retryable writer-admission pressure from broken infrastructure, so adapters classify without parsing error text.
- **Never contains:** SQL, schema, migrations, or transactions — those belong to the backend crates that own their records. Never record grammar, domain policy, or backend selection. Never PostgreSQL pooling, whose concurrency model does not need this and must not inherit it. Never a path by which a caller obtains a raw connection outside a lease.
- **Public surface:** the runtime and its two lease types, plus the lane, checkout-failure, and error vocabulary. One production implementation; no ports, because there is nothing here to invert — a caller either holds the runtime or does not write.
- **Depends on:** nothing internal. It is the family's only crate with no workspace dependency at all.
- **Never depends on:** any crate in the workspace, in any direction. A dependency here would pull the database driver cone into every dependent's dependents, which is the exact leakage this crate exists to bound.
- **Security & authority role:** an availability and correctness invariant rather than an authorization one, and the distinction is the point — this crate decides *that* only one writer proceeds, never *who* is entitled to write. That grant arrives from the kernel long before a statement reaches here. Its fail-closed behaviors are refusing a runtime that cannot prove its target, refusing a reentrant writer, and failing a checkout at its deadline rather than queueing without bound.
- **Why a separate crate:** the single-writer invariant is only enforceable where the pool is singular, and the crates that must share that pool — the storage fabric, a scheduled-trigger domain, and the assembly root — sit in three different families. A module inside any one of them would either duplicate the lane, which is the defect this crate exists to prevent, or force the other two to depend on that crate wholesale to reach a pool. A leaf with a driver cone and no dependents-of-its-own is the cheapest shape that lets all three share one admission point without any of them owning it.

### `ironclaw_secrets`

- **Purpose:** scoped, encrypted secret metadata and storage, one-shot leases, and the credential broker that mediates access to them.
- **Owns:**
  - the secret store port — lease-once and consume, a compare-and-swap one-shot primitive that guarantees a secret's raw material is readable exactly once per lease.
  - the generic secret store implementation over the filesystem fabric, and the credential broker built on it.
  - cryptography — authenticated encryption, additional-authenticated-data derivation, and master-key validation.
  - operating-system keychain integration, used to protect the master key itself.
- **Never contains:** runtime credential injection or staging, which is the kernel's obligation-handling job, not this crate's; provider HTTP; product or vendor flows.
- **Public surface:** the secret store port, an account store port, and a session store port. Exactly one production implementation of each, built on the filesystem fabric.
- **Depends on:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Never depends on:** anything above substrates. Only the auth engine — a domains-family crate — may depend on this crate directly; every other consumer reaches it through a mediated port.
- **Security & authority role:** secret custody. The invariant that raw material is readable only at one-shot consumption is this crate's entire reason to exist.
- **Why a separate crate:** a custody contract that must keep cryptography and keychain dependencies out of every other crate, and a direct-consumer boundary tight enough that a module sharing its host crate's full access could never enforce it.

### `ironclaw_network`

- **Purpose:** the network policy boundary and hardened outbound transport — target and method policy, DNS resolution with private-address denial, and redirect and size hardening.
- **Owns:**
  - the static policy enforcer that matches a requested target and method against policy before any call is made.
  - URL hardening and credential-in-path detection.
  - the HTTP egress port and its policy-checked implementation.
  - a resolver that denies private and reserved addresses before a connection is opened.
  - the pinned outbound transport implementation.
- **Never contains:** credential injection, which is the kernel's obligation-handling job; lane behavior; vendor allowlists, which are manifest data, not code.
- **Public surface:** the HTTP egress port, the transport port, and the resolver port. One production implementation of each.
- **Depends on:** `ironclaw_host_api`.
- **Never depends on:** anything above substrates.
- **Security & authority role:** the sole owner of egress policy. It keeps an HTTP client and TLS stack out of every crate above the kernel's mediated egress seam.
- **Why a separate crate:** sole egress-policy owner with a real HTTP and DNS dependency cone; a module here would put that cone in the build graph of every crate that needed even the policy types.

### `ironclaw_safety`

- **Purpose:** dependency-light prompt-injection detection, input validation, secret-leak scanning, credential detection, and display redaction.
- **Owns:**
  - a unified safety layer that composes a sanitizer, a validator, a policy engine, and a leak detector into one call.
  - the sanitizer — injection-pattern scanning over untrusted text.
  - the validator — structural and size validation for provider-bound content.
  - the leak detector — pattern-based detection of credential material in text about to leave a trust boundary.
  - credential detection, sensitive-path classification, and display redaction — the safe-to-show form of a value that must never be shown raw.
- **Never contains:** sandbox execution, credential storage, network policy, or dispatch — this crate detects and redacts; it never enforces containment or holds material itself.
- **Public surface:** the unified safety layer, plus the sanitizer, validator, and leak detector individually, each with a small, focused scanning interface a caller can depend on without the whole layer.
- **Depends on:** nothing internal at the normal-dependency tier.
- **Never depends on:** anything above substrates.
- **Security & authority role:** the mechanism that turns "does this text look like an attack, a leaked secret, or a sensitive path" into a typed, testable answer that kernel obligations, filesystem, memory, and hooks all act on.
- **Why a separate crate:** a pattern-matching dependency cone wide enough to isolate, serving detection needs from nearly every layer above it without any of those callers needing to know how the detection works.

### `ironclaw_observability`

- **Purpose:** zero-cost-when-off latency-trace macros shared by every crate that wants to time an operation without adopting a tracing dependency of its own.
- **Owns:** a small set of macros that record elapsed time and outcome against a dedicated trace target, only when that target is enabled, plus the re-exported tracing facade that makes them usable without a separate import.
- **Never contains:** state, policy, sinks, or any utility unrelated to timing and tracing. ✎ **Sharpened 2026-08-03 (WS6, PROPOSAL §12.12 D-K):** "unrelated" was doing too much work — the item this clause was written to evict, a JSON byte counter, *looked* related and was reached from two `latency.rs` modules. The test that actually discriminates: **a function that merely produces a value a trace happens to record belongs to whoever produces the thing being measured, not here.** Three of that counter's five call sites fed `ResourceUsage::set_output_bytes` — resource accounting, not a trace field.
- **Public surface:** the macros; no traits.
- **Depends on:** nothing internal. ✎ *2026-08-03: and exactly one external, `tracing`. `serde_json` left with the byte counter, so the dependency list is now the enforcement mechanism this family's charter says it should be — a second dependency here is the signal that whatever needs it belongs somewhere else.*
- **Never depends on:** anything else in the workspace.
- **Security & authority role:** none — the only crate in this family with no security-relevant surface. Its only decision is whether a trace fires, which is an observability toggle, not a privilege.
- **Why a separate crate:** the contracts tier's admission rule — instrumentation is not boundary vocabulary, so the macros cannot live in the shared vocabulary crate without weakening its charter; a leaf of their own is the cheapest home that keeps that rule statable.

## Family AGENTS.md requirements

Each family root states, for every crate beneath it, the same four things a reviewer needs without reading source: what admission test a new type must pass before it can live here; which two-or-more consumers justify it; which crate implements each port declared here, and where the boundary between declaring a port and implementing one sits; and the closed set of frameworks and cross-family dependencies this family may never acquire. `crates/substrates/AGENTS.md` states, specifically:

- Each crate's mediation story: who may call it directly, and which callers must instead go through kernel mediation — since this is the one thing every crate in the family answers differently, and the tightest of the seven, secrets, shows why that boundary matters most.
- The "mechanism, not authority" line: a substrate crate enforces its own local invariant but never decides whether the caller was entitled to invoke it — that decision always comes from the kernel before the call reaches here.
- The dependency-cone rationale per crate, so a reviewer sees why seven crates exist instead of one — each crate's dependency list is the enforcement mechanism, this file is the explanation.
- The three sanctioned substrate-on-substrate dependencies — filesystem on safety, filesystem on the libSQL runtime, secrets on filesystem — as the only internal edges within the family.
- The persistence rule, in two halves. **Admission is singular:** only this family's libSQL runtime may construct a libSQL pool or hand out a connection, because a second pool over the same database silently breaks the single-writer invariant. **Driver dependencies are a closed, shrink-only set:** outside the libSQL runtime, this family's storage fabric, the durable event backend, and the assembly root that opens each database once, no crate may depend on a database driver directly — with two documented, shrink-only exceptions, the trigger and hook predicate stores, carrying ADR-or-converge status.
- What backend parity does and does not mean: the same observable contract across backends — commit and rollback, ordering, uniqueness, error classification — but not the same connection machinery. Writer admission is a libSQL-specific contract because SQLite admits one writer and PostgreSQL does not; a future backend earns its own admission crate if its concurrency model demands one, and never a second lane over a database that already has one.
