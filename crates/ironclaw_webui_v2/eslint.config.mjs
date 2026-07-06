// Static-analysis gate for the WebUI v2 SPA source in `static/js`.
//
// Scope is deliberately narrow: this config enforces `no-undef` only. The
// `node --test` suites for these components stub every collaborator
// (`html`, `React`, `Button`, `Spinner`, …) through a `vm` context, so a
// production module that *references* a symbol it never imported still passes
// its unit test — the stub silently supplies the missing binding. `no-undef`
// is the guard the eval-harness cannot provide: it fails when a module uses an
// identifier that is neither imported, declared, nor a known browser/node
// global (the class of bug that shipped in the htm `${Component}` templates).
//
// eslint + globals are resolved from `frontend/node_modules` (the crate's single
// npm install root); CI runs `frontend/node_modules/.bin/eslint .` from this
// directory. `globals` is imported by relative path so the config resolves it
// from that same install without needing a second node_modules at the crate root.
import globals from "./frontend/node_modules/globals/index.js";

export default [
  {
    ignores: [
      "static/vendor/**",
      "static/dist/**",
      "frontend/**",
      "**/*.min.js",
    ],
  },
  {
    files: ["static/js/**/*.js", "static/js/**/*.mjs"],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "module",
      globals: {
        ...globals.browser,
        ...globals.worker,
      },
    },
    rules: {
      "no-undef": "error",
    },
  },
  {
    // Test modules import Node builtins and drive components through a `vm`
    // context, so they legitimately see Node globals on top of the browser set.
    files: ["static/js/**/*.test.mjs", "static/js/**/*.test.js"],
    languageOptions: {
      globals: {
        ...globals.node,
      },
    },
  },
];
