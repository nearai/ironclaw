# ironclaw_operator

The deployment-operator control plane: LLM provider administration, the
operator log ring, and OS service lifecycle. PROPOSAL §6.9.2; the crate map is
`AGENTS.md` beside this file.

Both files are new with the WS5 operator row. Before it, this crate had
**neither guidance nor a boundary rule** — the audit's clearest correlation was
guidance-presence ↔ discipline, and this crate is the worked example: its
`ironclaw_product` dependency survived every earlier sweep because nothing was
watching.

## The one rule

**This crate implements ports; it does not declare them, and it does not
depend on `ironclaw_product`.**

`ironclaw_operator` and `ironclaw_product` both sit in the `products` layer.
They are siblings. The layer matrix permits `products → products`, so the
dependency was *legal and invisible* — which is why it needed a purpose-built
gate rather than a matrix entry.

Every product-facing port this crate satisfies is declared in
`ironclaw_product_contracts`:

| Port | Module | Implemented here by |
| --- | --- | --- |
| `LlmConfigService` | `operator_llm` | `llm_admin::llm_config_service::RebornLlmConfigService` |
| `ActiveModelReader` | `operator_llm` | `llm_admin::active_model::ProviderActiveModelReader` |
| `OperatorLogsService` | `operator_service` | `operator_logs::OperatorLogBuffer` |
| `OperatorServiceLifecycleService` | `operator_service` | `operator_service_lifecycle::OperatorServiceLifecycle` |

A fifth, `OperatorStatusService`, is declared in the same `operator_service`
module and implemented by `ironclaw_reborn_composition` — readiness is the one
answer only the assembly root can compute.

> Corrected 2026-08-02 (Wave 2 docs-truth audit): the first two rows read module
> `llm_config`, which does not exist. #7004 created it; the #7018 consolidation
> found `main` had already moved the same DTOs into the pre-existing
> `operator_llm`, deleted the duplicate and repointed its import sites, so there
> is one definition and one import path. Grep `operator_llm`.

Two gates hold this:

- `reborn_operator_port_inversion.rs` — the residue of product-declared traits
  this crate implements is **frozen at zero and shrink-only**; the manifest is
  proved clean through `cargo metadata` (not a literal path, so a WS10
  directory move fails loudly); and each inverted port is pinned as
  declared-in-contracts / not-re-declared-in-product / implemented-by-its-owner.
- The `ironclaw_operator` `BoundaryRule` in `reborn_dependency_boundaries.rs`.

**Adding a port.** Declare the trait in `ironclaw_product_contracts`, implement
it here, and add a row to `INVERTED_PORTS` in the gate. Do not declare a
product-facing trait in this crate: a product-tier caller would then have to
name `ironclaw_operator`, which re-inverts the edge in the other direction.

## Routes

This crate owns one HTTP route — the public NEAR AI login callback — and hands
it back as an `ironclaw_host_ingress::PublicRouteMount`: a prebuilt router plus
the `IngressRouteDescriptor`s ingress uses to install auth, rate-limit, body,
and audit policy around it. **The crate owns the route; it does not own where
it mounts.**

It used to declare its own `OperatorPublicRouteMount`/`OperatorProtectedRouteMount`
pair with the same fields as the host-owned types, which forced a
composition-side shim whose entire body was
`PublicRouteMount::new(mount.router, mount.descriptors)`. Both are deleted (the
protected one never had a consumer at all). Do not reintroduce a local mount
type: the duplicate is what created the shim, and the shim is what hid the fact
that operator and ingress already agreed on the shape.

## Vendor names are allowed here — and only here

PROPOSAL §8.2's vendor rule sanctions vendor names in `packages/*`, `llm`
providers, **this crate**, `webui::auth` login providers, and recipes-as-data.
`nearai`, `codex`, `openai`, `anthropic` and friends are this crate's subject
matter, not leakage.

The corollary matters: the *ports* live in a contracts crate, and §8.2 does not
sanction vendor names there. `LlmConfigService` currently carries three
vendor-named methods (`start_nearai_login`, `complete_nearai_wallet_login`,
`start_codex_login`) and six vendor-named DTOs across the boundary. That is a
recorded finding on the WS5 row, not a licence — do not add a seventh. A new
vendor login belongs behind a neutral shape.

## Testing

Per-crate: `cargo test -p ironclaw_operator`. The ports' *contract* (argument
threading, error projection, wire forms, object safety) is tested in
`ironclaw_product_contracts`; what belongs here is whether **this**
implementation answers correctly — key masking, catalog overlay precedence,
config-file writes, the log ring's bounds, and service-lifecycle state mapping.

A contracts-tier test can only pin that the port hands an implementation its
arguments and that the shape admits a per-caller answer. It cannot pin that
this crate filters by caller. Do not rely on it for that.
