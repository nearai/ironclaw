# LFD Brief: memory-placement-product-layer — Memory placement (product/provider boundary)

**State**: built (`ironclaw_memory` provider-neutral contract crate +
`ironclaw_memory_native` provider — the #3537 lift already split the
`MemoryService` trait from the native impl). This LFD hardens the
product/provider BOUNDARY, host/admin policy mediation, and native-vs-fake
parity. **Bar**: 0.95 holdout, architecture/dependency tests green, zero
host-mediation bypasses. **Profile**: `memory_placement`.

## Outcome

A product-facing memory provider boundary where native memory is the default
provider but host/admin policy owns provider authorization (allow / deny /
constrain). Product code never special-cases native memory: a fake provider
swapped in behaves identically. Existing read/write/search/prompt behavior
stays behaviorally equivalent through the boundary; audit, auth, sandbox,
storage, streams, and network stay host-mediated; the forbidden dependency
direction (product code reaching native internals) does not exist.

## Spec sources

- `docs/reborn/contracts/storage-placement.md` (§3 ownership-vs-mechanics,
  §4 namespace/source-of-truth map, §5.2 memory docs)
- `docs/reborn/contracts/memory.md` (§4 backend contracts, §5 service split,
  §9 events/audit) and `docs/reborn/contracts/memory-profiles.md`
  (`HostPortCatalog` is a validation catalog, not a runtime registry)
- `crates/ironclaw_memory/` (the `MemoryService` trait boundary) +
  `crates/ironclaw_memory_native/` (native provider, incl. `contract_tests.rs`);
  `crates/ironclaw_product_workflow/src/reborn_services.rs` (product surface)
- Retention/versioning contracts inherited from
  `lfd/_briefs/long-term-memory.md` — scored HERE only as boundary parity
  (write-side correctness is **scored in self-learning-write-pipeline**).

## Stage 0 inner suite

`ironclaw_memory` + `ironclaw_memory_native` crate tests (incl. the
`contract_tests.rs` per-impl trait suite) + `tests/integration/group_memory/`.
Green every cycle.

## Eval themes (dev ~44 / holdout ~14; goal's 50/120 are designer growth targets)

1. Provider boundary + native default (12): read/write/search/prompt-inclusion
   routed through the provider trait → the native provider services the op
   (state query proves the native backend was touched); the boundary is the
   only path (forbidden: a product symbol reaching a native internal —
   dependency matcher).
2. Fake-provider parity (10): a **pinned fake provider** swapped for native →
   the identical product-layer ops produce structurally equivalent results
   (`state_eq` on op-result shape, never on native internals). Product code
   must not branch on provider identity. **Load-bearing fence.**
3. Policy allow/deny/constrain (8): host/admin policy denies a provider → op
   fails closed (`status == error` / gate denial); constrains (read-only,
   no-embeddings) → constrained op honored; allow → runs. Both directions
   priced; failure-direction ≥ 25%.
4. Host mediation (8): each op emits its `memory.*` audit event; embedding
   egress goes through host `ironclaw_network`, never provider-direct;
   forbidden: provider-issued raw egress or an unsanitized host-path event.
5. Op-family parity (6): read/write/search/prompt each behaviorally
   equivalent to the pre-boundary baseline (state contract per family) —
   "preserve reads but break writes" caught by pricing all four.

## Feature-specific cheats → fences

- **Special-case native provider in product code** → the fake provider is
  pinned in `tests/integration/support/**` (NOT the lane profile, so the
  implementer can't special-case around it); parity cases run the identical
  product path against both providers; caps: provider-specific branches in
  product diff ≤ 5. (Mandatory per ADDENDA.)
- **Re-export old internals under a new provider name as a fake boundary** →
  dependency lint + caps: new `pub use` of `ironclaw_memory_native::`
  internals from product crates = 0.
- **Always-allow policy stub** → denied/constrained cases REQUIRE
  `status == error` / gate denial; a permissive stub fails the
  failure-direction group.
- **Preserve storage but drop audit** → paired required matchers: every
  mutating op REQUIRES both the persisted state change AND its `memory.*`
  audit event.
- **Move memory authority into prompts** → prompt-inclusion cases forbid a
  write originating from prompt assembly; boundary/leak scan over the
  assembled envelope.

## caps.json extras

Provider-specific branches in product diff ≤ 5 (goal); new `pub use`
re-exports of `ironclaw_memory_native` internals from product crates = 0;
native-distinctive symbols in fake-provider-path assertions = 0.

## Live mode

No live external provider (goal). 2 live cases: real model drives
read/write through the boundary against native AND the fake provider →
structural parity contracts only (op-result shape + audit event), no
provider-identity leakage. Spend ceiling $10.
