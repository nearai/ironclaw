# WebUI v2 Frontend

This directory owns the WebUI v2 frontend toolchain. Use Node 22 with Corepack
enabled; the committed `pnpm-lock.yaml` is the source of truth for dependency
resolution.

## Commands

```bash
corepack pnpm install --frozen-lockfile
corepack pnpm dev
corepack pnpm lint
corepack pnpm typecheck
corepack pnpm test
corepack pnpm build
```

`corepack pnpm lint` enforces the authored-source conventions before running
the TypeScript typecheck: modules under `src/` use `.ts`/`.tsx`, relative module
imports are extensionless, and React markup does not use legacy `html\`...\``
tagged templates. Explicit filenames passed to file APIs such as `new URL(...)`
and generated JavaScript asset names are outside this module-import rule.

`corepack pnpm build` runs Vite and writes ignored preview output to
`frontend/dist/`. Cargo does not embed that local preview directory. When
`webui-v2-beta` is enabled, `crates/ironclaw_webui_v2/build.rs` runs
`corepack pnpm install --frozen-lockfile` and a Vite production build into
Cargo's `OUT_DIR`, then embeds that generated output into the Rust binary.

`./build.sh` is the one-shot local refresh helper. It vendors pinned browser
assets, installs dependencies with Corepack, and runs the Vite production build.
Use `./build.sh --no-vendor` when you only want to rebuild the SPA.

## Outputs

| Output | Made by | Commit? |
|---|---|---|
| `frontend/dist/` | `corepack pnpm build` / `./build.sh` | No |
| Cargo `OUT_DIR/webui-v2-frontend-dist/` | `build.rs` during Rust builds | No |
| `frontend/public/vendor/fonts/` | `vendor.sh` / `./build.sh` | Yes, only when intentionally refreshing self-hosted fonts |

## Runtime Assets

Vite owns the SPA entrypoint, CSS, markdown/syntax-highlighting libraries,
hashed assets, and the NEAR wallet connect entrypoint. The self-hosted fonts
remain separate same-origin files under `frontend/public/vendor/fonts/`.

The NEAR wallet connect popup is still a separate entrypoint with its own CSP and
must not be merged into the main SPA bundle.
