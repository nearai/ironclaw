---
paths:
  - "**/Cargo.toml"
  - "crates/**/*.rs"
  - "src/**/*.rs"
---
# Cargo Features — A Feature Must Earn Its Build

A Cargo feature is not a way to say "this part is optional," "this is
still beta," or "the substrate should work without this." Every one of
those readings produced a feature this repo later deleted. A feature is a
**second build of the workspace** that someone has to compile, lint,
test, and keep working — forever.

## The bar

A new `[features]` entry is justified by exactly one of:

1. **A heavy optional dependency** the default build should not compile.
   `bedrock` (three AWS SDK crates) and `clipboard` (`arboard` + `image`)
   qualify. The win must be a real dependency you can name, not "less
   code."
2. **A genuinely shipped build shape.** Some artifact this repo produces
   builds it OFF. If `Dockerfile.reborn`, `reborn-release-compile.yml`,
   and `scripts/ci/package-feature-flags.sh` all turn it ON, it is not a
   build shape — it is the product.
3. **A CI lane selector** that gates test *targets*, not production code
   — `integration`, `replay`, `libsql-restart-tests`. These carry zero
   `#[cfg]` in `src/`.
4. **A dev-only seam** kept out of production binaries. Exactly one name
   for this: **`test-support`**. Not `testing`, not `contract-tests`, not
   `dev-in-memory-session`.
5. **A privilege boundary** the type system can't express —
   `host-auth-mint` (only host runtimes may mint verified auth evidence).
   Rare; say so explicitly in the manifest comment.

If none of those fit, the answer is runtime configuration. Deployment
shape belongs in `DeploymentConfig` and `[storage]`, not in `#[cfg]`.

## What this rule is reacting to

The 2026-07 feature audit deleted 20 features and ~1,100 `#[cfg]` sites
across four commits. The failure modes, all of which looked reasonable
when introduced:

- **`webui-v2-beta`, `slack-v2-host-beta`, `telegram-v2-host-beta`,
  `openai-compat-beta`, `webhook-serve`** — 776 `#[cfg]` sites and 25
  dead alternate implementations maintained to protect a build nobody
  shipped. Every artifact enabled all of them. The `-beta` suffix had
  also outlived its truth: these were the product.
- **`root-llm-provider`** — "off" produced a binary that booted and then
  failed every request with "no LLM gateway wired." That is a
  compile-time spelling of a runtime state.
- **`pr3180-ready`, `pr7-ready`** — speculative gates for landings that
  never came, with zero `#[cfg]` sites, sitting in the manifest for two
  months.
- **`libsql`/`postgres` on `ironclaw_resources`, `ironclaw_run_state`,
  `ironclaw_outbound`** — declared, forwarded to by three crates, never
  read; they pulled `libsql`, `deadpool-postgres`, and `tokio-postgres`
  into builds that used none of them.
- **`full` on `ironclaw_reborn_cli`** — an alias no build invoked.
- **Six names for the one dev-seam concept.**

## Rules for adding one

- **State the bar in the manifest comment.** Name which of the five
  above it meets. A comment that only restates the feature name
  ("`slack-v2-host-beta` — the Slack host beta") tells a reviewer
  nothing.
- **Something must build it OFF, and CI must prove it.** If the only
  configuration that disables your feature is a bare `cargo test -p
  <crate>`, you have written dead code with extra steps.
- **Do not add a `#[cfg(not(feature = ...))]` alternate implementation**
  that errors, stubs, or returns `None` to explain the feature is
  missing. That is the single most common shape deleted in the audit.
  Prefer failing at composition with a runtime error.
- **A feature that only forwards to another crate's feature** belongs on
  the dependency declaration instead: `foo = { path = "…", features =
  ["bar"] }`.
- **Never gate on a feature to make a lint go away.**
  `#[cfg_attr(not(feature = "x"), allow(dead_code))]` means the item is
  dead in some build — fix the build shape, don't silence it.

## Rules for touching one

- **Feature-gated dead code does not show up under `--all-features`.**
  A helper reachable only from a `#[cfg(feature = "x")]` caller is live
  with the feature and a `-D warnings` error without it. PR CI runs only
  the slim `all-features` lane, so this class breaks `main` after a green
  PR. Run all three legs locally when you add, move, or remove a gate —
  see `.claude/rules/review-discipline.md` ("Required checks").
- **Deleting a feature is not just deleting `#[cfg]` lines.** Also
  handle: `#[cfg(not(...))]` items (delete them — they are dead
  alternates), optional dependencies that become mandatory, forwards in
  every dependent manifest, `required-features` on `[[test]]` /
  `[[bin]]` targets, `.github/workflows/`, `Dockerfile*`,
  `scripts/ci/package-feature-flags.sh` and its self-test, and
  `docs/plans/composition-pubuse.snapshot` when the public facade
  changes.
- **Persisted strings are not feature references.**
  `SLACK_OUTBOUND_PROVIDER_KEY_PREFIX = "slack-v2-host-beta"` is a
  secret-store key prefix. Renaming it corrupts existing rows. Check
  before a global find-and-replace.

## Review flags

- A new `[features]` entry whose manifest comment does not name which
  bar it meets.
- A new `#[cfg(not(feature = ...))]` block containing a stub, a bail, or
  an error string that names the feature.
- A feature named anything other than `test-support` for a dev-only
  seam.
- A `-beta`/`-preview`/`-v2` suffixed feature that every shipped artifact
  enables.
- A feature added "so the substrate builds without X" when nothing
  actually builds without X.
