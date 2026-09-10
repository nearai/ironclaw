# WebUI v2 Frontend

This directory owns the WebUI v2 frontend toolchain. Use Node 22.22 or newer
within the Node 22 release line with Corepack enabled; the committed
`pnpm-lock.yaml` is the source of truth for dependency resolution.

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
tagged templates. It also rejects `@ts-ignore` and limits `@ts-nocheck` to the
legacy allowlist in `scripts/ts-nocheck-baseline.txt`; stale allowlist entries
are ignored so concurrent suppression cleanup does not fail lint. Intentional
`@ts-expect-error` assertions remain valid. Explicit filenames passed to file
APIs such as `new URL(...)` and generated JavaScript asset names are outside
the module-import rule.

`corepack pnpm build` runs Vite and writes ignored preview output to
`frontend/dist/`. Cargo does not embed that local preview directory.
`crates/product/ironclaw_webui/build.rs` runs
`corepack pnpm install --frozen-lockfile` and a Vite production build into
Cargo's `OUT_DIR`, then embeds that generated output into the Rust binary. If
`corepack` is not on `PATH`, the build script falls back to
`npm exec --yes --package=pnpm@11.7.0 -- pnpm ...`.

`./build.sh` is the one-shot local refresh helper. It vendors pinned browser
assets, installs dependencies with Corepack, and runs the Vite production build.
Use `./build.sh --no-vendor` when you only want to rebuild the SPA.

## Visual regression (Chromatic)

`code_style.yml` carries a `webui-v2-chromatic` lane that publishes the built
Storybook catalog to Chromatic, so the reskin (#7781 WS3) accumulates a
rendered visual history instead of only a diff of Tailwind class strings.

**It runs only on pushes to `main`, never on pull requests.** That is a
security boundary, not a scheduling preference: same-repository PR branches
*do* receive repository secrets, and the publish step executes the checked-out
tree's own `build-storybook` script. A lane that ran on `pull_request` would
therefore hand `CHROMATIC_PROJECT_TOKEN` to PR-authored build code, which could
read it out of the environment — skipping forks does not help, because the
dangerous case is the same-repo branch.

The consequence is that a PR gets no visual diff of its own; `main` publishes
baselines and the history accrues there. Restoring per-PR diffing safely needs
the untrusted-build/trusted-publish split — build the catalog in a secretless
PR job, upload it as an artifact, and publish from a `workflow_run` job that
never checks out PR code.

The lane is also **optional and non-blocking** — deliberately absent from the
`code-style` roll-up's `needs:` list, per the #7782 WS6 decision to promote it
to a required gate only once the baseline proves stable. It self-skips with a
CI notice when `CHROMATIC_PROJECT_TOKEN` is unset, and the token is probed
before anything is checked out so an unprovisioned repository pays nothing.

`scripts/ci/ws12_workflow_contracts.py::validate_chromatic_visual_lane` pins
all of the above — the trusted-event gate, the probe ordering, the exact
version pin, `--exit-zero-on-changes`, and the absence from the roll-up — so
none of it can be edited away silently. The publish path itself has never
executed in CI, which is exactly why the static contract carries the weight.

Two pieces of repository configuration switch it on:

| Setting | Kind | Purpose |
|---|---|---|
| `CHROMATIC_PROJECT_TOKEN` | secret | From Chromatic project settings. Absent → the lane skips. |
| `CHROMATIC_CLI_VERSION` | variable | Exact CLI version, e.g. `13.1.2`. Set alongside the secret. |

The version is pinned through a variable rather than floating on `@latest`
because a visual-baseline tool that silently changes version changes the
baseline with it. If the secret is set and the variable is not, the lane fails
with an explanatory error rather than picking a version for you.

The lane shells out to `pnpm dlx chromatic` instead of using `chromaui/action`.
GitHub resolves every `uses:` during *Set up job*, before step-level `if:`
conditions run, so an action would have to be pinned and reachable even on the
runs where the publish is skipped; a `pnpm dlx` inside a gated `run:` is
genuinely inert. Adding `chromatic` to `package.json` was the other option and
would put a service dependency into the `--frozen-lockfile` install every other
frontend job performs.

`--exit-zero-on-changes` is on: a visual change reports but never fails the
step, because during a reskin the changes *are* the deliverable. `main` carries
the accepted baseline via `--auto-accept-changes`.

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
