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

## How the files actually load — measured, not assumed

This section exists because the first version of this convention got it wrong:
it made crate `AGENTS.md` canonical and `CLAUDE.md` a pointer, which moved the
working rules *out* of the path Claude Code auto-injects.

Measured 2026-08-06 with canary files and a discriminating control:

- Claude Code loads ancestor `CLAUDE.md` files at launch and **lazily injects a
  subtree `CLAUDE.md` when a file in that directory is read**. It does **not**
  read `AGENTS.md` natively.
- A **symlinked** nested `CLAUDE.md` injects its target's content. An
  `@AGENTS.md` import inside a nested `CLAUDE.md` also expands at lazy-load.
- Nested injection fires only for directories **below the session's cwd**, and
  appears **not to fire in subagent sessions at all**.

Two rules follow, and they are load-bearing:

1. **Wherever an `AGENTS.md` exists, a `CLAUDE.md` symlink sits beside it.**
   Same bytes, zero maintenance, no second document to drift. The root is the
   one exception: it keeps a real `CLAUDE.md` file because it carries a genuine
   Claude-only tail, so it uses `@AGENTS.md` plus that tail.
2. **Every document must stand alone when read deliberately.** Auto-injection is
   a convenience for interactive sessions, not a guarantee — subagents and
   out-of-tree reads get nothing. Never write a family or crate doc that only
   makes sense because something else was auto-loaded.

## Size budgets

Auto-loaded content is a tax on every turn; routed content is not. Budgets, not
hard gates — exceed one deliberately and say why:

| Tier | Budget |
|---|---|
| Root `AGENTS.md` | ≤200 lines |
| Family `AGENTS.md` | ≤150 lines |
| Crate `AGENTS.md` | ≤80 lines — and only where the crate has a **named trap** |
| `.claude/rules/*.md` firing on a broad glob | ~10KB for the set; narrow the glob before adding |

A line earns its place by answering: *would removing this cause a mistake?* and
*can the tree answer this itself?* If the tree can, state the command instead.

## Scope

This convention governs `crates/**` guidance **and** `.claude/rules/*.md`,
`.claude/skills/*/SKILL.md`, and the root pair. The rules layer is in scope
because that is where the worst drift was found: a rule whose `paths:` trigger
named a nonexistent file never fired at all.

## What enforces this

`scripts/ci/check-guidance.py` asserts that every path referenced in guidance
resolves, every rule/skill `paths:` glob matches at least one tracked file,
every crate appears in its family's table, and every crate has a `README.md`.
Mark a deliberately historical reference with the dated-correction glyph `✎` or
`<!-- check-guidance: path-ok -->`; its `KNOWN_MISSING` table is a last resort
and shrinks only.

⚠ **Some guidance is parsed by tests.** Before editing, run
`rg -l '<filename>' crates/app/ironclaw_architecture_tests/tests crates/*/*/tests scripts/ci`.
Two traps found the hard way: `ironclaw_wasm`/`ironclaw_mcp` pin exact phrases
including retired vocabulary that must **not** be "cleaned up"; and prose that
names a machine-parsed heading verbatim can shadow the parser's split anchor
(an `ironclaw_auth` charter test went red for exactly this).

## When you remove or rename a crate

The mirror of the checklist below: delete its `README.md`, `AGENTS.md` and the
`CLAUDE.md` symlink beside it, remove its row from the family table and from
`crates/AGENTS.md`, drop any Module Specs row, and re-run
`check-guidance.py` — the family-table and README checks fail closed on a crate
the tree still has and the docs forgot, and on a doc row whose crate is gone.

## When you add a crate

A new crate lands with its `README.md`, its family's `AGENTS.md` crate table
updated, its row in `crates/AGENTS.md`, and its `[package.metadata.ironclaw]
layer`. `scripts/ci/check-target-tree.py` fails the build if the package set and
the documented tree disagree, so the tree half is enforced; the guidance half is
this convention's job.
