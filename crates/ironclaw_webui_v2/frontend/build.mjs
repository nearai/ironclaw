// Bundles the WebUI v2 SPA with Vite.
//
// Input:  ./src/main.ts  (entry) + the local TypeScript modules it
//         imports, plus the React ecosystem (react, react-dom,
//         react-router, @tanstack/react-query, react-hook-form, htm)
//         resolved from ./node_modules.
// Output: $IRONCLAW_WEBUI_V2_DIST_DIR/app.{js,css}, wallet-connect.js, and
//         chunks/* when that env var is set, otherwise ../static/dist/* for
//         manual local builds. Locale chunks are code-split and dynamically
//         imported by lib/i18n.js.
//
// Cargo invokes this in `webui-v2-beta` builds and embeds the generated output
// from OUT_DIR. Re-run via ./build.sh after editing frontend/src/** when you
// want a local static/dist preview.
//
// What is deliberately NOT bundled (kept as same-origin <script>/<link>
// in index.html, vendored by vendor.sh):
//   - DOMPurify / marked / highlight.js (consumed as window globals)
//   - Google Fonts
//   - wallet-connect.js (separate isolated entry with relaxed CSP +
//     remote wallet executors — never merge into the app bundle)

import { build } from "vite";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { rmSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
const staticDir = resolve(here, "..", "static");
const sourceDir = resolve(here, "src");
const outdir = process.env.IRONCLAW_WEBUI_V2_DIST_DIR
  ? resolve(process.env.IRONCLAW_WEBUI_V2_DIST_DIR)
  : resolve(staticDir, "dist");

// Clean stale chunks (hashed names accumulate across builds otherwise).
rmSync(outdir, { recursive: true, force: true });

await build({
  configFile: resolve(here, "vite.config.ts"),
  build: {
    outDir: outdir,
    emptyOutDir: true,
    manifest: false,
    target: "es2022",
    minify: true,
    sourcemap: false,
    rollupOptions: {
      input: {
        app: resolve(sourceDir, "main.ts"),
        "wallet-connect": resolve(sourceDir, "wallet-connect.ts"),
      },
      external: ["@hot-labs/near-connect"],
      output: {
        entryFileNames: "[name].js",
        chunkFileNames: "chunks/[name]-[hash].js",
        assetFileNames: "[name][extname]",
      },
    },
  },
  logLevel: "info",
});

console.log(`webui v2 bundle written to ${outdir}`);
