// Static-analysis gate for the WebUI v2 SPA JavaScript source in `frontend/src`.
//
// Scope is deliberately narrow: this config enforces `no-undef` only. The
// The VM-based component suites stub every collaborator
// (`html`, `React`, `Button`, `Spinner`, …) through a `vm` context, so a
// production module that *references* a symbol it never imported still passes
// its unit test — the stub silently supplies the missing binding. `no-undef`
// is the guard the eval-harness cannot provide: it fails when a module uses an
// identifier that is neither imported, declared, nor a known browser/node
// global (the class of bug that shipped in the htm `${Component}` templates).
//
// eslint + globals are resolved from `frontend/node_modules` (the crate's single
// package install root). `globals` is imported by relative path so the config
// resolves it from that same install without needing a second node_modules at
// the crate root.
import globals from "./frontend/node_modules/globals/index.js";

export default [
  {
    ignores: [
      "static/vendor/**",
      "static/dist/**",
      "frontend/dist/**",
      "frontend/node_modules/**",
      "frontend/public/vendor/**",
      "**/*.min.js",
    ],
  },
  {
    files: ["frontend/src/**/*.js", "frontend/src/**/*.mjs"],
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
    files: ["frontend/src/**/*.test.mjs", "frontend/src/**/*.test.js"],
    languageOptions: {
      globals: {
        ...globals.node,
      },
    },
  },
];
