# `crates/loop/` — replaceable loop userland; ports are the membrane, never authority

**Layer(s):** `loops` (all four crates) · **Crates:** 4 · **May depend on:** `contracts/`, `substrates/`, `events/`, `domains/`, `kernel/`, siblings · **Depended on by:** `ironclaw_composition` (app), `ironclaw_extension_host` (loops sibling, extensions family), and — measured, downward — `ironclaw_assistant` on `ironclaw_loop_host` (PROPOSAL §6.10.1's 6-file/4-seam edge)

## What this family is

The loop-hosting tier: agent behavior that can be replaced wholesale without
touching anything privileged, plus the adapters that connect it to the kernel.
**The loop trust story:** a shipped loop is not trusted merely because it ships
with the product — nothing in this family constructs a capability grant, an
approval, a lease, or a secret. Every privileged effect crosses an
`ironclaw_loop_contracts` port into the kernel, which mediates it and hands
back only the sealed, redacted outcome; a loop's claimed exit is validated
against host-minted evidence before anything durable commits.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_agent_loop`](./ironclaw_agent_loop) | The sealed strategy-and-executor framework: loop families, planner, canonical executor, resumable state | You are changing what a turn *decides* to do next |
| [`ironclaw_loop_host`](./ironclaw_loop_host) | Base implementations of every `Loop*Port` over kernel services, incl. the model gateway and tool disclosure | You are adapting a kernel/domain service to a loop port |
| [`ironclaw_turn_runner`](./ironclaw_turn_runner) | The agent-turn executor, driver registry, and loop-host factory — the trusted adapter between a kernel work claim and loop userland | You are changing how a claimed run gets its drivers, ports, or exit disposition |
| [`ironclaw_hooks`](./ironclaw_hooks) | Trust-tiered hook middleware wrapping every `Loop*Port` call with policy and audit | You are changing hook trust classes, decision sinks, or the hook engine |

## What never belongs here

- **Any authority decision** — trust, authorization, approval, resource
  reservation, dispatch. Those are kernel-family acts
  (`crates/kernel/ironclaw_{trust,authorization,approvals,resources,capabilities}`).
- **Scheduling policy and lifecycle authority** — claim, lease, heartbeat,
  recovery belong to the kernel's process journal
  (`crates/kernel/ironclaw_processes`). Since #6696 the runner's
  `turn_scheduler.rs` is an agent-turn *projection* over the generic
  `ProcessSupervisor`; the kernel reaches this family only through the
  registered `ProcessKind::AgentTurn` executor port.
- **Product workflow** — binding, idempotency, command grammar, delivery
  semantics are `crates/product/ironclaw_assistant`'s. Product reads a claimed
  run's projected outcome; it never reaches in here for behavior.
- **Vendor protocol of any kind** — packages
  (`crates/extensions/packages/*`) and the sanctioned provider surfaces own
  vendor names. The one provider client in this family is `ironclaw_loop_host`'s
  model-gateway adapter, an exception by charter (PROPOSAL §6.7.2), and no
  other module may reach one — `ironclaw_turn_runner` deliberately no longer
  names `ironclaw_llm` at all (`reborn_runner_llm_wiring_is_isolated`).
- **Raw handles** — a raw secret, filesystem handle, network client, or
  process handle. Ports deliver scoped, mediated access only.
- **Raw content in resumable state** — state holds refs, cursors, counters,
  versions, and safe summaries; never a raw prompt, raw model output, tool
  argument, secret, host path, or provider diagnostic.

## The rules, and what enforces them

- **Layer matrix.** All four crates declare `layer = "loops"`; `loop/` may
  depend down (contracts → kernel) and never on `products` or `app`.
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_workspace_crates_declare_layers_and_follow_layer_matrix`
- **BoundaryRules** for `ironclaw_agent_loop` (contracts-only, the special
  matrix rule) and `ironclaw_hooks` (no runtime adapters or dispatcher
  concretions):
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_crate_dependency_boundaries_hold`
- **The sealed-strategy pattern is house style.** New loop behavior is a new
  strategy behind the sealed family-and-planner composition in
  `ironclaw_agent_loop`, never a branch inside the canonical executor.
- **The single declared `Loop*Port` decorator chain.** `ironclaw_loop_host`
  implements the base, kernel-facing adapter for every port;
  `ironclaw_turn_runner` composes that base into the concrete host for each
  claimed run (the *ordering* is the runner's); `ironclaw_hooks` wraps the
  composed host outermost. No other crate implements a `Loop*Port`. A new port
  implementation anywhere in the workspace joins this declared chain in the
  same change — never as a parallel, undeclared decorator. Port *definitions*
  live in `ironclaw_loop_contracts` alone, with one import path per port:
  `cargo test -p ironclaw_architecture_tests --test reborn_loop_port_location_scan`
- **The WS3 sheds stay shed.** The model gateway, model-route vocabulary,
  driver-host port adapters, and tool disclosure live in `ironclaw_loop_host`
  and must not reappear in the runner:
  `cargo test -p ironclaw_architecture_tests --test reborn_runner_sheds`
- **Prompt content for the loop tier lives in `ironclaw_loop_host/prompts/`**,
  never in the composition root:
  `cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries composition_root_embeds_no_prompt_content`
- **The dissolved ports crate's boundary survives as a module equality** —
  `loop_host`'s `skill_activation/` module may reach only what the old
  `ironclaw_first_party_extension_ports` crate could:
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries dissolved_ports_module_keeps_its_crate_boundary`
- **Same-layer edges are inventoried**, not free:
  `cargo test -p ironclaw_architecture_tests --test reborn_same_layer_edge_inventory`
- **A fifth crate must be earned.** It must name its own role in the
  claim-to-execution path and its own trust seal or multi-implementation port
  (families/loop.md); convenience alone does not warrant one.

## Crossing out of this family

- **Up to `ironclaw_composition` (app):** the only crate that assembles this
  family into a deployment; it calls runner/loop-host factories and registers
  the executor. Never depend back on it.
- **Down to `contracts/ironclaw_loop_contracts`:** to change a port's *shape*.
  Definitions live there; implementations live here.
- **Down to `kernel/`:** `ironclaw_turns` (turn admission, exit validation),
  `ironclaw_processes` (claims/leases/journal edges — see PROPOSAL §12.13 D-S
  for why the await-edge *store* is a journal projection while the *resolver*
  stays here), `ironclaw_host_runtime` and `ironclaw_capabilities` (the
  mediated services the port adapters wrap).
- **Sideways to `product/`:** never. Product consumes projections of what runs
  here; the reverse edge (`assistant → loop_host`) is measured product-side
  debt (PROPOSAL §6.10.1), not a license for `loop → products`.

## Sources

`docs/reborn/target-architecture/families/loop.md` · PROPOSAL §6.7.1–6.7.4,
§8, §12.13 D-S · gates: `crates/app/ironclaw_architecture_tests/tests/`
(`reborn_dependency_boundaries.rs`, `reborn_loop_port_location_scan.rs`,
`reborn_runner_sheds.rs`, `reborn_same_layer_edge_inventory.rs`,
`reborn_composition_boundaries.rs`) · conventions:
`docs/reborn/guidance-conventions.md`.
