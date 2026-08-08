# `crates/domains/` — one crate per business-record grammar, no authority decisions

**Layer(s):** `substrates` (all 12 manifests declare it; checked by
`reborn_workspace_crates_declare_layers_and_follow_layer_matrix`) ·
**Crates:** 12 · **May depend on:** `contracts/`, `substrates/`, `events/`,
plus five inventoried in-family edges (below) · **Depended on by:** `kernel/`,
`loop/`, `extensions/`, `product/`, `app/` — every tier above wires against
these contracts.

## What this family is

Each crate here owns exactly one subject's durable records — its schema, its
invariants (uniqueness, idempotency, compare-and-swap ordering, scope
isolation), and the typed service contract over `ScopedFilesystem` that every
caller uses. A domains crate answers *"what does this record mean and how does
it stay correct"*, never *"is this caller allowed to do this"* — authorization,
approval, and resource decisions are exhausted by the kernel before any call
lands here. Two crates are deliberately vendor-scoped (`ironclaw_llm`
providers, `ironclaw_auth` recipes-as-data) and two mint narrow trust
(`ironclaw_outbound` sealed grants, `ironclaw_triggers` trusted-fire minting);
no other crate in the family may acquire either property.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_attachments`](./ironclaw_attachments) | Channel-agnostic inbound attachment landing: the landing routine, the `InboundAttachmentLander`/`InboundAttachmentReader` ports with their filesystem default, and the shared size ceilings | Landing inbound file bytes for the agent, or reading advertised attachment budgets |
| [`ironclaw_auth`](./ironclaw_auth) | Product-facing auth: flow/interaction/credential-account/recovery/cleanup contracts, durable services, and the recipe-driven `AuthEngine` — credential custody without raw secret bytes | Adding or changing a product auth flow, credential-account behavior, or an OAuth recipe interaction |
| [`ironclaw_conversations`](./ironclaw_conversations) | External↔canonical conversation *binding*, actor pairing, and inbound idempotency; turn submission only via the `ConversationTurnSubmitter` port | Binding an external conversation/actor to a canonical thread, or accepting an inbound message exactly once |
| [`ironclaw_extractors`](./ironclaw_extractors) | Pure bytes→text extraction (PDF/OOXML/legacy Office/RTF/text), bomb-capped, with a content-free failure `Display` | Turning file bytes into text, with no I/O and no knowledge of the source |
| [`ironclaw_identity`](./ironclaw_identity) | External identity → stable `UserId` minting, the user profile/directory, and the `projects` records + access-gating service | Resolving who a login or actor is, admin user management, or project membership/ACL |
| [`ironclaw_llm`](./ironclaw_llm) | The `LlmProvider` contract, one adapter per model vendor, provider auth/sessions, registry, reliability decorators, recording — the family's vendor cone | Adding a model provider, changing provider selection/reliability, or trace recording of model calls |
| [`ironclaw_memory`](./ironclaw_memory) | The provider-neutral `MemoryService` contract, memory path/scope grammar, prompt-write-safety vocabulary, and the shared conformance suite | Changing what memory *means*; provider implementations live in `extensions/packages/memory-native` and `…/mem0` |
| [`ironclaw_outbound`](./ironclaw_outbound) | Metadata-only outbound authority: sealed access grants, at-most-once delivery-attempt reservation, delivery resolution, preferences and subscription cursors — never a transport | Deciding *whether/where* something may be pushed, or recording a delivery attempt |
| [`ironclaw_skills`](./ironclaw_skills) | Skill parsing, validation, deterministic selection scoring, scoped filesystem management, and the pure learning path | Changing skill grammar, selection, install records, or learning prompts |
| [`ironclaw_threads`](./ironclaw_threads) | The canonical transcript service: `SessionThreadService` (filesystem + in-memory), message ordering/status/redaction, tool-result records, display projections | Reading or writing thread/message history or transcript-derived views |
| [`ironclaw_trace_commons`](./ironclaw_trace_commons) | The Trace Commons client: envelope schema, deterministic redaction, submission queue/credits, device-key onboarding, and the autonomous capture pipeline | Contributing traces to the external Trace Commons service |
| [`ironclaw_triggers`](./ironclaw_triggers) | Scheduled-trigger records, cron/timezone validation, deterministic fire identity, the poller tick, and sealed trusted-submission minting (prompt-scanned at the mint); SQL backends held under ADR 0003 | Trigger records/schedules, or anything on the host-trusted fire path |

Two boundary facts that have been gotten wrong before, stated precisely:

- **`ironclaw_conversations` owns the *binding*, not the transcript.**
  `ironclaw_threads` owns canonical threads and all message content;
  conversations owns the mapping from an external conversation/actor to one of
  them plus inbound idempotency. The two crates may not even share a type name
  (`conversations_and_threads_declare_no_name_in_common`).
- **`ironclaw_identity` holds the `projects` records** (the former
  `ironclaw_projects` crate merged in, 2026-08-05) *and* the project
  access-gating service (`projects::service::RebornProjectService`,
  implementing the `ironclaw_product_contracts` port). The project *browse
  reader* stays in composition and the project-create capability stays above
  the substrate tier by design — both name crates identity's armed allowlist
  refuses (see `ironclaw_identity/CONTRACT.md`).

## What never belongs here

- **Backend selection.** A domains crate never branches on PostgreSQL, libSQL,
  or local-disk — composition chooses backends and mounts
  (`.claude/rules/database.md`; `ScopedFilesystem` is the floor). A
  hand-written SQL backend requires its own ADR; `ironclaw_triggers` under
  [ADR 0003](../../docs/internal/adr/0003-triggers-keeps-hand-written-sql.md) is the
  family's only such exception, and it still takes admission from
  `ironclaw_libsql_runtime` rather than owning connections.
- **Authority decisions.** Authorization, approvals, trust ceilings, resource
  reservation → `kernel/`. No crate here can construct a kernel-sealed witness
  or lease; the two narrow mints (outbound's sealed types, triggers'
  trusted-submission binding) are the chartered exceptions and are pinned by
  the gates below.
- **Product orchestration.** Channels, commands, views, cross-domain workflow
  → `product/`. A domains crate knows nothing about a WebUI handler or a Slack
  payload.
- **Transport and framework code.** No Axum, no transport sends
  (`ironclaw_outbound` is metadata-only). HTTP appears only inside the three
  chartered external-service cones: `ironclaw_llm`, `ironclaw_trace_commons`,
  and `ironclaw_auth`'s engine.
- **Vendor names or vendor branches** outside `ironclaw_llm` and
  `ironclaw_auth`'s recipe data → `reborn_extension_specificity.rs` fails the
  build (its only domains carve-outs are llm's provider files and
  trace_commons' redaction *denylists*).
- **Another domain's records.** Transcript content stays out of conversations
  and outbound; binding logic stays out of threads; memory provider backends
  stay out of `ironclaw_memory` (they are extension packages); attachment
  payload *parsing* stays in the channel adapters.
- **Install/activate/package lifecycle** → `extensions/`. A domains crate is
  available to every caller with no notion of being installed.
- **Reaching upward.** No dependency on `kernel/`, `loop/`, `extensions/`,
  `product/`, or `app/`. Where a domains crate needs something only an upper
  tier can do, it declares a port and the upper tier implements it
  (`ConversationTurnSubmitter` ← composition, `SkillInferencePort` ← loop
  host, `AuthRecipeResolver` ← extension host).

## The rules, and what enforces them

All gates run inside `cargo test -p ironclaw_architecture_tests` unless a
crate path is given.

- **Layer + dependency direction:**
  `reborn_workspace_crates_declare_layers_and_follow_layer_matrix` (every
  manifest's `[package.metadata.ironclaw] layer`, normal deps only) and
  `reborn_crate_dependency_boundaries_hold`, which carries per-crate
  `BoundaryRule` forbidden lists for `ironclaw_auth`, `ironclaw_conversations`
  (incl. the pinned *absence* of `ironclaw_safety` and `ironclaw_threads`),
  `ironclaw_outbound`, `ironclaw_threads`, `ironclaw_triggers` (incl. the
  pinned absence of `ironclaw_filesystem`), and armed **allowlists** for
  `ironclaw_identity` (`{host_api, filesystem, product_contracts}`) and
  `ironclaw_memory` (`{host_api, prompt_envelope}`).
- **In-family edges are a closed, inventoried set** — exactly five today:
  `conversations→triggers`, `attachments→extractors`, `attachments→threads`,
  `outbound→attachments`, `trace_commons→llm`.
  `reborn_every_same_layer_edge_is_inventoried_and_no_entry_is_stale` fails on
  a sixth (and on a stale entry), and
  `reborn_same_layer_edge_inventory_ratchets_down_only` stops regrowth.
- **Naming and single-home rules:**
  `reborn_conversations_threads_attachments.rs` —
  `conversations_and_threads_declare_no_name_in_common`,
  `the_external_ref_pair_is_declared_only_by_the_extension_contracts_crate`,
  `the_attachment_ports_and_size_ceilings_live_in_the_attachments_crate` (the
  same test pins `ProjectScopedAttachmentReader` in product and forbids
  attachments from acquiring a `loop_host` dep — #7010).
- **Trusted-trigger ingress ownership:**
  `untrusted_ingress_paths_cannot_submit_host_trusted_inbound` and
  `conversation_trusted_trigger_submitter_stays_conversation_or_composition_owned`
  (both in `reborn_dependency_boundaries.rs`); the prompt scan itself is an
  invariant of the sealed mint, proven at the caller tier by
  `tick_rejects_injection_prompt_before_any_trusted_submitter_is_reached`
  (`cargo test -p ironclaw_triggers`).
- **Memory providers stay out:**
  `only_the_sanctioned_residue_names_a_memory_provider` (shrink-only residue
  ledger; composition consumes the contract, the binary links providers).
- **Persistence idiom:** `reborn_persistence_driver_boundary.rs` pins which
  crates may name SQL drivers — `ironclaw_triggers` is the family's tagged
  ADR-held exception.
- **Module charters are contracts, not comments:** `cargo test -p
  ironclaw_auth --test module_charter` and `cargo test -p ironclaw_llm --test
  module_charter` enforce the sub-owner maps in auth's `AGENTS.md` and llm's
  `CONTRACT.md` respectively (every `src/**/*.rs` file has exactly one owner;
  auth's two engines must not name each other).

**Gate for adding a crate here:** it must own a genuinely distinct record
grammar with independent consumers — otherwise it is a module inside an
existing domain (`projects` folding into `ironclaw_identity` is the
precedent). It lands with a `README.md`, a row in this table, a
`[package.metadata.ironclaw] layer` entry (`scripts/ci/check-target-tree.py`
enforces the tree half), and — if it wants SQL, a vendor name, or a trust
mint — an argument nobody has successfully made since the current exceptions
were chartered.

## Crossing out of this family

- **Up to `kernel/`** — when a call must be *authorized*, approved, or
  resourced; domains code runs strictly after those decisions.
- **Up to `product/`** — when several domains must be orchestrated in one
  user-facing workflow, or a view must be rendered.
- **Up to `extensions/`** — when something is installable: manifests,
  lifecycle, vendor packages, memory providers.
- **Down to `substrates/`** — `ironclaw_filesystem` for persistence
  (`ScopedFilesystem`, `cas_update`), `ironclaw_secrets` (auth only),
  `ironclaw_safety` (threads, triggers, llm, trace_commons),
  `ironclaw_libsql_runtime` (triggers only, under its ADR).
- **Down to `contracts/`** — shared vocabulary (`ironclaw_host_api`,
  `ironclaw_common`, `ironclaw_extension_contracts`,
  `ironclaw_product_contracts`); never re-declare a contract type here.
- **Sideways to `events/`** — `ironclaw_auth`→`event_log` and
  `ironclaw_outbound`→`event_projections` read evidence and derived views;
  projections are never mutated from here.

## Sources

- Family spec: [`docs/reborn/target-architecture/families/domains.md`](../../docs/reborn/target-architecture/families/domains.md)
  (design record; where it and the tree disagree, the code and its gates win —
  see each crate's README for measured deltas).
- PROPOSAL entries: `docs/reborn/target-architecture/PROPOSAL.md` §6.4.1–§6.4.15
  (§5 tree, §8 layer matrix, §12.13 D-P/D-Q for the projects gating fold).
- Persistence rule: [`.claude/rules/database.md`](../../.claude/rules/database.md).
- Conventions this file follows: [`docs/reborn/guidance-conventions.md`](../../docs/reborn/guidance-conventions.md).
