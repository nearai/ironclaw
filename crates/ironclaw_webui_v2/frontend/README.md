# WebUI v2 frontend build tooling

This directory builds the WebUI v2 frontend artifacts that the Rust crate
embeds. When `webui-v2-beta` is enabled, Cargo runs the esbuild bundle step
from `build.rs`, writes the SPA bundle into `OUT_DIR`, and embeds that generated
output as `/v2/dist/*`. The `../static/vendor/` assets remain committed so the
main SPA keeps loading all runtime assets from the same origin.

## When to run

You do not need to run this after ordinary `../static/js/**` edits: Cargo
rebuilds the ignored `/v2/dist/*` bundle for WebUI-enabled builds.

Run it when you want a local `../static/dist/` preview, or when upgrading a
pinned vendored dependency:

```bash
./build.sh              # re-vendor CDN assets + npm ci + esbuild bundle
./build.sh --no-vendor  # just re-bundle (skip re-downloading vendored deps)
```

Then commit only the changed files under `../static/vendor/` when refreshing
vendored dependencies. `../static/dist/` is generated output and is ignored.

## What it produces

| Output | Made by | Loaded in index.html as |
|---|---|---|
| `/v2/dist/app.js` + `/v2/dist/chunks/*` | `build.mjs` (esbuild), run by Cargo into `OUT_DIR` or by `./build.sh` into local `static/dist/` | `<script type="module" src="/v2/dist/app.js">` |
| `static/vendor/tailwindcss-browser.js` | `vendor.sh` | `<script>` |
| `static/vendor/purify.min.js` | `vendor.sh` | `<script>` (window.DOMPurify) |
| `static/vendor/marked.umd.min.js` | `vendor.sh` | `<script>` (window.marked) |
| `static/vendor/highlight.min.js` | `vendor.sh` | `<script>` (window.hljs) |
| `static/vendor/fonts/fonts.css` + `*.woff2` | `vendor.sh` | `<link rel="stylesheet">` |

## What is intentionally NOT bundled

- **Tailwind runtime, DOMPurify, marked, highlight.js** — consumed as
  `window` globals / a DOM-scanning IIFE, not ES modules. Vendored as
  separate same-origin `<script>`s.
- **wallet-connect.js / wallet-connect.html** — a separate isolated
  popup entry with a deliberately relaxed CSP that loads remote NEAR
  wallet executors. It keeps its own importmap and must never be merged
  into the app bundle.

## The source modules under `static/js/**`

These are still embedded by the crate's `build.rs` (and covered by the crate's
Rust asset tests), but the browser no longer loads them individually — only the
generated bundle. They are the source of truth for the bundle; keep editing
them, not `dist/`.
