// Bundles the WebUI v2 SPA with esbuild.
//
// Input:  ../static/js/main.js  (entry) + the ~233 local ES modules it
//         imports, plus the React ecosystem (react, react-dom,
//         react-router, @tanstack/react-query, react-hook-form, htm)
//         resolved from ./node_modules.
// Output: ../static/dist/app.js  + ../static/dist/chunks/*  (locale
//         code-split chunks, dynamically imported by lib/i18n.js).
//
// The output is committed so `cargo build` embeds it without needing
// node. Re-run via ./build.sh after editing static/js/**.
//
// What is deliberately NOT bundled (kept as same-origin <script>/<link>
// in index.html, vendored by vendor.sh):
//   - Tailwind browser runtime (a DOM-scanning IIFE, not a module)
//   - DOMPurify / marked / highlight.js (consumed as window globals)
//   - Google Fonts
//   - wallet-connect.js (separate isolated entry with relaxed CSP +
//     remote wallet executors — never merge into the app bundle)

import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { rmSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
const staticDir = resolve(here, "..", "static");
const outdir = resolve(staticDir, "dist");

// Clean stale chunks (hashed names accumulate across builds otherwise).
rmSync(outdir, { recursive: true, force: true });

await build({
  entryPoints: { app: resolve(staticDir, "js", "main.js") },
  outdir,
  bundle: true,
  splitting: true,
  format: "esm",
  platform: "browser",
  target: ["es2022"],
  minify: true,
  sourcemap: false,
  legalComments: "none",
  // Bare specifiers (react, htm, …) live in this dir's node_modules,
  // but the importing source files sit under ../static/js, so esbuild's
  // default upward node_modules walk won't find them. nodePaths adds an
  // explicit resolution root, NODE_PATH-style.
  nodePaths: [resolve(here, "node_modules")],
  // React and friends gate dev-only code on process.env.NODE_ENV; without
  // this define the browser bundle would reference an undefined `process`.
  define: { "process.env.NODE_ENV": '"production"' },
  // Fixed entry name so index.html can hard-reference /v2/dist/app.js;
  // chunks stay content-hashed (i18n.js imports them by path the bundle
  // rewrites itself, so their names need not be known to index.html).
  entryNames: "[name]",
  chunkNames: "chunks/[name]-[hash]",
  logLevel: "info",
});

console.log("webui v2 bundle written to static/dist/");
