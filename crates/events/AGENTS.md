# `crates/events/` — evidence, derived views, transport streams: a one-way pipeline

**Layer(s):** `substrates` (all 4 crates) · **Crates:** 4 · **May depend on:** `crates/contracts/` + the storage fabric (`ironclaw_filesystem`, store only) + one read-only `ironclaw_outbound` edge (streams only) · **Depended on by:** producers across kernel/loop/domains/product record through `ironclaw_event_log` (14 workspace crates name it — `grep -rl '^ironclaw_event_log = ' --include=Cargo.toml crates`); consumers read through `ironclaw_event_projections`; composition constructs via `ironclaw_event_store`; `ironclaw_assistant` subscribes via `ironclaw_event_streams`.

## What this family is

The system's record of what happened, kept structurally distinct from what a
screen shows right now. It is a one-way pipeline, four stages deep: a producer
emits a redacted event or audit envelope; `ironclaw_event_log`'s traits accept
it; `ironclaw_event_store` appends it durably under a monotonic cursor;
`ironclaw_event_projections` folds the log into scoped, metadata-only read
models; `ironclaw_event_streams` authorizes and admits a subscription over
those projections. **Nothing runs backward**: a projection cannot mutate the
log it was folded from, a stream cannot invent state that did not arrive by
replaying one, and a dependency arrow pointing backward through
evidence → store → projection → stream is always wrong, regardless of what any
single crate's local rules permit.

The admission list, complete: redacted evidence vocabulary and log traits;
durable backend selection with fail-closed profile validation; replay-derived
read models; admission-checked stream delivery. Those four stages, and nothing
else.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_event_log`](./ironclaw_event_log) | Redacted event/audit vocabulary and the sink/log traits every producer records through — no storage driver | you record a fact, or you define what a recordable fact looks like |
| [`ironclaw_event_store`](./ironclaw_event_store) | Durable backend selection, fail-closed production-profile policy, and the concrete log adapters — the family's only DB/TLS cone | composition needs a durable log constructed, or a backend adapter needs work |
| [`ironclaw_event_projections`](./ironclaw_event_projections) | Replay-derived, metadata-only read models with scope/cursor/rebase semantics — provably non-writing | a consumer needs "what happened" folded into a typed view |
| [`ironclaw_event_streams`](./ironclaw_event_streams) | Transport-neutral stream manager: authorization, RAII admission, live/replay stitching, redaction validation — never sends | a subscription must be authorized, admitted, and fed without knowing its transport |

## What never belongs here

- **SSE, WebSocket, webhook framing, or any concrete transport** → the product
  family (`ironclaw_webui` and friends). `ironclaw_event_streams` stops at a
  transport-neutral subscription; framing is the subscriber's job.
- **Product view assembly, command handling, presentation** →
  `ironclaw_assistant`. A projection has no opinion about how a screen renders
  or what a user may do next.
- **A second write authority** → nowhere. No projection or stream may write
  back into a durable log or hold a store of its own; "cleanup" of durable
  rows is not a concept this family offers. Only `ironclaw_event_store` writes,
  and only through `ironclaw_event_log`'s traits.
- **Storage drivers outside the store** → `ironclaw_event_store` is the one
  crate permitted the DB/TLS cone, and even there the driver type may appear
  only inside its private `postgres_backed` module body (gate below). Every
  other crate in the family compiles no database and no TLS stack.
- **Domain record grammar** → `crates/domains/`. This family owns the
  cross-cutting fact stream, not a subject: no thread record, no trigger
  record, only the redacted shape of "a thing happened".
- **Raw secrets, host paths, tool input/output, approval reasons, invocation
  fingerprints, provider errors, lease material** → nowhere, in any persisted
  or streamed shape. Redaction happens at construction in
  `ironclaw_event_log`; `ironclaw_event_streams` re-validates and fails closed
  before anything crosses toward a subscriber.
- **Vendor names** → packages/providers/operator
  (`reborn_extension_specificity.rs` scans this family like any other).

## The rules, and what enforces them

All gates live in `crates/app/ironclaw_architecture_tests` (run:
`cargo test -p ironclaw_architecture_tests`).

- **Layer matrix.** Every crate here declares
  `[package.metadata.ironclaw] layer = "substrates"`; the matrix
  (`reborn_dependency_boundaries.rs`) forbids naming kernel, loops, products,
  or app. The family directory is ownership only; the layer metadata is the
  enforced truth.
- **Per-crate deny rules**
  (`reborn_dependency_boundaries.rs::boundary_rules()`): `ironclaw_event_log`,
  `ironclaw_event_projections`, `ironclaw_event_streams`, and
  `ironclaw_event_store` each carry a `BoundaryRule`. The structural teeth:
  projections may name neither `ironclaw_event_store` nor
  `ironclaw_filesystem` — "projections never write" is enforced by what the
  crate is permitted to link, not by review; streams may name neither
  `ironclaw_event_log` nor `ironclaw_event_store` nor `ironclaw_filesystem` —
  it reaches evidence only through projections.
- **Driver containment** (`reborn_persistence_driver_boundary.rs`):
  `only_chartered_crates_link_the_postgres_driver`,
  `only_chartered_crates_link_the_other_persistence_drivers`, and
  `event_store_names_the_driver_only_inside_its_private_backend_module` — the
  last one scans every `src/` file of the store minus the brace-matched private
  module body, so a driver mention anywhere else in the crate fails.
- **Same-layer edges** (`reborn_same_layer_edge_inventory.rs`): exactly five
  events-owned edges are pinned — `event_store → {event_log, filesystem}`,
  `event_projections → event_log`, `event_streams → {event_projections,
  outbound}`. A new same-layer edge fails; so does a stale one.
- **Watch vs push are two decisions.** `ironclaw_event_streams` authorizes a
  subscription independently of whether `ironclaw_outbound` has authorized a
  push; its outbound dependency is a single read-only candidate lookup. A
  write path into outbound from this family is always a defect.

## Crossing out of this family

- **`crates/contracts/` (down):** the identity/scope/audit vocabulary this
  family is typed on (`ironclaw_host_api` — ids, `ResourceScope`, audit
  stages, turn refs).
- **`crates/substrates/ironclaw_filesystem` (sideways, store only):** the
  storage fabric the durable adapters append through.
- **`crates/domains/ironclaw_outbound` (sideways, streams only):** the one
  read-only push-candidate lookup.
- **`crates/app/ironclaw_composition` (consumer):** the assembly root is the
  one place `ironclaw_event_store`'s backend-selection factory is called to
  construct the logs everyone else receives by injection.
- **Producers and readers everywhere else** enter through
  `ironclaw_event_log`'s traits and `ironclaw_event_projections`' services —
  never by parsing durable rows.

## Sources

- Design record: [`docs/reborn/target-architecture/families/events.md`](../../docs/reborn/target-architecture/families/events.md);
  PROPOSAL §6.3 (per-crate dispositions), §8 (dependency model).
- Contracts (frozen): `docs/reborn/contracts/events.md`,
  `docs/reborn/contracts/events-projections.md`.
- Conventions this file follows: [`docs/reborn/guidance-conventions.md`](../../docs/reborn/guidance-conventions.md).
- Moving a crate between families is not a rename — the family word never
  enters the crate name (PROPOSAL §5.1).
