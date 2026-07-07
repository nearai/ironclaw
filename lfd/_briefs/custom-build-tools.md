# LFD Brief: custom-build-tools — Custom build tools

**State**: greenfield in Reborn — v1 had `src/tools/builder/` (scaffold,
templates, validation, testing); Reborn executes WASM but has NO authoring
surface. **Bar**: 0.85 holdout. **Profile**: `tool_builder`.

## Outcome

A Reborn tool-authoring pipeline: a build request (from a turn or API) →
scaffolded project from templates → compiled to a WASM component →
validated (schema, limits) → installed as an extension through the normal
lifecycle → invocable in the same session, sandboxed (fuel, memory,
network allowlist enforced).

## Spec sources

- v1 `src/tools/builder/{core.rs,templates.rs,validation.rs,testing.rs}`
  (BuildRequirement, SoftwareType, Language — behavioral reference; port
  concepts, not code)
- `crates/ironclaw_wasm/`, `crates/ironclaw_wasm_limiter/`,
  `crates/ironclaw_extensions/` (manifest + lifecycle),
  `crates/ironclaw_wasm_product_adapters/`
- `src/tools/README.md`, registry manifest types in `src/registry/manifest.rs`
- Known limitation list (root CLAUDE.md): "Built tools get empty
  capabilities" — the spec must close this: build output declares
  capabilities, user grant flow gates them.

## Stage 0 inner suite

`ironclaw_wasm*` + `ironclaw_extensions` crate tests + new builder crate
tests per spec. Toolchain note: eval compiles TINY Rust→wasm32 components
(or WAT where the template allows) — spec pins the toolchain; profile setup
verifies `wasm32-wasip2` target present and marks cases `unsupported` with
a clear error if absent (goal.md tells the agent to install it in Stage 0).

## Eval themes (dev ~30 / holdout ~10)

1. Scaffold (6): build request (name, language, IO schema, capability
   needs) → project generated from template (state query on generated
   manifest + entry source; contracts on schema fields, not file bytes).
2. Compile + validate (6): scaffolded project compiles; invalid template
   inputs (bad identifiers, unsupported language) → typed validation errors
   (fail-closed, no partial install).
3. Manifest correctness (5): produced extension manifest declares the
   requested capabilities, version, entrypoint (state_pred); empty-
   capabilities regression forbidden (capability list must match request).
4. Install + invoke round-trip (8): built tool installs via extension
   lifecycle → invoked with case inputs → output matches the IO contract
   (tool_invocations + reply contracts). Holdout requests DIFFERENT tool
   semantics (unseen IO schemas).
5. Sandbox enforcement (5): built tool attempting off-allowlist egress →
   blocked (forbidden egress, required typed denial); fuel/memory bomb
   template case → limited, run survives (status ran + error event, not
   crash).

## Feature-specific cheats → fences

- **Prebuilt artifacts matched to eval** → lint cap: NEW binary files
  (.wasm, .wat compiled blobs, base64 blobs > 1KB) committed in diff = 0;
  artifacts must be produced at runtime under target/scratch (contract
  state_pred on artifact provenance path). Holdout IO schemas are unseen so
  a stash of prebuilt tools can't cover them.
- **Echo-tool** (tool ignores logic, returns canned output) → invoke
  contracts use per-case input→output pairs derivable only by implementing
  the requested transform (e.g. arithmetic/string ops parameterized per
  case); probe changes the IO values.
- **Copy bundled WASM as "built" output** → provenance state_pred +
  manifest must reflect the REQUESTED schema, which bundled tools don't.
- **Validation theater** (accept everything) → theme-2 invalid inputs
  REQUIRE typed errors; both directions priced.
- **Sandbox pass by never granting capabilities** → theme-4 requires
  successful egress ON the allowlist for tools that request it.

## caps.json extras

New binary/base64 blobs in diff: max 0 (lint pattern on file mode + size +
extension). Dev IO-schema literals in `crates/**` diff: max 0.

## Live mode

3 live cases: real model receives "build me a tool that does X" → required:
build-pipeline tool calls fire in order (scaffold→compile→install) and the
tool is invoked once successfully (structural; X is simple and stated).
