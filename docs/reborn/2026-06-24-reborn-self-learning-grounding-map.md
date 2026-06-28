# Reborn Self-Learning: Grounding Map

Date: 2026-06-24

Status: evidence companion to the implementation design. **Not** part of the published proposal — the proposal stays clean of citations and file references; this is where the grounding lives so each mechanism can be checked.

## Why this exists

Every mechanism in `2026-06-24-reborn-self-learning-implementation-design.md` was checked against how real systems actually do it, so nothing is "blindly added." This map records, per mechanism: a verdict, the concrete reference-repo evidence, the named-system/literature evidence, and any framing that was corrected as a result.

## Method

Ten reference agent codebases were read directly (one investigator each), the current `nearai/ironclaw` reborn substrate was traced for exists-vs-net-new, and the advanced mechanisms were verified against named systems/papers with sources.

| Group | Repos / sources |
| --- | --- |
| Lightweight agents (baselines) | nanoclaw, nanobot, picobot, GoGogot, praktor |
| Substantial memory engines | zeroclaw (Rust), nullclaw (Zig, zeroclaw lineage), picoclaw (Go), openclaw (TS) |
| Keystone self-learning agent | hermes-agent (Python) |
| Target substrate | nearai/ironclaw reborn (current state) |
| Named systems / literature | MemGPT/Letta, mem0, A-MEM, Zep/Graphiti, Generative Agents, Voyager, Reflexion, CoALA, Anthropic context-management, Elasticsearch/Azure/Weaviate/OpenSearch/Pinecone/Supabase/Qdrant, OWASP AISVS, LongMemEval/LOCOMO/MemoryAgentBench |

Verdict legend: **Corroborated** (real systems do this) · **Additive — justified** (beyond current practice, but motivated and grounded in a gap or a named result) · **Revised** (the doc over-claimed; framing was corrected).

## Read path

| Mechanism | Verdict | Reference-repo evidence | Literature evidence |
| --- | --- | --- | --- |
| Two-plane: online turn / offline learning; turn never blocks | Corroborated | hermes post-turn background-review fork; openclaw "dreaming" (Light/REM) offline pipeline; picoclaw evolution cold-path; zeroclaw fire-and-forget consolidation; GoGogot background summarization | Generative Agents reflection (importance-threshold 150, ~2–3×/day); mem0 async summary module; Letta sleep-time agents (arXiv 2504.13171) |
| Context planner as a distinct layer (curated, budgeted bundle ≠ raw search) | Corroborated | picoclaw pluggable `ContextManager` (slot/budget packing, provider-safe boundaries); nullclaw `memory_loader` (byte budget, priority slots, omission filter); openclaw bootstrap budget + compaction; hermes `MemoryManager` + `<memory-context>` wrap; ironclaw-reborn identity/prompt budget layer (exists) | MemGPT "virtual context management"; Anthropic "context engineering" + context-editing product |
| Hybrid FTS + vector retrieval, fused, with FTS-only fallback | Corroborated | praktor (RRF k=60 + FTS-only fallback); nullclaw (RRF k=60, 9-stage pipeline); zeroclaw (weighted-sum + LIKE fallback); openclaw (weighted-sum + MMR); hermes holographic (weighted-sum + graceful FTS fallback); ironclaw-reborn (FTS+vector+RRF in-crate, FTS-only at the live tool) | RRF (Cormack SIGIR 2009); Elasticsearch/Azure default RRF; Weaviate relativeScoreFusion; OpenSearch normalization; BM25 strong baseline (BEIR) |
| Retrieval lanes: deterministic-by-key, structured-trigger, session-recall, learned-rule | Corroborated (partial) | nanoclaw "narrowest relevant index" navigation; GoGogot status-gated recall; hermes/picoclaw/praktor session-recall as FTS; ironclaw-reborn deterministic `profile_set` router | Letta core blocks (always-in-context) + archival/recall tools; agentic RAG |
| Mid-turn retrieval as an authorized tool, not raw store access | Corroborated | hermes `session_search` tool (mediated, zero-LLM); zeroclaw/praktor/picoclaw/nullclaw `memory_recall` tools (mediated output); openclaw `memory_search/get` (capability, **no** `memory_write` tool — model never writes durable memory); ironclaw-reborn mediated memory capabilities | MemGPT/Letta self-editing memory tools; "Agentic RAG" survey (arXiv 2501.09136); Anthropic memory tool |
| Slot budgets + packing + omission reasons | Corroborated | nullclaw omission filter + per-entry truncation; openclaw budget-driven oldest-eviction + omission markers + identifier-preservation; picoclaw fresh-tail protection; nanoclaw fan-out budget | Anthropic context-editing (auto-clear stale tool calls; "smallest high-signal set") |
| Deterministic, ordered rerank precedence | **Revised** (kept as a starting heuristic) | Real systems use either LLM-judge (picobot, GoGogot), weighted-sum (zeroclaw, openclaw, hermes), or RRF + MMR (nullclaw, praktor) — **none uses a fixed multi-criterion precedence ladder like the doc's** | — |

## Learning pipeline

| Mechanism | Verdict | Reference-repo evidence | Literature evidence |
| --- | --- | --- | --- |
| Online cheap signals → offline classification | Corroborated | picoclaw `LearningRecord{Success, WinningPath, AttemptTrail}` online + offline cluster→draft; openclaw online recall-signal events → offline promotion; hermes review fork reads outcomes | Generative Agents reflection; mem0 async; sleep-time compute |
| Model **proposes** a typed item; **policy/deterministic gate disposes** | Additive — justified | openclaw promotion gate is a **deterministic score threshold, not an LLM-judge**; openclaw has **no `memory_write` tool** (model can't self-write active state); ironclaw "everything through dispatch + policy" architecture | Typed memory established (mem0 ADD/UPDATE/DELETE/NOOP; Letta blocks; A-MEM; CoALA semantic/episodic/procedural = facts/rules/skills). **But** a hard host-policy veto over the model's choice is *not* a named literature pattern — it is an IronClaw design choice |
| Typed signal envelope + idempotency + redaction before enqueue | Additive — justified | hermes encodes outcome signals as **prose heuristics** in a review prompt (less structured); picobot deterministic write-guard rejects heartbeat noise; nullclaw HMAC redaction at every embedding boundary; openclaw contamination guard (don't re-ingest own output) | Event-sourcing idempotency is standard practice (not memory-specific) |
| Recurrence threshold before a failure becomes a rule | Corroborated | picoclaw clusters repeated task-records before drafting; hermes nudge-interval gating | Generative Agents importance-sum threshold (150) before reflection |

## Security: provenance, scope, revocation

| Mechanism | Verdict | Reference-repo evidence | Literature evidence |
| --- | --- | --- | --- |
| Per-item source references drive permission **inheritance** | Additive — justified | hermes **provenance-gated curation** (write-origin tag: only auto-curate agent-created; never touch user-authored) — closest real match; picoclaw rich source-lineage on learned drafts; openclaw citations (source path#line) | Secure-RAG ACL inheritance onto derived chunks is **established**: Azure security-trimming + index projections, AWS Bedrock `.metadata.json`, Pinecone mirror-ACLs, Supabase RLS, Glean, OWASP AISVS C8 |
| Attribute to the run **owner**, not the message sender | Corroborated | ironclaw-reborn owner-scope attribution **exists and is enforced** (`ensure_scope_matches_context` fail-closed); zeroclaw per-row `agent_id`; hermes profile isolation | — |
| Scope can **never widen**; broadening is privileged | Corroborated | zeroclaw `AgentScopedMemory` **intersects** caller allowlists so scope can't widen; nanoclaw only the "main" group writes global memory | Multi-tenant isolation established (Pinecone namespaces, Weaviate shards, Qdrant partitions) |
| Revocation: source removed → derived artifacts revoked/expired/pending | Additive — justified, **revised framing** | No reference repo does source-removal revocation cascade (closest: openclaw stale-row GC on source change; ironclaw-reborn net-new) | Secure-RAG: **inheritance + isolation established; revocation is the hard part** — propagation lags (Azure "timing lag"; "until next ingestion"); erasing derived embeddings (RTBF) is structurally hard. Best practice = **enforce at query/use time** (which the doc's use-time revalidation does) and treat erasure as a separate engineered step |

## Lifecycle and verification

| Mechanism | Verdict | Reference-repo evidence | Literature evidence |
| --- | --- | --- | --- |
| Candidate → active lifecycle with a gate before use | Corroborated | picoclaw evolution: draft candidate→{accepted\|quarantined}, skill active→cold→archived→deleted, observe→draft→apply mode; hermes write-approval staging (candidate→approved→active, mandatory for agent-origin writes, **off by default**); openclaw `promotedAt?` candidate/promoted | — |
| Active ≠ injected; use-time checks still apply | Corroborated | ironclaw-reborn capability + scope checks at use; openclaw promoted-only-then-eligible | Letta: state persists, only a curated slice is in-context |
| Verification before behavior change; **"LLM says fixed" is not the gate** | Additive — justified (**strongest validation**) | **hermes has NO verification gate** — "an LLM says it's good IS the gate… the single biggest thing the design should improve on"; picoclaw verifies via LLM-judge + regex only ("exactly the LLM-says-fixed gate to flag"); openclaw uses a grounded-claim check, not a live re-run; zeroclaw/nullclaw none | Voyager: removing self-verification → **−73%** ("most important feedback"); "LLMs Cannot Self-Correct Reasoning Yet" (2310.01798); Stechly/KambhT (2402.08115); CodeT execution-agreement |
| Replay (regression lock) vs live re-run (behavioral proof); execution for skills | Additive — justified, **revised framing** | hermes/picoclaw lack it; zeroclaw "replay" is history reconstruction only | Execution/external verification is established (Voyager, CodeT). The **replay-vs-live distinction itself is the doc's own framing**, not a named concept — so it is presented as a design distinction, not attributed to a source |

## Curation and measurement

| Mechanism | Verdict | Reference-repo evidence | Literature evidence |
| --- | --- | --- | --- |
| Supersede-not-overwrite (versioned) + dedupe | Corroborated | zeroclaw `superseded_by` tombstone (row retained) + Jaccard dedupe; openclaw append-dated section + SHA1 claim-hash dedupe; hermes holographic `contradict()` + UNIQUE content; picoclaw merge/version-append | Zep invalidate-not-delete; mem0 update-not-overwrite + graph dedupe |
| Budgets + forgetting = archive, never hard-delete | Corroborated | hermes archive-never-delete + atomic consolidate-or-reject under char budget; nullclaw preserve_before_purge (re-chunk before delete); zeroclaw quotas (⚠ but hard-deletes) | A-MEM matches baselines at ~1–2.5k vs ~16–17k tokens (uncurated context = dead weight) |
| Uncurated memory can **hurt** → curation is mandatory | Corroborated | picobot unbounded blind-append (anti-pattern); praktor/zeroclaw dead usage signals (recorded, never wired) | **Well-supported & growing**: LOCOMO full-context (~73%) > mem0 (~68%); Goal-Directed-Search ≫ compression; LongMemEval 30–60% degradation. Caveat: **regime-dependent** (short histories that fit context can be net-negative) |
| Stateful-vs-stateless measurement | **Revised** | — | The ablation (with vs without memory) is standard (LongMemEval, LoCoMo, MemoryAgentBench, LifelongAgentBench) but there is **no canonical named metric**, and the delta does **not** cleanly isolate "the harness learned" from "the base model is good." Doc now frames it as an ablation (hold base model fixed; add a full-context baseline) |
| Usage metrics influence salience but are **not** an auto-optimizer | Corroborated (cautionary) | praktor/zeroclaw record `access_count` but never wire it in (dead signal); hermes asymmetric trust feedback (helpful +0.05 / unhelpful −0.10) is wired | Survey-of-self-evolving-agents reward-hacking warning |

## Resilience

| Mechanism | Verdict | Reference-repo evidence |
| --- | --- | --- |
| Turn always continues; memory is best-effort | Corroborated | Every repo: hermes (provider sync on a bounded-drain worker after a wedged provider blocked ~298s), zeroclaw/openclaw/nullclaw/praktor/picobot all swallow memory errors and proceed |
| Degrade vector→FTS; surface "unsupported" rather than false-empty; fail closed on permissions | Corroborated | praktor 3-tier degrade; nullclaw circuit breaker → keyword-only; openclaw vector-down → FTS/LIKE; ironclaw-reborn fail-closed on unsupported search (no silent vector→FTS at the tool; surfaces a distinct error) |

## Framing corrections applied to the proposal

These are the places the first draft over-claimed; the design text was tightened so it stays honest.

1. **Rank fusion ≠ RRF-as-universal.** "Fused by rank" now reads as "fused into one ranking (reciprocal-rank fusion *or* score normalization)," with the constant noted as tunable. (RRF is default only in Elasticsearch/Azure; Weaviate/OpenSearch normalize; zeroclaw/openclaw use weighted-sum.)
2. **Model-estimated TTL is a deliberate, clamped differentiator — not standard.** The load-bearing part is class defaults + source/validity bounds (Zep bi-temporal, Generative Agents decay); the model only *suggests* an expiry that policy clamps. LLMs are established at estimating *importance*, not lifespan.
3. **Revocation is enforced at use time, not instant.** The doc leans on use-time revalidation (the recommended query-time ACL recheck) and treats hard erasure/propagation as a separate, harder guarantee — matching secure-RAG reality.
4. **"LLM says fixed" is not a gate; prefer execution/external verification.** Strengthened, and the replay-vs-live split is presented as the doc's own design distinction, not a cited standard.
5. **Stateful-vs-stateless is an ablation, not a named metric**, and a full-context (no-curation) baseline is included so the memory layer must prove it pulls its weight.
6. **"Policy disposes" is an IronClaw design choice**, not common memory-system practice (named systems let the model pick the operation).
7. **Auto-injection and tool-call retrieval are combined, not exclusive** (deterministic slots loaded up front + mid-turn tool fetch) — matching Letta/Anthropic hybrid guidance.
8. **Curation justifies memory's existence** — the claim is not "memory always helps"; uncurated/over-compressed memory can underperform a plain baseline.

## Genuinely beyond current practice (deliberate, and why)

These go past what the reference set ships. Each is motivated by a documented gap, not invented for novelty:

- **Verification by live re-run / execution before a learned change takes effect.** The keystone self-learning agent (hermes) has *no* verification gate and its own teardown flags this as the biggest weakness; Voyager quantifies the cost of removing verification (−73%); the literature says LLMs can't self-verify. This is the design's central differentiator.
- **Typed signal envelope + deterministic router + policy validator** (vs hermes's prose-heuristic review and openclaw's score gate). More structured, auditable, and safety-gated — consistent with IronClaw's "everything through dispatch."
- **Source-of-claim provenance + permission inheritance + revocation cascade** (vs author/path/citation provenance in the repos). Grounded in secure-RAG ACL-inheritance practice; revocation honestly scoped to use-time enforcement.
- **Model-estimated-then-clamped TTL** atop class defaults and source/validity bounds. Novel at the suggestion layer; clamped because LLMs aren't calibrated for lifespan.

## Honest residual cautions

- The **deterministic multi-criterion rerank precedence** is the least-corroborated specific; it remains a starting heuristic (and is listed under "Decisions Still To Settle").
- **Revocation propagation** and erasure of derived/embedded state is hard everywhere; the design must treat it as a first-class engineered guarantee, not assume it.
- **Model-estimated TTL** has no calibration evidence — keep the clamp and monitor.
- Watch for the recurring real-world bug the repos show: **recording a signal/config field that is never wired in** (praktor/zeroclaw dead fields). Every metric the design records must actually feed a decision.
