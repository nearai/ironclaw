# How Memory Works in Reborn IronClaw

_Status: current-state explainer (2026-06-16). Describes the Reborn memory subsystem as implemented, to ground a redesign discussion. Not a contract. The frozen contract is `docs/reborn/contracts/memory.md`._

## TL;DR

- Reborn memory is **document-oriented**, not row-oriented. A "memory" is a Markdown/text **document** at a scoped virtual path, stored through `RootFilesystem` (libSQL/Postgres-backed). It is not a `(key, value, embedding)` row.
- There are **four capabilities**: `builtin.memory_write`, `builtin.memory_search`, `builtin.memory_read`, `builtin.memory_tree`. All memory mutation and retrieval goes through these.
- **Writes are explicit only.** The model decides to call `memory_write`. There is **no implicit/automatic extraction** of facts from a conversation in Reborn today.
- **Two prompt-injection paths exist**, but only one is live:
  - **Identity files** (`AGENTS.md`, `SOUL.md`, `USER.md`, `MEMORY.md`, …) — injected as system messages every turn. **Live.**
  - **RAG memory snippets** (semantic-search relevant memories auto-spliced into context) — fully plumbed but **not wired**; `ThreadBackedLoopContextPort` hardcodes `memory_snippets: Vec::new()`.
- Search is **hybrid FTS + vector with RRF fusion**, but in the default Reborn build **no embedding provider is wired**, so vector search is inactive and only FTS runs.

---

## 1. Data Model — a memory is a scoped document

There is no `MemoryItem` struct. A memory is identified by a **scope** + a **relative path**.

### Scope (the isolation key)

`MemoryDocumentScope` (`crates/ironclaw_memory/src/path.rs:10`):

```rust
pub struct MemoryDocumentScope {
    tenant_id: String,            // required
    user_id: String,              // required
    agent_id: Option<String>,     // optional
    project_id: Option<String>,   // optional
}
```

The full uniqueness key is the 4-tuple `(tenant_id, user_id, agent_id, project_id, relative_path)`.

### Virtual path

`MemoryDocumentPath` = scope + relative path, rendered as a virtual filesystem path:

```
/memory/tenants/{tenant}/users/{user}/agents/{agent-or-_none}/projects/{project-or-_none}/{relative/path}
```

So `USER.md` for user `u123` becomes a document at a fully-scoped path. Every backend query is prefixed with this scope, giving **physical namespace isolation** between users/tenants.

### Storage layout (per document)

The filesystem repo stores four record kinds for each logical document (`crates/ironclaw_memory/src/repo/filesystem.rs`):

| Purpose | Path | Kind |
|---|---|---|
| Document body | `<scope>/<relative>` | `memory_document` |
| Metadata sidecar | `<scope>/<relative>.meta` | (file) |
| Chunk projection (for search) | `<scope>/<relative>.chunks/<n>` | `memory_chunk` |
| Version history | `<scope>/<relative>.versions/<n>` | `memory_document_version` |

### Metadata

`DocumentMetadata` (`src/metadata.rs:19`) — `skip_indexing`, `skip_versioning`, `hygiene { enabled, retention_days=30 }`, `schema` (JSON-schema validation of content), plus a forward-compat `extra` map. Metadata is **inherited** from ancestor `.config` files, with the document's own overlay winning.

### Persistence & parity

The single Reborn repo is `FilesystemMemoryDocumentRepository`, wrapping `Arc<dyn RootFilesystem>`. **PostgreSQL/libSQL parity is owned by `ironclaw_filesystem`, not by `ironclaw_memory`** — the memory crate gained native FTS/vector/CAS by delegating to the filesystem layer instead of having per-backend SQL repos.

### Versioning & delete

- Every content-changing write archives the prior content to `.versions/<n>` (unless `skip_versioning` or content unchanged or first-create).
- **No soft-delete / "never delete" flag is implemented.** `delete` capability defaults to `false`; `retention_days` is modeled but **not enforced** (no hygiene sweeper). `DocumentDeleted` is a declared event kind but the current backend only emits `DocumentWritten`, `DocumentIndexed`, `SearchPerformed`.

---

## 2. The Four Capabilities

Defined in `crates/ironclaw_host_runtime/src/first_party_tools/memory.rs:27`:

```rust
pub const MEMORY_SEARCH_CAPABILITY_ID: &str = "builtin.memory_search";
pub const MEMORY_WRITE_CAPABILITY_ID:  &str = "builtin.memory_write";
pub const MEMORY_READ_CAPABILITY_ID:   &str = "builtin.memory_read";
pub const MEMORY_TREE_CAPABILITY_ID:   &str = "builtin.memory_tree";
```

| Capability | Params | Effect |
|---|---|---|
| `memory_write` | `content`, `target`, `append`(default true), `metadata`, `old_string`/`new_string`/`replace_all` (patch mode), `timezone` | `ReadFilesystem`+`WriteFilesystem`, `PermissionMode::Allow` (no approval gate) |
| `memory_search` | `query`/`q`/`text`/`pattern`, `limit` (1–20, default 5) | hybrid search |
| `memory_read` | `path` | read one doc |
| `memory_tree` | `path` (root), `depth` (1–10, default 1) | list as tree |

**`target` shortcuts** for `memory_write`: `"memory"`→`MEMORY.md`, `"daily_log"`→`daily/{date}.md`, `"heartbeat"`→`HEARTBEAT.md`, `"bootstrap"`→clears `BOOTSTRAP.md`, or any relative path.

**Write sub-operations:** `Append` (CAS loop, up to 8 retries), `Patch` (read→replace `old_string`→compare-and-write), `Replace` (full overwrite), `ClearBootstrap`.

---

## 3. Use Case: Explicit Save ("remember X")

There is **no separate "save to memory" intent**. The user asks the model to remember something; the model emits a `builtin.memory_write` tool call. End-to-end:

```
User: "Remember that I prefer dark roast coffee."
  │
  ▼
Model emits tool call: builtin.memory_write
  { target: "memory", append: true,
    content: "- Prefers dark roast coffee" }
  │
  ▼
Agent loop  (ironclaw_agent_loop/executor/capabilities.rs)
  │   CapabilityInvocation
  ▼
LoopCapabilityPort → HostRuntimeLoopCapabilityPort
  │
  ▼
CapabilityHost  (ironclaw_capabilities/src/host.rs)
  ├─ descriptor exists in ExtensionRegistry?
  ├─ TrustAwareCapabilityDispatchAuthorizer.authorize_dispatch_with_trust
  ├─ claim CapabilityLease (if approval-gated; memory_write is Allow, so no gate)
  └─ on Allow → CapabilityDispatcher.dispatch_json
  │
  ▼
BuiltinFirstPartyTools::dispatch → memory::dispatch  (first_party_tools/memory.rs:186)
  ├─ memory_services(): build MemoryDocumentScope from request.scope
  │                      ensure /memory mount grants {read,list,write,delete}
  │                      build MemoryContext (+ audit context, correlation id)
  └─ dispatch_write → parse_write_command → append/patch/replace
  │
  ▼
RepositoryMemoryBackend::write_document_with_backend_options  (backend.rs:472)
  ├─ ensure_path_matches_context   (scope defense-in-depth)
  ├─ validate UTF-8
  ├─ enforce_prompt_write_safety   (injection scan for protected paths)
  ├─ resolve_write_metadata        (inherit .config ancestry)
  ├─ optional JSON-schema validation
  ├─ repository.write_document_with_options   (CAS write + version archive)
  ├─ persist .meta sidecar
  ├─ emit MemorySignificantEvent::document_written
  └─ indexer.reindex_document      (re-chunk + FTS index)
  │
  ▼
FilesystemMemoryDocumentRepository → RootFilesystem (libSQL/Postgres)
```

**Result:** the document `MEMORY.md` for that user's scope now contains the appended line, versioned, FTS-indexed.

### "User interaction saves to memory" (implicit) — NOT present

Reborn has **no automatic memory extraction**. There is no post-turn hook that observes the conversation and decides to persist a fact. `PostCapabilityStage` only does compaction and subagent drain. `MemoryPromptContextService` is read-only. The only way memory is written is an explicit model-issued `memory_write`. (This is a notable gap and a likely redesign target.)

---

## 4. Use Case: Fetch on Request (model searches mid-turn)

```
Model emits tool call: builtin.memory_search { query: "coffee preference", limit: 5 }
  │
  ▼
CapabilityHost → memory::dispatch → dispatch_search  (memory.rs:331)
  │
  ▼
RepositoryMemoryBackend::search(context, request)  (ironclaw_memory/src/backend.rs)
  │
  ▼
MemoryDocumentRepository::search_documents  (repo/filesystem.rs)
  ├─ FTS branch     (libSQL FTS5 / pg FTS)        → ranked chunk hits
  ├─ vector branch  (Filter::VectorNearest)        → ranked chunk hits  [inactive: no embeddings wired]
  ├─ group chunks by document relative_path
  ├─ RRF fusion: score = 1/(k+rank), k=60          (or WeightedScore)
  ├─ normalize to [0,1], apply min_score, sort
  └─ truncate to limit
  │
  ▼
post-filter: results.retain(|r| r.path.scope() == context.scope())   (no cross-user leak)
  │
  ▼
Results returned to model as a tool result in the transcript
```

`MemorySearchResult` = `{ path, score, snippet, full_text_rank, vector_rank }`. Search defaults: `limit` 20 (capped per capability schema to ≤20), `pre_fusion_limit` 50, both branches on, RRF k=60.

**Caveat:** the default Reborn `build_backend` wiring passes **no embedding provider**, so the vector branch is inert; only FTS contributes. Hybrid search is built but runs single-modality until an embedding provider is wired at the host boundary.

---

## 5. Use Case: Prompt Injection (auto-context at turn start)

Two distinct slots in `LoopContextBundle`.

### 5a. Identity files — LIVE

A fixed allow-list of protected documents (`crates/ironclaw_memory/src/safety.rs:153`):

```
SOUL.md, AGENTS.md, USER.md, IDENTITY.md, SYSTEM.md,
MEMORY.md, TOOLS.md, HEARTBEAT.md, BOOTSTRAP.md,
context/assistant-directives.md, context/profile.json
```

Flow:

```
ThreadBackedLoopContextPort::load_loop_context()   (ironclaw_loop_support/src/lib.rs)
  └─ build_identity_messages()   (identity_context.rs:192)
        for each HostIdentityContextCandidate:
          fetch via HostIdentityContextSource
          wrap as LoopContextMessage → LoopContextBundle.identity_messages
  │
  ▼
InstructionBundleBuilder::build()   (ironclaw_turns/run_profile/instruction_bundle.rs:244)
  └─ push each as a system-role message, section label "identity"
```

So `USER.md` / `MEMORY.md` content lands in the system prompt **every turn**, automatically.

### 5b. RAG memory snippets — PLUMBED BUT NOT WIRED

The slot `LoopContextBundle.memory_snippets` and its `InstructionBundleBuilder` handling (`instruction_bundle.rs:326`, section label `"memory"`) are fully built. The service trait `MemoryPromptContextService` and production impl `ProductionMemoryPromptContextService` exist. Scope derivation, `MemoryContextPolicy::Disabled` escape hatch — all present.

**But the wire is missing:** `ThreadBackedLoopContextPort` has no `MemoryPromptContextService` field and hardcodes:

```rust
// ironclaw_loop_support/src/lib.rs:401
memory_snippets: Vec::new(),
```

So **semantic retrieval-augmented injection does not happen today.** Memory only enters context via (a) the fixed identity files, or (b) an explicit `memory_search` tool call the model chooses to make.

---

## 6. Scope Isolation (how a user only sees their own memory)

Enforced at three layers:

1. **Capability dispatch:** `memory_services()` builds `MemoryDocumentScope` straight from the authorized `request.scope` (tenant/user/agent/project). The host has already authorized this scope.
2. **Backend defense-in-depth:** `ensure_path_matches_context` / `ensure_scope_matches_context` reject any read/write/list whose path scope ≠ context scope (`backend.rs:380`). Backends must not infer broader authority from their own config.
3. **Search post-filter:** results are re-filtered `r.path.scope() == context.scope()` so even an unexpected backend row can't leak across users/tenants.

`ironclaw_memory/CLAUDE.md`: *"Every read/list/search/write/version/chunk operation must filter by the full `(tenant_id, user_id, agent_id, project_id)` tuple."*

---

## 7. Summary Table — what exists vs. what's missing

| Capability | Status |
|---|---|
| Document storage (scoped, versioned, CAS) | ✅ Live |
| `memory_write` / `read` / `search` / `tree` tools | ✅ Live |
| FTS search | ✅ Live |
| Vector / hybrid RRF search | ⚠️ Built, **no embedding provider wired** → FTS-only |
| Identity-file prompt injection | ✅ Live (every turn) |
| RAG memory-snippet auto-injection | ⚠️ Plumbed, **not wired** (`memory_snippets: Vec::new()`) |
| Implicit/automatic memory extraction from conversation | ❌ Not present |
| Soft-delete / "never delete" retention enforcement | ❌ Modeled (`retention_days`) but **not enforced**; no delete path |
| Approval gate on memory_write | ❌ `PermissionMode::Allow` (no gate) |

These four ⚠️/❌ rows are the natural seams for a new memory-system design.

---

## Key File Map

| Topic | File |
|---|---|
| Scope & path model | `crates/ironclaw_memory/src/path.rs` |
| Metadata / hygiene | `crates/ironclaw_memory/src/metadata.rs` |
| Filesystem repo, storage layout, CAS, versions | `crates/ironclaw_memory/src/repo/filesystem.rs` |
| Backend, scope guards, capabilities | `crates/ironclaw_memory/src/backend.rs` |
| Search, RRF fusion | `crates/ironclaw_memory/src/search.rs` |
| Chunking | `crates/ironclaw_memory/src/chunking.rs` |
| Indexer + embedding fallback | `crates/ironclaw_memory/src/indexer.rs` |
| Embedding provider seam | `crates/ironclaw_memory/src/embedding.rs` |
| Protected identity paths, prompt-write safety | `crates/ironclaw_memory/src/safety.rs` |
| Capability handlers (write/read/search/tree) | `crates/ironclaw_host_runtime/src/first_party_tools/memory.rs` |
| Identity injection | `crates/ironclaw_loop_support/src/identity_context.rs`, `.../src/lib.rs` |
| Memory-snippet RAG slot (unwired) | `crates/ironclaw_turns/src/run_profile/memory_context.rs`, `host.rs`, `instruction_bundle.rs` |
| Frozen contract | `docs/reborn/contracts/memory.md` |

---

## 8. Cross-System Comparison

How three other local agent harnesses handle memory, vs. IronClaw Reborn. Source codebases: `~/Code/hermes-agent`, `~/Code/pi`, `~/Code/claude-code`. (A web survey of industry systems — MemGPT/Letta, Mem0, Zep, Generative Agents, etc. — is appended in §9.)

### 8.1 One-paragraph characterization

- **IronClaw Reborn** — Document-oriented, scoped (`tenant/user/agent/project`) virtual-filesystem memory with CAS + version history. Hybrid FTS+vector search (vector inert: no embeddings wired). Identity files auto-inject; RAG snippets plumbed-but-unwired. **Writes explicit-only; no auto-extraction.**
- **Hermes** — The richest. A compact, char-capped built-in `MEMORY.md`/`USER.md` (frozen-snapshot system-prompt injection) + a SQLite FTS5 store of *every* transcript turn (`session_search`) + **pluggable external backends** (Holographic HRR vectors, Hindsight knowledge graph, Honcho user-modeling, Mem0). A **periodic background "review agent"** (every ~10 turns) auto-decides what to save. External backends do per-turn RAG injection via `<memory-context>` fence blocks.
- **Pi** — Effectively *no* persistent memory system. A session is a singly-linked JSONL tree of entries; context = linear replay from leaf to root. The only memory mechanism is **LLM compaction**: when the token budget is exceeded, old turns are summarized into a structured Markdown `CompactionEntry`. No search, no embeddings, no cross-session recall, no selectivity.
- **Claude Code** — Plain `.md` files, no DB/vectors. Two layers: (1) always-loaded **CLAUDE.md hierarchy** (managed > user > project > local) + a capped `MEMORY.md` index; (2) per-project **auto-memory** topic files with YAML frontmatter, selected per-turn by an **LLM-as-selector** (a Sonnet side-call ranks files by their `description`, ≤5/turn) — *not* embeddings. A **background `extractMemories` forked agent** auto-writes after every turn; a periodic `/dream` agent consolidates and prunes.

### 8.2 Comparison table

| Dimension | **IronClaw Reborn** | **Hermes** | **Pi** | **Claude Code** |
|---|---|---|---|---|
| Storage format | Scoped docs over `RootFilesystem` (libSQL/PG), CAS + versions | Flat `§`-delimited `.md` + SQLite FTS + pluggable backends | Append-only JSONL session tree | Plain `.md` files (CLAUDE.md + auto-memory dir) |
| Scoping | `tenant/user/agent/project` 4-tuple, enforced | Profile (+ bank/session for plugins) | Filesystem cwd only | Global/user/project/local + per-git-root auto-mem |
| Keyword/FTS search | ✅ FTS (libSQL FTS5 / PG) | ✅ FTS5 over all transcripts (`session_search`) | ❌ none | ⚠️ Grep fallback only (model-driven) |
| Vector/embeddings | ⚠️ Built (RRF) but **no provider wired** | ⚠️ Plugin-only (Holographic HRR = hashed, not neural; Hindsight/Honcho server-side) | ❌ none | ❌ none (LLM-as-selector over descriptions instead) |
| Graph/tree memory | ❌ | ⚠️ Hindsight knowledge graph (plugin) | tree = session lineage (not semantic) | ❌ (hierarchical file precedence) |
| Write — explicit tool call | ✅ `memory_write` | ✅ `memory` tool | n/a (auto-records all) | ✅ inline model writes + `/memory` |
| Write — automatic extraction | ❌ **none** | ✅ background review agent every ~10 turns + per-turn backend sync | ❌ (records verbatim, no extraction) | ✅ background `extractMemories` agent post-turn |
| Write — reflection/consolidation | ❌ | ⚠️ holographic auto-extract (opt-in); compaction insights | summarization compaction only | ✅ `/dream` periodic consolidation + prune |
| Prompt injection — always-on | identity files (every turn) | built-in MEMORY/USER frozen snapshot | system prompt (AGENTS/CLAUDE.md) | CLAUDE.md hierarchy + MEMORY.md index |
| Prompt injection — RAG/retrieved | ⚠️ slot exists, **unwired** | ✅ per-turn `<memory-context>` fence (backends) | ❌ | ✅ `relevant_memories` attachments (≤5, LLM-selected) |
| Selectivity (what to remember) | model decides (explicit only) | tiered: high-value built-in vs full transcript in DB | none — everything verbatim | model + extractor decide; 4 types (user/feedback/project/reference) |
| Lifecycle / forgetting | ❌ retention modeled, not enforced | char caps + dedup; optional temporal decay (holographic) | token-budget compaction (raw kept) | no TTL; `/dream` prunes; staleness warnings on age |
| Dedup | ❌ | ✅ exact-match + load-time dedup | ❌ | via consolidation agent |
| Approval gate on write | ❌ (`Allow`) | ⚠️ optional write-approval gate | n/a | model self-governed |

### 8.3 Patterns worth stealing for the redesign

1. **Background extraction agent** (Hermes review-agent, Claude Code `extractMemories`) — both run a *forked* agent sharing the conversation/prompt-cache to decide what to persist, decoupled from the main turn. Directly fills Reborn's #1 gap (no auto-save). Note the mutual-exclusion guard: skip the extractor if the main agent already wrote memory this turn.
2. **LLM-as-selector vs. embeddings** (Claude Code) — ranking memory files by a one-line `description` via a cheap side-call sidesteps the "no embedding provider wired" problem entirely and is prompt-cache friendly. A pragmatic stop-gap (or permanent choice) before committing to a vector stack.
3. **Tiered storage** (Hermes) — a tiny, always-injected high-value layer (preferences/corrections) separate from a large, search-only transcript layer. Maps cleanly onto Reborn's identity-files vs. searchable-docs split.
4. **Consolidation/decay loop** (Claude Code `/dream`, Hermes temporal decay) — periodic prune/merge keeps the always-injected layer small and the index prompt-cache stable. Fills Reborn's unenforced-retention gap.
5. **Frozen-snapshot injection for cache stability** (Hermes) — capture identity/memory once at session start so the provider prefix-cache isn't busted mid-session. Relevant to how Reborn wires the RAG slot.
6. **Staleness signaling** (Claude Code age headers) — annotate injected memories with age rather than silently serving stale facts.

---

## 9. Industry / Web Survey — AI Agent Memory Systems

_Survey of how modern AI agent / LLM-harness memory systems work across the industry (mid-2026), with named systems and source URLs. From a parallel web-research pass with adversarial claim verification._

> **Mapping note:** the survey below references IronClaw's hybrid search as `src/workspace/search.rs` (the **legacy** workspace). Reborn's equivalent is `crates/ironclaw_memory/src/search.rs` — same paradigm (FTS + vector, **RRF k=60** default, `WeightedScore` alt, `pre_fusion_limit=50`, dual-backend), confirmed in §1/§5 above. Read "IronClaw" in §9 as applying to the Reborn `ironclaw_memory` crate.

### 9.0 Paradigm comparison table

| Paradigm | Example systems | Storage/structure | Retrieval | Write strategy | Injection | Key tradeoffs |
|---|---|---|---|---|---|---|
| **Flat document/file** | Claude Code `CLAUDE.md`; MemGPT/Letta **core memory** blocks | Plain Markdown/XML, no index | None — whole file loaded | Hand-edited (CLAUDE.md) or agent self-edit `core_memory_append/replace` (MemGPT) | **Always-on** in system prompt | Zero latency, human-readable, git-versionable; overflows context at scale (~500 facts ≈ 15K tok), no semantic search, goes stale |
| **Key-value / fact store** | Mem0 facts; LangGraph `BaseStore`; Redis Agent Memory Server; Generative Agents stream | Atomic NL statements + metadata (id, hash, scope, timestamps) | Key lookup (<1ms) or query | LLM extraction into atomic facts; explicit `put` | Per-turn RAG | Structured, scoped, auditable; needs extraction pipeline; granularity matters |
| **Vector / embeddings** | MemGPT archival; LangMem; LlamaIndex; dense leg of every hybrid | float32 vectors (384–3072d) + payload; HNSW/IVFFlat/PQ | Dense ANN (cosine), MMR, temporal-decay reweighting | Embed-on-write | Per-turn RAG | Semantic recall, language-robust; misses exact IDs/codes/names, big footprint, stale on re-index |
| **Graph / KG** | **Zep/Graphiti**, **Mem0g**, **Cognee**, MS GraphRAG | Entity nodes + typed edges (Neo4j/Kuzu); bi-temporal edges (Zep) | BFS n-hop fused with cosine + BM25 via RRF/MMR | 2-stage LLM extraction (entities→triplets) + entity resolution/dedup | Per-turn RAG | Multi-hop + temporal reasoning, no LLM in read path (Zep ~300ms p95); high write cost, schema overhead, cold start |
| **Tree / hierarchical** | MemGPT paging; **RAPTOR**; Gen Agents reflection tree; GraphRAG communities | Multi-level summaries (leaves→root); OS tiers | ANN over flattened nodes ("collapsed tree" beats traversal) | Recursive LLM summarization; reflection | Paged virtual-context or per-turn | Multi-doc synthesis, long-convo compression; expensive build, lossy |
| **Keyword / FTS** | SQLite FTS5 (IronClaw, zero-infra); ES/OpenSearch BM25; Neo4j Lucene (under Graphiti) | Inverted index | BM25 (sub-ms FTS5) | Tokenize-on-write (cheap) | Per-turn RAG | Exact-match recall (IDs/codes/dates), cheap; misses paraphrase — never used alone |
| **Hybrid (BM25+ANN+RRF)** | **Mem0 v3**, **Zep**, **IronClaw**, LangMem, Perplexity, Glean | Dual sparse+dense index | BM25 + vector (+ graph BFS) fused by **RRF (k=60)**; optional cross-encoder rerank | Per-leg | Per-turn RAG | Best general recall, calibration-free fusion, ~+10ms over vector; needs two indices |

### 9.1 Storage paradigms (highlights)

Six paradigms; production systems combine several. Flat-file = MemGPT core blocks / Claude Code CLAUDE.md (overflow at scale). KV fact store = Mem0 / LangGraph `BaseStore` / Redis Agent Memory Server. Vector = MemGPT archival, dominant models OpenAI `text-embedding-3-small`, BGE (384d Letta default, 1024d Zep); indexes HNSW/IVFFlat/PQ. **Graph** = Zep/Graphiti (Neo4j, bi-temporal, 94.8% DMR vs MemGPT 93.4%, +18.5% LongMemEval at ~90% lower latency — [arXiv:2501.13956](https://arxiv.org/abs/2501.13956)), Mem0g, Cognee (decoupled Kuzu/LanceDB/SQLite). Tree = MemGPT tiers, RAPTOR (UMAP+GMM cluster→summarize; flatten-then-ANN beats traversal — [arXiv:2401.18059](https://arxiv.org/abs/2401.18059)). FTS = SQLite FTS5 (sub-ms, zero-infra) — always a hybrid leg, never alone.

### 9.2 Retrieval — the dominant finding

**Retrieval strategy dominates storage strategy.** Ablation ([arXiv:2603.02473](https://arxiv.org/html/2603.02473)): *how* you retrieve drives a ~20pt accuracy spread on LoCoMo (BM25 57.1% → cosine 73.4% → hybrid+rerank 77.2%); *what/how you store* moves only 3–8pt.

- **BM25** — exact IDs/codes/names; blind to paraphrase; unbounded scores break raw-score fusion.
- **Dense** — semantic/cross-lingual; fails exact IDs, clusters antonyms.
- **Hybrid via RRF** = production default. `RRF(d)=Σ 1/(k+rank)`, **k=60**. Sidesteps BM25↔cosine scale mismatch, no calibration. MS data: hybrid 48.4 vs vector 43.8 vs keyword 40.6; +25–30% recall on exact-match at +10ms.
- **Graph BFS** — surfaces n-hop context (Alice→employer→projects) neither BM25 nor vectors reach. Graphiti: cosine+BM25+BFS via RRF, **no LLM in read path** (~300ms p95 vs MemGPT 28.9s).
- **Recency+importance** (Gen Agents): `score = recency + importance + relevance`, `recency=0.995^hours` ([arXiv:2304.03442](https://arxiv.org/abs/2304.03442)).
- **Two-stage rerank**: hybrid→top50→cross-encoder→top10 (+50–100ms, best precision).

### 9.3 Write / what-to-remember

Five patterns by trigger/cost: **explicit tool call** (ChatGPT `bio`, LangGraph store-nodes; 0 extra LLM calls). **Automatic LLM extraction** (Mem0: per message-pair, Stage1 fact-extract → Stage2 ADD/UPDATE/DELETE/NOOP; ~7K tok/convo — [arXiv:2504.19413](https://arxiv.org/html/2504.19413v1)). **Reflection** (Gen Agents: fires at cumulative importance ≥150, ~2–3×/day → salient questions → insights). **Buffer summarization** (LangChain rolling summary; **LangMem** adds *background* `ReflectionExecutor`, debounced, post-response — no in-turn latency). **MemGPT self-editing** (inner monologue → `core_memory_append/replace`, `request_heartbeat` chains ops). **ChatGPT "Dreaming"** = background consolidation (vendor self-report 41.5%→82.8%, unverified).

**Key caveat:** raw chunked storage (zero LLM write cost) *matched or beat* LLM-extracted facts (81.1% vs 77.3%) — write-time filtering is an **efficiency lever, not an accuracy lever**.

### 9.4 Prompt injection

Four modes: **always-on core** (system prompt; MemGPT core ~86 tok, CLAUDE.md ~2.3–3.6K; best for persona/rules/profile; prompt-cache friendly). **Per-turn RAG** (workhorse; retrieve→inject before question at recency position; Mem0 ~6.9K vs ~26K full-context). **Paged virtual-context** (MemGPT: context=RAM, store=disk; page in via `archival_memory_search`, out via summarization at ~0.85× window; all state persisted — matches "never delete"). **Pointer indirection** (store big outputs externally, inject short ref).

Token budget: summarize at 70–80%; chunks 256–1024 tok, 10–20% overlap. **Placement matters** ("lost in the middle" U-shaped recall): static rules/persona at system-prompt top (primacy + cache), dynamic retrieved memory at end of user turn (recency) — [Anthropic context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents).

### 9.5 Selectivity, dedup, decay

**Importance**: Gen Agents 1–10 poignancy (stored once); Mem0 binary LLM gate. **Conflict handling** is the differentiator: Mem0 **hard-deletes**; LangMem LLM-decides (`enable_deletes=False` default); **Zep never deletes** — bi-temporal edge invalidation. No major system publicly discloses its cosine dedup cutoff (0.85/0.9 figures untraceable). **Episodic vs semantic**: converged taxonomy, divergent impl; MemGPT does *not* formally separate (tiers by location). **Forgetting/decay**: exponential `0.995^hours` (Gen Agents), Ebbinghaus spaced-repetition (MemoryBank), and the gold standard **Zep bi-temporal edges** (4 timestamps, invalidation not deletion → "who was Alice's employer in Nov 2023?" still resolves). Memora: **64% of memory errors = failure to invalidate stale memories** — active forgetting is the most under-built capability. **Selective vs store-everything**: full-context *beats* RAG by 15–25pt for first ~150 convos; RAG wins past ~300 on *cost* not accuracy ([ConvoMem arXiv:2511.10523](https://arxiv.org/abs/2511.10523)).

### 9.6 Implications for IronClaw (Reborn)

1. **Hybrid+RRF foundation is already correct.** `ironclaw_memory/src/search.rs` (FTS+vector, RRF k=60, dual-backend) matches the gold standard. Since retrieval dominates, highest-leverage upgrade = optional **cross-encoder rerank** (top-50→top-10). *Prerequisite: wire an embedding provider — vector leg is currently inert (§4).*
2. **"Never delete LLM data" is vindicated.** Zep invalidation + Letta persist-evict = IronClaw's principle. Gap to close = **active invalidation**: mark contradicted facts stale (`valid_at/invalid_at`), not delete. Fits "mark with timestamps, make filterable, always retain." Maps onto Reborn's unenforced `retention_days` (§1).
3. **Empirical case against eager extraction at IronClaw's scale.** Raw chunks beat extracted facts; full-context beats RAG below ~150–300 convos. Treat auto-extraction (Reborn's #1 gap) as an *efficiency/scoping* tool layered on raw episodes, not a correctness requirement.
4. **Two-tier injection mirrors the industry split.** Identity files = always-on core; memory tools = per-turn RAG. Formalize token budget (summarize 70–80%) + placement (identity at top for cache+primacy; retrieved memory at turn-end for recency). Directly informs wiring the unwired RAG snippet slot (§5b).
5. **Graph layer = clear future direction, high cost.** Multi-hop/temporal is what flat hybrid can't do. If pursued: keep LLM out of read path (Graphiti BFS+BM25+cosine via RRF ~300ms), reuse existing RRF fusion, don't build a parallel one.
6. **Distrust vendor/cross-paper benchmarks** — LoCoMo/LongMemEval swing 58–92% on harness alone. Validate any change against IronClaw's own tasks.

**Primary sources:** [Generative Agents 2304.03442](https://arxiv.org/abs/2304.03442) · [Zep/Graphiti 2501.13956](https://arxiv.org/abs/2501.13956) · [Mem0 2504.19413](https://arxiv.org/pdf/2504.19413) · [MemGPT 2310.08560](https://arxiv.org/abs/2310.08560) · [Retrieval-vs-Utilization 2603.02473](https://arxiv.org/html/2603.02473) · [Survey 2603.07670](https://arxiv.org/html/2603.07670v1) · [Anthropic Context Engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)

_Confidence note: vendor self-benchmarks (Mem0 73% token reduction; ChatGPT Dreaming 82.8%) and undisclosed dedup thresholds (0.85/0.9) are flagged unverified — not hard numbers._
