# Frontend Source Conventions Design

## Goal

Finish the WebUI v2 TypeScript migration and prevent authored frontend modules
from drifting back to legacy JavaScript, explicit-extension imports, or HTM
tagged templates.

## Scope

The conventions apply to authored code modules under
`crates/ironclaw_webui/frontend/src`:

- Code modules use `.ts` when they contain no JSX and `.tsx` when they contain
  JSX. JavaScript-family alternatives such as `.js`, `.jsx`, `.mjs`, `.cjs`,
  `.mts`, and `.cts` are rejected.
- Relative module imports, exports, and dynamic imports are extensionless.
- HTM-style `html\`...\`` tagged templates are rejected; React markup uses JSX.

The gate does not reject non-code assets such as CSS, images, fonts, or HTML
entrypoints. It also does not reject explicit filenames passed to file APIs such
as `new URL("./component.tsx", import.meta.url)`, because those are file reads,
not module imports. Generated JavaScript bundle names and HTML references to
those generated bundles remain valid.

## Migration

The two production `.js` modules become `.ts` modules. Useful `.mjs` and `.mts`
tests and VM helpers become `.ts`, restoring them to Vitest's configured test
discovery. The stale `button.test.mjs` is removed after its unique assertion is
moved into the existing `button.test.tsx`. The remaining HTM-tagged test helper
becomes JSX in a `.tsx` test. All relative module imports in frontend source are
made extensionless.

Comments that still name obsolete source extensions are updated where they
describe the current source layout. Runtime asset names and file-reading test
fixtures retain explicit extensions.

## Convention Gate

A small TypeScript-based checker lives with the frontend tooling. It recursively
examines `frontend/src`, reports every violation in one run, and exits nonzero
when it finds any of the following:

1. An authored code module with a JavaScript-family extension other than `.ts`
   or `.tsx`.
2. A relative static import, re-export, side-effect import, or dynamic import
   whose module specifier ends in a TypeScript or JavaScript source extension.
3. A tagged template whose tag is the identifier `html`.

The checker uses the existing TypeScript compiler dependency to parse syntax,
avoiding regex false positives in comments and string literals. Its scanning
and reporting logic is exposed as testable functions, while a narrow CLI entry
point handles filesystem traversal and exit status.

The frontend `lint` command runs the convention checker and TypeScript
typechecking. The existing ESLint configuration is removed because it only
targets `.js` and `.mjs`; after the migration it would inspect no authored
source and provide a misleading successful result.

## Testing

Vitest tests exercise the checker against temporary fixtures for each rejected
case and for the allowed exceptions. The migration is additionally verified by
running:

- `pnpm lint`
- `pnpm typecheck`
- `pnpm test`
- `pnpm build`
- the WebUI v2 Rust crate test required by the owning crate guidance

The restored tests must be included in Vitest's discovered test count. The
convention tests must demonstrate red/green behavior by failing before the gate
implementation exists and passing afterward.

## Compatibility and Rollback

This changes authored source and development checks only; browser bundle names
and public HTTP/static paths do not change. Rollback is a normal revert of the
conversion and checker commit. No persistence, configuration, security policy,
or runtime schema is affected.
