//! Integration coverage for third-party extension hook activation via the
//! hook-only projection model.
//!
//! These tests drive the REAL production composition functions
//! ([`build_hook_projection_registry`] + [`build_hook_dispatcher_builder_factory`])
//! against a fake [`RootFilesystem`] that serves a `/system/extensions`
//! manifest tree — the same code path `build_reborn_runtime` invokes, not a
//! loader look-alike. They assert the containment, tenant-isolation, quarantine,
//! DoS-cap, and capability-absence properties of the model.
//!
//! Tenant isolation (Option 1): the per-tenant [`RootFilesystem`] is the scope
//! boundary, not a tenant path segment. The discovery root is the fixed
//! `/system/extensions`; two tenants are modeled as two distinct filesystems.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_filesystem::{
    DirEntry, Entry, FileStat, FileType, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, VersionedEntry,
};
use ironclaw_hooks::identity::{ExtensionId as HookExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::registry::HookPointSpec;
use ironclaw_host_api::{TenantId, VirtualPath};
use ironclaw_reborn_composition::{
    HookProjectionRegistry, HooksActivationConfig, MAX_INSTALLED_EXTENSIONS_CONSIDERED,
    ThirdPartyDiscoveryInput, build_hook_dispatcher_builder_factory,
    build_hook_projection_registry,
};

// ── Fake RootFilesystem serving an in-memory extension tree ──────────────────

/// A minimal in-memory filesystem that answers `list_dir` / `stat` / `get`
/// (enough for `ExtensionDiscovery`, which reads `<root>/<dir>/manifest.toml`
/// via `read_file_bounded`). Directory children and file bodies are explicit.
#[derive(Default)]
struct FakeExtensionFs {
    /// dir path -> child (name, file_type)
    dirs: BTreeMap<String, Vec<(String, FileType)>>,
    /// file path -> body bytes
    files: BTreeMap<String, Vec<u8>>,
}

impl FakeExtensionFs {
    fn new() -> Self {
        Self::default()
    }

    fn add_dir_child(&mut self, dir: &str, name: &str, file_type: FileType) {
        self.dirs
            .entry(dir.to_string())
            .or_default()
            .push((name.to_string(), file_type));
    }

    /// Register an extension `id` under directory `<root>` with the given
    /// manifest TOML, materializing the dir child and the manifest file.
    fn add_extension(&mut self, root: &str, id: &str, manifest_toml: &str) {
        self.add_dir_child(root, id, FileType::Directory);
        let manifest_path = format!("{root}/{id}/manifest.toml");
        self.files
            .insert(manifest_path, manifest_toml.as_bytes().to_vec());
    }
}

#[async_trait]
impl RootFilesystem for FakeExtensionFs {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let key = path.as_str().trim_end_matches('/').to_string();
        let children = self.dirs.get(&key).cloned().unwrap_or_default();
        let mut out = Vec::new();
        for (name, file_type) in children {
            let child_path =
                VirtualPath::new(format!("{key}/{name}")).expect("valid child path in fake fs");
            out.push(DirEntry {
                name,
                path: child_path,
                file_type,
            });
        }
        Ok(out)
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let key = path.as_str().to_string();
        if let Some(body) = self.files.get(&key) {
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: body.len() as u64,
                modified: None,
                sensitive: false,
            })
        } else if self.dirs.contains_key(key.trim_end_matches('/')) {
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
                modified: None,
                sensitive: false,
            })
        } else {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        Ok(self.files.get(path.as_str()).map(|body| VersionedEntry {
            path: path.clone(),
            entry: Entry::bytes(body.clone()),
            version: RecordVersion::from_backend(0),
        }))
    }
}

// ── Manifest builders ────────────────────────────────────────────────────────

/// Build a discoverable v2 manifest (host_api capability_provider, so it is
/// accepted as an `InstalledLocal` source — legacy top-level `[[capabilities]]`
/// would be rejected for installed sources) plus an arbitrary hooks block.
fn manifest(id: &str, hooks_block: &str) -> String {
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
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"
required_host_ports = ["host.runtime.http_egress"]
{hooks_block}"#
    )
}

fn own_deny_hook(local: &str, target: &str) -> String {
    format!(
        r#"
[[hooks]]
id = "{local}"
kind = "before_capability"
scope = "own_capabilities"
body = {{ mode = "predicate", spec = {{ type = "deny_capability", reason = "blocked", when = {{ type = "name_equals", name = "{target}" }} }} }}
"#
    )
}

fn installed_hook_id(ext: &str, local: &str) -> HookId {
    HookId::derive(
        &HookExtensionId::new(ext).expect("ext id"),
        "0.1.0",
        &HookLocalId::new(local).expect("local id"),
        HookVersion::ONE,
    )
}

fn both_flags_on() -> HooksActivationConfig {
    HooksActivationConfig::enabled().with_third_party_enabled(true)
}

async fn projection_for(
    fs: &FakeExtensionFs,
    tenant: &TenantId,
    config: HooksActivationConfig,
) -> HookProjectionRegistry {
    build_hook_projection_registry(
        ironclaw_extensions::ExtensionRegistry::new(),
        Some(ThirdPartyDiscoveryInput {
            filesystem: fs,
            tenant_id: tenant,
        }),
        config,
    )
    .await
    .expect("projection registry assembles")
}

fn ids_at(registry: &HookProjectionRegistry, point: HookPointSpec) -> Vec<HookId> {
    let factory = build_hook_dispatcher_builder_factory(both_flags_on(), registry)
        .expect("factory builds")
        .expect("flag ON yields a factory");
    factory()
        .build_arc()
        .active_bindings_snapshot(point)
        .into_iter()
        .map(|b| b.hook_id)
        .collect()
}

fn before_capability_ids(registry: &HookProjectionRegistry) -> Vec<HookId> {
    let factory = build_hook_dispatcher_builder_factory(both_flags_on(), registry)
        .expect("factory builds")
        .expect("flag ON yields a factory");
    factory()
        .build_arc()
        .active_bindings_snapshot(HookPointSpec::BeforeCapability)
        .into_iter()
        .map(|b| b.hook_id)
        .collect()
}

/// Register an extension `id` directly under the fixed `/system/extensions`
/// discovery root (Option 1: the FS itself is the per-tenant scope, so the
/// virtual path carries no tenant segment).
fn with_extension(fs: &mut FakeExtensionFs, id: &str, hooks_block: &str) {
    fs.add_extension("/system/extensions", id, &manifest(id, hooks_block));
}

// ── Containment: hook present, capability absent ─────────────────────────────

#[tokio::test]
async fn third_party_hook_is_projected_while_capability_stays_out_of_capability_surface() {
    let tenant = TenantId::new("alpha").expect("tenant");
    let mut fs = FakeExtensionFs::new();
    with_extension(
        &mut fs,
        "cap-and-hook",
        &own_deny_hook("deny-run", "cap-and-hook.run"),
    );

    let registry = projection_for(&fs, &tenant, both_flags_on()).await;

    // The hook binding IS present in the hook dispatcher.
    let ids = before_capability_ids(&registry);
    assert!(
        ids.contains(&installed_hook_id("cap-and-hook", "deny-run")),
        "third-party hook must be projected into the dispatcher"
    );

    // The capability is ABSENT from the capability path by construction: a
    // `HookProjectionRegistry` exposes no conversion to `ExtensionRegistry` and
    // cannot reach `HostRuntimeServices::new`. The type system is the proof —
    // there is no API on `HookProjectionRegistry` that yields a capability
    // catalog or surface. (See the `ironclaw_architecture` source assertion for
    // the composition-root grep guard.) This test pins that the hook-bearing
    // package reached ONLY the hook factory.
}

// ── Tenant isolation: the per-tenant filesystem IS the boundary (Option 1) ───

#[tokio::test]
async fn tenant_isolation_each_tenant_sees_only_its_own_filesystem() {
    // The isolation proof for Option 1 (FS-scoped, no tenant path segment):
    // two tenants are modeled as two DISTINCT per-tenant filesystems, each
    // serving its own `/system/extensions` subtree. Tenant A's discovery runs
    // against A's FS and sees ONLY A's extension; it has no handle to B's FS, so
    // it cannot see B's extension, and vice versa. Discovery uses the same fixed
    // `/system/extensions` virtual root for both — isolation comes entirely from
    // which filesystem each runtime was handed, exactly as in production.
    let mut fs_a = FakeExtensionFs::new();
    with_extension(&mut fs_a, "a-ext", &own_deny_hook("a-hook", "a-ext.run"));

    let mut fs_b = FakeExtensionFs::new();
    with_extension(&mut fs_b, "b-ext", &own_deny_hook("b-hook", "b-ext.run"));

    let alpha = TenantId::new("alpha").expect("alpha");
    let beta = TenantId::new("beta").expect("beta");

    let a_ids = before_capability_ids(&projection_for(&fs_a, &alpha, both_flags_on()).await);
    assert!(
        a_ids.contains(&installed_hook_id("a-ext", "a-hook")),
        "tenant A's runtime (A's FS) must see A's hook"
    );
    assert!(
        !a_ids.contains(&installed_hook_id("b-ext", "b-hook")),
        "tenant A's runtime must NOT see B's hook (A holds no handle to B's FS)"
    );

    let b_ids = before_capability_ids(&projection_for(&fs_b, &beta, both_flags_on()).await);
    assert!(
        b_ids.contains(&installed_hook_id("b-ext", "b-hook")),
        "tenant B's runtime (B's FS) must see B's hook"
    );
    assert!(
        !b_ids.contains(&installed_hook_id("a-ext", "a-hook")),
        "tenant B's runtime must NOT see A's hook"
    );
}

// ── Escape / malformed-directory robustness ──────────────────────────────────

#[tokio::test]
async fn bad_directory_name_is_skipped_not_fatal() {
    let mut fs = FakeExtensionFs::new();
    // A child whose name is not a valid ExtensionId is skipped by discovery.
    fs.add_dir_child(
        "/system/extensions",
        "Not A Valid Id!!",
        FileType::Directory,
    );
    // A valid sibling still loads.
    with_extension(&mut fs, "good", &own_deny_hook("g", "good.run"));

    let tenant = TenantId::new("alpha").expect("tenant");
    let ids = before_capability_ids(&projection_for(&fs, &tenant, both_flags_on()).await);
    assert!(
        ids.contains(&installed_hook_id("good", "g")),
        "a valid sibling must still load past a bad directory name"
    );
}

#[tokio::test]
async fn manifest_id_mismatch_is_not_a_panic() {
    // Directory `claimed` but manifest declares a different id => discovery
    // returns ManifestIdMismatch; build_hook_projection_registry treats a
    // discovery error as fail-safe builtin-only (no panic, no third-party).
    let mut fs = FakeExtensionFs::new();
    fs.add_extension(
        "/system/extensions",
        "claimed",
        &manifest("actually-different", &own_deny_hook("h", "x.run")),
    );

    let tenant = TenantId::new("alpha").expect("tenant");
    let registry = projection_for(&fs, &tenant, both_flags_on()).await;
    // No panic; the mismatched package contributes nothing.
    assert!(
        before_capability_ids(&registry).is_empty(),
        "id-mismatched manifest must not project hooks (and must not panic)"
    );
}

// ── DoS cap: surplus extensions quarantined ──────────────────────────────────

#[tokio::test]
async fn surplus_extensions_beyond_consider_cap_are_quarantined_via_discovery() {
    let tenant = TenantId::new("alpha").expect("tenant");
    let mut fs = FakeExtensionFs::new();
    for i in 0..(MAX_INSTALLED_EXTENSIONS_CONSIDERED + 3) {
        let id = format!("ext-{i:04}");
        with_extension(&mut fs, &id, &own_deny_hook("h", &format!("{id}.run")));
    }

    let registry = projection_for(&fs, &tenant, both_flags_on()).await;
    let installed = before_capability_ids(&registry).len();
    assert!(
        installed <= MAX_INSTALLED_EXTENSIONS_CONSIDERED,
        "surplus extensions beyond the consider-cap must be quarantined (saw {installed})"
    );
    assert!(
        installed >= 1,
        "extensions under the cap must still install"
    );
}

// ── Flag OFF: builtin-only, identical to #3938 ───────────────────────────────

#[tokio::test]
async fn subflag_off_discovers_nothing() {
    let tenant = TenantId::new("alpha").expect("tenant");
    let mut fs = FakeExtensionFs::new();
    with_extension(&mut fs, "would-be", &own_deny_hook("h", "would-be.run"));

    // Master ON, third-party OFF => no discovery, builtin-only.
    let registry = projection_for(&fs, &tenant, HooksActivationConfig::enabled()).await;
    assert!(
        before_capability_ids(&registry).is_empty(),
        "third-party sub-flag OFF must discover/project nothing"
    );
}

// ── Per-hook-point trust matrix (installed path) ─────────────────────────────
//
// For an Installed-tier (third-party) extension at each hook point, the model
// guarantees:
//   * the Installed-tier ceiling — no Allow/Mutator is even REPRESENTABLE: the
//     manifest hook body is closed to `predicate` (deny/pause/rate) or `wasm`;
//     there is no Allow/Mutator variant, and the registrar only ever calls
//     `install_installed_*`. (Structural — pinned by the predicate-vocabulary
//     and registrar-only invariant; the tests below confirm the observable
//     consequences per point.)
//   * `before_capability` predicate deny/pause IS allowed (Gate is reachable at
//     BeforeCapability — we do NOT claim Gate is unreachable).
//   * non-`before_capability` points only accept WASM bodies from the manifest
//     (the registrar rejects a predicate body for any other kind), and a
//     WASM-bodied third-party hook has no runtime in the loader registrar, so it
//     is quarantined and the build continues.
//   * `owning_extension` cannot be spoofed: the registrar derives it from the
//     install argument (the discovered extension id), and there is no manifest
//     field to claim a different owner — so a hook's binding always attributes
//     to its own discovered extension.

/// BeforeCapability: a predicate DENY hook from an installed extension installs
/// and the dispatcher actually denies the matching capability — proving deny is
/// allowed (Gate reachable) at the Installed tier.
#[tokio::test]
async fn matrix_before_capability_installed_deny_is_allowed_and_fires() {
    use ironclaw_hooks::points::{BeforeCapabilityHookContext, SanitizedArguments};

    let tenant = TenantId::new("alpha").expect("tenant");
    let mut fs = FakeExtensionFs::new();
    with_extension(
        &mut fs,
        "gate-ext",
        &own_deny_hook("deny-run", "gate-ext.run"),
    );
    let registry = projection_for(&fs, &tenant, both_flags_on()).await;

    let factory = build_hook_dispatcher_builder_factory(both_flags_on(), &registry)
        .expect("factory")
        .expect("flag ON yields factory");
    let dispatcher = factory().build_arc();
    assert!(
        ids_at(&registry, HookPointSpec::BeforeCapability)
            .contains(&installed_hook_id("gate-ext", "deny-run")),
        "installed deny hook must bind at BeforeCapability"
    );

    // Dispatch: the hook's own capability is denied.
    let ctx = BeforeCapabilityHookContext::new(
        tenant.clone(),
        "gate-ext.run".to_string(),
        [0u8; 32],
        SanitizedArguments::unresolved(),
        Some(ironclaw_host_api::ExtensionId::new("gate-ext").expect("ext")),
    );
    let outcome = dispatcher.dispatch_before_capability(&ctx).await;
    assert!(
        !outcome.decision.permits(),
        "installed BeforeCapability deny predicate must deny on dispatch (Gate reachable)"
    );
}

/// BeforePrompt: a predicate body is invalid for this kind (registrar rejects
/// it), so the installed hook is quarantined; a sibling BeforeCapability hook
/// still installs (build continues).
#[tokio::test]
async fn matrix_before_prompt_installed_predicate_is_quarantined_build_continues() {
    let tenant = TenantId::new("alpha").expect("tenant");
    let mut fs = FakeExtensionFs::new();
    let before_prompt_predicate = r#"
[[hooks]]
id = "bp"
kind = "before_prompt"
scope = "own_capabilities"
body = { mode = "predicate", spec = { type = "deny_capability", reason = "x", when = { type = "name_equals", name = "bp-ext.run" } } }
"#;
    with_extension(&mut fs, "bp-ext", before_prompt_predicate);
    with_extension(&mut fs, "ok-ext", &own_deny_hook("ok", "ok-ext.run"));
    let registry = projection_for(&fs, &tenant, both_flags_on()).await;

    assert!(
        ids_at(&registry, HookPointSpec::BeforePrompt).is_empty(),
        "before_prompt predicate body must quarantine (registrar rejects non-before_capability predicate)"
    );
    assert!(
        ids_at(&registry, HookPointSpec::BeforeCapability)
            .contains(&installed_hook_id("ok-ext", "ok")),
        "a sibling valid extension must still install after the quarantine"
    );
}

/// AfterModel / AfterCapability / AfterCheckpoint / EventTriggered: each only
/// accepts a WASM body from the manifest, which the loader registrar has no
/// runtime for ⇒ quarantined, build continues. Drives all four observer/event
/// points in one matrix sweep.
#[tokio::test]
async fn matrix_wasm_only_points_quarantine_without_runtime_build_continues() {
    let tenant = TenantId::new("alpha").expect("tenant");

    for (kind, point) in [
        ("after_model", HookPointSpec::AfterModel),
        ("after_capability", HookPointSpec::AfterCapability),
        ("after_checkpoint", HookPointSpec::AfterCheckpoint),
        ("event_triggered", HookPointSpec::EventTriggered),
    ] {
        let mut fs = FakeExtensionFs::new();
        let wasm_hook = format!(
            r#"
[[hooks]]
id = "w"
kind = "{kind}"
scope = "own_capabilities"
body = {{ mode = "wasm", export = "evaluate" }}
"#
        );
        with_extension(&mut fs, "wasm-ext", &wasm_hook);
        with_extension(&mut fs, "ok-ext", &own_deny_hook("ok", "ok-ext.run"));
        let registry = projection_for(&fs, &tenant, both_flags_on()).await;

        assert!(
            ids_at(&registry, point).is_empty(),
            "{kind}: WASM-bodied third-party hook must quarantine (no runtime in loader registrar)"
        );
        assert!(
            ids_at(&registry, HookPointSpec::BeforeCapability)
                .contains(&installed_hook_id("ok-ext", "ok")),
            "{kind}: sibling valid extension must still install (build continues)"
        );
    }
}

/// owning_extension cannot be spoofed: the binding for an installed hook always
/// attributes to its OWN discovered extension id, regardless of what the
/// capability predicate names. (The registrar derives owner from the install
/// arg; the manifest has no owner field.)
#[tokio::test]
async fn matrix_owning_extension_is_derived_not_spoofable() {
    let tenant = TenantId::new("alpha").expect("tenant");
    let mut fs = FakeExtensionFs::new();
    // The predicate targets ANOTHER extension's capability name, but the hook
    // is `own_capabilities`-scoped and the binding owner is still `claimant`.
    with_extension(
        &mut fs,
        "claimant",
        &own_deny_hook("h", "some-other-ext.run"),
    );
    let registry = projection_for(&fs, &tenant, both_flags_on()).await;

    let factory = build_hook_dispatcher_builder_factory(both_flags_on(), &registry)
        .expect("factory")
        .expect("flag ON yields factory");
    let dispatcher = factory().build_arc();
    let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);
    let binding = bindings
        .iter()
        .find(|b| b.hook_id == installed_hook_id("claimant", "h"))
        .expect("claimant hook bound");
    assert_eq!(
        binding.owning_extension.as_ref(),
        Some(&ironclaw_host_api::ExtensionId::new("claimant").expect("ext")),
        "owning_extension must be the discovered extension id, never spoofable via the manifest"
    );
}
