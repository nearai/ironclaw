# ironclaw_host_ingress

Host-owned HTTP route-mount vocabulary: carrier types that pair a prebuilt
Axum router with the `IngressRouteDescriptor` policy descriptors ingress uses
to layer authentication, rate limiting, and body limits around routes someone
else built. 107 lines, exactly one job — it exists so neutral contracts never
need a web-framework dependency (`axum` is on the contracts-tier denied list,
which is this crate's stated purpose).

- **Family / layer:** `product` / **`substrates`** (re-layered down by #7092 —
  the `products` assignment was inherited from the directory, not earned;
  families are discoverability groupings, not trust boundaries) ·
  **Package:** `ironclaw_host_ingress` · **Manifest:** `crates/product/ironclaw_host_ingress/Cargo.toml`
- **Use this when:** a crate needs to hand a composed router (plus its policy
  descriptors) across a crate boundary without pulling in a listener's
  dependency cone.
- **Don't use this when:** defining route policy vocabulary → that is
  `ironclaw_host_api::ingress`; binding a listener, enforcing auth, or
  building middleware → `ironclaw_webui`.

## Public surface

- `PublicRouteMount` — a public sub-router + descriptors + optional
  `PublicRouteDrain` (the drain-hook trait for flushing background work at
  shutdown).
- `ProtectedRouteMount` — a protected sub-router + descriptors.
- The combined mount pairing both over one descriptor inventory.

## Depends on / consumed by

- **Normal workspace deps (1):** `ironclaw_host_api`. Nothing else, in **any**
  dependency kind — dev- and build-dependencies included.
- **Consumed by (5):** `ironclaw_webui`, `ironclaw_operator`,
  `ironclaw_openai_compat` (the reviewed fifth, 2026-08-05),
  `ironclaw_extension_host`, `ironclaw_composition`. The consumer set is not
  open — the downward re-layer froze it.

## Invariants

- **All-kinds dependency allowlist** `{ironclaw_host_api}` — the layer matrix
  alone would admit `substrates → substrates` edges like `ironclaw_filesystem`,
  so a purpose-built gate refuses them:
  `reborn_dependency_boundaries.rs::assert_host_ingress_names_no_other_workspace_crate`
  (runs inside `reborn_crate_dependency_boundaries_hold`). Do not "fix" a new
  dependency by widening the allowlist.
- **Frozen consumer set** after the #7092 downgrade: `DOWNGRADE_PINS` in
  `reborn_same_layer_edge_inventory.rs`.
- No listener binding, authentication enforcement, middleware construction, or
  persistence logic of its own — carriers only.

## Tests

```bash
cargo check -p ironclaw_host_ingress
cargo test -p ironclaw_architecture_tests    # the two gates above
```

## See also

Working rules: `AGENTS.md` · family rules: `crates/product/AGENTS.md` · design
record: `docs/internal/reborn/target-architecture/families/product.md` (§6.9.5 + the
layer note).
