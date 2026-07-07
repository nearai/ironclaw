# LFD Brief: self-learning-write-pipeline — Self-learning write pipeline

**State**: partial — a NEW event-driven ingestion layer on top of the built
memory provider (`ironclaw_memory` + `ironclaw_memory_native`) and the
production-ready distillation crate (`ironclaw_skill_learning`). **Bar**:
0.90 F1 holdout, zero policy-blocked writes persisted, zero untyped raw
transcript dumps. **Profile**: `memory_write`.

## Outcome

Authorized turn events are observed; a durable typed memory is written ONLY
when there is a useful, allowed learning signal. The pipeline classifies the
9 signal classes (explicit remember, correction, failure, recovery, repeated
workflow, success, preference, project fact, no-op chatter); each write
carries correct metadata (source, confidence, provenance, scope, idempotency
key, type ∈ closed enum, policy decision); duplicate events don't re-learn;
chatter / untrusted instructions / sensitive-disallowed / cross-scope /
prompt-injection are NOT written. Classifier output is not authority until
schema + policy + scope validation pass (repo Rule 5).

## Spec sources

- Goal §signal classes; `docs/reborn/contracts/memory.md` (§5.1 write/append/
  version, §6 metadata, §9 events); `docs/reborn/contracts/storage-placement.md`
  §5.2 (memory docs are the source of truth, not raw transcript blobs)
- `crates/ironclaw_memory_native/src/write_metadata.rs`,
  `schema.rs`, `events.rs` (typed artifact + metadata + significant-event
  surface); `crates/ironclaw_product_workflow/` (turn/event projections)
- Inherits write-path + retention (never-delete) themes from
  `lfd/_briefs/long-term-memory.md`, and distillation-validity fences from
  `lfd/_briefs/self-learning-loops.md`; distillation surface is
  `crates/ironclaw_skill_learning/` (+ `prompts/skill_extraction.md`,
  `prompts/skill_refinement.md`).

## Stage 0 inner suite

`ironclaw_memory*` crate tests + `tests/integration/group_memory/` write
cases (`scenario_write_then_read_cross_thread`). Green every cycle.

## Eval themes (dev 40 / holdout 14 across the 9 signal classes)

Per ADDENDA lane 17: this is the launch set; goal's 120 dev / 300 holdout
are designer GROWTH TARGETS. Each cycle includes ≥1 write-positive, ≥1
no-write, ≥1 duplicate case (goal cycle protocol).

1. Write-positive recall (12): explicit-remember / correction / recovery /
   repeated-workflow / success / preference / project-fact traces → typed
   artifact persisted (state query: type ∈ closed enum, scope,
   provenance/source ids, idempotency key, confidence). ≥1 case per class.
2. No-write precision (10): chatter / untrusted-instruction /
   sensitive-disallowed / cross-scope / prompt-injection → NO persisted
   artifact (forbidden: any persisted doc; forbidden: leak). Satisfies the
   ≥25% failure-direction floor.
3. Metadata correctness (6): source / confidence / provenance / scope /
   idempotency-key / type / policy-decision each correct (`state_pred` per
   field) — dropping any field fails.
4. Idempotency (6): duplicate turn event → exactly one persisted artifact,
   same idempotency key, no version bump / no second doc (state count == 1;
   forbidden: duplicate artifact).
5. Policy gate + classifier-not-authority (6): classifier says "write" but
   policy/scope validation rejects → `status == error` / gate denial, zero
   persisted (hard gate: zero policy-blocked writes). Both directions priced.

Cross-ref: provider-boundary placement is **scored in memory-placement**;
retrieval of these artifacts is **scored in long-term-memory-retrieval**;
type-filter-on-read is scored in retrieval — this lane owns type ASSIGNMENT
and versioning-on-overwrite (retention / never-delete).

## Feature-specific cheats → fences

- **Write every message** → no-write cases forbid any persisted artifact; F1
  scoring prices false positives; probe defeats memorization.
- **Write raw transcript dumps (untyped)** → persisted artifact must match
  the closed artifact-type enum (`schema.rs`); a raw-dump scan (n-gram
  overlap of persisted content vs input transcript > threshold) is a
  forbidden matcher; "zero untyped raw transcript dumps" is a hard gate.
- **Use classifier confidence as permission** → policy-gate cases REQUIRE
  `status == error` when policy rejects despite high confidence; the persisted
  doc must record `policy_decision` (state query) proving validation ran.
- **Hardcode/memorize dev trace phrases** → `probe.sh` paraphrases learning
  events + renames entities; holdout traces are new domains; answer-literal
  overlap lint; caps: dev trace literals in `crates/**` diff = 0.
- **Learn prompt-injection instructions** → injection traces sit in the
  no-write group with a forbidden `leak` matcher; safety cases hard-gated.
- **Treat duplicate as new learning** → idempotency cases require persisted
  count == 1 across duplicate events.

## caps.json extras

Prompt examples ≤ 12 (goal); artifact-type enum closed unless spec + tests
change together (enum additions outside `schema.rs` diff = 0); dev trace
entity literals in `crates/**` diff = 0; new inline multiline prompt strings
in `crates/ironclaw_skill_learning/**` diff = 0 (repo prompts/*.md rule).

## Live mode

No live private transcripts (goal). 4 live cases: real model classifies +
extracts from scripted turn events across signal classes → structural
contracts (typed artifact schema-valid, metadata present, no-write on
chatter/injection); extraction quality beyond structure is reviewed by the
human at acceptance. Spend ceiling $25.
