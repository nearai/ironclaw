---
name: reborn-feature
description: Navigate building a user-facing feature in the Reborn stack (a capability that crosses product_workflow → composition → webui_v2 → runtime/serve → frontend). Use when planning or implementing any new Reborn settings page, endpoint, facade method, or runtime-backed capability — especially before writing code, to avoid rebuilding what already exists and to wire it in one pass instead of layer-by-layer.
---

# Building a feature in the Reborn stack

A single user-facing feature here crosses ~6 crates. Most of the line count is
structural boilerplate (the ports/adapters tax), not logic. The two ways to lose
a day: (1) build something that already exists, (2) wire it layer-by-layer,
hitting "the next step needs something from a 2,500-line file I haven't read" a
dozen times. This skill kills both.

**Do these two passes BEFORE writing any code.**

## Pass 1 — Inventory what already exists (15 min, non-negotiable)

The building blocks are routinely already present and easy to miss. Real
example: `RebornProviderAdmin` (catalog list/set-active over `config.toml`) and
`ironclaw_llm`'s `SwappableLlmProvider` + `LlmReloadHandle` (live provider
hot-swap, zero per-turn-loop change) both existed during the LLM-config work and
were found *midway*, after plans assumed building them from scratch.

Run these before designing:

```bash
# Existing facade methods + ports you'd extend (don't add a parallel trait)
grep -rn "trait .*ProductFacade\|pub trait .*Service\b" crates/ironclaw_product_workflow/src
grep -rn "async fn " crates/ironclaw_product_workflow/src/reborn_services.rs

# Existing composition-side admin/service handles (often already do the read/write)
grep -rln "RebornProviderAdmin\|Reborn.*Admin\|Reborn.*Facade\|ProductCommandService" crates/ironclaw_reborn_composition/src

# Existing webui2 routes + handlers (mirror the pattern, don't invent a shape)
grep -n "WEBUI_V2_PATTERN_\|fn .*_descriptor" crates/ironclaw_webui/src/webui_v2/descriptors.rs

# Existing primitives in the extracted crates (config writers, swap/reload, secrets)
grep -rn "Swappable\|Reload\|UpdateSession\|FileExt\|SecretStore" crates/ironclaw_llm/src crates/ironclaw_reborn_config/src crates/ironclaw_secrets/src
```

Read the crate-local `AGENTS.md`/`CLAUDE.md` for each crate you'll touch (most
crates have one — they list the seams and the guardrails). When those are
absent, check `CONTRACT.md` or `README.md`, then `Cargo.toml` plus the crate's
primary `src/` entrypoint (`src/lib.rs` for libraries, `src/main.rs` for
binaries). If a building block exists, the feature shrinks to *wiring*, not
*building*.

## Pass 2 — Trace one full vertical (before the first edit)

Read the wiring path end-to-end ONCE so "what must I expose?" is answered before
you write the service, not discovered while wiring. The canonical request flow:

```
browser (webui_v2_static JS)
  └ apiFetch → ironclaw_webui handler (descriptor + route)
      └ Arc<dyn RebornServicesApi>  (ironclaw_product_workflow facade)
          └ port trait → composition impl (ironclaw_reborn_composition)
              └ substrate handles (secret store, config files, reload handle)
```

And the composition path that *supplies* that impl:

```
ironclaw_reborn_cli commands/serve.rs
  → build_runtime_input_with_options(boot) → RebornRuntimeInput (+ with_* builders)  [runtime/mod.rs]
  → build_reborn_runtime(input)            → RebornRuntime (defined in runtime.rs; factory.rs builds substrate)
  → build_webui_services(&runtime, ...)    → attaches facades (webui.rs)  ← attach your service here
```

Open composition's `factory.rs`, `runtime.rs`, `runtime_input.rs`, `webui.rs`,
and the CLI's `commands/serve.rs`
and answer up front: **what does my impl need (boot config? a store? a runtime
handle?), and is it already on `RebornRuntime` / `RebornServices`, or must I
thread it through?** Thread inputs via `RebornRuntimeInput::with_*` builders;
expose substrate to the facade via accessors — but keep raw substrate handles
private (see boundaries below).

## The per-layer boilerplate checklist

For a feature with N endpoints, expect to touch (in dependency order):

| Layer | Crate | What you add |
|---|---|---|
| Port | `ironclaw_product_workflow` | trait + DTOs + error type in `reborn_services/<feature>.rs`; re-export in `reborn_services.rs` + `lib.rs` |
| Facade | `ironclaw_product_workflow` | `Option<Arc<dyn Port>>` field + `with_*` builder + N `RebornServicesApi` methods (give them **default "unavailable" bodies** so existing fakes/tests compile untouched) + an error mapper (port error → `RebornServicesError`, whose ctors are `pub(super)`) |
| Impl | `ironclaw_reborn_composition` | the adapter (`mod <feature>.rs`, gated on the right feature, e.g. `root-llm-provider`); register in `lib.rs` |
| HTTP | `ironclaw_webui` | route constants + pattern + `*_descriptor()` (use `read_policy`/`mutation_policy`) + add to `webui_v2_routes()`; thin handler over `state.services()`; mount in `router.rs`; **update `tests/webui_v2_descriptors_contract.rs`** (it locks the table) |
| Wiring | `ironclaw_reborn_composition` + `ironclaw_reborn_cli` | thread inputs through `RebornRuntimeInput`/`RebornRuntime`; attach in `build_webui_services`; pass from `serve.rs` |
| Frontend | `ironclaw_webui_v2_static` | call endpoints via `apiFetch` in `static/js/pages/*/lib/*-api.js`; consume in hooks. No build step — `node --check <file>.js` to syntax-check |
| Tests | `tests/integration/` + crate tests | for whole-turn behavior, add a scripted-model harness case (see `tests/integration/CLAUDE.md` — mock only at the vendor-SDK seam); facade changes extend `crates/ironclaw_product_workflow/tests/reborn_services_contract.rs`, and handler changes extend `crates/ironclaw_webui/tests/webui_v2_handlers_contract.rs` |

## Boundary rules (the guardrails that will reject your PR)

- `ironclaw_reborn_composition` must **not** depend on the root `ironclaw` crate
  or `src/` — only extracted crates (`ironclaw_llm`, `ironclaw_secrets`,
  `ironclaw_auth`, …). v1 code under `src/channels/web/` is reference-only.
- webui_v2 handlers consume **only** `RebornServicesApi`. No dispatcher,
  extensions, host_runtime, DB, etc.
- Keep substrate handles (secret store, raw stores) **private** to factories;
  expose a facade-shaped handle, or build the consuming service inside
  composition and hand out only that. (See composition `CLAUDE.md`.)
- Tenant/agent/project identity comes from the trusted authenticated caller,
  never the request body.
- Persist-then-reload must be atomic-ish: either pre-validate, or treat the
  on-disk write as source of truth and log (never silently drop) a reload
  failure. See `.claude/rules/error-handling.md`.

## Decision forks cost money — price them

When the user picks "yes" at every scope fork (read-write **and** store secret
values **and** live reload **and** multi-tenant), surface the cost: each "yes"
roughly multiplies the surface. Offer the cheap path explicitly (e.g.
"read + select + restart-to-apply" vs "+ secret values + live hot-reload") so
they trade with eyes open.

## Verify per crate (don't wait for the whole graph)

```bash
cargo build -p ironclaw_product_workflow --all-features
cargo build -p ironclaw_webui
cargo build -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta libsql"
cargo build -p ironclaw_reborn_cli          # compiles the full serve graph
cargo clippy -p <crate> ... --tests          # gate per crate, not at the end
node --check path/to/changed.js              # frontend syntax (no build step)
```

## Reference implementation (in-tree — read these files, one per layer)

The WebChat v2 LLM-config feature is a complete worked example of this shape,
alive in the tree today:

- Port: `crates/ironclaw_product_workflow/src/reborn_services/llm_config.rs`
- Facade: `LlmConfigService` field + `with_llm_config_service` + delegating
  methods in `crates/ironclaw_product_workflow/src/reborn_services.rs`
- Impl: `crates/ironclaw_reborn_composition/src/llm_admin/llm_config_service.rs`
- HTTP: `get_llm_config_descriptor()` in
  `crates/ironclaw_webui/src/webui_v2/descriptors.rs` + handler in `handlers.rs`
- Wiring: `build_llm_config_service` attach in
  `crates/ironclaw_reborn_composition/src/webui.rs`
- Frontend: `crates/ironclaw_webui_v2_static/static/js/pages/settings/lib/settings-api.js`
