---
paths:
  - "crates/**/*.rs"
---
# Type Placement — One Definition, Owned by Its Contract

Companion to `types.md` (which governs type *shape*: newtypes, enums, wire
stability). This rule governs type *location and multiplicity*.

## The rule

**Every shared type has exactly one definition, living in the crate that owns
its contract.** Consumers import it. Nobody re-declares it, mirrors it, or
wraps it to move it across a crate boundary.

Placement decision, in order:

1. **Internal-only** (used by one crate) → private in that crate. Not `pub`,
   not exported, not this rule's concern.
2. **Domain type** (thread/turn/run/resource/capability/... shapes shared
   across crates) → the **domain vocabulary crate** that already owns that
   concept: `ironclaw_turns`, `ironclaw_threads`, `ironclaw_resources`,
   `ironclaw_events`, `ironclaw_run_state`, `ironclaw_host_api`, ...
3. **API contract type** (request/response/config for a trait or HTTP surface)
   → the crate that **defines the trait/route** (e.g. `RebornServicesApi`
   types live in `ironclaw_product_workflow`). Both sides of the boundary
   import from the contract owner.
4. **Cross-domain primitive** (identity newtypes, paths, hashing, attachment
   format, timezone) → `ironclaw_common`. This is the ONLY thing common
   accepts.

**`ironclaw_common` is not a DTO dumping ground.** It has fan-in ~64 — every
type added there rebuilds most of the workspace on change and couples
unrelated domains. A type belongs in common only if it is domain-free (would
be equally at home in any subsystem). "Several crates use it" is NOT the
test — that's what the domain vocabulary crates are for. Put shared types in
the **lowest crate both sides already depend on**, which is almost never
common.

## Why (measured 2026-07, semantically judged)

The workspace has ~2,900 public types. A field/variant-signature scan
(`scripts/check-type-duplicates.py`) found 178 cross-crate structural
candidates; agent review of every pair's definitions and usage judged **18
TRUE duplicates + 14 borderline identity-lockstep mirrors** — real but rare
(~1%), and mostly under *different names* (invisible to name matching). The
judged backlog lives in `docs/plans/2026-07-02-type-dedup-backlog.md`.
The dominant failure mode: a downstream crate re-declares an upstream type
verbatim "for decoupling," plus an identity `From` that never diverges.

The remaining complexity is contract *surface* (≈500 Request/Response/Config
types, each defined once), which placement cannot reduce — only interface
design (domain-port splits) and scaffolding do. Meanwhile compile ripple IS
controlled by placement: `common` fan-in 64, `host_api` 270, `turns` 110.
Edit ripple is expensive and rare; don't fix it by maximizing compile ripple.

## Mirror structs and `From` chains — a mapping must earn its keep

A second struct mirroring an existing one (plus `From`/`Into`) is allowed
ONLY when the two sides genuinely evolve independently:

- wire/API stability vs. internal churn (persisted row vs. domain type;
  public JSON contract vs. engine internals)
- security boundary (redacted view vs. full record)

If the `From` impl is field-for-field identity, the mirror is a violation:
delete it and import the source type. "The layers are conceptually separate"
does not justify a mirror — separateness without independent evolution is
free coupling plus a mapping tax.

**Wrapper/shim types that only re-export or re-package another crate's type
are banned** (e.g. a `FooServeConfig` wrapping `BarServeConfig` to avoid a
dependency edge). Take the dependency on the contract owner or invert the
seam — don't launder types through wrappers.

Resolution order for an existing mirror:

1. **Pass-through** (identical, identity `From`) → delete it; use the owner's
   type directly in signatures. Do NOT replace it with a `pub use` — consumers
   import from the owner.
2. **Additive lockstep** (owner's fields + extras) → embed the owner's type
   (`#[serde(flatten)]` for wire structs); wire output stays identical and
   new owner fields flow downstream with zero intermediate edits.
3. **Subtractive** (withholds fields) → keep the mirror; it is a redaction
   boundary and MUST stay manual so new sensitive fields do not auto-flow.
4. `pub use` is legitimate ONLY at an architecture-mandated contract facade
   (e.g. `product_workflow`, whose downstream is banned from depending on
   lower crates) — never as a path-preservation shim or dependency dodge.
   This is the same exception CLAUDE.md's "no `pub use` re-exports unless
   exposing to downstream consumers" already draws.

## Relocating a shared module — update imports, don't leave a re-export

When a type or module used by several crates has to move to a lower crate so
they can all reach it (the canonical case: a pure primitive shared across
layers moves into `ironclaw_common`, and CLAUDE.md already permits *depending on
`common`* from anywhere), **move it and update every consumer's import to the
new path**. Do NOT leave a `pub use old_path::* ` shim in the original crate to
preserve `old_crate::thing` call sites — that shim is exactly the
path-preservation re-export §-item-1 and item-4 above forbid. A plain private
`use new_crate::module as old_name;` alias at a call site is fine (it is an
import, not a re-export); a crate-root `pub use` that keeps the old public path
alive is not. Worked example: the LLM cost table moved
`ironclaw_llm::costs` → `ironclaw_common::llm_costs`, and each consumer
(`ironclaw_llm` providers, `ironclaw_runner`, `ironclaw_reborn_composition`,
the root crate) had its import repointed — no shim was left behind.

## Duplicate detection — signatures, not names

Duplicates are types doing the **same DTO job**, which usually means
*different names, same field/variant set* — name matching misses them.
Reproducible check:

```bash
python3 scripts/check-type-duplicates.py          # field/variant-signature scan
```

Output is candidates, not verdicts — judge each pair by reading both
definitions: TRUE-DUP (unify into the owner per the placement order),
JUSTIFIED-MIRROR (independent wire/domain evolution — document why), or
COINCIDENTAL (same shape, different concept). Judged baseline:
`docs/plans/2026-07-02-type-dedup-backlog.md`. A new TRUE-DUP-shaped pair
appearing in the scan requires justification in the PR description;
reviewers may block on it.

Same-name/different-concept collisions are also violations — rename one;
unique names are load-bearing for grep/agent discovery (see the naming-trap
examples: two `projection`s, `lifecycle.rs` that is skill management).

## Traits — an abstraction must earn its keep

The same discipline applies to traits. A trait is justified by exactly one of:

1. **Polymorphism** — 2+ production implementors (62% of the workspace's 352
   traits, measured 2026-07).
2. **Dependency inversion** — a port defined in a lower crate, implemented by
   a higher one. Single-impl BY DESIGN; "only one implementor" is the wrong
   metric here — deleting it re-couples the layers the boundary tests protect.
3. **Test seam** — the double/stub is the second implementor.
4. **`dyn` injection point** — object-safe surface wired at composition
   (includes security attenuation surfaces like the hooks gate sinks).

A trait with one same-crate impl, no double, no `dyn` use, and no inversion is
**ceremony** — call the concrete type; delete the trait. Judged 2026-07: only
8 of 352 traits (2.3%) failed this test (4 ceremony, 4 dead) — listed in
`docs/plans/2026-07-02-type-dedup-backlog.md`. New single-impl traits need
their §-reason stated in the PR description; reviewers may block on it.

Caution when auditing mechanically: naive `impl X for` grepping misses
generic/blanket impls, qualified paths, and macro-generated impls — verify by
reading before calling anything ceremony (two of our ten candidates turned
out to be mocked seams).

## What this rule does NOT do

Field-addition pain ("one new field touches trait + facade + wire + JS") is
NOT a placement problem — those are distinct contract layers, each defined
once. That cost is addressed by splitting `RebornServicesApi` into domain
ports (JIT) and by the reborn-feature scaffold/recipes, not by moving types.
Do not respond to that pain by relocating or merging types.
