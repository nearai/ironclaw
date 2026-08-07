# Guidance conventions — what documents a crate, a family, and the tree

**Status:** normative for `crates/**` guidance. Written 2026-08-05 alongside the
family-guidance program that followed the target-architecture restructure.

The restructure gave the tree ten families and a documented boundary for each.
This document says where that knowledge lives, so the next reader — human or
agent — can find the answer in one hop and so two files never state the same
rule twice.

## The four documents

| File | Audience | Answers | Required? |
|---|---|---|---|
| `crates/<family>/AGENTS.md` | both | *What is this family, what may it hold, what may it never hold, what enforces that* | Yes — one per family |
| `crates/<family>/<crate>/README.md` | both | *What is this crate, when do I want it, when do I want a different one* | Yes — one per crate |
| `crates/<family>/<crate>/AGENTS.md` | agents | *Working rules: invariants, traps, the gates, how to test* | Only when the crate has rules beyond orientation |
| `crates/<family>/<crate>/CLAUDE.md` or `CONTRACT.md` | both | The crate's **module spec** where one exists | Only for crates in the root `CLAUDE.md` Module Specs table |

`crates/AGENTS.md` is the routing map into the families; `crates/README.md` is
the human map. Neither restates a family's rules — they point.

## The rules that keep this from rotting

1. **One canonical home per fact.** If a rule is in the family `AGENTS.md`, the
   crate files link to it rather than repeat it. Where a crate today has both an
   `AGENTS.md` and a `CLAUDE.md` saying the same thing, consolidate into one and
   leave the other as a pointer. Two copies drift; they always have.
2. **Measured, not aspirational.** Every claim — a charter, a dependency, a
   consumer count, an invariant — is derived from the tree at writing time and
   is reproducible by a command a reader can run. Prefer naming the enforcing
   test over asserting the rule.
3. **Boundaries are stated as exclusions.** "What never belongs here, and where
   it goes instead" is the sentence that makes a boundary usable. A family
   document without an exclusion list has not done its job.
4. **Guidance is not inert.** Some guidance files are pinned by tests (route
   tables, module-charter maps, contract locks). Before editing one, check
   `rg -l '<file>' crates/app/ironclaw_architecture_tests/tests crates/*/*/tests`
   and run the owning crate's suite.
5. **Point at the spec, don't fork it.** `docs/reborn/target-architecture/`
   is the design record (`PROPOSAL.md` frozen + dated amendments, `CHECKLIST.md`
   live state, `families/*.md` per-family specs). Guidance links into it; it
   never gets copied, and where guidance and the design record disagree, the
   **code and its gates win** and both documents get a dated correction.
6. **`openwiki/` is generated.** Never hand-edit it.

## Family `AGENTS.md` — the shape

```markdown
# `crates/<family>/` — <the boundary in a half-line>

**Layer(s):** … · **Crates:** N · **May depend on:** … · **Depended on by:** …

## What this family is
Two or three sentences describing the *boundary*, not the inventory.

## The crates
| Crate | Charter (one line) | Go here when |

## What never belongs here
Bulleted exclusions, each naming where it goes instead.

## The rules, and what enforces them
The layer-matrix row, the family's BoundaryRules, the armed gates by test
name — each runnable.

## Crossing out of this family
Upstream/downstream neighbours and the one reason you'd cross to each.

## Sources
`docs/reborn/target-architecture/families/<family>.md`, PROPOSAL §, gates.
```

## Crate `README.md` — the shape

```markdown
# <package name>

One paragraph: what it is and why it exists as its own crate.

- **Family / layer:** … · **Package:** … · **Manifest:** `crates/…/Cargo.toml`
- **Use this when:** …
- **Don't use this when:** … → use `<crate>` instead

## Public surface
The entry points that matter — main traits, types, factory functions.

## Depends on / consumed by
Measured workspace edges, and the reason for any that would surprise a reader.

## Invariants
Only enforced ones, each citing its gate or test.

## Tests
The exact commands.

## See also
Family `AGENTS.md`; the module spec or `CONTRACT.md` if the crate has one.
```

## When you add a crate

A new crate lands with its `README.md`, its family's `AGENTS.md` crate table
updated, its row in `crates/AGENTS.md`, and its `[package.metadata.ironclaw]
layer`. `scripts/ci/check-target-tree.py` fails the build if the package set and
the documented tree disagree, so the tree half is enforced; the guidance half is
this convention's job.
