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
   `ironclaw_event_log`, `ironclaw_processes`, `ironclaw_host_api`, ...
3. **API contract type** (request/response/config for a trait or HTTP surface)
   → the crate that **defines the contract**. ProductSurface DTOs and
   descriptors live in `ironclaw_assistant`; host caller/error vocabulary lives
   in `ironclaw_host_api`; route-only wire types live in `ironclaw_webui`.
   Consumers import from the contract owner.
4. **Cross-domain primitive** (identity newtypes, paths, hashing, attachment
   format, timezone) → `ironclaw_common`. This is the ONLY thing common
   accepts.

**`ironclaw_common` is not a DTO dumping ground.** ~20 workspace crates depend on it — every
type added there rebuilds most of the workspace on change and couples
unrelated domains. A type belongs in common only if it is domain-free (would
be equally at home in any subsystem). "Several crates use it" is NOT the
test — that's what the domain vocabulary crates are for. Put shared types in
the **lowest crate both sides already depend on**, which is almost never
common.

## Why (measured 2026-07, semantically judged)

The workspace has ~3,500 public structs/enums (re-measured 2026-08:
`rg -c '^\s*pub (struct|enum) ' --glob 'crates/**/src/**/*.rs' crates/` — note the
family layout means `crates/*/src` no longer matches anything; crate sources are
two levels down, plus `crates/extensions/packages/*/src`).
A field/variant-signature scan (`scripts/check-type-duplicates.py`) reports
**203** cross-crate structural candidates from 2,003 eligible types on this
tree; the 2026-07 judging pass ran over a **178**-pair snapshot of that scan and
found **18 TRUE duplicates + 14 borderline identity-lockstep mirrors** — real
but rare (~1%), and mostly under *different names* (invisible to name matching).
Re-run the script before quoting either number; the unjudged delta is real. The
judged backlog lives in `docs/plans/2026-07-02-type-dedup-backlog.md`.
The dominant failure mode: a downstream crate re-declares an upstream type
verbatim "for decoupling," plus an identity `From` that never diverges.

The remaining complexity is contract *surface* (≈500 Request/Response/Config
types, each defined once), which placement cannot reduce — only interface
design (domain-port splits) and scaffolding do. Meanwhile compile ripple IS
controlled by placement. Measured crate-level fan-in on this tree (`grep -rl --include=Cargo.toml <crate> crates/`, count includes the crate's own manifest; re-measured 2026-08 after the WS6/WS7 renames): `host_api` **53**, `common` **20**, `turns` **12** — note this inverts the ordering an earlier version of this rule asserted, and `host_api` (the endorsed vocabulary home) carries the widest fan-in by design.
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
4. `pub use` is legitimate only at an architecture-mandated contract facade;
   never use it as a path-preservation shim or dependency dodge.
   This is the same exception the root AGENTS.md's "no `pub use` re-exports
   unless exposing to downstream consumers" already draws.

## Relocating a shared module — update imports, don't leave a re-export

When a type or module used by several crates has to move to a lower crate so
they can all reach it (the canonical case: a pure primitive shared across
layers moves into `ironclaw_common`, and the root AGENTS.md already permits *depending on
`common`* from anywhere), **move it and update every consumer's import to the
new path**. Do NOT leave a `pub use old_path::* ` shim in the original crate to
preserve `old_crate::thing` call sites — that shim is exactly the
path-preservation re-export §-item-1 and item-4 above forbid. A plain private
`use new_crate::module as old_name;` alias at a call site is fine (it is an
import, not a re-export); a crate-root `pub use` that keeps the old public path
alive is not. Worked example: the LLM cost table moved
`ironclaw_llm::costs` → `ironclaw_common::llm_costs`, and each consumer
(`ironclaw_llm` providers, `ironclaw_turn_runner`, `ironclaw_composition`,
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

1. **Polymorphism** — 2+ production implementors (the 2026-07 pass judged this
   true of ~62% of traits; the workspace has **385** `pub trait` definitions
   as of 2026-08, so re-count with
   `rg -c '^\s*pub trait ' --glob 'crates/**/src/**/*.rs' crates/` before quoting a share).
2. **Dependency inversion** — a port defined in a lower crate, implemented by
   a higher one. Single-impl BY DESIGN; "only one implementor" is the wrong
   metric here — deleting it re-couples the layers the boundary tests protect.
3. **Test seam** — the double/stub is the second implementor.
4. **`dyn` injection point** — object-safe surface wired at composition
   (includes security attenuation surfaces like the hooks gate sinks).

A trait with one same-crate impl, no double, no `dyn` use, and no inversion is
**ceremony** — call the concrete type; delete the trait. The 2026-07 pass found
only 8 traits failing this test (4 ceremony, 4 dead) — listed in
`docs/plans/2026-07-02-type-dedup-backlog.md`. That judgement has not been
re-run against the current trait set. New single-impl traits need
their §-reason stated in the PR description; reviewers may block on it.

Caution when auditing mechanically: naive `impl X for` grepping misses
generic/blanket impls, qualified paths, and macro-generated impls — verify by
reading before calling anything ceremony (two of our ten candidates turned
out to be mocked seams).

## What this rule does NOT do

Field-addition pain is a signal to reuse the ProductSurface conduits and
descriptors before adding a new trait or facade layer. Distinct wire and
domain types are still defined once at their contract owners; do not create
mirrors or abstractions just to make field propagation feel uniform.
