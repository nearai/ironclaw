# Frontend Source Conventions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the WebUI v2 TypeScript migration and add a test-backed gate that rejects non-TypeScript source modules, explicit relative import extensions, and HTM tagged templates.

**Architecture:** A focused TypeScript CLI uses the existing TypeScript compiler API to parse authored modules under `frontend/src`, while filesystem traversal checks module filenames. Vitest exercises the checker with temporary source trees; the package `lint` command runs the checker and the existing typecheck.

**Tech Stack:** TypeScript 6, Node.js 22 type stripping, Vitest 4, pnpm 11, Vite 8, React 19.

## Global Constraints

- Authored code modules under `frontend/src` use only `.ts` or `.tsx`.
- Relative static imports, re-exports, side-effect imports, and dynamic imports are extensionless.
- React markup uses JSX; `html\`...\`` tagged templates are forbidden.
- CSS/assets, generated JavaScript output names, and explicit filenames passed to `new URL(...)` remain allowed.
- Preserve unrelated untracked files and avoid generated output changes.

---

### Task 1: Add the source-convention checker test-first

**Files:**
- Create: `crates/ironclaw_webui/frontend/scripts/check-source-conventions.ts`
- Create: `crates/ironclaw_webui/frontend/src/test/source-conventions.test.ts`
- Modify: `crates/ironclaw_webui/frontend/tsconfig.json`

**Interfaces:**
- Produces: `ConventionViolation`, `checkSourceFile(filePath, sourceText)`, `checkSourceTree(sourceRoot)`, and `formatViolation(violation)`.
- Consumes: the existing `typescript` package and Node filesystem/path APIs.

- [ ] **Step 1: Write failing Vitest coverage**

Create fixture-driven tests that require the checker module and assert rejection of `.js/.jsx/.mjs/.cjs/.mts/.cts`, explicit relative imports/re-exports/dynamic imports, and `html\`...\``. Assert that `.ts/.tsx`, package imports, CSS, comments/string text, `new URL("./component.tsx", import.meta.url)`, and extensionless relative imports are allowed.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
pnpm --dir crates/ironclaw_webui/frontend exec vitest run src/test/source-conventions.test.ts
```

Expected: FAIL because `scripts/check-source-conventions.ts` does not exist.

- [ ] **Step 3: Implement the minimal checker**

Use `ts.createSourceFile` and AST traversal for import and tagged-template rules. Recursively enumerate the supplied root for the filename rule, return all violations deterministically sorted by path/line/kind, and make the CLI print each formatted violation before setting a nonzero exit code.

- [ ] **Step 4: Include tooling in typechecking and verify GREEN**

Add `scripts` to `tsconfig.json#include`, then rerun the focused test and `pnpm typecheck`. Expected: all convention tests pass and typechecking exits zero.

### Task 2: Convert all surviving modules and restore excluded tests

**Files:**
- Rename/modify: the 11 `.js/.mjs/.mts` modules currently tracked under `frontend/src`
- Delete: `crates/ironclaw_webui/frontend/src/design-system/button.test.mjs`
- Modify: `crates/ironclaw_webui/frontend/src/design-system/button.test.tsx`
- Rename/modify: `crates/ironclaw_webui/frontend/src/components/slack-channel-picker.test.ts` to `.tsx`
- Modify: affected imports and current-layout comments under `frontend/src`

**Interfaces:**
- Consumes: the existing VM test harness and converted OAuth/stream/project helpers.
- Produces: a source tree containing only `.ts/.tsx` code modules and no HTM tagged templates.

- [ ] **Step 1: Rename the excluded tests/helpers and run them to expose migration failures**

Rename useful files to `.ts`, update only the immediately required file-read targets, and run their focused Vitest paths. Expected: failures from TypeScript/JSX syntax or stale VM transformation assumptions demonstrate that the previously excluded coverage is now active.

- [ ] **Step 2: Convert production modules and test helpers**

Add narrow types where required, migrate reusable VM source stripping to the existing typed harness where practical, and preserve the tests' existing behavior. Rename JSX-bearing tests to `.tsx`.

- [ ] **Step 3: Consolidate the Button regression**

Add the missing no-spinner assertion to `button.test.tsx`, confirm that focused test passes, then delete the redundant `.mjs` test.

- [ ] **Step 4: Replace the remaining HTM tagged template**

Rewrite the Slack channel picker test helper as JSX in `slack-channel-picker.test.tsx` and run that focused test to confirm equivalent behavior.

- [ ] **Step 5: Run all frontend tests**

Run `pnpm test`. Expected: the restored test files are discovered and every test passes.

### Task 3: Remove extensionful imports and wire the gate into lint

**Files:**
- Modify: affected `frontend/src/**/*.ts` and `frontend/src/**/*.tsx` imports
- Modify: `crates/ironclaw_webui/frontend/package.json`
- Modify: `crates/ironclaw_webui/frontend/pnpm-lock.yaml`
- Delete: `crates/ironclaw_webui_v2/eslint.config.mjs`

**Interfaces:**
- Consumes: `scripts/check-source-conventions.ts` from Task 1.
- Produces: `pnpm lint` as the single convention-plus-typecheck gate.

- [ ] **Step 1: Run the checker against the unclean source tree**

Run `node --experimental-strip-types scripts/check-source-conventions.ts` from `frontend`. Expected: FAIL with every remaining explicit relative import extension.

- [ ] **Step 2: Make actual module imports extensionless**

Update static imports, re-exports, side-effect imports, and dynamic imports only. Preserve explicit file extensions in `new URL(...)`, generated asset references, and prose where the filename is intentionally literal.

- [ ] **Step 3: Replace stale ESLint wiring**

Add `lint:conventions`, make `lint` run `lint:conventions` plus `typecheck`, remove unused `eslint` and `globals` dependencies, refresh the frozen lockfile, and delete the ineffective ESLint config.

- [ ] **Step 4: Verify the lint gate**

Run `pnpm lint`. Expected: convention scan and typecheck both exit zero.

### Task 4: Verify, review, and publish

**Files:**
- Review: every changed file and the final branch diff

**Interfaces:**
- Produces: one focused commit set and a draft GitHub pull request.

- [ ] **Step 1: Run frontend verification**

Run `pnpm lint`, `pnpm typecheck`, `pnpm test`, and `pnpm build` with the repository's Node/pnpm runtime. Expected: all commands exit zero; the restored tests appear in the Vitest totals.

- [ ] **Step 2: Run the owning-crate test**

Run `cargo test -p ironclaw_webui_v2 --features webui-v2-beta`. Expected: all crate tests exit zero.

- [ ] **Step 3: Audit the diff and forbidden patterns**

Run `git diff --check`, inspect `git diff --stat` and `git diff`, confirm no JavaScript-family source modules remain, and confirm unrelated `.agents/` and `docs/reborn/memory-rd/` files are unstaged.

- [ ] **Step 4: Commit and publish**

Stage only the approved frontend/spec/plan files, commit tersely, push `codex/frontend-source-conventions`, and open a draft PR whose body documents root cause, migration, compatibility/rollback, and exact verification commands.
