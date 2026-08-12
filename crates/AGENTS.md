# `crates/` — the agent routing map

Everything under `crates/` is the IronClaw Reborn production stack: one Cargo
workspace, ten family directories, no legacy tier. This file routes you to the
right family in one hop. It deliberately holds no per-crate rules — family and
crate documents own those (the convention is
`docs/reborn/guidance-conventions.md`), and this map never restates them.

Derived from the live tree on 2026-08-05 (`cargo metadata --no-deps`,
`python3 scripts/ci/check-target-tree.py`). Re-derive any number here with
those commands before trusting it in a later month.

## Read order

1. **This map** — pick the family for the change.
2. **`crates/<family>/AGENTS.md`** — the family boundary: what it holds, what
   never belongs there, its crate table, and the gates that enforce it.
3. **`crates/<family>/<crate>/README.md`** — what the crate is, when you want
   it, when you want a different one. Every crate has one.
4. **Crate working rules** — `crates/<family>/<crate>/AGENTS.md` where the
   crate has rules beyond orientation, and the module spec (`CONTRACT.md`)
   for crates in the root `AGENTS.md` Module Specs table. Code
   follows spec; spec is the tiebreaker.
5. **Cross-crate behavior** — `docs/reborn/contracts/*.md` (source-of-truth
   contracts) and `docs/reborn/target-architecture/` (the design record:
   frozen `PROPOSAL.md` with dated amendments, `families/*.md` specs, live
   `CHECKLIST.md`).

Do not eagerly load every crate guide. Route, then read.

## The ten families

| Family | One line | A change belongs here when |
| --- | --- | --- |
| [`contracts/`](./contracts/AGENTS.md) | Neutral vocabulary and ports — the leaf tier; nothing executes, persists, or names a vendor. | You are adding or changing a shared type, identity, port trait, or DTO that more than one tier must see. |
| [`substrates/`](./substrates/AGENTS.md) | Privileged mechanisms the kernel mediates: filesystem, libSQL admission, secrets, network, safety scanning, observability macros. | You are changing storage/network/secret/scanning *mechanism*, not who may use it. |
| [`events/`](./events/AGENTS.md) | What already happened: redacted evidence vocabulary, durable stores, replay-derived projections, admission-checked streams. | You are changing event vocabulary, event persistence, a read model, or stream delivery. |
| [`domains/`](./domains/AGENTS.md) | Typed record/service owners behind the kernel: threads, conversations, triggers, memory, skills, auth, attachments, extractors, identity, llm, trace_commons, outbound, web_app. | You are changing a domain's record grammar, service contract, or invariants. |
| [`kernel/`](./kernel/AGENTS.md) | The authority perimeter, one crate per mediation stage: trust → authorization → approvals → resources → runtime_policy → capabilities → processes → turns → host_runtime. | You are changing what is *allowed to happen* or how recovery stays safe. |
| [`lanes/`](./lanes/AGENTS.md) | Execution for already-authorized work: wasm, wasm_limiter, mcp, sandbox. | You are changing how an approved invocation physically runs. |
| [`loop/`](./loop/AGENTS.md) | Replaceable agent behavior and its hosting: agent_loop, loop_host, turn_runner, hooks. | You are changing what the agent decides next, or the drivers/port adapters that host it. |
| [`extensions/`](./extensions/AGENTS.md) | Everything "installable package": manifest registry, generic host, product-side manager, shared support, and `packages/` — one self-contained directory per package, the only place vendor names may appear. | You are adding or changing an integration, its manifest, or extension lifecycle machinery. |
| [`product/`](./product/AGENTS.md) | First-party userland above the kernel: assistant, operator, openai_compat, webui, host_ingress. | You are changing user-facing behavior, a transport, or ProductSurface orchestration. |
| [`app/`](./app/AGENTS.md) | Assembly and enforcement: composition, the binary `ironclaw`, boot config, architecture tests. | You are wiring services/backends together or arming a workspace-wide gate. |

Three sound alike, and the cut is the model: **domains** are what the system
*knows* (records), **loop** is what the agent *decides* (untrusted strategy),
**lanes** are how an approved action *runs* (post-authorization mechanism).
The long-form runtime narrative is `crates/Architecture.md`; the human
inventory is `crates/README.md`; the design-record walkthrough is
`docs/reborn/target-architecture/README.md`.

## The layer matrix — who may depend on whom

The mechanically enforced dependency truth is each crate's
`[package.metadata.ironclaw] layer` key, checked by
`reborn_workspace_crates_declare_layers_and_follow_layer_matrix` in
`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`.
Seven layers, strictly ordered; a crate may take normal dependencies only on
its own layer or below (dev-dependencies are outside the matrix):

| Layer (low → high) | May depend on | Crates today |
| --- | --- | --- |
| `contracts` | contracts | 6 |
| `substrates` | contracts, substrates | 28 |
| `runtimes` | + runtimes | 5 |
| `kernel` | + kernel | 9 |
| `loops` | + loops | 5 |
| `products` | + products | 8 |
| `app` | + app (everything) | 5 |

The standing-exception list (`LAYER_MATRIX_EXCEPTIONS`, same file) is
**empty** — measured 2026-08-05 — and a ratchet test in that file fails the
build if it grows.

**A family directory is never a compilation or trust unit.** Family placement
is ownership and discoverability; the layer key is what CI checks, and the
two do not always rhyme:

- `events/` and `domains/` crates are all `substrates`-layer.
- `lanes/` crates are `runtimes`-layer; `loop/` crates are `loops`-layer.
- `extensions/` is deliberately *vertical*: registry = substrates, support =
  runtimes, host = loops, manager = products; under `packages/`, the channel
  adapter crates (slack, telegram, web-app) are products and the memory provider
  crates (memory-native, mem0) are substrates.
- Two placement surprises: `product/ironclaw_host_ingress` and
  `app/ironclaw_config` are `substrates`-layer.

Before adding a cross-family import, read the *target* family's `AGENTS.md`
exclusion list, then run the architecture suite below. Family `AGENTS.md`
files carry their members' exact layers.

## Workspace facts

**66 packages**: 64 under `crates/`, plus the root package
`ironclaw_integration_tests` (the in-process Reborn integration suite,
`tests/integration/`) and `tools/ironclaw_stress`. One documented exclusion:
`tools/ironclaw_silk_decoder`, a standalone helper that is
workspace-`exclude`d. Zero crates sit flat under `crates/` and zero owned
placement exceptions remain. The gate is
`python3 scripts/ci/check-target-tree.py`, which compares the workspace
against the documented tree (PROPOSAL §5); on 2026-08-05 it reports:
`target tree: OK (66 workspace members against 66 documented packages, 1
documented exclusion(s), 0 owned exception(s))` (re-derived 2026-08-08 with the
web-app channel's two crates).

Under `crates/extensions/packages/`, 15 package directories: 5 are workspace
crates (`slack`, `telegram`, `web-app`, `memory-native`, `mem0`) and 10 are data-only
(manifest + prompts/schemas, some with prebuilt WASM): github, gmail, the
five google-*, nearai-mcp, notion-mcp, web-access. Every package directory —
data-only ones included — carries its own `README.md`, so the read order
above applies at package level too: `crates/extensions/AGENTS.md` →
`crates/extensions/packages/<pkg>/README.md`.

Enumerate crate directories with `python3 scripts/ci/lib/crate_tree.py .` — a
flat `crates/ironclaw_*/` glob matches nothing.

## Build and test

Run from the repository root. Narrow first:

```bash
cargo test -p <crate_name>
cargo clippy -p <crate_name> --all-targets --all-features -- -D warnings
```

Then by risk:

```bash
cargo test -p ironclaw_architecture_tests   # dependency/layer/boundary gates
python3 scripts/ci/check-target-tree.py     # package set vs documented tree
scripts/reborn-e2e-rust.sh                  # deterministic Rust E2E gate (heavy);
                                            # groups: architecture-boundaries,
                                            # architecture-runtime, runtimes, substrates
```

Run the architecture suite whenever dependency edges, layer keys, crate
placement, or test-pinned guidance files change. Run the E2E script when
turns, runtime lanes, host services, authorization, approvals, networking,
secrets, ProductSurface behavior, or capability dispatch change. The full
workspace gate (`cargo fmt`, workspace clippy, `cargo test`) is the root
`AGENTS.md`'s; test tiers and the regression rules are
`.claude/rules/testing.md`.

> The legacy `check-boundaries.sh` script is deleted (measured 2026-08-05: it
> failed on a clean tree and its v1-targeted checks passed vacuously).
> Boundary enforcement for `crates/` is the architecture suite above.

## Cross-family change routes

Family `AGENTS.md` files own per-crate routing. Two whole-tree paths are
worth naming here because they cross most families:

- **A feature that reaches the user**: `product/` surface → kernel-mediated
  capability path → the domain that owns the record. Start from the
  `reborn-feature` skill; never bypass `ProductSurface` or `CapabilityHost`.
- **A new integration**: `extensions/packages/<name>/` with its manifest;
  host machinery stays generic in `extensions/`, wiring stays in `app/`.
  Start from the `reborn-extension-surfaces` skill.

## Guidance files, and which to trust

| File | Role |
| --- | --- |
| `crates/<family>/AGENTS.md` | Family boundary + crate table (one per family) |
| `crates/<family>/<crate>/README.md` | Crate orientation (one per crate) |
| `crates/<family>/<crate>/AGENTS.md` | Crate working rules (only where needed) |
| Crate `CONTRACT.md` | Module spec — the tiebreaker over code |
| `docs/reborn/contracts/*.md` | Cross-crate behavior contracts |
| `docs/reborn/target-architecture/` | Design record; guidance links it, never forks it |
| `openwiki/` | Generated prose wiki — never hand-edit |

One canonical home per fact. Where two documents disagree, the code and its
gates win, and both documents get a dated correction
(`docs/reborn/guidance-conventions.md`). Some guidance files are pinned by
tests or read by CI classifiers — before editing one, run
`rg -l '<file>' crates/app/ironclaw_architecture_tests/tests scripts/ci` and
the owning suite.
