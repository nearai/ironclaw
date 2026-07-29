
# `crates/domains/` — typed record/service domains

**Layer(s):** substrates · **Crates:** 13 — ironclaw_threads, ironclaw_conversations, ironclaw_triggers, ironclaw_memory, ironclaw_skills, ironclaw_auth, ironclaw_attachments, ironclaw_extractors, ironclaw_projects, ironclaw_identity, ironclaw_llm, ironclaw_traces, ironclaw_outbound · **Security posture:** typed record and service authorities behind the kernel — record grammar and invariants only, never an authorization, approval, or resource decision; two crates mint and seal trust narrowly (outbound, triggers), and one is a credential-custody domain (auth).

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/domains/
├── ironclaw_threads          canonical transcript service
├── ironclaw_conversations    external↔canonical binding & idempotency
├── ironclaw_triggers         scheduled triggers & trusted-fire minting
├── ironclaw_memory           provider-neutral memory contract
├── ironclaw_skills           skill parsing, selection & learning
├── ironclaw_auth             product auth & the recipe engine
├── ironclaw_attachments      attachment landing & its ports
├── ironclaw_extractors       pure bytes→text extraction
├── ironclaw_projects         project entity & membership ACL
├── ironclaw_identity         external identity → stable UserId
├── ironclaw_llm              provider contract, providers & decorators
├── ironclaw_traces           Trace Commons client & redaction
└── ironclaw_outbound         outbound authority: sealed grants, at-most-once
```

## Role

`crates/domains/` owns IronClaw Reborn's typed business records: sessions and threads, scheduled triggers, memory documents, skills, credentials, attachments, projects, identity, model providers, trace submissions, and outbound delivery state. Each crate in the family owns exactly one record grammar and the invariants that keep it valid — uniqueness, idempotency, scope isolation, compare-and-swap ordering — and exposes a typed service contract over `ScopedFilesystem` that the kernel and product layers wire against.

A domains crate never decides whether an effect may happen. Authorization, approval, and resource decisions are exhausted by the kernel before a domains-family call is ever reached. A domains crate answers "what does this record mean, and how do I keep it correct," never "is this caller allowed to do this."

The family favors narrow, single-purpose crates over shared infrastructure. Most domains crates serve a handful of independent callers through one production implementation; `ironclaw_memory` is the deliberate exception, with two independent implementations proven interchangeable by a shared conformance suite. Two crates carry a vendor-scoped charter permitted nowhere else in the family — see Boundaries, below.

## Boundaries — what makes this family distinct

- **vs `substrate/`** (filesystem, secrets, network, safety, observability): substrate crates are generic mechanism with no domain grammar — the filesystem substrate has no notion of a "trigger" or a "thread." Domains crates are the typed grammar layered on top of that mechanism; strip away every Reborn product concept and a substrate crate still compiles, a domains crate does not.
- **vs `kernel/`** (trust, authorization, approvals, resources, capabilities, processes, turns, host_runtime): kernel crates decide whether and how an effect happens; domains crates decide what a record means once the kernel has already admitted the request. No domains crate depends on a kernel crate, and no domains crate can construct a capability-authorization witness or a lease.
- **vs `events/`** (events, event_store, event_projections, event_streams): events are immutable, replayable evidence of what happened; domains own live state, mutated in place under compare-and-swap. A projection is a rebuildable read cache derived from events and is never a second write authority.
- **vs `product/`** (product, operator, openai_compat, webui, host_ingress): product orchestrates user-facing workflow, admission, and delivery across many domains at once inside a single request; a domains crate knows nothing about channels, commands, or views — it owns one record type's invariants only.
- **vs `extensions/`** (extensions, extension_host, extension_manager, packages): extensions are the installable-package concept — manifests, lifecycle, hosting, vendor packages. A domains crate is a durable business-record type available to every extension, product, and kernel caller, with no notion of install, activate, or package.
- **Two vendor-scoped charters, permitted nowhere else in the family:** `ironclaw_llm` is a closed provider cone holding model-vendor adapters and their authentication flows; `ironclaw_auth` keeps vendor differences as recipe data consumed by one generic engine, never a code branch. No other domains crate holds a vendor name.
- **The sealed-trust-types pattern:** two crates mint and seal trust through private constructors instead of kernel authorization machinery. `ironclaw_outbound` seals its delivery-attempt and access-grant types so only its own policy service can construct them. `ironclaw_triggers` seals the trusted-submission binding that identifies its own poller as a host-trusted sender. Both keep one narrow, reviewable trust responsibility inside a domain crate instead of promoting the crate into the kernel family.

## What belongs here / What never belongs here

- **Belongs:** record schemas and their invariants — validation, uniqueness, idempotency keys, compare-and-swap ordering; domain services exposing typed create/read/update/query operations over `ScopedFilesystem`; a default implementation of each service; the two vendor-scoped charters and the two sealed-mint responsibilities described above.
- **Never belongs — backend selection:** which storage backend actually serves a mount is a deployment decision made once, outside every domains crate. A domains crate never branches on backend.
- **Never belongs — authority decisions:** authorization, approval, and resource-reservation decisions stay in the kernel family.
- **Never belongs — transport and framework code:** no domains crate touches Axum; HTTP appears only inside the narrow egress needs of the vendor-scoped and external-service charters.
- **Never belongs — vendor names or vendor branches**, outside `ironclaw_llm` and `ironclaw_auth`.
- **Persistence idiom:** `ScopedFilesystem` is the floor. Every domains crate is backend-neutral by construction, depending only on the filesystem substrate's virtual-path, mount, and compare-and-swap authority — never a database driver directly. A crate that instead needs a hand-written SQL backend is a deliberate, narrow design choice that must be justified by an ADR; `ironclaw_triggers` is the one domain crate built this way, alongside `ironclaw_hooks` in the loop family.

## Dependency direction

Domains crates depend only on `substrate/`, `events/`, and `contracts/` — never on `kernel/`, `loop/`, `extensions/`, `product/`, or `app/`.

| May depend on | Never depends on |
|---|---|
| `substrate/` — filesystem for every crate; secrets and network only where a crate's charter needs them (auth) · `events/` — chiefly event_projections · `contracts/` — host_api, common, prompt_envelope | `kernel/` in full — capabilities, host_runtime, authorization, approvals, resources, trust, turns, processes · `loop/` · `extensions/` · `product/` · `app/` |

HTTP and vendor SDKs are permitted only inside `ironclaw_llm`, `ironclaw_traces`, and `ironclaw_auth`'s engine — the family's three named vendor and external-service cones. No other domains crate reaches HTTP directly.

Inside the family, dependency edges are shallow and few: `ironclaw_conversations` depends on `ironclaw_triggers` for trusted-submission binding vocabulary; `ironclaw_attachments` depends on `ironclaw_extractors` for pure bytes-to-text transformation; `ironclaw_traces` depends on `ironclaw_llm` to reuse its recording vocabulary. No other crate in the family depends on a sibling — the family's internal graph is a shallow forest, not a mesh.

## Security & authority

Most crates in this family hold no authority at all: a call into `ironclaw_threads` or `ironclaw_projects` is always downstream of a kernel admission decision, never a gate itself. Three crates are the named exceptions:

- **`ironclaw_outbound`** is the sole writer of delivery-attempt state and mints the sealed access-grant and delivery-binding types described above.
- **`ironclaw_triggers`** is the one host-trusted inbound path outside the generic ingress verifier — its trusted-submission binding is the sealed evidence that a fire came from its own poller.
- **`ironclaw_auth`** is a credential-custody domain: it holds durable token-lifecycle state, but never raw secret bytes, and it never makes an authorization decision itself.

Every other crate — threads, conversations, the memory family, skills, attachments, extractors, projects, identity, llm, traces — is a pure record and service authority with no minting power. None of the fifteen can construct a kernel-sealed authorization witness, a trust ceiling, or a capability lease; those constructors are unreachable from this family's dependency set.

## Crates

### `ironclaw_threads`

- **Purpose:** the canonical transcript service for a session — the single service contract every reader and writer of thread and message history uses.
- **Owns:**
  - A service-contract module defining the transcript trait.
  - A filesystem-backed implementation over `ScopedFilesystem`.
  - An in-memory implementation for deterministic tests.
  - Derived index modules for chronological, sequence, and lookup access.
  - Tool-result reference and record storage held inside the transcript.
  - Presentation-derived modules — attachment context, summary artifacts, capability display previews — that project transcript state for display without becoming a second source of truth.
- **Never contains:**
  - Turn lifecycle authority — the kernel owns admission.
  - Delivery policy — outbound and product own that.
  - Channel or binding logic — conversations owns that.
- **Public surface:** `SessionThreadService`, implemented once for filesystem-backed durability and once in memory for tests.
- **Depends on:** `ironclaw_common`, `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_safety`.
- **Never depends on:** anything in kernel/, loop/, extensions/, product/, or app/.
- **Security & authority role:** none — a pure record authority, consumed independently by conversations, product, extension_host, and composition.
- **Why a separate crate:** one contract with several independent consumers and two production-shaped implementations, substantial enough on its own — the transcript, its indexes, and its tool-result vocabulary — that folding it into a neighboring domain would make that neighbor a dumping ground.

### `ironclaw_conversations`

- **Purpose:** the adapter-safe boundary between product and channel adapters and the turn coordinator — external-to-canonical identity binding, actor pairing, and inbound idempotency.
- **Owns:**
  - An inbound module that resolves an external message into a canonical binding and submits it to the turn coordinator.
  - A conversation-state store, durable over `ScopedFilesystem`, and an in-memory implementation used as a real service inside tests, not a stub.
  - Identity and binding value types: adapter installation and kind, external actor and conversation references, external event and message-content references.
  - A trusted-trigger classification module that turns turn-submission failures into retry-or-reject decisions — kept here because this crate owns the inbound-turn error vocabulary those failures are expressed in.
- **Never contains:**
  - Payload parsing — channel packages own that.
  - Transcript content — threads owns that.
- **Public surface:** `ConversationStateStore`; `InboundConversationService`, the binding-resolution service every product and channel adapter calls; consumption — never minting — of the trusted-trigger binding triggers exposes.
- **Depends on:** `ironclaw_filesystem`, `ironclaw_host_api` (including its turn vocabulary), `ironclaw_safety`, `ironclaw_triggers`.
- **Never depends on:** the turn coordinator crate directly, or anything in loop/, product/, or app/.
- **Security & authority role:** guards the trusted-trigger ingress path jointly with triggers — the one host-minted inbound path in the system outside the generic ingress verifier.
- **Why a separate crate:** a distinct identity and idempotency authority, consumed independently by extension_host, product, and composition.

### `ironclaw_triggers`

- **Purpose:** scheduled-trigger records, schedule validation, deterministic fire identity, and the poller's per-tick evaluation step.
- **Owns:**
  - Trigger record grammar, cron and timezone validation, and deterministic fire identity.
  - A filesystem-routed persistence path alongside a dedicated SQL-backed persistence path, held under an ADR, for deployments that need it.
  - An in-memory implementation for deterministic tests.
  - A poller-tick module that evaluates due fires against repository, materializer, submitter, and state-lookup ports it defines and owns.
  - A trusted-submission module that seals the binding identifying a fire as coming from this crate's own poller.
- **Never contains:**
  - Poller lifecycle — starting, stopping, or scheduling the background loop.
  - First-party trigger management capabilities such as create, list, or remove.
  - Turn-coordinator wiring, or database connection and handle construction.
- **Public surface:** the repository, materializer, submitter, and state-lookup ports the poller tick is built from; the sealed trusted-submission binding.
- **Depends on:** `ironclaw_common`, `ironclaw_host_api` (including its turn vocabulary).
- **Never depends on:** the turn coordinator crate directly; anything above substrates.
- **Security & authority role:** host-trusted ingress minting — the sealed trusted-submission path is one of only two mint-capable authorities in the whole family.
- **Why a separate crate:** a distinct scheduling domain with a trusted-mint authority, consumed by conversations, product, and composition.

### `ironclaw_memory`

- **Purpose:** the provider-neutral memory contract — the single service trait every memory provider implements and every caller of memory depends on.
- **Owns:**
  - The `MemoryService` trait and its operation shapes.
  - Memory-document scope and path value types.
  - Prompt-write-safety vocabulary — the shape of what a provider must enforce before writing model-authored content to memory.
  - Significant-event and audit vocabulary for memory writes.
  - A shared conformance suite that every provider implementation wires against.
- **Never contains:**
  - A concrete backend.
  - Embedding computation.
- **Public surface:** `MemoryService` — the provider seam, implemented by the memory-provider extension packages and proven interchangeable by the shared conformance suite; `ironclaw.memory.*` as the naming convention for every memory-facing tool built on this contract.
- **Depends on:** `ironclaw_host_api`, `ironclaw_prompt_envelope`.
- **Never depends on:** any concrete provider crate — providers live above this family, as extension packages; anything above substrates.
- **Security & authority role:** none directly — a neutral contract.
- **Why a separate crate:** one neutral contract implemented by provider extension packages above it and consumed by every memory-reading caller below, proven real by a conformance suite rather than by convention alone.

Memory *providers* are not domains crates. Each provider — the bundled native one and the mem0-backed alternative — ships as an extension package (`extensions/packages/memory-native/`, `extensions/packages/mem0/`) declaring a `[memory]` manifest surface, implementing this crate's `MemoryService`, and passing this crate's conformance suite. Their specifications live in [extensions.md](extensions.md).

### `ironclaw_skills`

- **Purpose:** skill parsing, validation, selection, and management — the extension mechanism for prompt-level agent behavior — plus the pure-learning path that improves skills over time.
- **Owns:**
  - Skill grammar, parsing, and validation.
  - Deterministic selection scoring.
  - Filesystem-backed skill management, including scoped installs.
  - A pure-learning module.
  - An activation-observer contract and its observed-event vocabulary, so callers can project skill-activation state without reaching into the hosting tier directly.
- **Never contains:**
  - WASM hook execution.
  - Extension lifecycle.
  - First-party tool invocation.
- **Public surface:** `SkillInferencePort`, an inversion port implemented by the hosting tier; `SkillActivationObserver`.
- **Depends on:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Never depends on:** kernel/, loop/, product/, or app/.
- **Security & authority role:** none — a content and selection domain. Skill installation authority lives in extensions; the activation trust ceiling lives in trust.
- **Why a separate crate:** a self-contained contract with heavy parsing and selection logic, consumed independently by the hosting tier and by product's activation projection.

### `ironclaw_auth`

- **Purpose:** product-facing authentication — typed flow, interaction, credential-account, recovery, provider-exchange, continuation, and cleanup contracts, backed by durable services and one recipe-driven engine.
- **Owns:**
  - An engine module: token exchange, keepalive and refresh, dynamic client registration, and the value types — credential, flow, provider, domain, scope, account state — the engine is built from.
  - A product-auth module: the caller-facing API, durable flow/account/interaction/cleanup records, credential runtime selection and refresh locking, and the manual-token and gated-OAuth flows product surfaces need.
  - A test-support module of in-memory fakes for downstream conformance testing, compiled only under a test-support feature.
- **Never contains:**
  - Raw HTTP clients, host-runtime credential-injection adapters, or HTTP route serving.
  - Extension lifecycle mutation, or turn replay and resume.
  - Raw OAuth codes, PKCE verifiers, tokens, host paths, or raw secret values in any serializable shape.
- **Public surface:** the flow, interaction, credential-account, recovery, exchange, continuation, and cleanup trait set; `AuthRecipeResolver`, implemented by the extension host; redacted DTOs safe for every product surface to render.
- **Depends on:** `ironclaw_common`, `ironclaw_event_log`, `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_secrets`.
- **Never depends on:** the turn coordinator crate directly — a gate-prompt port exposed through host_api carries the vocabulary it needs instead.
- **Security & authority role:** the family's credential-custody domain — the one crate whose central job is holding token-lifecycle state, though never raw secret bytes, which stay behind secret-store handles.
- **Why a separate crate:** a recipe-driven design that is this crate's whole reason to exist — the family's second vendor-scoped charter. Model-provider session handling lives in `ironclaw_llm`, and host login lives in `webui`: three deliberately distinct credential concerns, not one stack.

### `ironclaw_attachments`

- **Purpose:** the single channel-agnostic routine that lands inbound attachment bytes into agent-accessible storage, plus the ports that route bytes to it.
- **Owns:**
  - The landing routine itself, and the scoped path it writes attachments under.
  - `InboundAttachmentLander` and `InboundAttachmentReader` — the ports every channel and protocol adapter calls — together with their filesystem-backed default implementation, port and implementation living beside each other.
  - The size-ceiling constants every caller of the landing routine shares.
- **Never contains:**
  - Channel-specific payload parsing — an adapter decodes its own payload into a normalized attachment before calling this crate.
  - Delivery or outbound attachment handling.
- **Public surface:** the landing routine; `InboundAttachmentLander` and `InboundAttachmentReader`, port and default implementation together.
- **Depends on:** `ironclaw_common`, `ironclaw_extractors`, `ironclaw_filesystem`, `ironclaw_host_api`.
- **Never depends on:** product/ or app/ — the ports and their implementation both live here, so no caller needs either.
- **Security & authority role:** writes through the project-scoped filesystem authority — the same authority the agent's file tools resolve through — so a write still requires an explicit mount grant, even though the crate itself makes no authorization decision.
- **Why a separate crate:** the single authority for a landing path several independent callers share, with its full contract — port, default implementation, and shared constants — in one place.

### `ironclaw_extractors`

- **Purpose:** pure, format-aware text extraction from file bytes — a leaf with no I/O and no knowledge of where the bytes came from.
- **Owns:**
  - A typed dispatch entry point that inspects a normalized MIME type and fans out to the right extractor: PDF, Office Open XML (word, slide, sheet), legacy Office, RTF, and UTF-8 text or code.
  - Decompression-bomb safety caps enforced on every ZIP-based format, bounding both per-entry and cumulative decompressed size.
- **Never contains:**
  - Attachment landing or storage.
  - Any async or I/O.
- **Public surface:** a single typed extraction function returning structured errors on failure, not a string.
- **Depends on:** `ironclaw_common`.
- **Never depends on:** anything else internal — this crate exists specifically to keep heavy document-parsing dependencies out of every consumer's build.
- **Security & authority role:** none — a pure transform; the bomb-safety caps are hardening, not an authorization decision.
- **Why a separate crate:** a pure leaf with heavy parsing dependencies kept out of consumers; `ironclaw_attachments` is its sole consumer.

### `ironclaw_projects`

- **Purpose:** the Project entity, project membership and access-control records, and the persistence contract that scopes threads, automations, and workspace memory to a project.
- **Owns:**
  - The `Project` entity and its membership and access-control-list types.
  - A live authorization check: resolving access is never cached, so revoking a grant takes effect on the next request.
  - A filesystem-backed repository implementation, with no SQL in this crate.
  - A project-scoped access-gating service, implementing the port product calls, so the domain owns its own access rules end to end.
- **Never contains:**
  - The product-facing port declaration itself, which lives in the product contracts crate.
  - Thread, automation, or memory content — those domains own their own records, scoped only by a project identifier.
- **Public surface:** `ProjectRepository`; the access-gating service implementation.
- **Depends on:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Never depends on:** anything in kernel/ or product/.
- **Security & authority role:** live access-control authority for project scope, enforced uncached on every request.
- **Why a separate crate:** a self-contained entity and access-control domain with a real production path, deliberately kept independent of the domains it scopes rather than folded into any one of them.

### `ironclaw_identity`

- **Purpose:** the canonical identity layer — mapping every external identity, whether a browser OAuth login or an external channel actor, to a stable user identifier, before any runtime state such as conversation binding or thread ownership is touched.
- **Owns:**
  - A resolver that mints, links, or looks up a user identity, keyed by tenant, surface, provider, provider instance, and external subject — with channel actors explicitly barred from minting, so an unrecognized actor fails closed rather than auto-provisioning.
  - The durable home of the minimal user profile: email, display name, and verified-email linkage, gated to browser-OAuth surfaces only.
  - A separate user-directory surface for administrative enumeration and management, kept apart from the resolver so administrative mutation can never perturb minting invariants.
  - The identity-binding store ports that map a provider identity to a user identifier — a persistence concern, so it lives beside the store that implements it rather than in a neutral vocabulary crate.
- **Never contains:**
  - Conversation binding — `ironclaw_conversations` consumes an already-resolved user identifier.
  - WebUI ingress logic.
- **Public surface:** an identity resolver; a user directory; the never-reach-upstream rule — this crate depends on nothing above the filesystem substrate, and nothing above composition may bypass it to touch identity state directly.
- **Depends on:** `ironclaw_host_api`, `ironclaw_filesystem` — the narrowest dependency set of any crate in the family.
- **Never depends on:** composition, product, or any crate above the substrate tier.
- **Security & authority role:** the sole authority deciding when a new user identifier is minted; verified-email linking is restricted to the browser-OAuth surface specifically so a channel actor asserting a verified email can never collide with an OAuth-linked user.
- **Why a separate crate:** bottom-of-stack identity authority with a strictly enforced, never-reach-upstream dependency rule.

### `ironclaw_llm`

- **Purpose:** the model-provider contract, its concrete providers, the provider registry, reliability decorators, and trace recording — the family's other explicitly vendor-scoped domain.
- **Owns:**
  - The `LlmProvider` contract and a concrete adapter for each supported provider.
  - Provider authentication and session handling for each vendor that needs it.
  - A registry and selection layer, wrapped in reliability decorators — retry, circuit-breaking, and failover — composed around any provider.
  - Recording: response caching, trace binding, and the vocabulary a caller uses to record what a model call did.
  - Cost, transcript, and model-selection vocabulary, so callers never need a separate crate for model-adjacent bookkeeping.
  - A versioned model catalog shipped as a crate asset.
- **Never contains:**
  - Product prompt content.
  - Turn orchestration.
- **Public surface:** `LlmProvider` — the family's other named vendor charter, alongside auth's recipes.
- **Depends on:** `ironclaw_common`, `ironclaw_safety`.
- **Never depends on:** anything in kernel/ — authorization to call a model at all is a kernel decision made before dispatch ever reaches this crate.
- **Security & authority role:** none directly — provider selection, credentials, and session refresh are its job, never the authorization decision itself.
- **Why a separate crate:** isolates a heavy provider cone — vendor SDKs and their authentication flows — from every non-LLM consumer's build. Model-provider session handling is a deliberately separate concern from `ironclaw_auth`'s product-facing credential flows and from `webui`'s host login.

### `ironclaw_traces`

- **Purpose:** the Trace Commons client — envelope schema, deterministic redaction, submission queue, credits, and device-key onboarding for contributing traces to the external service.
- **Owns:**
  - Chartered modules for schema, redaction, the submission queue and its holds, credits, and credential handling — kept as separate, independently reviewable concerns rather than one undifferentiated pipeline.
  - A host-facing trace client.
  - Device-key onboarding: issuance, invitation handling, and the onboarding protocol.
  - Storage-path resolution through `ScopedFilesystem`, like every other domain crate.
- **Never contains:**
  - The model-callable trace-submission tool itself, which lives in the first-party extension package as a caller of this crate's client.
  - Raw environment or filesystem access outside `ScopedFilesystem`.
- **Public surface:** the trace client; the redaction pipeline, as the crate's security-critical obligation — every submission is deterministically redacted before it leaves the process.
- **Depends on:** `ironclaw_common`, `ironclaw_host_api`, `ironclaw_llm` (for recording vocabulary), `ironclaw_safety`.
- **Never depends on:** anything in kernel/; HTTP is permitted here specifically for Trace Commons submission.
- **Security & authority role:** the family's security-critical redaction obligation — deterministic redaction of sensitive content before external submission is this crate's central promise.
- **Why a separate crate:** a distinct external-service domain with a redaction obligation serious enough to warrant its own reviewable boundary, and an HTTP cone isolated from every other crate's build.

### `ironclaw_outbound`

- **Purpose:** metadata-only outbound policy and state — notification opt-in, sealed access grants, subscription cursors, and at-most-once delivery-attempt reservation. Never a transport itself.
- **Owns:**
  - A delivery-attempt store enforcing at-most-once semantics through a compare-and-swap reservation from prepared to sending, recoverable after a crash.
  - A policy service that is the only constructor of this crate's sealed grant and binding types.
  - A resolution engine that turns delivery intent into concrete targets.
  - Communication-preference and subscription-cursor records.
- **Never contains:**
  - Any transport send — this crate has no HTTP client dependency.
  - Projection mutation, which belongs to event_projections — outbound only reads projection state to decide what to push.
- **Public surface:** an outbound state-store port; sealed access-grant and delivery-binding types, constructible only through the policy service.
- **Depends on:** `ironclaw_event_projections`, `ironclaw_filesystem`, `ironclaw_host_api` (including its turn vocabulary).
- **Never depends on:** the turn coordinator crate directly; any HTTP or transport client; anything in kernel/, loop/, product/, or app/.
- **Security & authority role:** authority — the sole writer of delivery-attempt state and the sealed-grant minting point; alongside triggers, one of only two domains-family crates that mints trust rather than only recording it. Watch-authorization and push-authorization are kept as separate decisions by design.
- **Why a separate crate:** a distinct durable authority consumed independently by product, extension_host, and the streaming layer; the sealed-type pattern is what lets this stay a domain crate instead of moving into the kernel family.

## Family AGENTS.md requirements

The family root states, verbatim, for every crate that lives under it:

- The family's role, and its boundary against substrate/, kernel/, events/, product/, and extensions/.
- The allowed layer — substrates, with no exception — and the allowed dependency direction described above.
- The two vendor-scoped charters (llm, auth) and the two sealed-mint responsibilities (outbound, triggers), named explicitly so a new crate cannot casually acquire either.
- The persistence idiom: `ScopedFilesystem` is the floor; a hand-written SQL backend requires an ADR.
- A gate for adding a new crate to the family: does this own a genuinely distinct record grammar with independent consumers, or does it belong as a module inside one of the other domain crates? The family's own shape — many narrow crates, few shared dependencies — is the standard a new addition is held to.

Every crate in the family carries its own local guidance file, in whichever of the AGENTS, CLAUDE, or CONTRACT conventions fits its content, stating what it owns, what it must never contain, and its allowed dependency direction in crate-specific terms.
