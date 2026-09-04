---
name: architecture-diagram
description: Use when asked to generate, build, or update an interactive architecture diagram or explorer from a codebase — optionally overlaying a design doc's current→target transition (what collapses, what gets deleted, staged) and live refactor progress from merged/open PRs. Triggers include "architecture diagram/explorer/map", "visualize the architecture", "render the target architecture", "show the refactor/before→after as a diagram", "diagram the transition/progress".
---

# Interactive Architecture Diagram

Produces **one self-contained HTML explorer** — clickable nodes that drive an
inspector panel, optional refactor-status dots, a per-slice progress section, and
a "what gets deleted, when" demolition ledger. No external fonts/scripts/network:
it opens offline and is theme-aware.

The renderer is **data-driven**: you edit only JS data blocks in a bundled
template, so there is no HTML to hand-author and no tag-balance risk. The engine
draws the stack, inspector, dots, progress cards, and ledger from that data.

Template: `references/explorer-template.html` (relative to this skill). Copy it to
the output path, then fill the `── FILL IN ──` data blocks (`META`, `LAYERS`,
`DATA`, `STATUS`, `SLICES`, `PROGRESS`, `DEMOLITION`, `DEMO_META`). Empty an array
to hide its section (`SLICES = []` hides progress; `DEMOLITION = []` hides the
ledger; `STATUS = {}` hides the status overlay and gives a plain code map).

## Three input modes — do only the ones in scope

| Ask | Fill | Sections |
| --- | --- | --- |
| "diagram the architecture from the code" | `LAYERS`, `DATA` | diagram only |
| "…and show the target from this design doc" | + `DATA.note`, `DEMOLITION` | + demolition ledger, before→after |
| "…and where the refactor is / PR progress" | + `STATUS`, `SLICES` | + status dots, progress cards |

## Step 1 — Scope one subject

One system, one thesis (the single idea the diagram encodes), the layers it splits
into. Write that thesis into `META.heroTitle` (wrap the key word in `<em>` for the
accent). Do **not** diagram everything — pick the axis the request is about.

## Step 2 — Build the component model from the code (recipes, not a hardcoded map)

Derive `LAYERS` (bands, rails, strips) and `DATA` (per-node role/interface/types)
by *reading structure*, not from memory. Prefer the knowledge graph
(`CLAUDE.md` → "Query the Knowledge Graph First"); fall back to grep:

```bash
# Crates / top-level modules → candidate layers
ls crates/ ; sed -n '1,80p' crates/Architecture.md crates/AGENTS.md 2>/dev/null   # this repo's component map + routing
# Traits are the seams (interfaces/rails); one prod impl vs many tells you dyn-vs-enum
grep -rn "pub trait " crates/<crate>/src
grep -rc "impl .* for " crates/<crate>/src        # impl counts per trait → is it a real seam?
# Key types per node
grep -rn "pub struct \|pub enum " crates/<crate>/src | head
```

For **this repo**, orient with the `ironclaw-reborn-orientation` skill first, and
**exclude the v1 legacy enclave** from a Reborn diagram — `ironclaw_engine`,
`ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`, `ironclaw_embeddings` are
v1-only and do not belong in a Reborn flow (that skill is the tiebreaker on which
side a crate is on).

Node `kind` drives color: `Kernel`/`Capability` = accent, `Substrate`/`Interface`
= slate, `Product` = neutral, `Lane` = red (untrusted). Use `variant:"kernel"` for
the slow-moving core band and `variant:"untrusted"` for anything running untrusted
code.

## Step 3 — (optional) Extract the current→target diff from a design doc

If a design doc is in scope, read it and mine three things:

- **before→after** — its own reduction table becomes `PROGRESS.stats` / node
  `DATA.note` (`cls:"kept"` for what's preserved, `cls:"gone"` for what's removed).
- **what gets deleted, staged** — its migration plan (often "§9 slices") becomes
  `DEMOLITION`: one `stage` per slice; each row is `{removed, mag, becomes, gate}`
  where **gate = the condition that lets the deletion happen** (the "delete only
  once its replacement carries the load" ordering). **Measure `mag` from the tree**,
  not the doc's estimate — code moves:
  ```bash
  wc -l <file-of-the-doomed-type>                      # LOC of a store/DTO family
  grep -rc "<TypeName>" crates/*/src                    # blast radius of a type
  find crates/<crate>/src -name '*.rs' | xargs wc -l | tail -1   # a crate's current size
  ```
- **enforcement ratchets** — if the doc defines allowlist ratchets ("§10"), their
  frozen lists are the authoritative *remaining* counts (Step 4).

## Step 4 — (optional) Map progress from PRs + ratchets

Refactor PRs usually tag the design-doc section/slice in their **title** (e.g.
`Slice C.1`, `§4.3`, an umbrella issue `#NNNN`). Map by parsing titles:

```bash
gh pr list --state merged --author <user> --limit 80 \
  --json number,title --jq '.[] | "\(.number)\t\(.title)"'
gh pr list --state open   --author <user> --limit 40 \
  --json number,title --jq '.[] | "\(.number)\t\(.title)"'
gh repo view --json nameWithOwner --jq .nameWithOwner        # → META.repo for PR links
```

Bucket the PR numbers into each `SLICES[].merged` / `.open`, and set node
`STATUS[id]` = `done`, `wip`, or `planned`. **The bar for `done` is that the
replacement is *load-bearing* — not that a PR merged, a type exists, or a ratchet
went green** (`.claude/rules/discovery-claims.md`). Three checks, in order:

```bash
# 1. the new thing exists
grep -rl "pub struct Authorized\|enum RuntimeLane" crates/<crate>/src
# 2. the old thing is gone or provably unused — a rename is NOT a collapse
grep -rl "OldMirrorDto\|LocalDev" crates/*/src
# 3. THE decisive one — is the replacement actually consumed at the call sites it
#    was meant to replace? grep for the branch/handler/DTO it should have removed:
grep -rc "RuntimeProfile::\|match profile\|fn build_local_dev_" crates/<composition>/src
```

If check 3 still finds the old wiring, the axis is `wip`, not `done`, **however
green the ratchet is** (see the guardrail below — this exact trap bit this skill's
first run). Remaining debt per axis = the count still in the frozen ratchet
allowlist:

```bash
awk '/const FROZEN_[A-Z_]*:/,/\];/' crates/ironclaw_architecture/tests/<ratchet>.rs \
  | grep -oE '"[A-Za-z0-9_]+"' | wc -l
```

## Step 5 — Populate the template

```bash
cp .claude/skills/architecture-diagram/references/explorer-template.html <output>.html
```
Choose `<output>`: beside the design doc it visualizes (e.g. `docs/reborn/<doc>-explorer.html`)
or a path the user names. Edit only the JS data blocks. The file is git-untracked
until someone commits it; say so in the handoff.

## Step 6 — Verify before handing off

```bash
node --check <(sed -n '/<script>/,/<\/script>/p' <output>.html | sed '1d;$d')   # JS parses
# every LAYERS/STATUS id has a DATA entry (nodes are rendered from data, so parse the data blocks):
python3 - <output>.html <<'PY'
import re,sys
h=open(sys.argv[1]).read()
blk=lambda a,b:(lambda s:s[:s.find(b) if b in s else len(s)])(h[h.index(a):])
data=set(re.findall(r'\n  ([A-Za-z0-9_]+):\s*\{', blk('const DATA','const STATUS')))
ids =set(re.findall(r'id:"([^"]+)"', blk('const LAYERS','const DATA')))
stat=set(re.findall(r'\n  ([A-Za-z0-9_]+):\s*\{', blk('const STATUS','const SLICES')))
print("LAYERS ids missing from DATA:", sorted(i for i in ids  if i not in data))
print("STATUS ids missing from DATA:", sorted(k for k in stat if k not in data))
PY
```
Both lists must be empty. Then open it (`open <output>.html`) or drive it with the
browser tools to eyeball both themes and that clicking a node fills the inspector.

## Design + honesty guardrails

- **One accent** (the core/kernel hue); slate is for interfaces/ports as
  *information*; semantic green/red only for preserved-vs-removed. Don't spend the
  accent twice. (The template already encodes this — keep it.)
- **Label relocated vs deleted.** In the ledger, code that *moves out of a crate*
  is not the same as code *deleted*; say which. Don't inflate a "LOC removed" stat
  with relocations.
- **Pips are indicative groupings; ratchet counts are authoritative.** If you show
  both, let the numbers be the ratchet counts.
- **A green ratchet proves NAMES, not BEHAVIOR — this is the load-bearing lesson.**
  A type-name / rename / boundary ratchet passing (or an allowlist reaching 0)
  means the *names* are gone, never that the *wiring* collapsed. Real miss
  (2026-07-18): the `LocalDev*` type-name ratchet hit 0 and a `DeploymentConfig`
  type existed, so the "Local\* → config data" axis was marked **done** — but
  composition still had **39 `RuntimeProfile::` refs, 5 `match profile` sites, 11
  `build_local_dev_*` fns**, and `DeploymentConfig` only *resolved a mode to
  policy*, it did not select substrate backends as data (the doc's §5.6 shape).
  The types were **renamed** to shared families, not collapsed to config. Rule:
  mark an axis `done` only after check 3 above (old call sites/branches gone),
  and in the ledger distinguish "renamed" from "collapsed" and "resolves a value"
  from "replaced the setup."
- **A type existing ≠ it being wired.** New vocabulary often lands *additively* —
  defined, frozen by a ratchet, unit-tested — while nothing returns or consumes it
  yet. Same run (2026-07-18): `Invocation`/`Authorized`/`Outcome`/`Resolution`
  existed in `host_api`, but production still returned the old types, `Authorized`
  was minted-then-discarded, and `authorize()` was a delegating scaffold. For a
  vocabulary/return-type migration, grep the *return path* (`grep -rn "-> .*OldType"`
  and "who returns `NewType`?"), not just the type definition, before green.
- **A security fix can be real while the doc's named API is aspirational.** Verify
  the *protection* (e.g. the resolver fails closed), and when the mechanism differs
  from the doc's sketch, mark it done-with-a-note, not done-against-the-doc's-types.
- **No fabricated progress.** Every `STATUS`/`SLICES` claim traces to a real PR
  number or a grep against HEAD. "Did not find a PR" ≠ "planned by design" — mark
  uncertainty rather than laundering it (`.claude/rules/discovery-claims.md`).
- The point-in-time nature is real: stamp `META.asOf` and note that dots reflect
  that date.

## Provenance (dated worked example)

Built this way on 2026-07-18 for `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`
→ `docs/reborn/2026-07-17-architecture-simplification-explorer.html` (all three
modes: code model + design-doc transition + PR/ratchet progress + demolition
ledger). Read it as an exemplar of a fully-populated dataset; it is a living copy,
not a spec — re-derive facts with the recipes above rather than trusting its
numbers.

The same run produced the "names, not behavior" guardrail: an axis was marked
`done` off a green ratchet and had to be corrected to `wip` after a reviewer asked
"where is `DeploymentConfig` actually replacing the setup?" and the answer was
"nowhere yet." When you regenerate a progress diagram from a design doc, treat the
doc's own "done / landed" language and its ratchet claims as **unverified** — run
check 3 (old call sites gone?) for every axis before painting a node green.
