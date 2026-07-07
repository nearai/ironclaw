# LFD Brief: reborn-memory-platform — Reborn memory platform (meta)

**State**: meta-lane over pillars 16/17/18/19 on the built substrate
(`ironclaw_memory` contract crate + `ironclaw_memory_native` provider,
#3537 lift). This lane does NOT re-implement memory — it rolls the four
pillar holdouts up and adds cross-pillar longitudinal scenarios. **Bar**:
meta-score ≥ 0.92 holdout AND all four pillar holdout bars pass; any pillar
security hard-gate zeros the platform. **Profile**: `memory_platform`.

Source-substitution note (record in LOG.md, per goal): the seed
`docs/reborn/2026-06-22-memory-as-product-layer.md` does **not** exist in
tree or history. Real seeds are the contract docs + Hermes plan below.

## Outcome

Platform readiness = provider boundary is policy-mediated (16), turn events
write typed/scoped/validated memory (17), retrieval attaches bounded
source-attributed context (18), and benchmarks catch known-bad memory (19).
The load-bearing NEW signal here: a memory written in session 1 is retrieved
in session 2 and flagged by benchmark regression logic — end to end, through
the product/provider boundary, with no provider bypassing host/admin policy.

## Spec sources

- `docs/reborn/contracts/memory.md`, `memory-profiles.md`,
  `storage-placement.md` (the frozen platform contract)
- `docs/plans/2026-06-23-hermes-style-context-management.md` (envelope +
  compaction + result-handle model the retrieval pillar rides on)
- The four pillar `lfd/<lane>/` packages (16/17/18/19) — this lane consumes
  their scorers and holdout aggregates, it never edits them.

## Stage 0 inner suite

No new memory implementation. The four pillar Stage-0 suites stay green
every cycle, plus the shared floor: `ironclaw_memory` +
`ironclaw_memory_native` crate tests + `tests/integration/group_memory/`.
The platform profile only assembles cross-pillar longitudinal cases.

## Eval themes (own longitudinal dev ~18 / holdout ~12, + pillar rollup)

The meta-scorer is mostly a weighted READ of pillar holdouts; its own
case budget is deliberately small. Goal's 30 dev / 60 holdout cross-pillar
counts are designer GROWTH TARGETS (capped 30 dev-visible); the launch set
is 15–20 longitudinal cases.

1. Pillar rollup (weighted 25% each): meta = weighted mean of 16/17/18/19
   holdout aggregates. Hard gate: any pillar outcome with a satisfied
   security-forbidden matcher (leak / cross-scope / policy-bypass) zeros the
   platform BEFORE weighting.
2. Longitudinal write→retrieve→benchmark (10): session-1 authorized turn
   writes a typed artifact (scored in write); session-2 retrieval attaches
   the SAME artifact id in the envelope with attribution (scored in
   retrieval); benchmark regression classifies it (scored in bench). Here we
   price only the cross-session seam: same-id equality across sessions +
   policy/audit events; forbidden: cross-session cross-scope leak.
3. Cross-cutting policy (5): a denied-provider write in session 1 must not
   surface in session-2 retrieval (paired forbidden across pillars).
4. Repeated-question delta (4): an updated fact supersedes across sessions
   but the old version is retained (cross-session consistency only;
   per-pillar versioning mechanics scored in write).

## Feature-specific cheats → fences

- **Fork a parallel memory stack to make the umbrella pass** → dependency
  lint + caps: new `impl MemoryService`/`impl MemoryBackend` in
  platform-owned diff = 0; pillar lanes own concrete behavior.
- **Hide a pillar security failure behind the weighted average** →
  `score.sh` reads each pillar's hard-gate flag and zeros the platform
  before aggregation (mechanical, not reviewer-judged).
- **Score pillars but break the write→retrieve seam** → longitudinal cases
  require the phase-1 artifact id to appear as a phase-2 envelope id
  (`state_eq` across sessions); a per-pillar pass can't fabricate it.
- **Hardcode cross-pillar scenario ids/wording** → `probe.sh` randomizes
  scenario ids, entity names, and session labels; probe-gap gauge.
- **Claim source evidence without recording the missing memo** → LOG.md
  lint asserts the `2026-06-22-...` substitution note is present.

## caps.json extras

New memory-provider impls in platform diff = 0; cross-pillar seed scenarios
≤ 20 dev-visible; pillar-answer literals in platform diff = 0.

## Live mode

No live private memory (goal). 2 live longitudinal cases: real model writes
then retrieves across two sessions on a synthetic seeded corpus →
cross-session same-id + attribution structural contracts only; benchmark
verdict stays deterministic. Spend ceiling $25.
