# ironclaw_architecture_tests

The workspace's architecture-contract test suite — the mechanism that fails
the build whenever a crate's dependency graph or public surface drifts from
the declared layer and family model. Every boundary rule the design record
claims is only as real as a contract test here: the layer ladder, per-crate
`BoundaryRule`s, contracts-purity allowlists, sealed-evidence mint pins, the
persistence-idiom rule, transport residue freezes, module-charter gates, and
the ratchet self-tests. It is the enforcement mechanism, not a participant —
folding these tests into a crate they police would let that crate pass its own
boundary checks.

- **Family / layer:** `app` / `app` · **Package:** `ironclaw_architecture_tests` · **Manifest:** `crates/app/ironclaw_architecture_tests/Cargo.toml`
- **Use this when:** arming a boundary as a mechanical test, re-deriving a
  `BoundaryRule` after a sanctioned edge change, or adding a ratchet.
- **Don't use this when:** you want production code, a runtime behavior, or a
  type another crate imports — none of that may live here; and a change that
  makes a boundary test easier to pass, rather than the design more correct,
  is the failure mode to refuse.

## Public surface

None — nothing in the workspace imports it. `src/lib.rs` is 4 lines; the
crate is its `tests/` directory (37 top-level gate files at last count —
`ls crates/app/ironclaw_architecture_tests/tests/*.rs | wc -l`), with shared
scanners in `tests/ratchet_support/` (the crate-inventory resolver that lets
path-keyed gates survive family moves, pinned by `reborn_crate_inventory.rs`).

## Depends on / consumed by

- **Normal workspace deps (0).** One **dev-only** vocabulary import
  (`ironclaw_host_api`) — needed to pin one allowlist against its owning
  crate's real definition rather than a copy. A production dependency here
  would make the crate's own zero-production-dependency claim false.
- **Consumed by:** nothing. CI runs it
  (`cargo test -p ironclaw_architecture_tests`).

## Invariants

- Test-only: no production code, no runtime role.
- Gates fail **loudly** with the exact forbidden edge and crate name, and fail
  **closed** when a scanned root resolves to nothing (the WS0 "inert guard"
  lesson — every scanner carries a sabotage self-test; see
  `reborn_ratchet_support_scanners.rs`).
- Enforcement reads the workspace's declared structure (`cargo metadata`) and
  source text; it does not link the crates it polices.
- Ratchets are one-directional and re-baselined in the same PR as the change
  they govern; read counts off the ratchet's own failure output, never by eye.

## Tests

```bash
cargo test -p ironclaw_architecture_tests                    # the whole suite
cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries
cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries
```

## See also

Working rules: `AGENTS.md` · family rules: `crates/app/AGENTS.md` · design
record: `docs/internal/reborn/target-architecture/families/app.md` (§6.10.4) + PROPOSAL
§11 (the enforcement additions).
