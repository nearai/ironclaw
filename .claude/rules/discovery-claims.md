---
paths:
  - "crates/**"
  - "openwiki/**"
  - "**/*.md"
---
# Discovery Claims — Verify Before You Assert, Especially Negatives

Companion to the "Query the Knowledge Graph First" discovery workflow in
`CLAUDE.md`. That rule governs *how you search*; this one governs *how you
report what you found* — because a confidently-wrong finding steers a plan more
than a slow search does.

## The rule

**A load-bearing claim needs evidence at the definition/write site, not a
summary. A negative claim ("X doesn't exist", "nothing persists Y", "there's no
table for Z") needs a dedicated confirming search before you state it.**

"Load-bearing" = the claim changes a decision: build-new vs. extend-existing,
which crate owns a concept, whether a migration is needed, whether a boundary is
already enforced. For those, the bar is a concrete pointer — `file:line`, the
function that writes/defines it, or the trait method — not a doc's framing.

## Why this rule exists

While planning the IronClaw admin-user API (2026-07), a broad "map the whole
stack" exploration reported that IronClaw *has no user store* — "UserId minting is
the only trace a user leaves." That was wrong: `ironclaw_identity`
persists a `StoredUser` record (email, display_name, timestamps) on every SSO
login (`filesystem_store.rs` `resolve_or_create`). The miss flipped the intended
architecture (build a new store) until a targeted re-check corrected it. Three
failure modes combined, and each maps to a discipline below.

## Disciplines

1. **Distinguish "I didn't find X" from "X doesn't exist."** Say which, and name
   the search you actually ran. Absence inferred from shallow coverage is not
   absence. A broad survey samples each area shallowly — it is the wrong
   instrument for a "there is no…" conclusion.

2. **A negative that flips a decision gets a dedicated trace before you rely on
   it.** Verify along the path most likely to hold the thing: the *write* path
   for "nothing persists Y" (who calls the store on the relevant event?), the
   *definition* path for "no type/table for Z" (grep the struct/DDL, not the
   prose). Prefer a narrow write-path trace over re-reading the survey.

3. **Docs describe intent; code is ground truth.** A crate's `CONTRACT.md`,
   `openwiki/` page, or self-description states what it is *for* — it may
   under-state what it incidentally *does* (the identity crate calls itself a
   "resolver" and stores profiles as a side effect). `CLAUDE.md` already says to
   verify anything the knowledge graph asserts against live code; the same
   applies to prose docs. Read the implementation before asserting a negative
   the docs seem to imply.

## When you can't fully verify

Ship the claim with its uncertainty attached, not laundered out: "I found no
enumeration method on the resolver surface (searched `filesystem_store.rs` +
trait defs); a store may exist that I didn't trace." That lets the next step
target the gap instead of inheriting a false certainty. Do **not** upgrade "did
not find" to "does not exist" to sound decisive.
