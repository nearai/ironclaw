//! Unit-test matrix for the hook activation / projection / factory path.
//!
//! Moved out of the production module (serrrfirat's #3951 P1 finding #4: keep
//! the production hooks modules focused; the test matrix lives here).

use super::*;
// The activation logic was decomposed into focused submodules (#3951 finding
// #4); the test matrix reaches every production item — including the
// crate-private `pub(super)` ones — through these submodule globs.
use super::factory::*;
use super::projection::*;

use std::sync::Arc;

use ironclaw_hooks::dispatch::HookDispatcherBuilder;

use crate::error::RebornBuildError;
use ironclaw_extensions::ExtensionRegistry;

use ironclaw_extensions::v2::ManifestSource;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage};
use ironclaw_hooks::HookPhase;
use ironclaw_hooks::identity::{HookId, HookVersion};
use ironclaw_hooks::points::ObserverHookContext;
use ironclaw_hooks::registry::HookPointSpec;
use ironclaw_hooks::sink::{ObserverHook, ObserverSink};
use ironclaw_host_api::{HostPortCatalog, VirtualPath};

/// Canonical identity path for the TEST-ONLY first-party no-op observer.
/// Lives here (not in the production catalog) so the activation machinery
/// can be exercised end-to-end through the real composition path without
/// shipping a no-op hook in production.
const TEST_NOOP_OBSERVER_CANONICAL_PATH: &str =
    "ironclaw_reborn_composition::hooks::tests::NoOpObserverHook";

/// A test-only first-party no-op observer. Observers cannot affect
/// outcomes; this one records nothing. It proves the builtin install +
/// dispatch path end to end through `build_hook_dispatcher_builder_factory_with`.
#[derive(Debug, Default)]
struct NoOpObserverHook;

#[async_trait::async_trait]
impl ObserverHook for NoOpObserverHook {
    async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {}
}

/// Test-only first-party installer: installs the [`NoOpObserverHook`] at
/// `AfterCapability`. Passed to
/// [`build_hook_dispatcher_builder_factory_with`] in place of the empty
/// production catalog so the activation machinery is covered with a real
/// first-party binding.
fn install_test_first_party_hook(
    builder: HookDispatcherBuilder,
) -> Result<HookDispatcherBuilder, RebornBuildError> {
    let hook_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
    builder
        .install_builtin_observer(
            hook_id,
            HookPhase::Telemetry,
            HookPointSpec::AfterCapability,
            Box::new(NoOpObserverHook),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("failed to install test first-party no-op observer hook: {error}"),
        })
}

/// Build a single-capability `reborn.extension_manifest.v2` manifest TOML
/// for `id`, optionally carrying a `[[hooks]]` block. The capability id is
/// provider-prefixed (`<id>.run`) as `ExtensionPackage::from_manifest`
/// requires.
fn manifest_toml(id: &str, hooks_block: &str) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "{id} extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[capabilities]]
id = "{id}.run"
description = "Run {id}"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"
{hooks_block}"#
    )
}

/// Parse `toml` into a validated package rooted at the conventional
/// extensions path and insert it into a fresh registry. `source` selects the
/// trust posture: `InstalledLocal` (untrusted → quarantine on bad hooks) vs
/// `HostBundled` (trusted → fail-closed-whole-build).
fn registry_with_manifest_source(
    id: &str,
    toml: &str,
    source: ManifestSource,
) -> ExtensionRegistry {
    let manifest =
        ExtensionManifest::parse(toml, source, &HostPortCatalog::empty()).expect("manifest parses");
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{id}")).expect("valid root path"),
    )
    .expect("package builds from manifest");
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).expect("package inserts");
    registry
}

/// As [`registry_with_manifest_source`] with the default untrusted
/// (`InstalledLocal`) source.
fn registry_with_manifest(id: &str, toml: &str) -> ExtensionRegistry {
    registry_with_manifest_source(id, toml, ManifestSource::InstalledLocal)
}

/// Extract a plain registry's hook-only projections into a
/// [`HookProjectionRegistry`] for the hook-factory tests (the factory only
/// accepts the hook-only newtype). Mirrors the production extraction.
fn projection(registry: ExtensionRegistry) -> HookProjectionRegistry {
    HookProjectionRegistry::from_projections(
        registry
            .extensions()
            .filter_map(HookProjection::from_package)
            .collect(),
    )
}

#[test]
fn config_defaults_to_disabled() {
    assert!(!HooksActivationConfig::default().is_enabled());
    assert!(!HooksActivationConfig::disabled().is_enabled());
    assert!(HooksActivationConfig::enabled().is_enabled());
}

#[test]
fn third_party_flag_requires_master_flag_and_defaults_off() {
    // Default + master-only configs must report third-party OFF.
    assert!(!HooksActivationConfig::default().is_third_party_enabled());
    assert!(!HooksActivationConfig::disabled().is_third_party_enabled());
    assert!(
        !HooksActivationConfig::enabled().is_third_party_enabled(),
        "master flag alone must NOT enable third-party"
    );
    // Sub-flag on but master off ⇒ still OFF (master gate dominates).
    assert!(
        !HooksActivationConfig::disabled()
            .with_third_party_enabled(true)
            .is_third_party_enabled(),
        "sub-flag without the master flag must stay OFF"
    );
    // Both on ⇒ ON.
    assert!(
        HooksActivationConfig::enabled()
            .with_third_party_enabled(true)
            .is_third_party_enabled(),
        "master + sub-flag both on must enable third-party"
    );
    // Setting the sub-flag must never silently flip the master flag.
    assert!(
        !HooksActivationConfig::disabled()
            .with_third_party_enabled(true)
            .is_enabled()
    );
}

#[test]
fn truthy_tokens_enable_only_canonical_values() {
    for token in ["1", "true", "TRUE", "Yes", "on", " on "] {
        assert!(is_truthy(token), "{token:?} should be truthy");
    }
    for token in ["0", "false", "", "off", "no", "enabled", "2", "tru"] {
        assert!(!is_truthy(token), "{token:?} should be falsy");
    }
}

#[test]
fn disabled_config_yields_no_factory() {
    let registry = projection(ExtensionRegistry::new());
    let result =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::disabled(), &registry)
            .expect("disabled build never errors");
    assert!(result.is_none(), "flag OFF must compose no dispatcher");
}

#[test]
fn enabled_config_with_empty_production_catalog_yields_valid_zero_binding_factory() {
    // The PRODUCTION first-party catalog is empty. Flag ON + empty
    // first-party set + no extension hooks must still compose a valid
    // dispatcher — a zero-binding dispatcher, not a panic/error. This pins
    // the empty-catalog-is-valid contract.
    let registry = projection(ExtensionRegistry::new());
    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("enabled build with empty registry + empty catalog succeeds")
            .expect("flag ON yields a factory even with an empty catalog");
    // The factory mints a valid dispatcher with no first-party bindings.
    let dispatcher = factory().expect("mint hook builder").build_arc();
    let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
    assert!(
        bindings.is_empty(),
        "empty production catalog must yield zero first-party bindings, saw {bindings:?}"
    );
}

#[test]
fn activation_installs_a_test_first_party_hook_through_the_real_path() {
    // The activation machinery is exercised end-to-end with a TEST-ONLY
    // first-party hook (not a production-shipped no-op). We drive the same
    // composition path via the `*_with` seam and confirm the test hook is
    // bound at AfterCapability.
    let registry = projection(ExtensionRegistry::new());
    let factory = build_hook_dispatcher_builder_factory_with(
        HooksActivationConfig::enabled(),
        &registry,
        None,
        install_test_first_party_hook,
    )
    .expect("enabled build with a test first-party hook succeeds")
    .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();
    let test_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
    let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
    assert!(
        bindings.iter().any(|binding| binding.hook_id == test_id),
        "test-only first-party hook must be installed through the real composition path"
    );
}

#[test]
fn factory_mints_independent_dispatchers_per_call() {
    let registry = projection(ExtensionRegistry::new());
    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("enabled build succeeds")
            .expect("flag ON yields a factory");
    let a = factory().expect("mint hook builder").build_arc();
    let b = factory().expect("mint hook builder").build_arc();
    assert!(
        !Arc::ptr_eq(&a, &b),
        "each factory call must mint a fresh dispatcher (per-run isolation)"
    );
}

// ─── Helpers for the projection / quarantine test matrix ─────────────────

use ironclaw_hooks::identity::{ExtensionId as HookExtensionId, HookLocalId};

/// Derive the registrar-minted hook id for an installed predicate hook.
fn installed_hook_id(ext: &str, version: &str, local: &str) -> HookId {
    HookId::derive(
        &HookExtensionId::new(ext).expect("valid extension id"),
        version,
        &HookLocalId::new(local).expect("valid hook local id"),
        HookVersion::ONE,
    )
}

/// `[[hooks]]` block for a valid `own_capabilities` deny predicate (needs no
/// grant, no WASM runtime).
fn own_deny_hook(local: &str, target: &str) -> String {
    format!(
        r#"
[[hooks]]
id = "{local}"
kind = "before_capability"
scope = "own_capabilities"
body = {{ mode = "predicate", spec = {{ type = "deny_capability", reason = "blocked by manifest hook", when = {{ type = "name_equals", name = "{target}" }} }} }}
"#
    )
}

/// Build a [`HookProjectionRegistry`] directly from `(id, source, hooks)`
/// triples (no discovery), driving the real install-time projection path.
fn projection_with(packages: &[(&str, ManifestSource, String)]) -> HookProjectionRegistry {
    let mut registry = ExtensionRegistry::new();
    for (id, source, hooks_block) in packages {
        let manifest = ExtensionManifest::parse(
            &manifest_toml(id, hooks_block),
            *source,
            &HostPortCatalog::empty(),
        )
        .expect("manifest parses");
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{id}")).expect("valid root path"),
        )
        .expect("package builds from manifest");
        registry.insert(package).expect("package inserts");
    }
    projection(registry)
}

// ─── Atomic quarantine + trust-discrimination coverage ───────────────────

/// A valid `own_capabilities` predicate hook from an untrusted
/// (`InstalledLocal`) extension installs through the real
/// `HookRegistrar::install` path at the `Installed` tier, alongside the
/// test-only first-party hook (extension install does not displace
/// first-party).
#[test]
fn valid_extension_hook_manifest_installs_at_installed_tier() {
    let registry = projection(registry_with_manifest(
        "valid-ext",
        &manifest_toml("valid-ext", &own_deny_hook("deny-run", "valid-ext.run")),
    ));

    let factory = build_hook_dispatcher_builder_factory_with(
        HooksActivationConfig::enabled(),
        &registry,
        None,
        install_test_first_party_hook,
    )
    .expect("enabled build with a valid extension hook succeeds")
    .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();

    let expected = installed_hook_id("valid-ext", "0.1.0", "deny-run");
    let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);
    assert!(
        bindings.iter().any(|binding| binding.hook_id == expected),
        "installed extension hook must be bound at BeforeCapability; saw {bindings:?}"
    );

    let test_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
    let after = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
    assert!(
        after.iter().any(|binding| binding.hook_id == test_id),
        "test-only first-party hook must remain installed alongside extension hooks"
    );
}

/// A malformed hook payload from an UNTRUSTED (`InstalledLocal`) extension
/// must be QUARANTINED — the build SUCCEEDS, that extension's hook is
/// absent, and (critically) no panic. This is the third-party degradation
/// contract: an attacker-controlled installed manifest cannot crash
/// composition nor fail the whole build.
#[test]
fn malformed_installed_extension_hook_is_quarantined_not_fatal() {
    let hooks_block = r#"
[[hooks]]
id = "broken-hook"
kind = "before_capability"
body = { mode = "nonsense" }
"#;
    let registry = projection(registry_with_manifest(
        "broken-ext",
        &manifest_toml("broken-ext", hooks_block),
    ));

    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("malformed INSTALLED manifest must NOT fail the build (quarantine)")
            .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();
    assert!(
        dispatcher
            .active_bindings_snapshot(HookPointSpec::BeforeCapability)
            .is_empty(),
        "quarantined extension must contribute no bindings"
    );
}

/// The SAME malformed payload from a TRUSTED (`HostBundled`) package must
/// fail the whole build closed with `InvalidConfig` — builtin/host-bundled
/// hooks are fail-closed-whole-build, never quarantined.
#[test]
fn malformed_host_bundled_extension_hook_fails_closed() {
    let hooks_block = r#"
[[hooks]]
id = "broken-hook"
kind = "before_capability"
body = { mode = "nonsense" }
"#;
    // HostBundled ids are reserved to the `ironclaw.` prefix.
    let registry = projection(registry_with_manifest_source(
        "ironclaw.broken",
        &manifest_toml("ironclaw.broken", hooks_block),
        ManifestSource::HostBundled,
    ));

    match build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry) {
        Err(RebornBuildError::InvalidConfig { reason }) => {
            assert!(
                reason.contains("ironclaw.broken") && reason.contains("broken-hook"),
                "fail-closed error must name the offending host-bundled extension + hook, got: {reason}"
            );
        }
        Ok(_) => panic!("malformed host-bundled manifest must fail the whole build"),
        Err(other) => panic!("expected InvalidConfig, got: {other}"),
    }
}

/// Atomic quarantine: an extension with two VALID hooks and one INVALID
/// hook must install NONE of its three hooks (whole-set atomicity), while a
/// sibling valid extension's hook IS installed.
#[test]
fn extension_with_one_invalid_hook_quarantines_the_whole_set_sibling_survives() {
    let bad_set = format!(
        "{}{}{}",
        own_deny_hook("ok-1", "mixed.run"),
        own_deny_hook("ok-2", "mixed.run"),
        // invalid third hook
        r#"
[[hooks]]
id = "bad-3"
kind = "before_capability"
body = { mode = "nonsense" }
"#
    );
    let registry = projection_with(&[
        ("mixed", ManifestSource::InstalledLocal, bad_set),
        (
            "good",
            ManifestSource::InstalledLocal,
            own_deny_hook("good-1", "good.run"),
        ),
    ]);

    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("partial-invalid set must quarantine, not fail the build")
            .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();
    let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);

    // None of `mixed`'s hooks installed.
    for local in ["ok-1", "ok-2", "bad-3"] {
        let id = installed_hook_id("mixed", "0.1.0", local);
        assert!(
            !bindings.iter().any(|b| b.hook_id == id),
            "atomic quarantine must drop ALL of the offending extension's hooks ({local} leaked)"
        );
    }
    // Sibling `good` survives.
    let good_id = installed_hook_id("good", "0.1.0", "good-1");
    assert!(
        bindings.iter().any(|b| b.hook_id == good_id),
        "a sibling valid extension's hooks must still install"
    );
}

/// An untrusted extension claiming `scope = same_tenant` (a wider scope
/// than its own capabilities) with no host-verified grant is QUARANTINED by
/// the registrar's trust-attenuation check — build succeeds, hook absent.
#[test]
fn installed_extension_claiming_ungranted_wider_scope_is_quarantined() {
    let hooks_block = r#"
[[hooks]]
id = "cross-tenant-deny"
kind = "before_capability"
scope = "same_tenant"
requires_grant = "cross-tenant-policy"
body = { mode = "predicate", spec = { type = "deny_capability", reason = "wider-scope deny", when = { type = "name_equals", name = "other-ext.run" } } }
"#;
    let registry = projection(registry_with_manifest(
        "reachy-ext",
        &manifest_toml("reachy-ext", hooks_block),
    ));

    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("ungranted wider-scope INSTALLED hook must quarantine, not fail the build")
            .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();
    assert!(
        dispatcher
            .active_bindings_snapshot(HookPointSpec::BeforeCapability)
            .is_empty(),
        "ungranted wider-scope hook must be quarantined (no binding)"
    );
}

/// Third-party WASM stays OUT: the projection registrar has no
/// `wasm_runtime`, so a WASM-bodied installed hook fails install → under
/// quarantine the extension is dropped and the build continues (Step 6
/// negative test). A sibling predicate-only extension still installs.
#[test]
fn wasm_bodied_third_party_hook_is_quarantined_build_continues() {
    let wasm_block = r#"
[[hooks]]
id = "wasm-hook"
kind = "before_capability"
scope = "own_capabilities"
body = { mode = "wasm", export = "evaluate" }
"#;
    let registry = projection_with(&[
        (
            "wasm-ext",
            ManifestSource::InstalledLocal,
            wasm_block.to_string(),
        ),
        (
            "pred-ext",
            ManifestSource::InstalledLocal,
            own_deny_hook("pred-1", "pred-ext.run"),
        ),
    ]);

    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("WASM-bodied third-party hook must quarantine, not fail the build")
            .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();
    let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);
    assert!(
        !bindings
            .iter()
            .any(|b| b.hook_id == installed_hook_id("wasm-ext", "0.1.0", "wasm-hook")),
        "WASM-bodied third-party hook must be quarantined (no runtime in loader registrar)"
    );
    assert!(
        bindings
            .iter()
            .any(|b| b.hook_id == installed_hook_id("pred-ext", "0.1.0", "pred-1")),
        "sibling predicate extension must still install after a WASM quarantine"
    );
}

/// Containment: a package whose root escapes the tenant root via `..` is
/// rejected by `enforce_root_containment` (FS-hardening v1).
#[test]
fn root_containment_rejects_traversal_and_non_child() {
    let tenant_root = VirtualPath::new("/system/extensions/alpha").expect("root");
    // Strict child OK.
    assert!(
        enforce_root_containment(
            &tenant_root,
            &VirtualPath::new("/system/extensions/alpha/ext-1").expect("child")
        )
        .is_ok()
    );
    // Sibling tenant is not a child.
    assert!(
        enforce_root_containment(
            &tenant_root,
            &VirtualPath::new("/system/extensions/beta/ext-1").expect("sibling")
        )
        .is_err(),
        "another tenant's tree must not be a child of this tenant root"
    );
    // The tenant root itself is not a strict child.
    assert!(enforce_root_containment(&tenant_root, &tenant_root).is_err());
}

/// `tenant_extension_root` returns the fixed `/system/extensions` root
/// (Option 1 — FS-scoped isolation; the per-tenant `RootFilesystem`, not a
/// path segment, is the isolation boundary). The signature still requires
/// identity so callers must thread it (and the containment defense knows the
/// root), but the path is profile-independent. The cross-tenant proof lives
/// in the integration test driving two distinct per-tenant filesystems.
#[test]
fn tenant_root_is_the_fixed_system_extensions_root() {
    let a = tenant_extension_root(&ironclaw_host_api::TenantId::new("alpha").expect("a"))
        .expect("root a");
    let b = tenant_extension_root(&ironclaw_host_api::TenantId::new("beta").expect("b"))
        .expect("root b");
    assert_eq!(a.as_str(), "/system/extensions");
    assert_eq!(b.as_str(), "/system/extensions");
}

/// DoS cap: more than `MAX_INSTALLED_EXTENSIONS_CONSIDERED` hook-bearing
/// untrusted extensions ⇒ the surplus is quarantined (skipped), and the
/// build still succeeds. We use a small synthetic set keyed off the const
/// boundary so the test stays fast yet pins the ceiling.
#[test]
fn surplus_extensions_beyond_consider_cap_are_quarantined() {
    let mut packages: Vec<(String, ManifestSource, String)> = Vec::new();
    for i in 0..(MAX_INSTALLED_EXTENSIONS_CONSIDERED + 2) {
        let id = format!("ext-{i:03}");
        let hooks = own_deny_hook("h", &format!("{id}.run"));
        packages.push((id, ManifestSource::InstalledLocal, hooks));
    }
    let refs: Vec<(&str, ManifestSource, String)> = packages
        .iter()
        .map(|(id, src, hooks)| (id.as_str(), *src, hooks.clone()))
        .collect();
    let registry = projection_with(&refs);

    let factory =
        build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
            .expect("surplus extensions must quarantine, not fail the build")
            .expect("flag ON yields a factory");
    let dispatcher = factory().expect("mint hook builder").build_arc();
    let installed = dispatcher
        .active_bindings_snapshot(HookPointSpec::BeforeCapability)
        .len();
    assert!(
        installed <= MAX_INSTALLED_EXTENSIONS_CONSIDERED,
        "no more than the consider-cap of extensions may install (saw {installed})"
    );
    assert!(
        installed >= 1,
        "the first extensions under the cap must still install"
    );
}

/// Flag OFF (master ON + third-party OFF) keeps the projection registry
/// builtin-only: a registry carrying only an untrusted package still yields
/// a builtin-only set when assembled through `build_hook_projection_registry`
/// with the sub-flag off — behavior identical to #3938.
#[tokio::test]
async fn third_party_subflag_off_yields_builtin_only_projection() {
    use ironclaw_filesystem::InMemoryBackend;

    let fs = InMemoryBackend::new();
    let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
    let builtin = ExtensionRegistry::new();
    // Master ON, third-party OFF.
    let config = HooksActivationConfig::enabled();
    let projection_registry = build_hook_projection_registry(
        builtin,
        Some(ThirdPartyDiscoveryInput {
            filesystem: &fs,
            tenant_id: &tenant,
        }),
        config,
    )
    .await
    .expect("projection registry builds");
    assert_eq!(
        projection_registry.projections().count(),
        0,
        "sub-flag OFF must not merge any third-party packages (builtin-only)"
    );
}

// ─── Tolerant + bounded DISCOVERY-stage coverage (Criticals 1 & 2) ───────

/// A DISCOVERY-valid `InstalledLocal` v2 manifest carrying one projectable
/// hook. Unlike [`manifest_toml`] (which uses the legacy top-level
/// `[[capabilities]]` accepted only on the direct-parse path), the discovery
/// contracts require the `ironclaw.capability_provider/v1` host_api form for
/// installed sources, so the discovery-stage tests below use this shape. The
/// `[[hooks]]` array-of-tables is a top-level sibling placed last.
fn manifest_toml_with_hook(id: &str) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "{id} extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run {id}"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"

[[hooks]]
id = "deny-run"
kind = "before_capability"
scope = "own_capabilities"
body = {{ mode = "predicate", spec = {{ type = "deny_capability", reason = "blocked by manifest hook", when = {{ type = "name_equals", name = "{id}.run" }} }} }}
"#
    )
}

/// Write `body` as `/system/extensions/<id>/manifest.toml` on `fs`.
async fn write_manifest<F: ironclaw_filesystem::RootFilesystem>(fs: &F, id: &str, body: &str) {
    fs.write_file(
        &ironclaw_host_api::VirtualPath::new(format!("/system/extensions/{id}/manifest.toml"))
            .expect("manifest path"),
        body.as_bytes(),
    )
    .await
    .expect("write manifest");
}

/// Critical 2 (discovery-stage): one malformed manifest among valid siblings
/// must quarantine ONLY the bad package — the valid siblings are still
/// merged into the projection registry, and the build does NOT fall back to
/// builtin-only.
#[tokio::test]
async fn malformed_sibling_manifest_does_not_drop_the_whole_third_party_set() {
    use ironclaw_filesystem::InMemoryBackend;

    let fs = InMemoryBackend::new();
    write_manifest(&fs, "good-a", &manifest_toml_with_hook("good-a")).await;
    write_manifest(&fs, "bad", "not valid toml {{{").await;
    write_manifest(&fs, "good-b", &manifest_toml_with_hook("good-b")).await;

    let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
    let config = HooksActivationConfig::enabled().with_third_party_enabled(true);
    let projection_registry = build_hook_projection_registry(
        ExtensionRegistry::new(),
        Some(ThirdPartyDiscoveryInput {
            filesystem: &fs,
            tenant_id: &tenant,
        }),
        config,
    )
    .await
    .expect("a malformed sibling must not fail the build (tolerant discovery)");

    let ids: Vec<String> = projection_registry
        .projections()
        .map(|p| p.extension_id.as_str().to_string())
        .collect();
    assert!(
        ids.contains(&"good-a".to_string()) && ids.contains(&"good-b".to_string()),
        "valid siblings must survive a malformed package; saw {ids:?}"
    );
    assert!(
        !ids.contains(&"bad".to_string()),
        "the malformed package must be quarantined, not merged"
    );
    assert_eq!(
        ids.len(),
        2,
        "exactly the two valid third-party packages must be merged (not builtin-only)"
    );
}

/// Critical 2 boundary: root unreadable is the ONLY case that falls back to
/// builtin-only.
#[tokio::test]
async fn unreadable_extension_root_falls_back_to_builtin_only() {
    use async_trait::async_trait;
    use ironclaw_filesystem::{
        DirEntry, FileStat, FilesystemError, FilesystemOperation, RootFilesystem,
    };
    use ironclaw_host_api::VirtualPath;

    struct UnreadableRootFs;

    #[async_trait]
    impl RootFilesystem for UnreadableRootFs {
        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "extensions root unreadable".to_string(),
            })
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }
    }

    let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
    let config = HooksActivationConfig::enabled().with_third_party_enabled(true);
    let projection_registry = build_hook_projection_registry(
        ExtensionRegistry::new(),
        Some(ThirdPartyDiscoveryInput {
            filesystem: &UnreadableRootFs,
            tenant_id: &tenant,
        }),
        config,
    )
    .await
    .expect("unreadable root falls back to builtin-only, not a hard error");
    assert_eq!(
        projection_registry.projections().count(),
        0,
        "an unreadable extensions root must yield builtin-only"
    );
}

/// Refinement 3: a package that fails to merge (duplicate id) must NOT
/// consume the per-tenant hook budget. We prove the budget accounting moved
/// AFTER the successful insert by showing a later distinct package still
/// merges even though a duplicate was processed first (the duplicate did not
/// burn budget). The duplicate itself is quarantined.
#[tokio::test]
async fn failed_merge_does_not_consume_hook_budget() {
    use ironclaw_filesystem::InMemoryBackend;

    let fs = InMemoryBackend::new();
    // Two distinct valid hook-bearing packages.
    write_manifest(&fs, "alpha", &manifest_toml_with_hook("alpha")).await;
    write_manifest(&fs, "beta", &manifest_toml_with_hook("beta")).await;

    let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
    let config = HooksActivationConfig::enabled().with_third_party_enabled(true);

    // Seed the builtin registry with a package whose id collides with
    // `alpha`, so discovery's `registry.insert(alpha)` FAILS (duplicate).
    // The failed merge must not consume budget; `beta` must still merge.
    let mut builtin = ExtensionRegistry::new();
    let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
        .expect("default host api contracts");
    let dup = ExtensionPackage::from_manifest(
        ExtensionManifest::parse_with_host_api_contracts(
            &manifest_toml_with_hook("alpha"),
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("dup manifest parses"),
        VirtualPath::new("/system/extensions/alpha").expect("dup root"),
    )
    .expect("dup package builds");
    builtin.insert(dup).expect("seed duplicate");

    let projection_registry = build_hook_projection_registry(
        builtin,
        Some(ThirdPartyDiscoveryInput {
            filesystem: &fs,
            tenant_id: &tenant,
        }),
        config,
    )
    .await
    .expect("duplicate merge is quarantined, build succeeds");

    let ids: Vec<String> = projection_registry
        .projections()
        .map(|p| p.extension_id.as_str().to_string())
        .collect();
    // `alpha` appears once (the seeded builtin); the discovered duplicate was
    // quarantined. `beta` merged — proving the quarantined duplicate did not
    // consume budget that would have blocked beta.
    assert!(
        ids.contains(&"beta".to_string()),
        "a package after a quarantined duplicate must still merge; saw {ids:?}"
    );
    assert_eq!(
        ids.iter().filter(|id| id.as_str() == "alpha").count(),
        1,
        "the duplicate must be quarantined, not double-merged"
    );
}

// ─── Tenant attribution for install-time quarantine audits (#3951 P1 #3) ──

/// The tenant-attributed production entry point
/// (`build_hook_dispatcher_builder_factory_for_tenant`) must emit install-time
/// quarantine audits under the REAL tenant id, not the synthetic
/// `"reborn-hook-projection"` fallback the no-tenant convenience entry point
/// uses. Drives the same call site `build_reborn_runtime` uses, and asserts on
/// the deterministic thread-local audit capture (see `audit::test_capture`) so
/// the test is immune to `tracing`'s process-wide max-level filter under
/// parallel `cargo test`.
#[test]
fn for_tenant_entry_point_attributes_install_time_quarantine_to_real_tenant() {
    // An untrusted extension whose hook payload is malformed → quarantined
    // at INSTALL time (not discovery), exercising the `project_extension_hook_sets`
    // audit path that previously lost tenant attribution.
    let hooks_block = r#"
[[hooks]]
id = "broken-hook"
kind = "before_capability"
body = { mode = "nonsense" }
"#;
    let registry = projection(registry_with_manifest(
        "broken-ext",
        &manifest_toml("broken-ext", hooks_block),
    ));

    let real_tenant = ironclaw_host_api::TenantId::new("acme-real-tenant").expect("tenant");

    let (factory, captured) = super::audit::test_capture::with_capture(|| {
        build_hook_dispatcher_builder_factory_for_tenant(
            HooksActivationConfig::enabled(),
            &registry,
            &real_tenant,
        )
        .expect("malformed untrusted hook is quarantined, build succeeds")
        .expect("flag ON yields a factory")
    });
    // Building a dispatcher confirms the survivors replay cleanly (the
    // quarantine already fired during composition, inside `with_capture`).
    let _dispatcher = factory().expect("mint hook builder").build_arc();

    let tenants: Vec<&str> = captured
        .iter()
        .map(|(tenant, _ext)| tenant.as_str())
        .collect();
    assert!(
        captured
            .iter()
            .any(|(tenant, ext)| tenant == real_tenant.as_str() && ext == "broken-ext"),
        "install-time quarantine audit must carry the real tenant id `{}` for the \
         quarantined extension; captured {captured:?}",
        real_tenant.as_str()
    );
    assert!(
        !tenants.contains(&"reborn-hook-projection"),
        "the tenant-attributed entry point must NOT fall back to the synthetic \
         audit tenant; captured {captured:?}"
    );
}
