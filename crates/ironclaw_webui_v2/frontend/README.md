# WebUI v2 Frontend

This directory owns the WebUI v2 frontend toolchain. Use Node 22 with Corepack
enabled; the committed `pnpm-lock.yaml` is the source of truth for dependency
resolution.

## Commands

```bash
corepack pnpm install --frozen-lockfile
corepack pnpm dev
corepack pnpm typecheck
corepack pnpm test
corepack pnpm build:vite
```

At this point in the stack, Cargo still embeds the legacy esbuild bundle from
`build.mjs`, so `corepack pnpm build` and `corepack pnpm build:legacy` both run
that legacy bundle path. `corepack pnpm build:vite` is the Vite production build
for the new scaffold and writes ignored output to `frontend/dist/`.

`./build.sh` is the one-shot local refresh helper. It vendors pinned browser
assets, installs with `corepack pnpm install --frozen-lockfile`, and builds the
legacy bundle used by this branch.

## Outputs

| Output | Made by | Commit? |
|---|---|---|
| `frontend/dist/` | `corepack pnpm build:vite` | No |
| `../static/dist/` | `build.mjs` / `./build.sh` | No |
| `../static/vendor/` | `vendor.sh` / `./build.sh` | Yes, only when intentionally refreshing vendor assets |

## Runtime Assets

The vendored browser globals remain separate same-origin files:

- DOMPurify
- marked
- highlight.js
- bundled fonts

The NEAR wallet connect popup is still a separate entrypoint with its own CSP and
must not be merged into the main SPA bundle.
