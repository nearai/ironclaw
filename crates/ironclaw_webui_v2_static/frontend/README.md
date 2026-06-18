# WebUI v2 frontend build tooling

This directory builds the generated frontend artifacts that the Rust
crate embeds. **`cargo build` does not run any of this** — it embeds the
already-committed output under `../static/dist/` and `../static/vendor/`,
so Rust builds stay hermetic (no node, no network).

## When to run

Run after you edit anything under `../static/js/**`, or to upgrade a
pinned dependency:

```bash
./build.sh              # re-vendor CDN assets + npm ci + esbuild bundle
./build.sh --no-vendor  # just re-bundle (skip re-downloading vendored deps)
```

Then **commit** the changed files under `../static/dist/` and
`../static/vendor/`. If you change JS without rebuilding, the served app
keeps using the stale committed bundle.

## What it produces

| Output | Made by | Loaded in index.html as |
|---|---|---|
| `static/dist/app.js` + `static/dist/chunks/*` | `build.mjs` (esbuild) | `<script type="module" src="/v2/dist/app.js">` |
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

These are still embedded by the crate's `build.rs` (and covered by the
crate's Rust asset tests), but the browser no longer loads them
individually — only the bundle. They are the source of truth for the
bundle; keep editing them, not `dist/`.
