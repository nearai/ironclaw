# `crates/substrate/` — privileged mechanism substrates

**Layer(s):** substrates · **Crates:** 5 — `ironclaw_filesystem`, `ironclaw_secrets`, `ironclaw_network`, `ironclaw_safety`, `ironclaw_observability` · **Security posture:** each crate is a mediated mechanism invoked on behalf of an already-decided effect; none of the five makes an authority decision itself — containment, custody, policy enforcement, and detection only, fail-closed by local invariant, never by ambient trust.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/substrate/
├── ironclaw_filesystem        storage fabric: mounts, containment, CAS
├── ironclaw_secrets           secret custody & one-shot leases
├── ironclaw_network           egress policy & hardened transport
├── ironclaw_safety            scanning & redaction primitives
└── ironclaw_observability     latency-trace macros
```

## Role

Substrate holds the durable, reusable mechanisms the kernel mediates: storage fabric, secret storage, network policy and transport, safety scanning, and cross-cutting tracing. A crate belongs here exactly when it is a backend-generic mechanism with a real containment story and a fail-closed local invariant; it does not belong here if it makes an authority decision, which is kernel's job, or if it owns domain record grammar, which is domains' job. Five crates satisfy that test, and each is isolated from the others by a genuine dependency cone — a database driver stack, an operating-system keychain integration, an HTTP client and DNS resolver, or a pattern-matching engine — never by categorization for its own sake.

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

- **Depends on:** `ironclaw_host_api` and `ironclaw_observability`, at the family floor, for every crate. Filesystem additionally depends on safety for a single sensitive-path classification. Secrets additionally depends on filesystem, since durable secret metadata is itself filesystem-backed.
- **Never depends on:** any other substrate crate beyond the two exceptions above — a lattice of substrate-on-substrate dependencies would recreate the driver-cone leakage this family exists to prevent. No crate in this family depends on anything above substrates.
- **Depended on by:** kernel is the primary consumer — the kernel's service graph mediates filesystem, secrets, and network for everything above it, so most upper-tier access to this family is indirect. Domains hold direct filesystem access, since backend-neutral persistence is the point of the storage-placement rule. The durable event backend is built on filesystem. Composition selects backends and constructs the concrete implementations. No lane, loop, extension, or product crate may hold a direct dependency on a substrate crate — mediated services arrive by injection only.
- **Inversions:** the filesystem trait is itself a dependency-inversion target — every crate above substrate holds a handle against the trait, never against a concrete backend, so a backend change never touches a domain crate.

## Security & authority

Every crate in this family executes a kernel-mediated responsibility without deciding, itself, who may invoke it. Filesystem enforces path containment once handed a mount view. Secrets enforces one-shot consumption once handed a lease. Network enforces policy once handed a network policy. Safety enforces detection and redaction rules that are data, not authority. The one crate in this family authorized to sit beside a product-tier credential flow without kernel mediation in between is the auth engine, because it owns the token-custody flows — OAuth handshakes, refresh, session issuance — that need lease access on every request; every other product-tier surface reaches secrets through a mediated port instead.

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
- **Depends on:** `ironclaw_host_api`, `ironclaw_observability`, and `ironclaw_safety` for a single sensitive-path classification used when redacting a path for display.
- **Never depends on:** anything above substrates.
- **Security & authority role:** path containment and mount authority are kernel-listed responsibilities, executed here on every call; this crate is also the sole point of isolation for the durable-storage driver cone, so nothing above it needs to compile against a database client directly.
- **Why a separate crate:** one contract, many production backends, and a driver cone wide enough that no crate should acquire it by accident — isolating it here is what lets the rest of the workspace stay backend-agnostic.

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
- **Never depends on:** anything above substrates. Only the auth engine, among product-tier crates, may depend on this crate directly; every other product-tier surface reaches it through a mediated port.
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
- **Never contains:** state, policy, sinks, or any utility unrelated to timing and tracing.
- **Public surface:** the macros; no traits.
- **Depends on:** nothing internal.
- **Never depends on:** anything else in the workspace.
- **Security & authority role:** none — the only crate in this family with no security-relevant surface. Its only decision is whether a trace fires, which is an observability toggle, not a privilege.
- **Why a separate crate:** a leaf macro surface consumed across kernel, loop, and app-tier crates alike; folding it into any one consumer would force every other consumer to depend on that crate just for a timing macro.

## Family AGENTS.md requirements

Each family root states, for every crate beneath it, the same four things a reviewer needs without reading source: what admission test a new type must pass before it can live here; which two-or-more consumers justify it; which crate implements each port declared here, and where the boundary between declaring a port and implementing one sits; and the closed set of frameworks and cross-family dependencies this family may never acquire. `crates/substrate/AGENTS.md` states, specifically:

- Each crate's mediation story: who may call it directly, and which callers must instead go through kernel mediation — since this is the one thing every crate in the family answers differently, and the tightest of the five, secrets, shows why that boundary matters most.
- The "mechanism, not authority" line: a substrate crate enforces its own local invariant but never decides whether the caller was entitled to invoke it — that decision always comes from the kernel before the call reaches here.
- The dependency-cone rationale per crate, so a reviewer sees why five crates exist instead of one — each crate's dependency list is the enforcement mechanism, this file is the explanation.
- The two sanctioned substrate-on-substrate dependencies — filesystem on safety, secrets on filesystem — as the only internal edges within the family.
- The persistence rule: outside this family's filesystem crate and the durable event backend, no crate may depend on a database driver directly.
