# Railway Extension Ownership Migration Packaging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the existing libSQL extension-ownership migration binary in the Reborn Railway runtime image without changing normal startup behavior.

**Architecture:** `Dockerfile.reborn` compiles the migration crate as a libSQL-only operator binary in the existing builder stage, then copies it into `/usr/local/bin` in the runtime stage. A `full-migration` feature keeps the legacy v1 read stack enabled by default while allowing the ownership-only operator binary to compile without the legacy root crate, whose source is intentionally absent from the Reborn Docker builder. The existing Reborn CLI smoke suite enforces both build and copy steps as a static deploy-artifact contract.

**Tech Stack:** Rust 2024, Cargo, Docker multi-stage builds, Rust smoke tests.

## Global Constraints

- Keep the existing `ironclaw-reborn-entrypoint` and default startup behavior unchanged.
- Build `ironclaw-reborn-extension-ownership-migration` with `--no-default-features --features libsql`.
- Do not add automatic migration execution, remote-libSQL support, or a separate migration image.
- Do not add new dependencies.

---

### Task 1: Package the migration binary

**Files:**
- Modify: `crates/ironclaw_reborn_cli/tests/smoke.rs`
- Modify: `Dockerfile.reborn`
- Modify: `crates/ironclaw_reborn_migration/Cargo.toml`
- Modify: `crates/ironclaw_reborn_migration/src/lib.rs`
- Modify: `crates/ironclaw_reborn_migration/src/report.rs`
- Modify: `crates/ironclaw_reborn_migration/src/target.rs`

**Interfaces:**
- Consumes: the existing `ironclaw-reborn-extension-ownership-migration` binary target from `ironclaw_reborn_migration`.
- Produces: `/usr/local/bin/ironclaw-reborn-extension-ownership-migration` in the Reborn runtime image.

- [x] **Step 1: Write the failing Dockerfile contract test**

Add `dockerfile_reborn_ships_extension_ownership_migration` to `crates/ironclaw_reborn_cli/tests/smoke.rs`. It must assert that the builder stage contains all of:

```text
--package ironclaw_reborn_migration
--no-default-features
--features libsql
--bin ironclaw-reborn-extension-ownership-migration
```

It must also assert that the runtime image contains this exact copy contract:

```dockerfile
COPY --from=builder /app/target/dist/ironclaw-reborn-extension-ownership-migration /usr/local/bin/ironclaw-reborn-extension-ownership-migration
```

- [x] **Step 2: Run the targeted test and verify RED**

Run:

```bash
cargo test -p ironclaw_reborn_cli --test smoke dockerfile_reborn_ships_extension_ownership_migration -- --exact
```

Expected: FAIL because `Dockerfile.reborn` does not yet build or copy the migration binary.

- [x] **Step 3: Add the minimal Dockerfile implementation**

Add this builder command after the existing CLI build:

```dockerfile
RUN cargo build \
    --profile dist \
    --package ironclaw_reborn_migration \
    --no-default-features \
    --features libsql \
    --bin ironclaw-reborn-extension-ownership-migration
```

Add this runtime copy beside the existing `ironclaw-reborn` copy:

```dockerfile
COPY --from=builder /app/target/dist/ironclaw-reborn-extension-ownership-migration /usr/local/bin/ironclaw-reborn-extension-ownership-migration
```

- [x] **Step 4: Keep the ownership-only build independent of the legacy root crate**

Make the existing full migration an explicit default feature and gate its source/conversion-only modules. The `--no-default-features --features libsql` ownership binary must not contain the workspace root `ironclaw` package in its dependency graph, while default builds must retain the existing full migration behavior.

- [x] **Step 5: Run targeted and crate verification**

Run:

```bash
cargo test -p ironclaw_reborn_cli --test smoke dockerfile_reborn_ships_extension_ownership_migration -- --exact
cargo test -p ironclaw_reborn_cli --test smoke dockerfile_reborn_builds_with_postgres_feature -- --exact
cargo test -p ironclaw_reborn_cli
cargo clippy -p ironclaw_reborn_cli --all-targets --all-features -- -D warnings
git diff --check
```

Expected: every command exits successfully with zero test failures and zero warnings.

- [x] **Step 6: Commit and publish**

```bash
git add Dockerfile.reborn crates/ironclaw_reborn_cli/tests/smoke.rs crates/ironclaw_reborn_migration/Cargo.toml crates/ironclaw_reborn_migration/src/lib.rs crates/ironclaw_reborn_migration/src/report.rs crates/ironclaw_reborn_migration/src/target.rs docs/superpowers/plans/2026-07-13-railway-extension-ownership-migration-packaging.md
git commit -m "build(reborn): ship extension ownership migration"
git push -u origin codex/ship-extension-ownership-migration
```

Open a PR describing the unchanged entrypoint, libSQL-only migration binary, smoke-test coverage, and QA backup/dry-run/apply procedure.
