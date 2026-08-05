
# `crates/domains/` — typed record/service domains

**Layer(s):** substrates · **Crates:** 12 — ironclaw_threads, ironclaw_conversations, ironclaw_triggers, ironclaw_memory, ironclaw_skills, ironclaw_auth, ironclaw_attachments, ironclaw_extractors, ironclaw_identity, ironclaw_llm, ironclaw_trace_commons, ironclaw_outbound · **Security posture:** typed record and service authorities behind the kernel — record grammar and invariants only, never an authorization, approval, or resource decision; two crates mint trust narrowly (outbound — sealed constructors; triggers — ratchet-pinned minting), and one is a credential-custody domain (auth).

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
├── ironclaw_identity         external identity → stable UserId
├── ironclaw_llm              provider contract, providers & decorators
├── ironclaw_trace_commons    Trace Commons client & redaction
└── ironclaw_outbound         outbound authority: sealed grants, at-most-once
```

## Role

`crates/domains/` owns IronClaw Reborn's typed business records: sessions and threads, scheduled triggers, memory documents, skills, credentials, attachments, projects, identity, model providers, trace submissions, and outbound delivery state. Each crate in the family owns exactly one record grammar and the invariants that keep it valid — uniqueness, idempotency, scope isolation, compare-and-swap ordering — and exposes a typed service contract over `ScopedFilesystem` that the kernel and product layers wire against.

A domains crate never decides whether an effect may happen. Authorization, approval, and resource decisions are exhausted by the kernel before a domains-family call is ever reached. A domains crate answers "what does this record mean, and how do I keep it correct," never "is this caller allowed to do this."

The family favors narrow, single-purpose crates over shared infrastructure. Most domains crates serve a handful of independent callers through one production implementation; `ironclaw_memory` is the deliberate exception, with two independent implementations proven interchangeable by a shared conformance suite. Two crates carry a vendor-scoped charter permitted nowhere else in the family — see Boundaries, below.

## Boundaries — what makes this family distinct

- **vs `substrates/`** (filesystem, libsql_runtime, secrets, network, safety, observability): substrate crates are generic mechanism with no domain grammar — the filesystem substrate has no notion of a "trigger" or a "thread." Domains crates are the typed grammar layered on top of that mechanism; strip away every Reborn product concept and a substrate crate still compiles, a domains crate does not.
- **vs `kernel/`** (trust, authorization, approvals, resources, capabilities, processes, turns, host_runtime): kernel crates decide whether and how an effect happens; domains crates decide what a record means once the kernel has already admitted the request. No domains crate depends on a kernel crate, and no domains crate can construct a capability-authorization witness or a lease.
- **vs `events/`** (event_log, event_store, event_projections, event_streams): events are immutable, replayable evidence of what happened; domains own live state, mutated in place under compare-and-swap. A projection is a rebuildable read cache derived from events and is never a second write authority.
- **vs `product/`** (assistant, operator, openai_compat, webui, host_ingress): product orchestrates user-facing workflow, admission, and delivery across many domains at once inside a single request; a domains crate knows nothing about channels, commands, or views — it owns one record type's invariants only.
- **vs `extensions/`** (extension_registry, extension_host, extension_manager, packages): extensions are the installable-package concept — manifests, lifecycle, hosting, vendor packages. A domains crate is a durable business-record type available to every extension, product, and kernel caller, with no notion of install, activate, or package.
- **Two vendor-scoped charters, permitted nowhere else in the family:** `ironclaw_llm` is a closed provider cone holding model-vendor adapters and their authentication flows; `ironclaw_auth` keeps vendor differences as recipe data consumed by one generic engine, never a code branch. No other domains crate holds a vendor name.
- **The narrow-trust-mint pattern:** two crates mint trust instead of promoting it into kernel authorization machinery, each held by a different mechanism. `ironclaw_outbound` seals its delivery-attempt and access-grant types with private constructors — only its own policy service can construct them. `ironclaw_triggers` mints the trusted-submission binding that identifies its own poller as a host-trusted sender; its constructor is public, and the protection is the ownership ratchet tests that pin who may call it. Both keep one narrow, reviewable trust responsibility inside a domain crate instead of promoting the crate into the kernel family.

## What belongs here / What never belongs here

- **Belongs:** record schemas and their invariants — validation, uniqueness, idempotency keys, compare-and-swap ordering; domain services exposing typed create/read/update/query operations over `ScopedFilesystem`; a default implementation of each service; the two vendor-scoped charters and the two sealed-mint responsibilities described above.
- **Never belongs — backend selection:** which storage backend actually serves a mount is a deployment decision made once, outside every domains crate. A domains crate never branches on backend.
- **Never belongs — authority decisions:** authorization, approval, and resource-reservation decisions stay in the kernel family.
- **Never belongs — transport and framework code:** no domains crate touches Axum; HTTP appears only inside the narrow egress needs of the vendor-scoped and external-service charters.
- **Never belongs — vendor names or vendor branches**, outside `ironclaw_llm` and `ironclaw_auth`.
- **Persistence idiom:** `ScopedFilesystem` is the floor. Every domains crate is backend-neutral by construction, depending only on the filesystem substrate's virtual-path, mount, and compare-and-swap authority — never a database driver directly. A crate that instead needs a hand-written SQL backend is a deliberate, narrow design choice that must be justified by an ADR; `ironclaw_triggers` is the one domain crate built this way, alongside `ironclaw_hooks` in the loop family. **Both ADRs are written and both decided KEEP (2026-08-04): [`docs/adr/0003-triggers-keeps-hand-written-sql.md`](../../../adr/0003-triggers-keeps-hand-written-sql.md) and [`docs/adr/0004-hooks-keeps-its-predicate-state-backends.md`](../../../adr/0004-hooks-keeps-its-predicate-state-backends.md).** They are exceptions for different reasons and should not be cited as one precedent: triggers' claim/lease semantics are not expressible on the fabric *and* both its backends ship by profile, whereas hooks' backends are complete but **unwired** (composition hard-codes the in-memory one) and are kept as staged work against multi-host counters. A third crate wanting this exception needs its own ADR clearing the same bar, not a reference to these. Such a crate still does not get its own connections: it owns its SQL and its transactions, and takes admission from the substrate runtime that owns the pool, so its writes queue on the same lane as every other writer to that database.

## Dependency direction

Domains crates depend only on `substrates/`, `events/`, and `contracts/` — never on `kernel/`, `loop/`, `extensions/`, `product/`, or `app/`.

| May depend on | Never depends on |
|---|---|
| `substrates/` — filesystem for every crate; secrets and network only where a crate's charter needs them (auth) · `events/` — chiefly event_projections · `contracts/` — host_api, common, prompt_envelope · plus the three chartered same-family edges only: conversations→triggers, attachments→extractors, trace_commons→llm | `kernel/` in full — capabilities, host_runtime, authorization, approvals, resources, trust, turns, processes · `loop/` · `extensions/` · `product/` · `app/` |

HTTP and vendor SDKs are permitted only inside `ironclaw_llm`, `ironclaw_trace_commons`, and `ironclaw_auth`'s engine — the family's three named vendor and external-service cones. No other domains crate reaches HTTP directly.

Inside the family, dependency edges are shallow and few: `ironclaw_conversations` depends on `ironclaw_triggers` for trusted-submission binding vocabulary; `ironclaw_attachments` depends on `ironclaw_extractors` for pure bytes-to-text transformation; `ironclaw_trace_commons` depends on `ironclaw_llm` to reuse its recording vocabulary. No other crate in the family depends on a sibling — the family's internal graph is a shallow forest, not a mesh.

> ✎ **Corrected 2026-08-02 (Wave 2 truth audit) — there is a fourth edge, and Wave 2 created it.** Prior text, quoted: *"No other crate in the family depends on a sibling — the family's internal graph is a shallow forest, not a mesh."* `ironclaw_attachments` now depends on `ironclaw_threads` (`attachments/src/ports.rs:23`, `src/project_scoped.rs:24`, for `ThreadScope`), acquired with the WS5 attachments widening. It is a legal downward-within-layer edge and the forest is still a forest — four edges, no cycles — but the sentence claimed an exhaustive list and is now short by one. The `attachments` entry's own **Depends on** line below carries the same omission plus a second one, and is corrected there.

## Security & authority

Most crates in this family hold no authority at all: a call into `ironclaw_threads` or the identity crate's project module is always downstream of a kernel admission decision, never a gate itself. Three crates are the named exceptions:

- **`ironclaw_outbound`** is the sole writer of delivery-attempt state and mints the sealed access-grant and delivery-binding types described above.
- **`ironclaw_triggers`** is the one host-trusted inbound path outside the generic ingress verifier — its trusted-submission binding is the sealed evidence that a fire came from its own poller.
- **`ironclaw_auth`** is a credential-custody domain: it holds durable token-lifecycle state, but never raw secret bytes, and it never makes an authorization decision itself.

Every other crate — threads, conversations, memory, skills, attachments, extractors, identity, llm, traces — is a pure record and service authority with no minting power. None of the twelve can construct a kernel-sealed authorization witness, a trust ceiling, or a capability lease; those constructors are unreachable from this family's dependency set.

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
  - Identity and binding value types: adapter installation and kind, external actor and conversation references, external event and message-content references. ✎ **Corrected 2026-08-02 (Wave 2 truth audit): the external actor and conversation references are no longer owned here — WS5 unified them.** They were declared twice, field-divergently, and the divergence was live: this crate's copies derived `PartialEq`/`Hash` (so they included the per-event message id) while the canonical type excludes the reply-target hint by hand, meaning two refs for the same route compared equal or unequal **depending on which copy the caller happened to hold**. Both are now declared only at `ironclaw_extension_contracts::external` (`:69,144`) and this crate gained the dep — `substrates → contracts`, downward, no exception — with `product`'s two field-by-field translators deleted. `reborn_conversations_threads_attachments.rs` pins the single home. What this crate *does* still own, and deliberately, is the **durable grammar**: `stored_refs` keeps writing the released on-disk spelling `{space_id, conversation_id, thread_id, message_id}` and reads either, so the rename is invisible to storage in both directions and a rollback stays safe. That belongs with the crate that owns the *records*, not the crate that owns the *type*.
  - A trusted-trigger classification module that turns turn-submission failures into retry-or-reject decisions — kept here because this crate owns the inbound-turn error vocabulary those failures are expressed in.
- **Never contains:**
  - Payload parsing — channel packages own that.
  - Transcript content — threads owns that.
- **Public surface:** `ConversationStateStore`; `InboundConversationService`, the binding-resolution service every product and channel adapter calls; consumption — never minting — of the trusted-trigger binding triggers exposes.
- **Depends on:** `ironclaw_filesystem`, `ironclaw_host_api` (including its turn vocabulary), `ironclaw_safety`, `ironclaw_triggers`. ✎ *Add `ironclaw_extension_contracts`, acquired by the WS5 unification above (2026-08-02).*
- **Never depends on:** the turn coordinator crate directly, or anything in loop/, product/, or app/. ✎ **Flagged 2026-08-02 (Wave 2 truth audit) — this is the family's target, and it reads as an achieved invariant while the live tree does the opposite and a crate guardrail *mandates* it.** `ironclaw_conversations` depends on `ironclaw_turns` directly and heavily (`types.rs`, `error.rs`, `memory.rs`, `traits.rs`, `trusted_trigger.rs`, `conversation_state_store.rs`), and `crates/ironclaw_conversations/CLAUDE.md` instructs contributors to *preserve* the typed `ironclaw_turns::TurnError` rather than flatten it. The edge is the one surviving `LAYER_MATRIX_EXCEPTION` from PROPOSAL §8.3's row 1, and it is not vocabulary: `InboundTurnService` is generic over `C: TurnCoordinator`, holds `Arc<C>`, and calls `submit_turn` — turn **admission authority**, which no contracts crate can dissolve. It clears when the inbound submit orchestration moves to the product tier, which is the still-open WS5 `product` row (the exception's `removes_in = "WS5"` has therefore been passed without falling — PROPOSAL §8.3). Until then this line is the destination, not a description, and the two documents disagree on purpose rather than by accident.
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
- **Depends on:** `ironclaw_common`, `ironclaw_filesystem`, `ironclaw_host_api` (including its turn vocabulary).
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
- **Why a separate crate:** one neutral contract implemented by provider extension packages above it and consumed by every memory-reading caller above, proven real by a conformance suite rather than by convention alone.

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
- **Depends on:** `ironclaw_common`, `ironclaw_extractors`, `ironclaw_filesystem`, `ironclaw_host_api`. ✎ **Corrected 2026-08-02 (Wave 2 truth audit): two more, both acquired by the WS5 widening that built this entry's public surface.** `ironclaw_threads` (for `ThreadScope`, which the ports are keyed on) and — the one that is a live decision rather than bookkeeping — **`ironclaw_product_contracts`**. Both ports error with `ProductSurfaceError` because their failing caller is a product surface and the WebUI bytes endpoint maps the code straight onto its 404/403. That is layer-legal (contracts is the lowest layer, and this is the *neutral* product-tier contract crate, not `ironclaw_assistant`), but it does put product-tier error vocabulary in a substrate. The alternative — narrow the ports onto an attachments-owned error and map at the product boundary — **moves an HTTP status mapping**, which is behavior rather than placement, so it is recorded as a `[decision]` on CHECKLIST WS5 and not taken.
- **Never depends on:** product/ or app/ — the ports and their implementation both live here, so no caller needs either. ✎ *Still true as written, and worth reading precisely after the correction above: the crate names `ironclaw_product_contracts`, which is a **contracts**-family crate, not `product/`. The rule holds on its letter; the `[decision]` above is about whether it holds on its spirit.*
- ✎ **Known carve-out, 2026-08-02: one adapter did not arrive and cannot.** `ProjectScopedAttachmentReader` implements `ironclaw_loop_host::LoopAttachmentReadPort` as well as `InboundAttachmentReader`; `loop_host` is a `loops` crate and this is a `substrates` crate, so the dep would be an illegal upward edge — and even granting it, the impl would then have neither side in `ironclaw_assistant` and the orphan rule forbids leaving it behind. It stays in product, and `reborn_conversations_threads_attachments.rs` pins it *there* **and** asserts `ironclaw_attachments` never acquires a `loop_host` dep, so "finishing the row" cannot quietly mean dragging the loop tier into a substrate. Closing this needs either a re-word of the charter to exclude the read adapter or a re-home of the port first — **tracked in #7010**, and the CHECKLIST WS5 row stays `[~]` until it closes.
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
- **Public surface:** a single typed extraction function returning structured errors on failure, not a string. ✎ *2026-08-03 (WS6): true as of the §6.4.10 slice, and it is **five** items, not one — a MIME-driven entry point, a filename-driven one, the canonical truncation helper, the outcome classification, and the error type. Everything else went private, including two items that were `pub` with zero external callers.*
- **Depends on:** `ironclaw_common`.
- **Never depends on:** anything else internal — this crate exists specifically to keep heavy document-parsing dependencies out of every consumer's build.
- **Security & authority role:** ✎ **2026-08-03 (WS6): not "none".** The bomb-safety caps are hardening, and the *transform* is pure — but the failure type is a redaction boundary. Extraction diagnostics can echo document content (parser messages, ZIP entry names), and three consumers put extraction outcomes in front of a model. So `ExtractionError`'s `Display` renders the classification and nothing else, and its `Debug` is the logging shape; a variant whose `#[error(…)]` interpolates a field re-opens the leak this replaced. The crate decides nothing about authority, but it does decide what a failure is allowed to say.
- **Why a separate crate:** a pure leaf with heavy document-parsing dependencies kept out of its consumers — the attachment landing routine, the kernel's tool-output mediation, and the first-party file tools all extract text without inheriting each other's surfaces or the parser cone.

### `ironclaw_identity`

- **Purpose:** the canonical identity layer — mapping every external identity, whether a browser OAuth login or an external channel actor, to a stable user identifier, before any runtime state such as conversation binding or thread ownership is touched — and, as its `projects` module, the Project entity with its membership and access-control records.
- **Owns:**
  - A resolver that mints, links, or looks up a user identity, keyed by tenant, surface, provider, provider instance, and external subject — with channel actors explicitly barred from minting, so an unrecognized actor fails closed rather than auto-provisioning.
  - The durable home of the minimal user profile: email, display name, and verified-email linkage, gated to browser-OAuth surfaces only.
  - A separate user-directory surface for administrative enumeration and management, kept apart from the resolver so administrative mutation can never perturb minting invariants.
  - The identity-binding store ports that map a provider identity to a user identifier — a persistence concern, so it lives beside the store that implements it rather than in a neutral vocabulary crate.
  - The `projects` module: the `Project` entity, membership and access-control-list records, `ProjectRepository`, and the project-scoped access-gating service implementing the port the product contracts declare — with access resolved live on every request, never cached, so revocation takes effect immediately.
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

### `ironclaw_trace_commons`

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

- The family's role, and its boundary against substrates/, kernel/, events/, product/, and extensions/.
- The allowed layer — substrates, with no exception — and the allowed dependency direction described above.
- The two vendor-scoped charters (llm, auth) and the two sealed-mint responsibilities (outbound, triggers), named explicitly so a new crate cannot casually acquire either.
- The persistence idiom: `ScopedFilesystem` is the floor; a hand-written SQL backend requires an ADR.
- A gate for adding a new crate to the family: does this own a genuinely distinct record grammar with independent consumers, or does it belong as a module inside one of the other domain crates? The family's own shape — many narrow crates, few shared dependencies — is the standard a new addition is held to.

Every crate in the family carries its own local guidance file, in whichever of the AGENTS, CLAUDE, or CONTRACT conventions fits its content, stating what it owns, what it must never contain, and its allowed dependency direction in crate-specific terms.
