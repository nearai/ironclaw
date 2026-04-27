---
title: "feat: Add Kubernetes configuration to onboarding wizard"
type: feat
status: active
date: 2026-04-10
origin: docs/brainstorms/2026-04-10-kubernetes-onboarding-requirements.md
---

# feat: Add Kubernetes configuration to onboarding wizard

## Overview

Step 8 of the setup wizard is Docker-specific. This plan adds runtime
selection (Docker vs Kubernetes), namespace configuration, and
connectivity validation for Kubernetes, making the wizard runtime-aware
when both features are compiled in.

## Problem Frame

Users compiling IronClaw with `--features docker,kubernetes` have no
interactive way to select Kubernetes as their container runtime during
onboarding. The wizard always runs the Docker path. Users targeting
Kubernetes clusters get no guided setup for namespace or connectivity.
(See origin: `docs/brainstorms/2026-04-10-kubernetes-onboarding-requirements.md`)

## Requirements Trace

- R1. Runtime selection menu when both features compiled (default: Docker)
- R2. Skip menu when only one runtime feature compiled
- R3. Persist selected runtime to DB as `sandbox.container_runtime`
- R4. `resolve_runtime_backend()` respects DB setting; env var takes precedence
- R5. Prompt for namespace with default `ironclaw`
- R6. Validate cluster connectivity with chosen namespace
- R7. Retry-or-disable flow on connectivity failure (mirrors Docker UX)
- R8. Persist namespace to DB as `sandbox.k8s_namespace`
- R9. Runtime-agnostic introductory text in Step 8
- R10. Runtime-neutral confirm prompt

## Scope Boundaries

- No RBAC validation or permission checks during onboarding
- No image pull validation for Kubernetes
- No namespace auto-creation
- No orchestrator service name configuration (env-var only)
- No changes to quick mode (it does not run Step 8)
- No changes to `write_bootstrap_env()` -- these settings go in DB only

## Context & Research

### Relevant Code and Patterns

- `SandboxSettings` struct: `src/settings.rs` lines 674-753
- `SandboxModeConfig::resolve()`: `src/config/sandbox.rs` lines 57-120
- `db_first_or_default` / `db_first_bool`: `src/config/helpers.rs` lines 470-497
- `to_sandbox_config()`: `src/config/sandbox.rs` lines 122-158
- `SandboxConfig` (runtime struct): `src/sandbox/config.rs`
- `resolve_runtime_backend()` / `connect_runtime()`: `src/sandbox/runtime.rs`
- `step_docker_sandbox()`: `src/setup/wizard.rs` ~2866-2980
- `persist_after_step()`: `src/setup/wizard.rs` ~3392-3408
- Settings serialization: `Settings::to_db_map()` / `from_db_map()` in `src/settings.rs` -- fields are auto-serialized as dotted paths via serde, no manual mapping needed
- `KubernetesRuntime::connect()`: `src/sandbox/kubernetes.rs` lines 40-57

### Config Precedence Pattern

The established pattern in `SandboxModeConfig::resolve()` uses
`db_first_or_default(settings_value, default_value, "ENV_VAR_NAME")`:
- If `settings_value != default_value` → use settings (DB wins)
- Else → check env var → else use default

This plan follows the same pattern for `container_runtime` and
`k8s_namespace`.

## Key Technical Decisions

- **`resolve_runtime_backend()` gets an override parameter**: Changed from
  `fn resolve_runtime_backend()` to
  `fn resolve_runtime_backend(config_override: Option<&str>)`. Precedence:
  env var `CONTAINER_RUNTIME` > `config_override` (from DB) > compiled
  features default. This avoids setting process-level env vars from DB
  values, which is fragile. Call sites that have config pass the override;
  CLI tools without config pass `None`. (See origin key decision: "Persist
  to DB settings, not bootstrap .env")

- **Namespace follows the same precedence**: env var
  `IRONCLAW_K8S_NAMESPACE` > DB setting `sandbox.k8s_namespace` > default
  `"ironclaw"`. Resolved in `SandboxModeConfig`, threaded to
  `KubernetesRuntime` via `SandboxConfig`.

- **`KubernetesRuntime::connect()` accepts namespace parameter**: Changed
  from reading env var directly to accepting `namespace: &str` parameter.
  Centralizes resolution in the config layer instead of scattering env var
  reads across modules.

- **No changes to `write_bootstrap_env()`**: The runtime and namespace
  settings are not needed before DB connection. They follow the same path
  as `sandbox.enabled` -- DB settings loaded by `Config::from_db_with_toml()`.

## Open Questions

### Resolved During Planning

- **How does the DB runtime setting reach `resolve_runtime_backend()`?**
  Via an `Option<&str>` parameter. `SandboxModeConfig` resolves the value
  from DB + env, stores it, and downstream code (SandboxManager,
  JobManager, orchestrator) passes it through. Doctor CLI passes `None`.

- **Does `IRONCLAW_K8S_NAMESPACE` env var override the DB setting?**
  Yes. Follows `db_first_or_default` pattern: env var wins over DB when
  the DB value equals the default.

### Deferred to Implementation

- Exact error message wording for Kubernetes connectivity failures
  (follow Docker UX as closely as practical)
- Whether `step_claude_code_sandbox()` should run for Kubernetes
  (likely yes -- Claude Code mode is runtime-agnostic)

## Implementation Units

- [ ] **Unit 1: Add settings fields and config resolution**

  **Goal:** Add `container_runtime` and `k8s_namespace` to the settings
  and config layers so they are persisted, loaded, and available at runtime.

  **Requirements:** R3, R4, R8

  **Dependencies:** None

  **Files:**
  - Modify: `src/settings.rs` -- add fields to `SandboxSettings`
  - Modify: `src/config/sandbox.rs` -- add fields to `SandboxModeConfig`, resolve with `db_first_or_default`, carry to `to_sandbox_config()`
  - Modify: `src/sandbox/config.rs` -- add fields to `SandboxConfig`
  - Test: `src/config/sandbox.rs` (existing `tests` module)

  **Approach:**
  - Add `container_runtime: Option<String>` and `k8s_namespace: Option<String>` to `SandboxSettings` with `#[serde(default)]` (default `None`)
  - In `SandboxModeConfig::resolve()`, use `db_first_or_default` to resolve `container_runtime` against `CONTAINER_RUNTIME` env var (default: `None`), and `k8s_namespace` against `IRONCLAW_K8S_NAMESPACE` (default: `"ironclaw"`)
  - Add corresponding fields to `SandboxModeConfig` and `SandboxConfig`
  - `to_sandbox_config()` maps them through

  **Patterns to follow:**
  - `SandboxModeConfig::resolve()` existing field resolution pattern
  - `db_first_or_default` helper in `src/config/helpers.rs`
  - Other `SandboxSettings` fields like `policy`, `image`

  **Test scenarios:**
  - Happy path: `container_runtime` set in settings, no env var → resolved value matches settings
  - Happy path: `k8s_namespace` set to `"custom-ns"` in settings → resolved to `"custom-ns"`
  - Edge case: `container_runtime` set in settings AND `CONTAINER_RUNTIME` env var set → env var wins
  - Edge case: `k8s_namespace` left at default in settings, `IRONCLAW_K8S_NAMESPACE` env var set → env var wins
  - Happy path: settings round-trip via `to_db_map()` / `from_db_map()` preserves both new fields

  **Verification:**
  - `cargo test -p ironclaw config::sandbox` passes
  - New fields survive DB serialization round-trip

- [ ] **Unit 2: Thread runtime override into resolve_runtime_backend**

  **Goal:** Make `resolve_runtime_backend()` accept a config-provided
  override so DB-persisted runtime selection is respected at startup.

  **Requirements:** R4

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `src/sandbox/runtime.rs` -- add parameter to `resolve_runtime_backend()` and `connect_runtime()`
  - Modify: `src/sandbox/mod.rs` -- update re-exports if needed
  - Modify: `src/sandbox/manager.rs` -- pass override from config
  - Modify: `src/orchestrator/job_manager.rs` -- pass override from config
  - Modify: `src/orchestrator/mod.rs` -- pass override in `detect_and_connect_runtime()`
  - Modify: `src/cli/doctor.rs` -- pass `None`
  - Test: `src/sandbox/runtime.rs` (existing `tests` module)

  **Approach:**
  - Change signature: `resolve_runtime_backend(config_override: Option<&str>)`
  - Precedence: `CONTAINER_RUNTIME` env var > `config_override` > `default_backend_for_compiled_features()`
  - `connect_runtime(config_override: Option<&str>)` passes through
  - `SandboxManager` needs access to the config-resolved `container_runtime`. It already stores a `SandboxConfig` -- read from there
  - `ContainerJobManager` stores config similarly -- read from there
  - `setup_orchestrator()` in `orchestrator/mod.rs` receives config -- pass the value
  - Doctor CLI has no config at call time -- pass `None` (env var + compiled features still work)

  **Patterns to follow:**
  - Existing `resolve_runtime_backend()` structure
  - How `SandboxManager::new()` receives `SandboxConfig`

  **Test scenarios:**
  - Happy path: `config_override` = `Some("kubernetes")`, no env var → resolves to `Kubernetes`
  - Happy path: `config_override` = `None`, no env var → falls through to compiled features default
  - Edge case: env var `CONTAINER_RUNTIME=docker`, `config_override` = `Some("kubernetes")` → env var wins, resolves to `Docker`
  - Error path: `config_override` = `Some("kubernetes")`, `kubernetes` feature not compiled → error message
  - Happy path: `config_override` = `None`, no env var, only `kubernetes` compiled → resolves to `Kubernetes`

  **Verification:**
  - `cargo test -p ironclaw sandbox::runtime` passes
  - `cargo check --all-features` clean
  - `cargo check --no-default-features --features kubernetes` clean

- [ ] **Unit 3: Thread namespace to KubernetesRuntime**

  **Goal:** `KubernetesRuntime::connect()` uses the config-resolved
  namespace instead of reading the env var directly.

  **Requirements:** R5, R8

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `src/sandbox/kubernetes.rs` -- change `connect()` to accept namespace parameter
  - Modify: `src/sandbox/runtime.rs` -- `connect_runtime_backend()` passes namespace from config
  - Modify: `src/orchestrator/mod.rs` -- pass namespace when connecting K8s runtime
  - Test: `src/sandbox/kubernetes.rs` (existing `tests` module)

  **Approach:**
  - Change `KubernetesRuntime::connect()` to `connect(namespace: &str)`
  - Remove the internal `IRONCLAW_K8S_NAMESPACE` env var read from `connect()`
  - `connect_runtime_backend()` needs access to namespace when backend is Kubernetes. Add a `namespace: Option<&str>` parameter or accept a small config struct
  - The orchestrator service name can still come from env var (out of scope for this plan per scope boundaries)

  **Patterns to follow:**
  - How `DockerRuntime::connect()` is a no-arg connect (Docker has no namespace concept)
  - Keep the asymmetry acceptable -- `connect_runtime_backend` can pattern-match on `RuntimeBackend::Kubernetes` to pass namespace

  **Test scenarios:**
  - Happy path: `connect("custom-ns")` → runtime uses `"custom-ns"` as namespace
  - Happy path: existing `build_pod_spec` tests still pass (they already accept namespace as parameter)

  **Verification:**
  - `cargo check --all-features` clean
  - Existing Kubernetes unit tests pass

- [ ] **Unit 4: Rewrite wizard Step 8 for runtime-aware flow**

  **Goal:** The wizard presents runtime-agnostic copy, a runtime selection
  menu (when both compiled), and a Kubernetes configuration flow with
  namespace prompt and connectivity validation.

  **Requirements:** R1, R2, R5, R6, R7, R9, R10

  **Dependencies:** Units 1, 2, 3

  **Files:**
  - Modify: `src/setup/wizard.rs` -- rewrite `step_docker_sandbox()`

  **Approach:**
  - Rename internal method to `step_container_sandbox()` (update call site in `run()`)
  - Replace Docker-specific intro copy with runtime-agnostic text
  - Change confirm prompt to "Enable container sandbox?"
  - When both features compiled: present `select_one("Container runtime", &["Docker", "Kubernetes"])` menu
  - When only one feature: skip menu, use that runtime
  - Store selection in `self.settings.sandbox.container_runtime`
  - **Docker path:** existing flow unchanged (detect, retry, ensure_worker_image)
  - **Kubernetes path:**
    1. Prompt namespace: `input_with_default("Kubernetes namespace", "ironclaw")`
    2. Store in `self.settings.sandbox.k8s_namespace`
    3. Connect and check: `KubernetesRuntime::connect(&namespace)` + `is_available()`
    4. On failure: print error, `confirm("Retry?")`, if retry fails → disable
  - `persist_after_step()` saves both new fields to DB
  - `step_claude_code_sandbox()` still runs if sandbox is enabled (runtime-agnostic)

  **Patterns to follow:**
  - `step_docker_sandbox()` existing structure for Docker path
  - `select_one()` usage in `step_database()` for menu pattern
  - `confirm()` + retry pattern already in Docker path

  **Test scenarios:**
  - This unit is interactive wizard code with no pure-logic test targets.
    Verification is via compilation and manual onboarding flow.

  **Verification:**
  - `cargo check --all-features` clean
  - `cargo check --no-default-features --features kubernetes` clean
  - Manual: `cargo run -- onboard` with both features shows runtime menu

- [ ] **Unit 5: Update documentation**

  **Goal:** Update setup README, CHANGELOG, and .env.example to reflect
  the new wizard behavior.

  **Requirements:** R9, R10

  **Dependencies:** Unit 4

  **Files:**
  - Modify: `src/setup/README.md` -- update Step 8 section
  - Modify: `CHANGELOG.md` -- add entry under Unreleased
  - Modify: `.env.example` -- document `sandbox.container_runtime` and `sandbox.k8s_namespace` as DB settings

  **Approach:**
  - Step 8 section in README: update to describe the runtime selection
    menu, Kubernetes namespace prompt, and connectivity flow
  - CHANGELOG: "Added: Interactive container runtime selection in onboarding wizard (Docker/Kubernetes)"
  - .env.example: add a comment explaining that `CONTAINER_RUNTIME` and
    `IRONCLAW_K8S_NAMESPACE` can also be set via the wizard and stored in DB

  **Patterns to follow:**
  - Existing Step 8 section structure in `src/setup/README.md`
  - CHANGELOG format

  **Test scenarios:**
  - N/A (documentation only)

  **Verification:**
  - README Step 8 section matches the implemented wizard behavior
  - CHANGELOG has the new entry

## System-Wide Impact

- **Interaction graph:** `resolve_runtime_backend()` gains a parameter,
  affecting all 6 call sites. Each is updated in Unit 2 with pass-through
  of `None` or config-derived value.
- **Error propagation:** Unchanged. Kubernetes connectivity errors in the
  wizard result in sandbox being disabled, same as Docker path.
- **State lifecycle risks:** None. New settings fields are `Option<String>`
  with `None` default, backward-compatible with existing DB rows.
- **API surface parity:** The `ironclaw config set sandbox.container_runtime kubernetes`
  CLI path will work automatically via `Settings::set()` once the field
  exists. No additional CLI work needed.
- **Unchanged invariants:** Docker-only onboarding path is preserved
  exactly. Quick mode is unchanged (does not run Step 8).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Adding `Option<&str>` parameter to `resolve_runtime_backend()` touches 6 call sites | Mechanical change, all call sites identified. Doctor CLI passes `None` for backward compat |
| `KubernetesRuntime::connect()` signature change breaks existing callers | Only 3 call sites: `connect_runtime_backend()`, wizard, doctor. All updated in this plan |
| Settings migration: old DBs lack new fields | `#[serde(default)]` on new fields → `None` on load → falls through to env var / compiled default. Zero-migration |

## Sources & References

- **Origin document:** [docs/brainstorms/2026-04-10-kubernetes-onboarding-requirements.md](docs/brainstorms/2026-04-10-kubernetes-onboarding-requirements.md)
- Settings persistence: `src/settings.rs` (`to_db_map`, `from_db_map`)
- Config resolution: `src/config/sandbox.rs` (`SandboxModeConfig::resolve`)
- Runtime selection: `src/sandbox/runtime.rs` (`resolve_runtime_backend`)
- Wizard: `src/setup/wizard.rs` (`step_docker_sandbox`)
