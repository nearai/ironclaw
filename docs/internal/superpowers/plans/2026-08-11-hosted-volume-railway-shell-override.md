# Hosted Volume Railway Shell Override Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow `hosted-single-tenant-volume` to opt into the existing Railway sandbox shell with one strict environment variable while preserving its storage paths and exposing Railway-only shell guidance to the model.

**Architecture:** Keep the configured CLI profile separate from the effective runtime profile. The configured profile continues to select durable paths; base volume plus `IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL=true` resolves an effective `HostedSingleTenantVolumeSandboxedRailway` profile for policy and process binding. `ironclaw_extension_support` owns the Railway guidance asset, the CLI attaches it only alongside a Railway process binding, composition carries the opaque input without branching on its profile label, and provider-neutral host-runtime code appends it to both builtin-shell descriptor copies.

**Tech Stack:** Rust, Tokio, Cargo workspace tests, `thiserror`/`anyhow`, Markdown prompt assets loaded by `include_str!()`.

## Global Constraints

- The override name is exactly `IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL`.
- Only configured profile `hosted-single-tenant-volume` consumes the override.
- Accepted values are exactly the existing strict boolean grammar: `1`, `true`, `0`, `false`; malformed values fail only for the base volume profile.
- The configured base profile continues to select `hosted-single-tenant-volume` durable storage paths.
- The dedicated `hosted-single-tenant-volume-sandboxed-railway` profile remains supported.
- Railway shell never falls back to Docker or an unsandboxed host process.
- Railway guidance is present only for the effective Railway profile; local Docker receives generic user-sandbox guidance only.
- The guidance must state that sandbox `/workspace` is separate from the IronClaw user workspace and files do not automatically appear in both.
- Prompt content does not live in `ironclaw_composition` or a runtime lane; first-party extension support exports it as inert vendor-specific data.
- No new Cargo feature, serialized profile, dependency edge, or root integration-test file.
- No production `.unwrap()` or `.expect()`.

---

### Task 1: Resolve the release-only Railway shell alias without changing storage identity

**Files:**
- Modify: `crates/app/ironclaw_cli/src/runtime/mod.rs:708-812`
- Modify: `crates/app/ironclaw_cli/src/runtime/mod.rs:911-966`
- Modify: `crates/app/ironclaw_cli/src/runtime/mod.rs:2667-2810`
- Modify: `crates/app/ironclaw_composition/src/input.rs:335-610`
- Modify: `.env.example:243-252`
- Modify: `docs/reborn/deploy-reborn-cli-docker.md:119-139`

**Interfaces:**
- Consumes: `crate::operator_env::strict_bool_env_var(&str) -> anyhow::Result<Option<bool>>`, existing Railway process-binding environment validation, and `composition_profile(RebornProfile)`.
- Produces: `railway_shell_runtime_profile(RebornProfile) -> anyhow::Result<RebornProfile>` and a service build where `RuntimeServicesInput.profile` is configured identity while `RebornHostBindings::profile()` is effective runtime identity.

- [ ] **Step 0: Add the observation-only storage-root test seam**

Add a `#[cfg(any(test, feature = "test-support"))]` accessor named `local_filesystem_storage_root_for_test` on `RebornHostBindings`; it returns the actual root carried by `RebornStorageInput::LocalFilesystem` and `None` for other storage shapes. This accessor changes no production behavior and exists before the behavioral red test so that failure is specifically about the missing alias.

- [ ] **Step 1: Extend the existing CLI profile tests with the alias-positive red case**

Add the new env guard to the existing Railway fixture setup and add a focused test beside `railway_sandbox_profile_selects_remote_transport_without_connecting_docker`:

```rust
#[test]
fn hosted_volume_railway_shell_override_preserves_storage_profile() {
    let _lock = lock_runtime_env();
    let (_enabled, _interval) = clear_trigger_poller_env();
    let _override = EnvGuard::set(
        "IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL",
        "true",
    );
    let _project = EnvGuard::set("IRONCLAW_REBORN_RAILWAY_PROJECT_ID", "project-test");
    let _environment =
        EnvGuard::set("IRONCLAW_REBORN_RAILWAY_ENVIRONMENT_ID", "environment-test");
    let _project_token = EnvGuard::set("RAILWAY_TOKEN", "railway-test-token");
    let _api_token = EnvGuard::clear("RAILWAY_API_TOKEN");
    let _cli_path = EnvGuard::clear("IRONCLAW_REBORN_RAILWAY_CLI_PATH");
    let _idle_timeout = EnvGuard::clear("IRONCLAW_REBORN_RAILWAY_IDLE_TIMEOUT_MINUTES");
    let _worker_image = EnvGuard::clear("IRONCLAW_REBORN_RAILWAY_WORKER_IMAGE");
    let _docker = EnvGuard::set("DOCKER_HOST", "tcp://127.0.0.1:1");

    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    let config = RebornBootConfig::resolve_from_env_parts(
        Some(reborn_home.clone().into_os_string()),
        None,
        None,
        Some("hosted-single-tenant-volume".into()),
    )
    .expect("boot config");

    let runtime_services = build_services_input_with_options(
        &config,
        RuntimeInputCaller::Run,
        RuntimeInputOptions::default(),
    )
    .expect("runtime services");

    assert_eq!(
        runtime_services.profile,
        ironclaw_config::RebornProfile::HostedSingleTenantVolume
    );
    assert_eq!(
        runtime_services.services_input.profile(),
        RebornCompositionProfile::HostedSingleTenantVolumeSandboxedRailway
    );
    assert_eq!(
        runtime_services
            .services_input
            .runtime_policy()
            .expect("runtime policy")
            .process_backend
            .as_str(),
        "user_sandbox"
    );
    assert_eq!(
        runtime_services
            .services_input
            .local_filesystem_storage_root_for_test(),
        Some(reborn_home.join("hosted-single-tenant-volume").as_path())
    );
}
```

Import `build_services_input_with_options` in the test module. Clear the new env var in every existing base-volume test so ambient test state cannot opt it in accidentally.

- [ ] **Step 2: Run the alias-positive test and verify RED**

Run:

```bash
cargo test -p ironclaw --bin ironclaw hosted_volume_railway_shell_override_preserves_storage_profile
```

Expected: FAIL because the effective services profile remains `HostedSingleTenantVolume` and its process backend remains `none`.

- [ ] **Step 3: Add strict/scoped override tests before production changes**

Add one table-driven test for the resolver contract:

```rust
#[test]
fn railway_shell_override_is_strict_and_scoped_to_base_volume() {
    let _lock = lock_runtime_env();

    for enabled in ["1", "true"] {
        let _override = EnvGuard::set(
            "IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL",
            enabled,
        );
        assert_eq!(
            railway_shell_runtime_profile(
                ironclaw_config::RebornProfile::HostedSingleTenantVolume
            )
            .expect("valid override"),
            ironclaw_config::RebornProfile::HostedSingleTenantVolumeSandboxedRailway
        );
    }

    for disabled in ["0", "false"] {
        let _override = EnvGuard::set(
            "IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL",
            disabled,
        );
        assert_eq!(
            railway_shell_runtime_profile(
                ironclaw_config::RebornProfile::HostedSingleTenantVolume
            )
            .expect("valid override"),
            ironclaw_config::RebornProfile::HostedSingleTenantVolume
        );
    }

    let _override = EnvGuard::set(
        "IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL",
        "not-a-bool",
    );
    let error = railway_shell_runtime_profile(
        ironclaw_config::RebornProfile::HostedSingleTenantVolume,
    )
    .expect_err("base volume consumes malformed override");
    assert!(error.to_string().contains(
        "IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL must be one of 1, true, 0, false"
    ));

    for profile in ironclaw_config::RebornProfile::all()
        .iter()
        .copied()
        .filter(|profile| {
            *profile != ironclaw_config::RebornProfile::HostedSingleTenantVolume
        })
    {
        assert_eq!(
            railway_shell_runtime_profile(profile).expect("unrelated profile ignores override"),
            profile
        );
    }
}
```

The CLI's dev-dependency already enables composition `test-support`, so the caller-level test above inspects the assembled storage root rather than recomputing it. Also extend the storage-root initialization test to initialize using `RuntimeServicesInput.profile` from the alias-positive assembly and assert the base directory exists while `hosted-single-tenant-volume-sandboxed` does not.

- [ ] **Step 4: Run the strict/scoped tests and verify RED for the missing resolver**

Run:

```bash
cargo test -p ironclaw --bin ironclaw railway_shell_override_is_strict_and_scoped_to_base_volume
```

Expected: FAIL to compile because `railway_shell_runtime_profile` does not exist yet. This is the second red proof; do not add production code before recording it.

- [ ] **Step 5: Implement the minimal configured/effective profile split**

Add the constant and resolver near the existing Railway sandbox env constants:

```rust
const RAILWAY_SANDBOX_SHELL_OVERRIDE_ENV: &str =
    "IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL";

fn railway_shell_runtime_profile(profile: RebornProfile) -> anyhow::Result<RebornProfile> {
    if profile != RebornProfile::HostedSingleTenantVolume {
        return Ok(profile);
    }
    match crate::operator_env::strict_bool_env_var(RAILWAY_SANDBOX_SHELL_OVERRIDE_ENV)? {
        Some(true) => Ok(RebornProfile::HostedSingleTenantVolumeSandboxedRailway),
        Some(false) | None => Ok(profile),
    }
}
```

In `build_services_input_with_options`, keep both values:

```rust
let configured_profile = effective_profile(config, config_file.as_ref())?;
let runtime_profile = railway_shell_runtime_profile(configured_profile)?;
reject_unsupported_runtime_sections(config_file.as_ref(), caller, configured_profile)?;
```

Dispatch on `runtime_profile`, but pass `configured_profile` separately into local storage assembly. Refactor the two local builders to accept both names:

```rust
fn build_sandboxed_local_runtime_services_input(
    runtime_profile: RebornProfile,
    storage_profile: RebornProfile,
    owner_id: &str,
    config: &RebornBootConfig,
    options: RuntimeInputOptions,
) -> anyhow::Result<RebornHostBindings>
```

and:

```rust
fn build_standalone_local_runtime_services_input(
    runtime_profile: RebornProfile,
    storage_profile: RebornProfile,
    owner_id: &str,
    config: &RebornBootConfig,
    options: RuntimeInputOptions,
) -> anyhow::Result<RebornHostBindings> {
    let local_runtime_root = local_runtime_storage_root(config, storage_profile);
    let workspace_root = local_runtime_workspace_root(storage_profile)?;
    let services_input = local_runtime_build_input_with_options(
        composition_profile(runtime_profile),
        owner_id,
        local_runtime_root,
        RebornRuntimeProfileOptions {
            confirm_host_access: options.confirm_host_access,
        },
    )?;
    Ok(services_input.with_local_runtime_workspace_root(workspace_root))
}
```

All ordinary callers pass the same profile for both arguments. The alias path passes `(runtime_profile, configured_profile)`. In the live function, change only `local_runtime_storage_root(config, profile)` to use `storage_profile`, `local_runtime_workspace_root(profile)` to use `storage_profile`, and `composition_profile(profile)` plus its context message to use `runtime_profile`; leave the host-home and MCP-binding blocks unchanged. Resolve the memory binding policy from `composition_profile(runtime_profile)` so it agrees with the assembled deployment. Return `RuntimeServicesInput { profile: configured_profile, .. }` so storage-initialization consumers retain base identity.

- [ ] **Step 6: Run the focused CLI tests and verify GREEN**

Run:

```bash
cargo test -p ironclaw --bin ironclaw hosted_volume_railway_shell_override_preserves_storage_profile
cargo test -p ironclaw --bin ironclaw railway_shell_override_is_strict_and_scoped_to_base_volume
cargo test -p ironclaw --bin ironclaw build_runtime_input_accepts_hosted_single_tenant_volume_profile
cargo test -p ironclaw --bin ironclaw railway_sandbox_profile_selects_remote_transport_without_connecting_docker
```

Expected: PASS. The alias-positive test must not connect to Docker; the base default remains processless.

- [ ] **Step 7: Document the release-only operator switch**

Add this commented block beside the existing Railway sandbox variables in `.env.example`:

```text
# Release-only alias: for IRONCLAW_REBORN_PROFILE=hosted-single-tenant-volume,
# opt into the existing Railway user-sandbox shell without changing durable
# profile paths. Ignored by every other profile.
# IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL=false
```

Add the same activation rule, required Railway project/environment/token variables, storage-path preservation, and rollback-by-unset note under the hosted volume Railway section in `docs/reborn/deploy-reborn-cli-docker.md`.

- [ ] **Step 8: Commit Task 1**

```bash
git add crates/app/ironclaw_cli/src/runtime/mod.rs \
  crates/app/ironclaw_composition/src/input.rs \
  .env.example \
  docs/reborn/deploy-reborn-cli-docker.md
git commit -m "fix(cli): enable Railway shell for hosted volume profile"
```

---

### Task 2: Add Railway-only model guidance through the existing builtin-shell surface

**Files:**
- Create: `crates/extensions/ironclaw_extension_support/prompts/railway_shell.md`
- Modify: `crates/extensions/ironclaw_extension_support/src/lib.rs:1-40`
- Modify: `crates/app/ironclaw_cli/src/runtime/mod.rs:911-966`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/mod.rs:255-310`
- Modify: `crates/kernel/ironclaw_host_runtime/src/lib.rs:90-120`
- Modify: `crates/kernel/ironclaw_host_runtime/tests/first_party_builtin_tools.rs:442-497`
- Modify: `crates/app/ironclaw_composition/src/input.rs:167-245`
- Modify: `crates/app/ironclaw_composition/src/factory/production_build_assembly.rs:7-95`
- Modify: `crates/app/ironclaw_composition/src/factory.rs:1127-1169`
- Modify: `crates/app/ironclaw_composition/src/factory/tests.rs:1-120`
- Modify: `crates/app/ironclaw_composition/src/factory/production_backend_assembly.rs:513-517`
- Modify: `crates/app/ironclaw_composition/src/runtime/tests/core.rs:1044-1115`
- Modify: `crates/app/ironclaw_composition/src/runtime/tests/core.rs:3304-3355`

**Interfaces:**
- Consumes: CLI-selected supplemental shell guidance, `builtin_first_party_package_for_process_backend`, and the existing `ExtensionPackage` descriptor/manifest pair.
- Produces: `ironclaw_extension_support::RAILWAY_SHELL_CAPABILITY_GUIDANCE`, an opaque `RebornHostBindings` guidance field, and provider-neutral `ironclaw_host_runtime::append_builtin_shell_guidance(&mut ExtensionPackage, &str) -> Result<(), ExtensionError>`.

- [ ] **Step 1: Extend the existing Railway whole-turn test with model-visible guidance assertions**

In the first model-call branch of `SandboxShellCallingGateway`, capture the visible descriptor and provider tool definition and assert both contain stable clauses and are equal:

```rust
let surface = capabilities
    .visible_capabilities(VisibleCapabilityRequest)
    .await
    .map_err(model_capability_error)?;
let shell_id = CapabilityId::new(SHELL_CAPABILITY_ID).expect("valid built-in shell id");
let shell = surface
    .descriptors
    .iter()
    .find(|descriptor| descriptor.capability_id == shell_id)
    .expect("builtin shell must be visible for Railway sandbox profile");
for clause in [
    "fresh workers inside a Railway Sandbox",
    "Outbound internet uses Railway NAT",
    "Railway credentials and host-control tooling are not available",
    "separate from the IronClaw workspace",
    "do not automatically appear in both locations",
] {
    assert!(shell.safe_description.contains(clause), "missing {clause:?}");
}
let shell_tool = capabilities
    .tool_definitions()
    .map_err(model_capability_error)?
    .into_iter()
    .find(|definition| definition.capability_id == shell_id)
    .expect("shell provider tool definition");
assert_eq!(shell.safe_description, shell_tool.description);
```

Keep the existing request registration, sandbox transport, `sandboxed: true`, and shutdown assertions unchanged.

- [ ] **Step 2: Run the whole-turn test and verify RED**

Run:

```bash
cargo test -p ironclaw_composition --lib railway_sandbox_profile_routes_model_shell_call_to_user_sandbox_process_port
```

Expected: FAIL because the visible shell description contains only generic user-sandbox guidance and no Railway clauses.

- [ ] **Step 3: Add provider-neutral descriptor/manifest parity coverage**

Extend `builtin_first_party_process_backend_package_and_handlers_keep_shell`:

```rust
let mut railway_package =
    builtin_first_party_package_for_process_backend(ProcessBackendKind::UserSandbox).unwrap();
append_builtin_shell_guidance(
    &mut railway_package,
    "Additional provider-specific shell guidance.",
)
.unwrap();
let railway_shell = railway_package
    .capabilities
    .iter()
    .find(|descriptor| descriptor.id.as_str() == SHELL_CAPABILITY_ID)
    .expect("guided shell descriptor");
let railway_manifest_shell = railway_package
    .manifest
    .capabilities
    .iter()
    .find(|capability| capability.id.as_str() == SHELL_CAPABILITY_ID)
    .expect("guided shell manifest");
assert!(
    railway_shell
        .description
        .contains("Additional provider-specific shell guidance.")
);
assert_eq!(railway_manifest_shell.description, railway_shell.description);
```

Also assert the baseline `UserSandbox` and `LocalHost` packages do not contain `Railway`.

- [ ] **Step 4: Run the host-runtime test and verify RED for the missing helper**

Run:

```bash
cargo test -p ironclaw_host_runtime --test first_party_builtin_tools builtin_first_party_process_backend_package_and_handlers_keep_shell
```

Expected: FAIL to compile because `append_builtin_shell_guidance` is not exported yet.

- [ ] **Step 5: Add the Railway guidance asset to first-party extension support**

Create `crates/extensions/ironclaw_extension_support/prompts/railway_shell.md` with exactly:

```markdown
Shell commands run in fresh workers inside a Railway Sandbox. Persist files only under `/workspace`; processes, environment changes, working-directory changes, and system packages do not survive between calls. Outbound internet uses Railway NAT. Railway credentials and host-control tooling are not available inside the worker. The sandbox `/workspace` is separate from the IronClaw workspace where users save and manage files; files do not automatically appear in both locations.
```

Export it from `ironclaw_extension_support/src/lib.rs`:

```rust
pub const RAILWAY_SHELL_CAPABILITY_GUIDANCE: &str =
    include_str!("../prompts/railway_shell.md");
```

This asset records transport lifecycle facts only; it grants no authority and contains no credentials.

- [ ] **Step 6: Generalize the existing host-runtime append logic without naming Railway**

Refactor `append_user_sandbox_shell_guidance` to call a public neutral helper:

```rust
pub fn append_builtin_shell_guidance(
    package: &mut ExtensionPackage,
    guidance: &str,
) -> Result<(), ExtensionError> {
    let guidance = guidance.trim();
    if guidance.is_empty() {
        return Err(ExtensionError::InvalidManifest {
            reason: "built-in shell guidance must not be empty".to_string(),
        });
    }
    let capability_id = CapabilityId::new(SHELL_CAPABILITY_ID)?;
    let descriptor = package
        .capabilities
        .iter_mut()
        .find(|candidate| candidate.id == capability_id)
        .ok_or_else(|| ExtensionError::InvalidManifest {
            reason: format!("built-in first-party package is missing capability {capability_id}"),
        })?;
    let manifest = package
        .manifest
        .capabilities
        .iter_mut()
        .find(|candidate| candidate.id == capability_id)
        .ok_or_else(|| ExtensionError::InvalidManifest {
            reason: format!("built-in first-party manifest is missing capability {capability_id}"),
        })?;
    for description in [&mut descriptor.description, &mut manifest.description] {
        if !description.ends_with(' ') {
            description.push(' ');
        }
        description.push_str(guidance);
    }
    Ok(())
}
```

Keep the current generic guidance constant and implement `append_user_sandbox_shell_guidance` as `append_builtin_shell_guidance(package, GUIDANCE)`. Export only the neutral helper from `ironclaw_host_runtime::lib.rs`.

- [ ] **Step 7: Carry CLI-selected Railway guidance through composition as opaque data**

Add `supplemental_builtin_shell_guidance: Option<&'static str>` to `RebornHostBindings`, initialize it to `None`, expose a neutral `with_supplemental_builtin_shell_guidance(&'static str)` builder, and carry the field through `build_production_shaped` into `RebornProductionBuildContext`. Add a test-support getter for the carried value. In the CLI's Railway arm of `build_sandboxed_local_runtime_services_input`, attach `ironclaw_extension_support::RAILWAY_SHELL_CAPABILITY_GUIDANCE` after assembling the standalone input. The Docker arm attaches nothing. Because both the dedicated Railway profile and base-volume alias reach the same Railway arm, they receive identical guidance without a second profile check. Extend the alias-positive CLI test and the existing dedicated-Railway CLI test to assert the carried guidance contains `Railway Sandbox`; extend the local-Docker test to assert the getter returns `None`.

Add the opaque option to `production_builtin_extension_registry`. After constructing the process-backend-specific package and before lifecycle extensions are added:

```rust
let mut package = builtin_first_party_package_for_process_backend(process_backend).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party package is invalid: {error}"),
        }
    })?;
if let Some(guidance) = supplemental_builtin_shell_guidance {
    ironclaw_host_runtime::append_builtin_shell_guidance(
        &mut package,
        guidance,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("supplemental built-in shell guidance is invalid: {error}"),
    })?;
}
```

Do not key this on `RebornCompositionProfile` or `ProcessBackendKind::UserSandbox`; profiles are telemetry labels, and a backend-only branch would leak Railway wording into local Docker and other user-sandbox consumers.

- [ ] **Step 8: Add the input-negative assertion and verify GREEN**

In the host-runtime test, keep the baseline `UserSandbox` negative assertion. In a focused composition registry test, build two registries with `ProcessBackendKind::UserSandbox`, one with supplemental guidance and one without; assert only the explicitly guided registry's `builtin.shell` description contains `Railway Sandbox` and the IronClaw-workspace distinction. Name the test `supplemental_shell_guidance_is_input_specific`. Update the existing whole-turn Railway fixture to call `with_supplemental_builtin_shell_guidance(RAILWAY_SHELL_CAPABILITY_GUIDANCE)` explicitly before build, then retain the model-visible assertions from Step 1.

Run:

```bash
cargo test -p ironclaw_host_runtime --test first_party_builtin_tools builtin_first_party_process_backend_package_and_handlers_keep_shell
cargo test -p ironclaw_composition --lib railway_sandbox_profile_routes_model_shell_call_to_user_sandbox_process_port
cargo test -p ironclaw_composition --lib supplemental_shell_guidance_is_input_specific
```

Expected: PASS. The whole-turn test proves the model-visible descriptor and provider definition receive the guidance while the shell call still reaches the sandbox port.

- [ ] **Step 9: Run ownership and prompt-path gates**

Run:

```bash
cargo test -p ironclaw_extension_support
cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries composition_root_embeds_no_prompt_content
cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries composition_public_pub_use_surface_matches_snapshot
cargo test -p ironclaw_architecture_tests --test reborn_extension_specificity
bash scripts/ci/check-include-str-paths.sh
bash scripts/ci/check-composition-budget.sh
```

Expected: PASS. No Markdown or `include_str!` is added under composition, no new composition public re-export is required, and the prompt include path resolves.

- [ ] **Step 10: Commit Task 2**

```bash
git add crates/extensions/ironclaw_extension_support/prompts/railway_shell.md \
  crates/extensions/ironclaw_extension_support/src/lib.rs \
  crates/app/ironclaw_cli/src/runtime/mod.rs \
  crates/kernel/ironclaw_host_runtime/src/first_party_tools/mod.rs \
  crates/kernel/ironclaw_host_runtime/src/lib.rs \
  crates/kernel/ironclaw_host_runtime/tests/first_party_builtin_tools.rs \
  crates/app/ironclaw_composition/src/input.rs \
  crates/app/ironclaw_composition/src/factory/production_build_assembly.rs \
  crates/app/ironclaw_composition/src/factory.rs \
  crates/app/ironclaw_composition/src/factory/tests.rs \
  crates/app/ironclaw_composition/src/factory/production_backend_assembly.rs \
  crates/app/ironclaw_composition/src/runtime/tests/core.rs
git commit -m "fix(runtime): describe Railway sandbox shell"
```

---

### Task 3: Verify the complete release-branch patch and review it hard

**Files:**
- Verify: all files changed by Tasks 1 and 2
- Review output: `.review/findings.json` and `.review/findings.md` (ignored local artifacts)

**Interfaces:**
- Consumes: the two task commits and the approved design contract.
- Produces: red/green evidence, focused test results, architecture evidence, local code-review findings, and thermo-review disposition.

- [ ] **Step 1: Run formatting and focused regression suites**

```bash
cargo fmt --check
cargo test -p ironclaw --bin ironclaw hosted_volume_railway_shell_override_preserves_storage_profile
cargo test -p ironclaw --bin ironclaw railway_shell_override_is_strict_and_scoped_to_base_volume
cargo test -p ironclaw_host_runtime --test first_party_builtin_tools builtin_first_party_process_backend_package_and_handlers_keep_shell
cargo test -p ironclaw_composition --lib railway_sandbox_profile_routes_model_shell_call_to_user_sandbox_process_port
cargo test -p ironclaw_composition --lib supplemental_shell_guidance_is_input_specific
```

Expected: PASS with no warnings or skipped assertions.

- [ ] **Step 2: Run the narrow owning-crate and architecture suites**

```bash
cargo test -p ironclaw
cargo test -p ironclaw_extension_support
cargo test -p ironclaw_host_runtime
cargo test -p ironclaw_composition
cargo test -p ironclaw_architecture_tests
bash scripts/ci/check-include-str-paths.sh
bash scripts/ci/check-composition-budget.sh
python3 scripts/ci/docs_publication_boundary.py
git diff --check
```

Expected: PASS. If a broad suite exposes an unrelated pre-existing failure, record the exact command and prove the focused suites remain green; do not weaken tests.

- [ ] **Step 3: Audit changed production files for repository hazards**

```bash
git diff 2e69798d3...HEAD -- crates/app/ironclaw_cli/src/runtime/mod.rs \
  crates/app/ironclaw_composition/src \
  crates/kernel/ironclaw_host_runtime/src \
  crates/extensions/ironclaw_extension_support/src \
  | rg -n "\.unwrap\(|\.expect\(|map_err\(\|_\||/tmp|/private/tmp|\[[^]]+\.\."
```

Expected: no new production `.unwrap()`/`.expect()`, lost error causes, hardcoded temporary paths, or suspicious slicing.

- [ ] **Step 4: Run `$code-review` in Local mode**

Run the complete local multi-agent review against the feature diff using the approved intent. Treat missing CodeGraph in this persistent worktree as degraded evidence unless the user separately approves initialization. Address every actionable finding with confidence at least 50; rerun focused tests after fixes.

- [ ] **Step 5: Run `$thermo-nuclear-code-quality-review` on the final diff**

Use this required extra constraint verbatim:

> No over-engineering. Prefer the simplest direct fix that follows repo boundaries and deletes or avoids complexity. Do not demand abstraction unless it makes the code materially simpler.

Reject scattered profile checks, a composition-owned prompt asset, a Railway name in the neutral host-runtime helper, descriptor/manifest drift, or any implementation that uses the effective runtime profile for storage paths.

- [ ] **Step 6: Apply review fixes and repeat verification when needed**

For every accepted finding, add or adjust a regression assertion first when behavior changes, apply the minimal fix, rerun its focused test, then rerun formatting and the affected owning-crate suite. Repeat review only if the fixes materially change the design or execution path.

- [ ] **Step 7: Commit review fixes if any**

Stage each reviewed file explicitly (never `git add -A`), then run:

```bash
git commit -m "fix: address Railway shell override review"
```

Skip this commit when reviews produce no changes. Do not push or open a PR without an explicit user request.
