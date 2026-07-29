
# Family: `crates/domains/` — typed record/service domains

**Layer(s):** `substrates` (target, for all 15 — `ironclaw_skills` reassigns `loops`→`substrates`, PROPOSAL.md §3 Deliberate Adjustment D)
**Crates (target):** 15 — `ironclaw_threads`, `ironclaw_conversations`, `ironclaw_triggers`, `ironclaw_memory`, `ironclaw_memory_native`, `ironclaw_memory_mem0`, `ironclaw_skills`, `ironclaw_auth`, `ironclaw_attachments`, `ironclaw_extractors`, `ironclaw_projects`, `ironclaw_identity` (renamed), `ironclaw_llm`, `ironclaw_traces` (renamed), `ironclaw_outbound`
**Security posture:** typed record/service authorities behind the kernel — record grammar, invariants, and CAS-mutated state, never an authorization/approval/resource decision; two crates mint/seal trust narrowly (`outbound` sealed delivery grants, `triggers` trusted-submit binding), one is a credential-custody domain (`auth`), and none may depend on `kernel/` or above.

## Identity — what this family IS

`crates/domains/` is where IronClaw Reborn's *nouns* live. Each crate owns exactly one record grammar — a `SessionThreadRecord`, a `TriggerRecord`, a `StoredUser`, a `MemoryService` document, a `Project`, an `OutboundDeliveryAttempt` — plus the invariants that keep it valid (uniqueness, idempotency, scope isolation, CAS ordering) and a typed service contract over `ScopedFilesystem` that kernel and product wire against.

Per PROPOSAL.md §6.4's family role, domains crates are "domain owners: each owns its record grammar, invariants, and service contract over `ScopedFilesystem` (backend-neutral), exposing typed ports that kernel/product wire." Nothing in this family decides *whether* an effect may happen — that is exhausted before a domains-family call is reached (T3/T5 in §7's trust-transition walkthrough have already run by the time e.g. `threads` or `outbound` is invoked). A domains crate answers "what does this record mean, and how do I keep it correct," never "is this caller allowed to do this."

Structurally the family is the audit's largest crate count (15 of 64 target `crates/` packages) but has the smallest *individual* blast radius: most crates here have 2–8 consumers, a single production implementation apiece (memory being the deliberate, tested exception with two), and — with the sole exception of `auth`'s comprehensive blocklist — the plainest boundary rules in the workspace. This is deliberate: crate-gate criterion 1 ("independent contract, real consumers") is satisfied by *narrowness*, not by shared infrastructure.

Two crates in this family are the two places PROPOSAL.md explicitly tolerates vendor names outside `extensions/packages/*` — see "What makes it distinct" below. None of the 15 crates carry a `[#6696]` PR-contingency tag anywhere in §6.4 or §9 rows 16–30; §12.3's contingency list ("processes widening, run_state deletion, approvals widening, runner scheduler/await-edge shed") is exhaustively a `kernel/`+`loop/`-family matter. Every mapping below is valid whether or not PR #6696 lands.

## What makes it distinct

- **vs `substrate/`** (filesystem, secrets, network, safety, observability): substrate crates are generic mechanism with zero domain grammar — `ironclaw_filesystem` has no idea what a "trigger" or a "thread" is. Domains crates are the typed grammar built *on top of* that mechanism.
- Remove all of Reborn's product concepts and a substrate crate still compiles; a domains crate does not.
- **vs `kernel/`** (trust, authorization, approvals, resources, capabilities, processes, turns, host_runtime): kernel decides *whether and how* an effect happens; domains decide what a record *means* once the kernel has already admitted the request.
- §8.2's forbidden-edge matrix is explicit here: domains crates may never depend on kernel (including no `capabilities`/`host_runtime`), and no domains crate mints an `Authorized` witness or a capability lease.
- **vs `events/`** (events, event_store, event_projections, event_streams): events are immutable, replayable *evidence* of what happened; domains own *live, current* state, mutated in place under CAS.
- A `TriggerRecord` or `StoredUser` is primary record state; a projection is a rebuildable read-cache derived from events and is never a second write authority (invariant 11).
- **vs `product/`** (product, operator, openai_compat, webui, host_ingress): product orchestrates user-facing workflow/admission/delivery UX *across many domains at once* in a single request; a domains crate knows nothing about channels, commands, or views — it owns one record type's invariants only.
- **vs `extensions/`** (extensions, extension_host, extension_manager, packages/\*): extensions are the installable-package concept — manifests, lifecycle, hosting, vendor packages. A domains crate is a durable business-record type available to *every* extension/product/kernel caller, with no notion of "install," "activate," or "package."
- **The two explicitly vendor-scoped exceptions** (§6.4 family role: "vendor branches [forbidden] except inside the two explicitly vendor-scoped domains: `llm` providers and `auth` recipes-as-data"): `ironclaw_llm` is a closed provider cone (NEAR AI, OpenAI, Anthropic, Ollama, Bedrock, GitHub Copilot, Codex); `ironclaw_auth` keeps vendor differences as recipe *data* consumed by one generic `AuthEngine`, never a code branch.
- No other domains crate may hold a vendor name — the specificity scanner's shrink-only allowlist (§8.1 rule 4, §11.2.8) is the mechanical enforcement of this exact carve-out.
- **The sealed-trust-types pattern** (outbound grants, trigger minting): rather than being promoted into `kernel/`, two crates hold a narrow, code-reviewable trust-minting responsibility through sealed types instead of a kernel crate's authorization machinery.
- `ironclaw_outbound::types` seals `ThreadProjectionAccessGrant`/`ValidatedReplyTargetBinding` behind `pub(crate)` fields with doc comments reading "Sealed against external construction — obtain instances only by calling …" (confirmed live, `src/types.rs:103,118,233,267`), constructible only via `OutboundPolicyService`.
- `ironclaw_triggers::trusted_submit` mints `TriggerTrustedInboundBinding` and exposes `is_trusted_trigger_adapter_kind` as the *only* sanctioned way to recognize the trusted-trigger adapter identity — its own doc comment: "callers in other crates must use this function rather than comparing … directly."
- Both are named in §7's walkthrough (T3, T8) as staying sealed in this family rather than migrating to `kernel/` — the family's alternative to promoting a crate out of `domains/` every time it needs one narrow trust responsibility.

## What belongs here / What must never be here

- **Belongs:** record schemas and their invariants (validation, uniqueness, idempotency keys, CAS ordering); domain services (typed CRUD/query/resolve over `ScopedFilesystem`); domain factories producing a default/native implementation; the two vendor cones named above; the two sealed-mint responsibilities named above.
- **Must never be here:** backend *selection* decisions — that is composition's `RootFilesystem` catalog choice, per `storage-placement.md`'s hybrid rule ("file-shaped → `RootFilesystem`; structured control-plane → typed repos owned by service domain; derived → owning projection layer").
- Authority decisions (trust/authorization/approval/resource-reservation) stay in `kernel/`.
- HTTP surfaces beyond the two vendor cones' own charters never appear here — Axum lives only in `product/ironclaw_host_ingress`.
- Vendor branches outside `llm`/`auth` are forbidden everywhere else in the family, with zero exceptions.
- Domain stores never branch on backend — a domains crate that inspects "is this libSQL or Postgres" is as much a boundary violation as a kernel crate inspecting a lane would be.
- **Persistence idiom:** `ScopedFilesystem` is the floor. Every domains crate is backend-neutral by construction because it depends only on the filesystem's virtual-path/mount/CAS authority, never a driver — why `threads`, `conversations`, `memory_native`, `projects`, and `identity` carry zero SQL.
- Hand-written SQL is a named exception requiring an ADR (§12.10: "trigger/hook SQL convergence vs ADR"). **Today exactly two crates violate this: `ironclaw_triggers`** (`src/libsql.rs` 1,847 + `src/postgres.rs` 1,500 = 3,347 lines, confirmed in source) **and `ironclaw_hooks`** (the `loop/`-family sibling exception, not itself in this family).
- §11.2.6 seeds the shrink-only `libsql`/`deadpool-postgres`/`tokio-postgres` allowlist with exactly `{triggers, hooks}` so no third crate can add hand-SQL without the same ADR path.

## Dependency direction

Per §8.1's projection and §8.2's forbidden-edge matrix, domains crates may depend on `substrate/`, `events/`, and `contracts/` — and nothing above:

| From `domains/` | May depend on | Forbidden |
|---|---|---|
| any crate in this family | `substrate/` (filesystem, secrets\*, network\*, safety), `events/` (chiefly `event_projections`), `contracts/` (`host_api`, `common`, `prompt_envelope`) | `kernel/` (no `capabilities`/`host_runtime`/`authorization`/`approvals`/`resources`/`trust`/`turns`/`processes`), `loop/`, `extensions/`, `product/`, `app/` |
| `llm`, `traces`, `auth` (engine half), `memory_mem0` | HTTP/vendor SDKs — the family's named exception (§8.2: "HTTP only in llm/traces/auth-engine/mem0 via their charters") | any other domains crate reaching HTTP directly |

Within the family, sanctioned intra-family edges (verified in current `Cargo.toml`s, retained at target):

- `conversations → triggers` — trusted-trigger binding consumption.
- `memory_native → memory` and `memory_mem0 → memory` — the provider seam.
- `attachments → extractors` — a pure bytes→text transform, not authority.
- `traces → llm` — trace recording reuses `ironclaw_llm::recording`.

No other crate in this family depends on a sibling — the family is deliberately fan-out-light; the intra-family graph is a shallow forest, not a mesh. Every current direct `→ ironclaw_turns` edge in this family (`conversations`, `triggers`, `outbound`) sheds once turn vocabulary completes in `host_api::turn`, dissolving three of the seven standing W4.3 `LAYER_MATRIX_EXCEPTIONS` (§8.3); `auth`'s separate `→ turns` "follow-up" exception dissolves the same way through a `host_api`/`extension_contracts` gate-prompt port. Once both dissolve, no crate in this family will hold a direct dependency on any `kernel/`-family crate at all — the layer matrix's forbidden edge and the family's actual dependency graph converge exactly.

## Security & authority role

The great majority of this family holds **zero** authority: a call into `ironclaw_threads` or `ironclaw_projects` is always downstream of a kernel admission decision, never a gate itself. Three crates are the named exceptions:

- **`ironclaw_outbound`** — sole writer of delivery-attempt state; mints the sealed `ThreadProjectionAccessGrant`/`ValidatedReplyTargetBinding` types; the CAS `Prepared→Sending` reservation is the "T8" trust transition in §7 (decision-to-speak → external delivery).
- **`ironclaw_triggers`** — the trusted-submit minting path is "the one host-minted inbound path" alongside T2's generic ingress verifier (§7's T3 walkthrough: "Trusted-trigger ingress … stays sealed in `triggers`/`conversations`").
- **`ironclaw_auth`** — a **credential-custody domain**: durable product-auth flow/account/interaction state and the recipe-driven `AuthEngine`, but never raw secret bytes (those stay behind `SecretStore` handles per its own charter) and never an authorization decision itself.

Every other crate — `threads`, `conversations` (apart from its trusted-trigger consumption), the memory family, `skills`, `attachments`, `extractors`, `projects`, `identity`, `llm`, `traces` — is a pure record/service authority with no minting power at all. None of the fifteen may construct a `capabilities`-family `Authorized` witness, a `trust`-family sealed `TrustClass`, or a `authorization`-family `CapabilityLease` — those constructors are not importable from this family under §8.2's forbidden-edge matrix, so the boundary is compiler-enforced, not review-enforced.

## Crate specifications

### `ironclaw_threads`

- **Path & disposition:** `crates/ironclaw_threads/` → `crates/domains/ironclaw_threads/` — retain + move, no split (PROPOSAL.md §9 row 16; §6.4.1).
- **Purpose:** the canonical transcript service — the `SessionThreadService` contract plus its two production-shaped implementations.
- **Target contents** (11,208 lines total; no guidance file today):
  - `src/contract.rs` (907 ln) — the `SessionThreadService` trait and its request/response DTOs.
  - `src/filesystem_service.rs` (3,610 ln) — the durable impl over `ScopedFilesystem`.
  - `src/filesystem_service/thread_index.rs` (975 ln) — the thread-listing derived index.
  - `src/filesystem_service/{message_sequence_index.rs (192), message_lookup_index.rs (191)}` — sequence/lookup derived indexes.
  - `src/in_memory.rs` (1,384 ln) — the semantic-test impl.
  - `src/service.rs` (568 ln) — shared service-level helpers.
  - `src/tool_result_reference.rs` (2,171 ln) + `src/tool_result_records.rs` (82 ln) — tool-call result storage/reference vocabulary held in the transcript.
  - `src/attachment_context.rs` (288 ln), `src/summary_artifacts.rs` (267 ln), `src/capability_display_preview.rs` (244 ln) — transcript-adjacent derived/presentation data.
  - `src/title.rs` (130 ln) — `derive_thread_title`, deliberately not re-exported (internal helper for the two backend impls only, per its own doc comment).
- **Migration delta:** pure family move — no gains, no sheds, no deletions.
  - Its only cross-crate obligation is passive: `ironclaw_conversations` (below) renames *its own* colliding trait so `ironclaw_threads::SessionThreadService` remains the one canonical name in the workspace.
- **Owns:**
  - Canonical `SessionThreadRecord`/message transcript state.
  - Tool-result references and records.
  - Thread indexes (by sequence, by lookup key, by listing).
- **Must never contain:**
  - Turn lifecycle authority (kernel/turns owns admission).
  - Delivery policy (outbound/product).
  - Channel/binding logic (conversations).
- **Allowed internal deps:** `ironclaw_common`, `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_safety` (current `Cargo.toml`, unchanged at target).
- **Forbidden deps:** everything ≥ `kernel`.
- **Public contracts & ports:**
  - `SessionThreadService` — 2 production-shaped implementations (`FilesystemSessionThreadService`, in-memory).
  - No injected sub-ports; the trait itself is the whole surface.
- **Consumers:** 5, per §6.4.1 — conversations, product, extension_host, composition, and the in-memory test-support path.
- **Security & authority role:** none — pure record authority.
- **Why a crate (not a module):** criterion 1 — one contract, 5 consumers, 2 impls; the transcript is large and hot enough (11.2k lines) that folding it into a sibling would make that sibling the new dumping ground.
- **Enforcement:** `reborn_crate_dependency_boundaries_hold`; gains a guidance file and the family⇄layer check (§11.2.1, §11.4).

### `ironclaw_conversations`

- **Path & disposition:** retain + move, **internal renames mandatory** (§9 row 17; §6.4.2).
- **Purpose:** the adapter-safe boundary between product/channel adapters and `ironclaw_turns::TurnCoordinator` — external↔canonical binding, actor pairing, accepted-message/turn-submission idempotency, and the trusted-trigger submitter.
- **Target contents** (5,205 lines total; has AGENTS.md + CLAUDE.md):
  - `src/inbound.rs` (1,895 ln) — accepts external inbound, resolves binding, submits to turns; also where trusted-trigger prompt safety-scanning currently lives.
  - `src/memory.rs` (1,701 ln) — `InMemoryConversationServices`, confirmed **production code**, not test-only, despite the name.
  - `src/conversation_state_store.rs` (727 ln) — `ConversationStateStore` + `RebornFilesystemConversationServices`, the durable impl.
  - `src/types.rs` (324 ln) — shared conversation-binding value types.
  - `src/ids.rs` (255 ln) — `AdapterInstallationId`/`AdapterKind`/`ExternalActorRef`/`ExternalConversationIdentity`/`ExternalConversationRef`/`ExternalEventId`/`InboundMessageContentRef`.
  - `src/traits.rs` (151 ln) — the service trait(s), including the colliding trait renamed below.
  - `src/trusted_trigger.rs` (62 ln) — `classify_inbound_error`, kept here because, per its own doc comment, "it is the crate that owns `InboundTurnError`."
- **Migration delta — mandatory renames:**
  - Rename the same-named trait in `src/traits.rs` off `SessionThreadService` (→ e.g. `InboundConversationService`) — the audit's worst naming collision: identical name, different DTOs, versus `ironclaw_threads::SessionThreadService`.
  - Unify `ExternalActorRef`/`ExternalConversationRef` with the `host_api` pair — one canonical definition survives; the other, plus product's field-by-field translators, is deleted.
  - Move the trusted-trigger prompt safety-scanning in `inbound.rs` behind the triggers/kernel seam it guards, as a module move (not a behavior change).
- **Owns:**
  - External↔canonical identity binding.
  - Accepted-message/turn-submission idempotency.
  - Actor pairing.
- **Must never contain:**
  - Payload parsing (channel packages own that).
  - Transcript content (threads).
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_safety`, `ironclaw_triggers`, plus turn vocabulary via `host_api::turn` (sheds the direct `ironclaw_turns` dep visible in today's `Cargo.toml`, dissolving its W4.3 exception).
- **Forbidden deps:** kernel beyond vocabulary; loop/product/app.
- **Public contracts & ports:**
  - `ConversationStateStore`.
  - The renamed inbound-binding service trait.
  - `TriggerTrustedInboundBinding` consumption (not minting — that stays in `triggers`).
- **Consumers:** extension_host, product, composition — the DTO unification with `host_api` is a hard target requirement for all three, not optional cleanup.
- **Security & authority role:** guards the trusted-trigger ingress path jointly with `triggers` (§7 T3: "the one host-minted inbound path … stays sealed in `triggers`/`conversations`").
- **Why a crate:** distinct identity/idempotency authority consumed by extension_host/product/composition; the binding logic is genuinely separate from both the transcript it feeds and the turn kernel it feeds into.
- **Enforcement:** the `conversations → turns` W4.3 exception's dissolution is a pinned architecture-test change (§8.3 row 1); a rename ratchet forbids re-introducing the `SessionThreadService` name outside `ironclaw_threads`.

### `ironclaw_triggers`

- **Path & disposition:** retain + move (§9 row 18; §6.4.3).
- **Purpose:** scheduled-trigger records, cron/timezone validation, deterministic fire identity, the poller's deterministic `tick_once` step, and trusted-submit minting.
- **Target contents** (12,908 lines total incl. ~5,679 lines of in-crate tests; has AGENTS.md, **missing CLAUDE.md** — confirmed, audited gap):
  - `src/lib.rs` (1,462 ln) — `TriggerRecord`/schedule/source-provider vocabulary + repository traits.
  - `src/libsql.rs` (1,847 ln) — **hand-written libSQL backend, half of the family's documented persistence-idiom exception.**
  - `src/postgres.rs` (1,500 ln) — hand-written Postgres backend, the other half (3,347 combined lines).
  - `src/in_memory.rs` (751 ln) — test backend.
  - `src/worker.rs` (100 ln) — `TriggerPollerWorker::tick_once` entry point.
  - `src/worker/{ports.rs (672), due_fire.rs (258), active_cleanup.rs (261)}` — the repository/materializer/submitter/state-lookup ports the tick is built from.
  - `src/worker/{report.rs (92), config.rs (83), failure.rs (65)}` — tick reporting, configuration, and failure classification.
  - `src/trusted_submit.rs` (138 ln) — `TriggerTrustedInboundBinding`, `TRIGGER_TRUSTED_ADAPTER_KIND`, `is_trusted_trigger_adapter_kind`.
- **Migration delta:** no split, no rename, no gains/sheds — a pure family move.
  - Its standing obligation is the persistence-idiom resolution: converge `libsql.rs`/`postgres.rs` onto the filesystem fabric, or write the ADR that keeps them (§12.6, §12.10).
- **Owns:**
  - Trigger record grammar and cron/timezone validation.
  - Deterministic fire identity.
  - The trusted-submit binding.
- **Must never contain** (per its own AGENTS.md):
  - Poller lifecycle, background worker startup/shutdown, or composition wiring.
  - First-party trigger capabilities (create/list/remove).
  - Trusted inbound *turn* wiring or outbound delivery resolution.
  - libSQL/PostgreSQL handle construction or connection-string validation — composition/bootstrap owns those.
- **Allowed internal deps:** `ironclaw_common`, `ironclaw_host_api`, turn vocabulary via `host_api::turn` (sheds the direct `ironclaw_turns` dep visible today, dissolving its W4.3 exception).
- **Forbidden deps:** composition/bootstrap-only concerns.
- **Public contracts & ports:**
  - The repository/materializer/submitter/state-lookup port set the poller tick is built from.
  - `TriggerTrustedInboundBinding` as the trusted-submit evidence.
- **Consumers:** conversations, product, composition (poller lifecycle wiring only, not the tick logic itself).
- **Security & authority role:** **security-relevant** — host-trusted ingress minting; the trusted-submitter path is pinned by the existing trusted-trigger tests; one of only two domains-family crates with mint authority.
- **Why a crate:** distinct domain + trusted-mint authority, consumed by conversations/product/composition.
- **Enforcement:** the trusted-trigger minting scans (§11.1, kept); the shrink-only `{triggers, hooks}` hand-SQL allowlist (§11.2.6).
- **Open questions (§12.10):** "trigger/hook SQL convergence vs ADR" — explicitly unresolved; not forced by this proposal.

### `ironclaw_memory`

- **Path & disposition:** retain + move (§9 row 19; §6.4.4).
- **Purpose:** the provider-neutral memory contract — `MemoryService`, its operation shapes, document scope/path value types, prompt-write-safety vocabulary, and the shared conformance suite every provider wires.
- **Target contents** (2,512 lines total; has AGENTS.md + CLAUDE.md):
  - `src/service.rs` (948 ln) — the `MemoryService` trait + operation shapes.
  - `src/safety.rs` (497 ln) — prompt-write-safety vocabulary.
  - `src/test_support.rs` (355 ln, `#[cfg(any(test, feature = "test-support"))]`) — the provider conformance suite: scope isolation across tenant/user/agent/project.
  - `src/test_support.rs` also owns lane disjointness for dual-lane providers (F4 regression) and `record_interaction` round-trip visibility (F5 regression).
  - `src/path.rs` (302 ln) — document scope/path grammar.
  - `src/events.rs` (193 ln) — `MemorySignificantEvent`/audit vocabulary.
  - `src/metadata.rs` (76 ln), `src/context.rs` (70 ln), `src/hash.rs` (14 ln) — supporting vocabulary.
- **Migration delta:** pure move; no gains/sheds named.
  - Its `{host_api, prompt_envelope}` allowlist is retained verbatim, though confirmed: the crate does not use the `prompt_envelope` slot today (no such normal dep in its current `Cargo.toml`).
- **Owns:**
  - The `MemoryService` contract and its operation shapes.
  - Memory document scope/path grammar.
  - Prompt-write-safety vocabulary and the conformance suite.
- **Must never contain:**
  - A concrete backend (native/mem0 own that).
  - Embedding computation.
- **Allowed internal deps:** `ironclaw_host_api` (current `Cargo.toml`).
- **Forbidden deps:** everything else internal — a machine-enforced allowlist.
- **Public contracts & ports:**
  - `MemoryService` — the **justified 2-production-impl seam** (native + mem0), proven real by the shared conformance suite each provider wires.
  - `ironclaw.memory.*` tool-id constants, which live in this contract.
- **Consumers:** memory_native, memory_mem0 (implementers); host_runtime's `MemoryServiceResolver` and the first-party memory tools (readers/callers).
- **Security & authority role:** none directly.
- **Why a crate:** criteria 1+4 — one neutral contract, 2 production providers, a conformance suite that proves the seam.
- **Enforcement:** the conformance suite (kept + extended, §11.2.10); the `{host_api, prompt_envelope}` allowlist.

### `ironclaw_memory_native`

- **Path & disposition:** retain + move (§9 row 20; §6.4.5).
- **Purpose:** the native filesystem-backed `MemoryService` provider — the first of the two production implementations.
- **Target contents** (7,998 lines total; has AGENTS.md + CLAUDE.md):
  - `src/service.rs` (822 ln) — the `MemoryService` impl.
  - `src/backend.rs` (1,233 ln) — `MemoryBackend`/`MemoryBackendCapabilities`/`RepositoryMemoryBackend`.
  - `src/repo/filesystem.rs` (1,750 ln) + `src/repo/in_memory.rs` (179 ln) + `src/repo/mod.rs` (261 ln) — the repository trait and its two impls.
  - `src/filesystem.rs` (885 ln) — `MemoryBackendFilesystemAdapter`/`MemoryDocumentFilesystem`.
  - `src/indexer.rs` (711 ln) + `src/search.rs` (484 ln) — FTS indexing/search.
  - `src/safety.rs` (509 ln) — the prompt-write-safety *enforcement engine* (implements the vocabulary `ironclaw_memory::safety` defines).
  - `src/contract_tests.rs` (446 ln, gated) — wires `ironclaw_memory::test_support` against this backend.
  - `src/{path.rs (231), metadata.rs (106), chunking.rs (103), embedding.rs (112 — the dead port, see below), schema.rs (42), write_metadata.rs (31), events.rs (26)}` — supporting vocabulary and the deletion target.
- **Migration delta — deletes** (all verified live in source this session):
  - The dead `EmbeddingProvider` port (`src/embedding.rs`, 112 ln, confirmed 0 impls) — a near-verbatim duplicate of the separately-deleted `ironclaw_embeddings::EmbeddingProvider`.
  - Vector search silently degrades to FTS-only without this port — a behavior consequence of the deletion, not a fix in itself.
  - Its six path-preservation re-export shims (`lib.rs`'s dozen `pub use module::{…}` groups, six of which are pure path-preservation with no new vocabulary).
  - The unused `ironclaw_prompt_envelope` normal dependency (present in `Cargo.toml`, zero use sites confirmed) — drop it, or start using it.
- **Owns:**
  - Memory document path grammar.
  - The filesystem/in-memory repositories.
  - FTS indexing/search and prompt-write-safety enforcement.
- **Must never contain:**
  - The neutral `MemoryService` vocabulary (that's `ironclaw_memory`).
  - A second backend (mem0 is its sibling, not a mode of this crate).
  - Virtual-path/mount authority — one layer down, in `ironclaw_filesystem`; this crate owns only memory-specific path grammar built on top of it.
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_memory`, `ironclaw_safety` (allowlist `{host_api, filesystem, memory, prompt_envelope, safety}`).
- **Forbidden deps:** everything above substrates; no HTTP (that's mem0's cone).
- **Public contracts & ports:**
  - Implements `ironclaw_memory::MemoryService`.
  - Wires the shared conformance suite via `contract_tests.rs`.
- **Consumers:** the composition-layer memory provider factory (the sole production selector between this and mem0).
- **Security & authority role:** none directly — a record/search backend; the prompt-write-safety engine it hosts enforces vocabulary the kernel/loop tier consumes, but the enforcement call itself is not an authority grant.
- **Why a crate:** criterion 4 — the always-on, no-network half of the 2-production-impl seam; isolates FTS/indexing weight from the neutral contract's wide indirect consumer set.
- **Enforcement:** the memory provider conformance suite; a new dead-code ratchet once `EmbeddingProvider` is deleted (§11.2.9 dead-surface ratchet flip).
- **Open questions:** the compact entry (§6.4.4–6.4.6) flags restoring real vector search after the dead `EmbeddingProvider` port is deleted as "§12.10" — the enumerated §12.10 list itself does not carry a separate bullet for this; reproduced here exactly as the proposal's own cross-reference states it, without resolving it. Vector search stays FTS-only-degraded until decided.

### `ironclaw_memory_mem0`

- **Path & disposition:** retain + move (§9 row 21; §6.4.6).
- **Purpose:** the second `MemoryService` production implementation — a mem0-backed provider proving the memory layer genuinely swappable (issues #3537/#5264).
- **Target contents** (2,018 lines total; **no guidance file today** — confirmed, one of the audit's named guidance-less crates):
  - `src/service.rs` (1,256 ln) — `Mem0MemoryService`, mapping IronClaw memory ops onto mem0's `add`/`search`/`list` REST endpoints.
  - Non-clean mappings in `service.rs` are marked `MAPPING GAP` in source — 6 gaps per the audit.
  - `src/transport.rs` (369 ln) — `Mem0Transport` trait + `Mem0HttpTransport` (real `reqwest`, SSRF-checked, bounded timeout, redirects disabled).
  - `Mem0HttpTransport` optionally authenticates via bearer token, omitted for self-hosted OSS mem0 running `AUTH_DISABLED=true`.
  - `src/url_check.rs` (201 ln) — its own SSRF check, a duplicated-pattern with `ironclaw_llm::url_check` (497 ln) rather than a shared one.
  - `src/config.rs` (30 ln), `src/error.rs` (73 ln) — configuration and error vocabulary.
- **Migration delta:** pure move; **adds the missing guidance file** (explicit §6.4.4–6.4.6 fix).
  - Stays composition-only — a dedicated test pins that no other crate names "mem0."
  - Stays feature-gated — off by default; compiled in only with the `memory-mem0` feature; runtime fail-closed if misconfigured.
- **Owns:**
  - The mem0 REST mapping.
  - Its transport seam and its SSRF guard.
- **Must never contain:**
  - The neutral `MemoryService` vocabulary.
  - Filesystem-backend logic (native's job).
  - Any non-composition naming of "mem0."
- **Allowed internal deps:** `ironclaw_host_api`, `ironclaw_memory` — the narrowest of the three memory crates.
- **Forbidden deps:** everything else internal; HTTP is allowed here specifically as this crate's whole reason to exist.
- **Public contracts & ports:**
  - Implements `ironclaw_memory::MemoryService`.
  - A mock-transport seam keeps the mapping unit-testable without live network.
- **Consumers:** the composition-layer memory provider factory, exactly as with `memory_native` — the two are mutually exclusive at runtime, never composed together.
- **Security & authority role:** none directly; carries the SSRF-check obligation for its one HTTP egress path.
- **Why a crate:** criterion 6 — keeps mem0's `reqwest`/HTTP cone off-by-default and out of every other crate's build; the second impl that makes `ironclaw_memory`'s seam real.
- **Enforcement:** composition-only mem0-naming test (kept); feature-gate/fail-closed runtime check (kept).

### `ironclaw_skills`

- **Path & disposition:** retain-narrow + move, **layer reassignment `loops`→`substrates`** (§9 row 22; §6.4.7; §3 Adjustment D).
- **Purpose:** SKILL.md (YAML frontmatter + markdown prompt) parsing/validation/selection/management, plus pure learning — the extension mechanism for prompt-level agent behavior.
- **Target contents** (10,673 lines today; has AGENTS.md, missing CLAUDE.md):
  - `src/{types.rs (803), parser.rs (512), validation.rs (688)}` — the SKILL.md grammar and its v1→v2 migration path.
  - `src/v2.rs` (325 ln) — `V2SkillMetadata` and friends, kept live (v2 data shape, not dead surface).
  - `src/management.rs` (962 ln) + `src/management/install_bundle.rs` (428 ln) — filesystem-backed skill management and bundle installation.
  - `src/scoped_management.rs` (239 ln), `src/install_metadata.rs` (51 ln) — scoped install variants and install metadata.
  - `src/learning.rs` (356 ln) — the pure-learning half.
  - `src/selector.rs` (1,394 ln) — deterministic scoring (`prefilter_skills`), v1-only per its own lib.rs doc.
  - `registry`/`catalog` features are **default-on**; `v2`/`gating` are **unconditionally compiled** — all confirmed in `Cargo.toml`/`lib.rs`.
- **Migration delta:**
  - **Deletes** `src/registry.rs` (2,642 ln, feature `registry`, default-on) — zero external consumers found.
  - **Deletes** `src/catalog.rs` (815 ln, feature `catalog`, default-on) and `src/gating.rs` (203 ln, unconditional) — same finding, ~3,660 combined deletable lines, or an explicit revival naming a real consumer.
  - **Rewrites** the stale v1-era `lib.rs` module doc, which currently frames the crate around "V1 Agent (remove after migration)" for a v1 monolith already gone from the workspace.
  - **Gains** `SkillActivationObserver` (the crate's sole current trait) + its observed-event type, moved in from `ironclaw_first_party_extension_ports` (5,443 ln today).
- **Owns:**
  - SKILL.md grammar/validation.
  - Skill selection scoring.
  - Filesystem-backed skill management and the activation-observer contract.
- **Must never contain:**
  - WASM hook execution (loop/hooks).
  - Extension lifecycle (extensions family).
  - First-party tool invocation (packages/first_party).
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Forbidden deps:** kernel, loop, product, app — the layer reassignment to `substrates` legalizes today's actual (kernel/hosting-tier) consumer set instead of requiring a standing exception.
- **Public contracts & ports:**
  - `SkillInferencePort` — the intended inversion port (single-impl by design; real impl in the manager/composition adapter).
  - `SkillActivationObserver` after the gain above.
- **Consumers:** the hosting tier (host_runtime today, first_party package after §6.5.9's shed), product's activation projection.
- **Security & authority role:** none — a content/selection domain; skill *installation* authority lives in `extensions`, activation trust-ceiling lives in `trust`.
- **Why a crate:** contract + heavy parsing/selection logic with multiple hosting-tier call sites; the layer move legalizes reality rather than changing behavior.
- **Enforcement:** dead-surface ratchet after deletion (§11.2.9); family⇄layer consistency check now passes without an exception (§11.2.1).

### `ironclaw_auth`

- **Path & disposition:** retain-narrow + move (§9 row 23; §6.4.8).
- **Purpose:** product-facing authentication — typed auth-flow/secure-interaction/credential-account/recovery/provider-exchange/continuation/cleanup contracts, durable filesystem-backed services, and the recipe-driven `AuthEngine` (vendor differences are recipe data, never a code branch).
- **Target contents — two chartered top-level modules** (23,395 lines total incl. ~10,600 lines of in-crate tests; has AGENTS.md + CLAUDE.md), matching the crate's current organic split:
  - **`engine` half — flow machinery:** `src/engine/{mod.rs (527), keepalive.rs (646), exchange.rs (603), dcr.rs (411), http.rs (208)}`.
  - **`engine` half — value types:** `src/{oauth.rs (509), credential.rs (1,227), flow.rs (411), provider.rs (171), domain.rs (649)}`.
  - **`engine` half — supporting vocabulary:** `src/{scope.rs (96), ids.rs (194), account_state.rs (175), interaction.rs (94), cleanup.rs (107), error.rs (116)}`.
  - **`product_auth` half — API + durable:** `src/product_auth/api/auth.rs` (1,919 ln); `src/product_auth/durable/{mod.rs (976), flows.rs (864), interactions.rs (350), accounts.rs (296)}`.
  - **`product_auth` half — durable supporting:** `src/product_auth/durable/{cleanup.rs (173), paths.rs (138), provider.rs (31), domain.rs (12)}`.
  - **`product_auth` half — credentials/oauth:** `src/product_auth/credentials/{runtime_credentials.rs (585), product_auth_refresh_lock.rs (288), manual_token_flow.rs (279)}`; `src/product_auth/oauth/oauth_gate.rs` (738 ln).
  - `src/test_support/conformance.rs` (407 ln, gated) — downstream conformance harness.
- **Migration delta:**
  - **Deletes** `src/loopback_oauth.rs` (455 ln) — its own AGENTS.md already calls it legacy-only ("folded from `ironclaw_oauth` in W2.1 and deleted with v1"); its sole historical consumer no longer exists.
  - **Gates** `src/fakes.rs` (1,490 ln) behind the crate's existing `test-support` feature — confirmed live today as a plain unconditional `mod fakes;` in `lib.rs` (no `#[cfg(...)]`), so it ships in every release binary today.
  - **Drops** the direct `ironclaw_turns` normal dependency (confirmed present in current `Cargo.toml`) via a gate-prompt port defined in `host_api`/`extension_contracts`, dissolving the `auth → turns` **follow-up** exception (distinct from the seven W4.3 exceptions — §8.3's last row).
  - **Re-charters** the engine/product_auth split as two documented top-level modules instead of an organic one.
- **Owns:**
  - Durable product-auth flow/account/interaction/cleanup records.
  - The recipe-driven `AuthEngine`.
  - Redacted DTOs safe for WebUI/CLI/chat/API/projection rendering.
- **Must never contain** (per its own AGENTS.md, retained verbatim):
  - New V1 route handlers, V1 pending maps, V1 extension-manager authority, V1 `SecretsStore` access.
  - Raw HTTP clients, host-runtime credential-injection adapters, HTTP route serving, extension lifecycle mutation, turn replay/resume.
  - Raw OAuth codes/PKCE verifiers/tokens/host paths/raw secret values in any serializable shape.
- **Allowed internal deps:** `ironclaw_common`, `ironclaw_events`, `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_secrets`.
- **Forbidden deps:** almost everything else — already the family's most comprehensive blocklist.
- **Public contracts & ports:**
  - The flow/interaction/credential-account/recovery/exchange/continuation/cleanup trait set.
  - `AuthRecipeResolver` (implemented by `extension_host::recipes`).
  - `fakes.rs`'s in-memory services for downstream conformance testing, now feature-gated.
- **Consumers:** 8, per §6.4.8 — extension_host, webui, product, operator (session flows), and their respective test suites.
- **Security & authority role:** **credential-custody domain** — the family's one crate whose central job is holding token-lifecycle state (never raw secret bytes, which stay behind `SecretStore` handles).
- **Why a crate:** 8 consumers, an already-comprehensive boundary rule, and the recipe-driven design that is this crate's whole reason to exist — the family's second vendor-scoped exception.
- **Enforcement:** `reborn_crate_dependency_boundaries_hold`; a new ratchet pinning `mod fakes` to a `test-support`-gated declaration, closing the "ships in release" hygiene bug the same pattern used for `host-auth-mint` (§11.2.5) closes elsewhere.
- **Open questions (§12.10):** "the three-OAuth-stacks question (auth engine ∣ webui login ∣ llm provider sessions) — deliberate today, consolidation unscoped." `ironclaw_auth`'s engine is one vertex of that triangle.

### `ironclaw_attachments`

- **Path & disposition:** retain-widen + move (§9 row 24; §6.4.9).
- **Purpose:** the single channel-agnostic attachment-landing routine, **plus its ports** — ending a 3-crate accidental seam (ports in product, impls in composition, routine here).
- **Target contents** (1,002 lines total; zero traits today; **no guidance file** — confirmed, one of the audit's named guidance-less crates):
  - `src/landing.rs` (477 ln) — `AttachmentLanding`, `land_attachment`, `attachment_scoped_path`, `ATTACHMENTS_DIR`, `DEFAULT_MAX_ATTACHMENT_BYTES`.
  - `src/inbound.rs` (498 ln) — `InboundAttachment`, `land_inbound_attachments`.
  - `src/lib.rs` (27 ln) — crate root, currently the crate's whole guidance surface (module docs only, no external file).
- **Migration delta — gains** (origin labeled):
  - `InboundAttachmentLander`/`InboundAttachmentReader` port traits, moved in from `ironclaw_product` (their current home).
  - Their default implementations over `ScopedFilesystem`, moved in from `ironclaw_reborn_composition` (today's only impls).
  - The size-ceiling constants currently duplicated in three crates (webui/openai_compat import their own copies today) converge on this crate's single set.
  - **Adds the missing guidance file.**
- **Owns:**
  - The landing routine.
  - Its ports and their default filesystem-backed implementation.
  - The size-ceiling constants.
- **Must never contain:**
  - Channel-specific parsing (decoded into `InboundAttachment` before reaching this crate).
  - Delivery/outbound attachment handling.
- **Allowed internal deps:** `ironclaw_common`, `ironclaw_extractors`, `ironclaw_filesystem`, `ironclaw_host_api` — the `extractors` edge is the one intra-family dependency in this crate, for pure data transformation, not authority.
- **Forbidden deps:** product/composition — the point of the widen is that this crate no longer needs either.
- **Public contracts & ports:**
  - `InboundAttachmentLander`/`InboundAttachmentReader` — now single-sourced, port + default impl together.
  - Directly fixes one of the §9 anti-pattern inventory's named "accidental trait/DTO seams."
- **Consumers:** webui, openai_compat, telegram/slack channel packages via their shared inbound path — all 3 confirmed callers of the landing routine today.
- **Security & authority role:** writes through the project-scoped `ScopedFilesystem` authority — the same authority the agent's file tools resolve through — so a write still requires a `MountPermissions` write grant even though the crate makes no authorization decision itself.
- **Why a crate:** single-authority landing path with 3 consumers, now with its full contract in one place instead of three.
- **Enforcement:** dependency-boundary test extended to drop the old product/composition attachment edges once the move lands.

### `ironclaw_extractors`

- **Path & disposition:** retain + move (§9 row 25; §6.4.10).
- **Purpose:** pure MIME→text extraction with decompression-bomb safety caps — a leaf with no I/O and no knowledge of where bytes came from.
- **Target contents** (single file, `src/lib.rs`, 1,080 ln total; **no guidance file today**):
  - `extract_text(data: &[u8], mime: &str, filename: Option<&str>) -> Result<String, String>` (signature confirmed in source) — the one dispatch entry point.
  - Dispatches by normalized MIME to PDF, OOXML (docx/pptx/xlsx), legacy-Office, RTF, and UTF-8 text/code extractors internally.
  - `ExtractionError` and the ZIP bomb-safety caps: `MAX_DECOMPRESSED_ENTRY` = 50 MB per entry, `MAX_DECOMPRESSED_TOTAL` = 100 MB cumulative.
  - MIME normalization is delegated to `ironclaw_common::normalize_mime_type` rather than duplicated locally.
- **Migration delta:**
  - **Typed error across the boundary** — confirmed `extract_text` returns `Result<String, String>` today, losing `ExtractionError`'s structure at the public edge; target makes the public signature typed.
  - **Removes the caller-less `extract_text`** from the public surface if no external caller needs the umbrella dispatcher over a specific format extractor (a surface-hygiene finding, not a behavior change).
  - **Adds a guidance file.**
- **Owns:**
  - The MIME→text dispatch table and format-specific extractors.
  - The bomb-safety caps.
- **Must never contain:**
  - Attachment landing/storage (`ironclaw_attachments`, its sole consumer-shaped dependent).
  - Any async/I/O.
- **Allowed internal deps:** `ironclaw_common` only — the narrowest normal-dep set of any crate in this family besides `ironclaw_memory`.
- **Forbidden deps:** everything else internal; this crate exists specifically to keep heavy `pdf`/`zip` parsing out of every consumer's build.
- **Public contracts & ports:**
  - `extract_text` (typed after the fix) — a pure function contract, no traits.
- **Consumers:** `ironclaw_attachments` is the sole in-workspace consumer today.
- **Security & authority role:** none — a pure transform; the bomb-safety caps are hardening, not an authorization decision.
- **Why a crate:** pure leaf with heavy deps kept out of consumers — criterion 6, compile/dependency-cone isolation.
- **Enforcement:** standard boundary test; the new guidance file closes the audited gap.

### `ironclaw_projects`

- **Path & disposition:** retain-widen + move (§9 row 26; §6.4.11 — the W2 standing decision honored: not folded into another domain).
- **Purpose:** the Project entity, project membership/access-control records, and the `ProjectRepository` persistence contract that scopes threads/automations/workspace memory.
- **Target contents** (842 lines total; has CLAUDE.md, **missing AGENTS.md**):
  - `src/lib.rs` (424 ln) — `Project` entity, membership/ACL types, `ProjectRepository` trait.
  - Its own doc: "Authorization is **live** — `resolve_access` is called per request and never cached, so revoking a grant takes effect immediately."
  - `src/store.rs` (418 ln) — `FilesystemProjectRepository`, the sole current impl, persisting over `ScopedFilesystem` with no SQL in this crate.
- **Migration delta — gains** (origin labeled): the authorization-gating adapter, moved in from `ironclaw_reborn_composition::support::fs::project_service` (665 lines today, per audit A1).
  - This is the service half that actually performs project-scoped access gating for callers.
  - The corresponding *port* (what product calls) stays in `product_contracts` per the family-wide port-relocation rule (Adjustment A) — only the domain-owned *service implementation* moves here.
- **Owns:**
  - `Project` entity and membership/ACL records.
  - Live per-request access resolution.
  - The gained gating-service implementation.
- **Must never contain:**
  - The product-facing port declaration (contracts, not here).
  - Thread/automation/memory content itself — those domains own their own records; projects only scopes them via `project_id`.
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Forbidden deps:** everything ≥ kernel; product (the port lives in contracts).
- **Public contracts & ports:**
  - `ProjectRepository` (1 production impl today).
  - The gained implementation of the `product_contracts`-declared gating port.
- **Consumers:** product (via `ProjectService`, today an always-`Some` `Option<Arc<_>>` — an audited smell worth tightening once the gating service moves here), webui, composition.
- **Security & authority role:** **live** access-control authority for project scope — `resolve_access` is the enforcement point, called uncached on every request.
- **Why a crate:** a healthy, machine-enforced boundary with a real production path; the W2 decision already tested folding it into a bigger domain and chose standalone.
- **Enforcement:** standard dependency-boundary test; adds the missing AGENTS.md at the move.

### `ironclaw_identity` (renamed from `ironclaw_reborn_identity`)

- **Path & disposition:** `crates/ironclaw_reborn_identity/` → `crates/domains/ironclaw_identity/` — **rename** + move (§9 row 27; §6.4.12).
- **Purpose:** the canonical Reborn identity layer — maps every external identity (WebUI OAuth logins, external channel/product actors) to a stable `UserId`, and is the durable home of the minimal user profile, *before* any runtime state (conversation binding, thread ownership) is touched.
- **Target contents** (3,072 lines total; has `CONTRACT.md` only — a recognized guidance convention per §11.4's "AGENTS/CLAUDE/CONTRACT" list), grounded in current `CONTRACT.md` + source:
  - `src/identity_store.rs` (618 ln) — `RebornIdentityStore`, the sole implementer of both traits below.
  - `src/identity_store/{directory.rs (485), paths.rs (123), record.rs (84)}` — the admin-directory query surface, path construction, and record shapes.
  - Implements `RebornIdentityResolver` (`resolve_or_create`/`lookup`/`bind`/`adopt_migrated_identity`).
  - Implements the separate `RebornUserDirectory` admin trait (`list_users`/`get_user`/`create_user`/`update_profile`/`update_status`/`update_role`/`record_last_login`/`delete_user`/`count_active_admins`).
  - `src/key.rs` (164 ln) — the canonical key `(tenant_id, surface_kind, provider_kind, provider_instance_id, external_subject_id)`.
  - `src/user_directory.rs` (166 ln) — directory-surface support.
- **Migration delta:**
  - **Gains** `host_api::user_identity`'s store ports (160 ln today — `RebornIdentityProviderId`, `RebornUserIdentityBindingStore`, confirmed live in source) — persistence ports don't belong in the neutral vocabulary crate.
  - This directly targets the audited "two parallel identity-binding stores" ambiguity (`host_api::RebornUserIdentityBindingStore` vs. this crate's own resolver); the crate's own `CONTRACT.md` already tracks the resolution as issue #5618.
  - **Trims** the three zero-production-caller resolver methods (`lookup`, `bind`, `adopt_migrated_identity` — confirmed present as trait methods, called only by tests beyond `resolve_or_create`'s own internal use) per issue #5618, unless a real consumer is named first.
- **Owns:**
  - The external-identity→`UserId` resolver.
  - The `StoredUser`/`StoredExternalIdentity`/`StoredVerifiedEmailIndex` record shapes persisted under `/tenant-shared/reborn-identity`.
  - The admin user directory.
- **Must never contain:**
  - Conversation binding (`ironclaw_conversations` consumes an already-resolved `UserId`).
  - WebUI ingress logic.
- **Allowed internal deps:** `ironclaw_host_api`, `ironclaw_filesystem` — **the tightest allowlist in the family**, machine-enforced.
  - `reborn_crate_dependency_boundaries_hold` allows exactly these two edges; its own `CONTRACT.md` states it "must never reach upstream" (composition, product).
- **Forbidden deps:** everything else, absolutely.
- **Public contracts & ports:**
  - `RebornIdentityResolver` (mint-capable `resolve_or_create`; link-only `lookup`/`bind`/`adopt_migrated_identity`).
  - `RebornUserDirectory` — deliberately a separate trait from the resolver, so admin CRUD cannot perturb the mint/link/create invariants; both implemented by one `RebornIdentityStore`.
- **Consumers:** its only external consumer today is composition, which re-exports a curated subset via its own service (per `CONTRACT.md`).
- **Security & authority role:** the sole authority deciding when a new `UserId` is minted — `SurfaceKind::ChannelActor` is explicitly rejected as non-mintable (channel actors must fail closed, never auto-provision).
  - Verified-email linking is restricted to the OAuth surface specifically to stop a channel actor asserting a verified email from colliding with an OAuth user's index (its `CONTRACT.md` invariant 1).
- **Why a crate:** bottom-of-stack identity authority with a machine-enforced never-reach-upstream rule.
- **Enforcement:** `reborn_crate_dependency_boundaries_hold`'s two-edge allowlist (kept verbatim through the rename); no compatibility `pub use` shim at the rename per the type-placement house rule (§11.3) — consumers repoint in the same change.
- **Open questions (§12.10):** "`identity`'s dual binding-store (host_api `RebornUserIdentityBindingStore` vs identity's resolver) — one must become canonical (issue #5618)."
  - Also: the `reborn_` rename batch this crate belongs to (composition/config/cli-dir/openai_compat/event_store/**identity**/traces) is explicitly "**recommended-but-severable**: every other change in this proposal works with old names" — every gain/trim above lands under the current name if the rename itself is deferred.

### `ironclaw_llm`

- **Path & disposition:** retain-narrow + move (§9 row 28; §6.4.13).
- **Purpose:** the `LlmProvider` contract, its concrete providers, the provider registry, reliability decorators (retry/circuit-breaker/failover), and trace recording — the family's other explicitly vendor-scoped domain.
- **Target contents, by the module list's five sub-owner clusters** (49,060 lines total incl. `reasoning.rs`; has AGENTS.md + CLAUDE.md; confirmed via `lib.rs` mod declarations):
  - **providers:** `{nearai_chat.rs (4,226), rig_adapter.rs (4,356), codex_chatgpt.rs (2,283)}`.
  - **providers continued:** `{openai_codex_provider.rs (1,590) + openai_codex_session.rs (769), bedrock.rs (1,556, `#[cfg(feature = "bedrock")]`), github_copilot.rs (787), provider.rs (1,419)}`.
  - **auth-sessions:** `{gemini_oauth.rs (3,088), anthropic_oauth.rs (1,029), github_copilot_auth.rs (747), codex_auth.rs (377), auth.rs (336), session.rs (874)}`.
  - **registry/decorators:** `{registry.rs (1,308), failover.rs (1,345), retry.rs (1,200), circuit_breaker.rs (1,038)}`.
  - **registry/decorators continued:** `{smart_routing.rs (1,856), resolution.rs (959, `#[cfg(feature = "registry-provider-factory")]`)}`.
  - **recording:** `{recording.rs (2,610), trace_binding.rs (405), response_cache.rs (798)}`.
  - **supporting:** `{tool_schema.rs (626) + tool_schema/placeholder_stripping.rs (450), config.rs (839), runtime.rs (723), models.rs (470), url_check.rs (497 — its own SSRF check), testing/ (gated)}`.
  - `lib.rs` (1,825 ln) — carries a live `arch-exempt: large_file` marker citing plan #6175 for a future split.
- **Migration delta:**
  - **Deletes** `src/reasoning.rs` (4,503 ln — confirmed present, re-exported, zero external references; `SUPERSEDED` v1-engine remnant per §2.6).
  - **Gains** `llm_costs.rs`, `provider_transcript.rs`, `model_selection.rs` from `ironclaw_common` (confirmed present in `common` today at 383/155/51 lines).
  - **Fixes** `providers.json` — today an `include_str!` reaching two directory levels above the crate root at the repo root; becomes a crate asset or composition-supplied data, closing under the new cross-crate `include_str!`/`include_bytes!` scan (§11.2.7).
  - Rewrites stale v1 guidance; **adds its own boundary rule** (today only its consumers are ruled, not its own outbound edges).
- **Owns:**
  - `LlmProvider` contract and all concrete provider adapters.
  - The registry and reliability decorators.
  - Cost/transcript/model-selection vocabulary (post-gain), recording/trace-binding.
- **Must never contain:**
  - Product prompt content.
  - Turn orchestration.
- **Allowed internal deps:** `ironclaw_common`, `ironclaw_safety`.
- **Forbidden deps:** everything ≥ kernel; the vendor cone is one of the few places outside `packages/*`/`operator`/webui-login/recipes-as-data where vendor SDKs are sanctioned (§8.1 rule 4).
- **Public contracts & ports:**
  - `LlmProvider` — the family's other named vendor exception besides `auth`'s recipes.
- **Consumers:** 8, per §6.4.13 — product, runner/loop_host (model gateway), traces (recording reuse), operator (admin), openai_compat.
- **Security & authority role:** none directly — provider selection/credentials/session refresh are its job, but authorization to *call* a model at all is a kernel decision made before dispatch reaches here.
- **Why a crate:** provider cone isolation (vendor SDKs, OAuth flows for Anthropic/Gemini/GitHub Copilot/Codex, AWS Bedrock SDK) kept out of every non-LLM consumer's build.
- **Enforcement:** dead-code ratchet after `reasoning.rs` deletion; the new outbound boundary rule (§11.2); the `include_str!`/`include_bytes!` scan closing the `providers.json` reach-in.
- **Open questions (§12.10):** "the three-OAuth-stacks question (auth engine ∣ webui login ∣ llm provider sessions) — deliberate today, consolidation unscoped." LLM provider sessions is this crate's vertex of that triangle.

### `ironclaw_traces` (renamed from `ironclaw_reborn_traces`)

- **Path & disposition:** `crates/ironclaw_reborn_traces/` → `crates/domains/ironclaw_traces/` — **rename** + move + **restructure** (§9 row 29; §6.4.14).
- **Purpose:** the Trace Commons / TraceDAO client — envelope schema, deterministic redaction, submission queue/holds/telemetry, credits, device-key onboarding.
- **Target contents** (20,808 lines total; **no guidance file today**):
  - `src/contribution.rs` (**17,467 ln — 84% of the crate today, a single undifferentiated file**) covering schema, redaction application, queue, holds, and credits.
  - `src/client.rs` (564 ln) — the host-facing trace client.
  - `src/redaction.rs` (251 ln) — deterministic redaction helpers.
  - `src/onboarding.rs`/`mod.rs` (504 ln) — device-key onboarding entry point.
  - `src/onboarding/{device_key.rs (463), invite.rs (451), protocol.rs (135)}` — onboarding sub-modules.
  - `src/conversation_message.rs` (13 ln) — `ConversationMessage`, re-exported for legacy `history`-module compat.
  - **Two boundary-laundering re-export modules, confirmed live in source:** `src/recording` (`pub mod recording { pub use ironclaw_llm::recording::*; }`).
  - `src/paths` (`pub use ironclaw_common::paths::*;`) — both exist solely so `reborn-cli` avoids a direct `ironclaw_llm`/`ironclaw_common` dependency.
- **Migration delta:**
  - **Splits** `contribution.rs`'s 17,467 lines into chartered modules — schema / redaction / queue / credits / credentials — five owners where there is one file today.
  - **Takes a `ScopedFilesystem`** instead of raw `dirs`/env access for storage-path resolution (today's one bypass of the filesystem authority every other domain crate routes through).
  - **Drops** the `recording`/`paths` re-export-laundering modules — once the rename removes `reborn-cli`'s reason for the indirection, consumers import `ironclaw_llm`/`ironclaw_common` directly.
  - **Adds guidance files.**
  - **Adjacent, sequence-coupled but not a move of this crate:** the `trace_commons` model-callable tool (1,867 lines today in `ironclaw_host_runtime::first_party_tools`) moves to the first-party package per §6.8.4.
- **Owns:**
  - The trace envelope schema and redaction pipeline.
  - Submission queue/holds/telemetry and credits.
  - Device-key onboarding.
- **Must never contain:**
  - The model-callable tool invocation itself (packages/first_party, after the adjacent shed completes).
  - Raw `dirs`/env filesystem access (fixed by the `ScopedFilesystem` migration).
- **Allowed internal deps:** `ironclaw_common`, `ironclaw_host_api`, `ironclaw_llm`, `ironclaw_safety` — the `llm` edge is real (trace recording reuses `ironclaw_llm::recording`) and survives the re-export cleanup as a direct dependency instead of a laundered one.
- **Forbidden deps:** everything ≥ kernel; HTTP is allowed here specifically (Trace Commons submission) as the family's vendor/external-service exception.
- **Public contracts & ports:**
  - The trace client and the redaction pipeline.
  - No multi-impl traits today — a single external-service integration, chartered internally after the split.
- **Consumers:** the `trace_commons` tool (once moved to first_party), CLI's `traces preview` command, composition's trace-capture wiring.
- **Security & authority role:** **security-critical redaction obligation** — deterministic redaction of sensitive JSON before submission is this crate's central promise; the family role names this crate by exactly that phrase.
- **Why a crate:** a distinct external-service domain with a security-critical redaction obligation, an isolated HTTP/submission cone.
- **Enforcement:** the new guidance file; the cross-crate re-export ban closing the `recording`/`paths` laundering; the file-size architecture budget (`contribution.rs` is well over the 1,500-line `arch-exempt` threshold today, uncovered by any exemption).
- **Open questions (§12.10):** the `reborn_` rename batch this crate belongs to (composition/config/cli-dir/openai_compat/event_store/identity/**traces**) is explicitly "recommended-but-severable" — every split/gain/fix above lands under the current name if the rename itself is deferred.

### `ironclaw_outbound`

- **Path & disposition:** retain + move (§9 row 30; §6.4.15).
- **Purpose:** metadata-only outbound policy/state — notification opt-in, sealed claim→grant trust types, subscription cursors, at-most-once delivery-attempt reservation, and the resolution engine. Never a transport itself.
- **Target contents** (8,547 lines total; has AGENTS.md + CLAUDE.md):
  - `src/outbound_state_store.rs` (3,109 ln) — `OutboundStateStorePort` (a 20-method fat port, one impl, store+policy conflated — flagged as module-charter work, **not** a split).
  - `OutboundStateStore` in the same file implements 4 traits — the port, plus 3 others sharing its storage.
  - `src/resolution_engine.rs` (1,208 ln) — `OutboundResolutionEngine`.
  - `src/{delivered_gate_routes.rs (730), delivery_targets.rs (632), delivery_resolution.rs (531)}` — delivery routing/resolution.
  - `src/service.rs` (493 ln) — the `OutboundPolicyService` that mints the sealed types.
  - `src/types.rs` (357 ln) — `ThreadProjectionAccessGrant`/`ThreadProjectionAccessClaim`, `ValidatedReplyTargetBinding`/`ReplyTargetBindingClaim` (confirmed sealed via `pub(crate)` fields + explicit "Sealed against external construction" doc comments).
  - `src/{communication_preferences.rs (290), validation.rs (239), store.rs (232), ids.rs (172), triggered_run_delivery.rs (165)}` — supporting vocabulary and stores.
  - `src/run_final_reply_target.rs` (106 ln — `RouteCurrentRunFinalReply`, dead, see below), `src/{run_delivery_cleanup.rs (84), run_final_reply_handoff.rs (21)}`.
- **Migration delta:** **deletes** `RouteCurrentRunFinalReply` (`src/run_final_reply_target.rs:75` — confirmed 0 implementations by source inspection this session, matching §2.6's "0 impls" finding) along with its unused request/error types.
  - The 20-method `OutboundStateStorePort` is explicitly scoped as module-charter follow-up, not a crate split.
- **Owns:**
  - Notification opt-in policy.
  - Sealed claim→grant trust types and subscription cursors.
  - The CAS `Prepared→Sending` at-most-once delivery-attempt state (confirmed in `claim_delivery_attempt_for_send`).
  - The resolution engine.
- **Must never contain:**
  - Any transport send (verified — no HTTP client dependency in the crate).
  - Projection mutation (`event_projections` owns that; outbound only reads via `push_candidates_for_update`).
- **Allowed internal deps:** `ironclaw_event_projections`, `ironclaw_filesystem`, `ironclaw_host_api`, turn vocabulary via `host_api::turn` (sheds the direct `ironclaw_turns` dep visible in today's `Cargo.toml`, dissolving its W4.3 exception).
- **Forbidden deps:** any HTTP/transport client; kernel/loop/product/app.
- **Public contracts & ports:**
  - `OutboundStateStorePort`.
  - The sealed `ThreadProjectionAccessGrant`/`ValidatedReplyTargetBinding` types, constructible only through `OutboundPolicyService`.
- **Consumers:** product's `DeliveryCoordinator`, extension_host's egress transports, event_streams (the `push_candidates_for_update` read edge only).
- **Security & authority role:** **authority** — sole writer of delivery-attempt state and the sealed-grant minting point; alongside `triggers`, one of the two domains-family crates that mints/seals trust rather than only recording it.
  - Watch-authorization (`event_streams`) and push-authorization (here) are kept as separate decisions by design (§7, T8).
- **Why a crate:** distinct durable authority consumed by product/extension_host/streams; the sealed-type pattern is exactly what lets this stay a domain crate instead of migrating into `kernel/`.
- **Enforcement:** the outbound metadata-only verification (no HTTP deps — an ingress-scan-enforced property); dead-code ratchet after the `RouteCurrentRunFinalReply` deletion.

## Family AGENTS.md obligations

Per §11.4, `crates/domains/AGENTS.md` restates the §6.4 family contract verbatim: family role, what belongs/does-not, the allowed layer range, the family's named ports, and the "before adding a crate here" gate.

- **Allowed layer range:** `substrates` only — a domains-family crate declaring any other layer fails the new family⇄layer consistency check (§11.2.1). This is how `ironclaw_skills`' `loops`→`substrates` reassignment gets legalized rather than left as a standing exception.
- **Named ports and sealed types to restate verbatim:** `MemoryService`'s 2-impl seam (`ironclaw_memory` + its conformance suite); `ironclaw_outbound`'s sealed `ThreadProjectionAccessGrant`/`ValidatedReplyTargetBinding`; `ironclaw_triggers`' `TriggerTrustedInboundBinding` trusted-submit binding.
- **"Before adding a crate here" gate:** does this really own a distinct record grammar with real consumers, or is it a module of an existing domain? The family's own crate count (15, mostly single-impl, mostly 2–8 consumers) is the evidence that narrowness — not consolidation — is the house style here.
- **The `storage-placement.md` hybrid rule, restated verbatim:** "file-shaped → `RootFilesystem`; structured control-plane → typed repos owned by service domain; derived → owning projection layer."
- **The standing "domain stores never branch on backend" rule**, plus the persistence-idiom rule with its shrink-only exception set: `ScopedFilesystem` is the floor; hand-written SQL requires an ADR; today `{triggers, hooks}` are the only crates in the allowlist (`hooks` being the `loop/`-family sibling exception, not itself a domains crate), and §11.2.6 pins the set so it cannot silently grow.

**Guidance-file catch-up** (verified this session). Three of these are the domains-family instances of the audit's explicitly named "six guidance-less crates" (§11.4), and one is the audit's explicitly named missing-CLAUDE gap:

- `ironclaw_memory_mem0` — no guidance file today; gains one at the move.
- `ironclaw_attachments` — no guidance file today; gains one at the move.
- `ironclaw_traces` — no guidance file today; gains one at the move (and restructure).
- `ironclaw_triggers` — has AGENTS.md, **missing CLAUDE.md**, explicitly named in the same §11.4 sentence; gains one at the move.

Independently confirmed this session, as supporting (not proposal-asserted) evidence of the same discipline gap:

- `ironclaw_extractors` has no guidance file (§6.4.10 itself calls for "add a guidance file").
- `ironclaw_skills` has AGENTS.md but no CLAUDE.md.
- `ironclaw_projects` has CLAUDE.md but no AGENTS.md.
- `ironclaw_identity` (renamed) has only `CONTRACT.md`, which already satisfies the recognized AGENTS/CLAUDE/CONTRACT convention.

Crate-local guides keep that same convention going forward — no new format is introduced by this family. Renamed crates (`ironclaw_identity`, `ironclaw_traces`) ship with **no** `pub use` compatibility shim at the rename, per the type-placement house rule (§11.3): consumers are repointed in the same change that performs the rename.

## Current → target summary

| # | Current crate | Target path | Disposition | Notes (PROPOSAL.md citation) |
|---|---|---|---|---|
| 16 | `ironclaw_threads` | `domains/ironclaw_threads` | retain + move | §6.4.1 |
| 17 | `ironclaw_conversations` | `domains/ironclaw_conversations` | retain + move, internal renames | §6.4.2; `SessionThreadService` collision fix mandatory; DTO unification with host_api |
| 18 | `ironclaw_triggers` | `domains/ironclaw_triggers` | retain + move | §6.4.3; persistence-idiom ADR-or-converge |
| 19 | `ironclaw_memory` | `domains/ironclaw_memory` | retain + move | §6.4.4; justified seam (2 production providers + conformance suite) |
| 20 | `ironclaw_memory_native` | `domains/ironclaw_memory_native` | retain + move | delete dead embedding port + 6 path-shims; drop unused prompt_envelope dep |
| 21 | `ironclaw_memory_mem0` | `domains/ironclaw_memory_mem0` | retain + move | add guidance files; stays composition-only + feature-gated |
| 22 | `ironclaw_skills` | `domains/ironclaw_skills` (layer loops→substrates) | retain-narrow + move | §6.4.7; delete ~4k dead lines; absorb activation-observer vocab |
| 23 | `ironclaw_auth` | `domains/ironclaw_auth` | retain-narrow + move | §6.4.8; delete loopback_oauth; gate fakes.rs; drop turns dep via port |
| 24 | `ironclaw_attachments` | `domains/ironclaw_attachments` | retain-widen + move | §6.4.9; absorbs its product ports + composition impls |
| 25 | `ironclaw_extractors` | `domains/ironclaw_extractors` | retain + move | §6.4.10; typed error; guidance file |
| 26 | `ironclaw_projects` | `domains/ironclaw_projects` | retain-widen + move | §6.4.11; absorbs its composition service adapter (W2 decision honored) |
| 27 | `ironclaw_reborn_identity` | `domains/ironclaw_identity` | **rename** + move | §6.4.12; absorbs host_api user-identity store ports; resolves dual-binding-store ambiguity |
| 28 | `ironclaw_llm` | `domains/ironclaw_llm` | retain-narrow + move | §6.4.13; delete reasoning.rs; fix providers.json reach; add boundary rule |
| 29 | `ironclaw_reborn_traces` | `domains/ironclaw_traces` | **rename** + move + restructure | §6.4.14; split 17.5k-line file; drop re-export laundering; ScopedFilesystem |
| 30 | `ironclaw_outbound` | `domains/ironclaw_outbound` | retain + move | §6.4.15; delete 0-impl trait |

All 15 rows are PROPOSAL.md §9 rows 16–30 exactly; none carry a `[#6696]` contingency tag (contrast `kernel/`'s `approvals`/`processes`/`run_state` and `loop/`'s `runner`, all of which do).
