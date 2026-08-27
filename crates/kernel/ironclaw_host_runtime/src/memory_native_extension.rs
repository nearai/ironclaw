//! The bundled memory provider manifests (issue #3537).
//!
//! This module owns the host-bundled memory provider extensions — their v3
//! TOML manifests, the host service identities that back them, and the
//! functions that turn each bundled manifest into a registrable
//! [`ExtensionPackage`] plus its declared lifecycle
//! ([`BundledMemoryProvider`]).
//!
//! The BOUND provider is loaded on the **always-on first-party lane** (like
//! the builtin toolset), not the catalog/lifecycle lane: composition resolves
//! the compose-time `[memory]` binding, loads THAT provider's bundle, and
//! inserts its package directly into the extension registry at startup. There
//! is no install/enable lifecycle. Co-locating the manifests with the service
//! identities embodies the "bundled TOML alone is not authority" rule: each
//! manifest declares a `first_party` runtime whose `service` must match the
//! host-registered provider identity; the binding layer (`memory_binding`)
//! decides which provider serves, and the manifest's `[[tools]]` +
//! `[memory].lifecycle` are the single source of truth for that provider's
//! surface.
//!
//! The declared tools are model-visible memory tools under the stable
//! `ironclaw.memory.*` ids. Input schemas are served inline on the always-on
//! lane (see `first_party_tools::resolve_native_memory_input_schema_ref`), so
//! no asset materialization is required. The bound provider's
//! `[memory].guidance_doc` — its model-facing memory guidance, appended to the
//! system prompt by composition — is resolved the same way: generically,
//! against the asset table the provider BEING BUNDLED supplies to
//! [`memory_provider_bundle`], never by a host-side match on a specific
//! provider's constants. See [`BundledMemoryProvider::guidance`]. The prompt of
//! every `[memory].scheduled_ops` pass the provider declares resolves through
//! the same table and the same fail-closed posture
//! ([`BundledMemoryProvider::scheduled_pass_prompts`], #7664).

use ironclaw_extension_contracts::memory::{
    MemoryDescriptor, MemoryScheduledOpKind, MemoryScheduledTrigger,
};
use ironclaw_extension_registry::{
    ExtensionError, ExtensionInstallationError, ExtensionManifestRecord, ExtensionManifestV2,
    ExtensionPackage, ManifestSource, default_host_api_contract_registry,
};
use ironclaw_host_api::{host_port::default_host_port_catalog, path::VirtualPath};

/// Reserved host-bundled extension id for the native memory provider.
pub const NATIVE_MEMORY_EXTENSION_ID: &str = "ironclaw.memory";

/// Every host-bundled memory provider package id. These are the only
/// extension ids the always-on memory lane can register a package under, so
/// identity checks that must hold for "the bound memory provider" (registry
/// provider allowlist, inline schema serving, first-party trust entries) key
/// off this list instead of hardwiring the native id.
pub const MEMORY_PROVIDER_PACKAGE_IDS: &[&str] =
    &[NATIVE_MEMORY_EXTENSION_ID, MEM0_MEMORY_EXTENSION_ID];

/// Host service identity declared by the manifest's `first_party` runtime. The
/// host must register a matching service for the bundled manifest to be
/// authoritative; this constant is the single source of truth both the manifest
/// (via a parse-time assertion test) and the binding layer compare against.
pub const NATIVE_MEMORY_PROVIDER_SERVICE: &str = "native_memory_provider";

/// Virtual package root for the bundled native memory extension. Used as a
/// stable identity for the registered package; on the always-on lane the
/// manifest's schemas are served inline rather than read from this path.
const NATIVE_MEMORY_PACKAGE_ROOT: &str = "/system/extensions/ironclaw.memory";

/// Raw bundled manifest TOML for the native memory extension.
pub const NATIVE_MEMORY_MANIFEST_TOML: &str =
    include_str!("../../../extensions/packages/memory-native/manifest.toml");

/// Resolve a bundled provider's declared `[memory].guidance_doc` against ITS
/// OWN bundled asset table (#7185).
///
/// The `[memory].guidance_doc` ref names a file inside the provider's own
/// package, and bundled providers ride the always-on lane, so nothing is
/// materialized to the package root to read it back from. The text comes from
/// the OWNING package's public API rather than by compiling its asset tree into
/// this crate — a reach-in would bypass the dependency graph the boundary gates
/// police (`reborn_cross_crate_include_scan` §11.2.7, shrink-only). Each
/// bundled provider exports its own `(ref, text)` asset table (empty when it
/// ships no guidance); [`memory_provider_bundle`] passes in exactly the table
/// belonging to the provider it is bundling, so resolution never depends on
/// which provider is compiled into the host — a non-native provider that
/// declares guidance resolves through this same generic lookup instead of
/// silently falling through a host-side match on another provider's constants.
///
/// An absent `guidance_doc` is a normal no-guidance state (`Ok(None)`).
/// FAIL LOUD once a ref is declared: a manifest ref the provider's own asset
/// table does not carry is a manifest/asset desync (a rename that touched one
/// and not the other), not something to drop silently — the model would
/// simply lose its guidance with nothing failing. Same posture as this
/// module's existing "bundled provider manifest missing `[memory]`" failure.
fn resolve_guidance_doc(
    descriptor: &MemoryDescriptor,
    assets: &[(&'static str, &'static str)],
    label: &str,
) -> Result<Option<&'static str>, String> {
    let Some(doc_ref) = descriptor.guidance_doc.as_ref() else {
        return Ok(None);
    };
    resolve_declared_asset(doc_ref.as_str(), assets, label, "guidance_doc").map(Some)
}

/// Resolve every declared `[memory].scheduled_ops` pass prompt against the
/// provider's OWN bundled asset table, keyed by the trigger it rides (#7664).
///
/// Same posture and same reason as [`resolve_guidance_doc`]: the manifest names
/// an asset inside the provider's own package, bundled providers ride the
/// always-on lane with nothing materialized to read back from, and the ref is
/// resolved through the OWNING package's public asset table rather than by
/// compiling its asset tree into this crate.
///
/// FAIL LOUD on a ref the table does not carry. A pass op whose prompt silently
/// resolved to nothing would be a scheduled model run with an empty
/// instruction: it would still be dispatched, still spend a budget, and still
/// hold write tools over the user's memory — strictly worse than refusing to
/// construct the provider. A declaration with no matching asset is a
/// manifest/asset desync (a rename that touched one and not the other).
fn resolve_scheduled_pass_prompts(
    descriptor: &MemoryDescriptor,
    assets: &[(&'static str, &'static str)],
    label: &str,
) -> Result<Vec<(MemoryScheduledTrigger, &'static str)>, String> {
    descriptor
        .scheduled_ops
        .iter()
        .map(|scheduled| match &scheduled.op {
            MemoryScheduledOpKind::Pass(pass) => {
                let prompt =
                    resolve_declared_asset(pass.prompt.as_str(), assets, label, "scheduled pass")?;
                Ok((scheduled.trigger, prompt))
            }
        })
        .collect()
}

/// One declared asset ref, looked up in the provider's own table. `what` names
/// the declaration in the failure so an operator reading it knows which key of
/// `[memory]` desynced.
fn resolve_declared_asset(
    asset_ref: &str,
    assets: &[(&'static str, &'static str)],
    label: &str,
    what: &str,
) -> Result<&'static str, String> {
    assets
        .iter()
        .find(|(candidate_ref, _)| *candidate_ref == asset_ref)
        .map(|(_, text)| *text)
        .ok_or_else(|| {
            format!(
                "{label} memory provider manifest declares {what} '{asset_ref}' but its bundled \
                 asset table has no matching entry"
            )
        })
}

/// Reserved (host-bundled) extension id for the mem0 memory backend. Mirrors
/// `ironclaw_memory_mem0::MEM0_MEMORY_EXTENSION_ID`; the `[memory]` binding
/// selects it by this id.
pub const MEM0_MEMORY_EXTENSION_ID: &str = "mem0.local.memory";

/// Host service identity declared by the mem0 backend manifest's `first_party`
/// runtime.
pub const MEM0_MEMORY_PROVIDER_SERVICE: &str = "mem0_memory_provider";

/// Raw bundled manifest TOML for the mem0 memory backend: mem0's own tool
/// declarations (under the stable `ironclaw.memory.*` ids) plus its honest
/// lifecycle set. The mem0 `MemoryService` is constructed from `[memory]`
/// config in composition, gated by the `memory-mem0` feature.
pub const MEM0_MEMORY_MANIFEST_TOML: &str =
    include_str!("../../../extensions/packages/mem0/manifest.toml");

/// Parse the bundled `ironclaw.memory` manifest into the internal manifest
/// model. Fail-closed: the reserved id, `first_party` runtime, `[memory]`
/// surface, schema refs, and provider-prefixed tool ids are validated by the
/// parser.
pub fn native_memory_manifest() -> Result<ExtensionManifestV2, ExtensionInstallationError> {
    // No package root is being materialized here — this accessor only
    // needs the validated manifest shape, not a bound package.
    Ok(memory_manifest_record(NATIVE_MEMORY_MANIFEST_TOML, None)?
        .manifest()
        .clone())
}

/// Parse + validate a bundled memory manifest into its resolved record.
/// `ExtensionManifestRecord::from_toml` is the single parse entry point; it
/// dispatches on `schema_version` (v2 or v3) and normalizes into one model.
fn memory_manifest_record(
    toml: &str,
    root: Option<VirtualPath>,
) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
    let host_ports = default_host_port_catalog().map_err(|error| {
        ExtensionInstallationError::InvalidManifest {
            reason: error.to_string(),
        }
    })?;
    let contracts = default_host_api_contract_registry().map_err(|error| {
        ExtensionInstallationError::InvalidManifest {
            reason: error.to_string(),
        }
    })?;
    ExtensionManifestRecord::from_toml(
        toml,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
        root,
    )
}

/// Virtual package root for the bundled mem0 memory backend.
const MEM0_MEMORY_PACKAGE_ROOT: &str = "/system/extensions/mem0.local.memory";

/// A bundled memory provider's registrable package plus the lifecycle hooks
/// its manifest declares — exactly what composition consumes when this
/// provider is the bound one: the package's declared tools are registered on
/// the always-on lane, and the lifecycle set gates every host-initiated
/// memory call.
#[derive(Debug)]
pub struct BundledMemoryProvider {
    pub package: ExtensionPackage,
    pub lifecycle: MemoryDescriptor,
    /// The bound provider's own memory guidance for the model, resolved from
    /// its declared `guidance_doc` against its own asset table — see
    /// [`resolve_guidance_doc`]. `None` when the provider declares no
    /// `guidance_doc`.
    pub guidance: Option<String>,
    /// Resolved prompt text for every scheduled pass op the provider declares,
    /// keyed by the trigger it rides (#7664) — see
    /// [`resolve_scheduled_pass_prompts`]. Empty when the provider schedules
    /// nothing. The DECLARATION itself (cadence, tool selection, model-call
    /// budget) stays on `lifecycle`; only the asset needed resolving.
    pub scheduled_pass_prompts: Vec<(MemoryScheduledTrigger, String)>,
}

/// Build the registrable provider bundle for the bundled native memory
/// extension. The composition layer inserts the package into the always-on
/// extension registry (alongside the builtin package) when native is the
/// bound provider.
pub fn native_memory_provider_bundle() -> Result<BundledMemoryProvider, ExtensionError> {
    memory_provider_bundle(
        NATIVE_MEMORY_MANIFEST_TOML,
        NATIVE_MEMORY_PACKAGE_ROOT,
        "native memory",
        ironclaw_memory_native::MEMORY_ASSETS,
    )
}

/// Build the registrable provider bundle for the bundled mem0 memory backend,
/// used when the compose-time `[memory]` binding selects mem0 AND the mem0
/// provider is actually constructible.
///
/// `memory_assets` is the mem0 provider's own asset table. This crate does
/// not (and, per the memory-provider naming gate, must not) depend on
/// `ironclaw_memory_mem0` — only the provider packages and the binary may name
/// a memory provider — so composition, which already depends on it behind the
/// `memory-mem0` feature, passes the table in.
pub fn mem0_memory_provider_bundle(
    memory_assets: &'static [(&'static str, &'static str)],
) -> Result<BundledMemoryProvider, ExtensionError> {
    memory_provider_bundle(
        MEM0_MEMORY_MANIFEST_TOML,
        MEM0_MEMORY_PACKAGE_ROOT,
        "mem0",
        memory_assets,
    )
}

/// Backward-compatible package-only accessor for the native provider bundle.
pub fn native_memory_first_party_package() -> Result<ExtensionPackage, ExtensionError> {
    Ok(native_memory_provider_bundle()?.package)
}

fn memory_provider_bundle(
    toml: &str,
    package_root: &str,
    label: &str,
    memory_assets: &'static [(&'static str, &'static str)],
) -> Result<BundledMemoryProvider, ExtensionError> {
    let invalid = |error: &dyn std::fmt::Display| ExtensionError::InvalidManifest {
        reason: format!("{label} memory provider package is invalid: {error}"),
    };
    let root = VirtualPath::new(package_root)?;
    let record =
        memory_manifest_record(toml, Some(root.clone())).map_err(|error| invalid(&error))?;
    // The manifest is the single source of truth for the provider's surface:
    // a bundled provider manifest without `[memory]` is a contract break, not
    // an empty lifecycle.
    let lifecycle = record.resolved().memory.clone().ok_or_else(|| {
        invalid(&format!(
            "{label} memory provider manifest declares no [memory] surface"
        ))
    })?;
    let guidance = resolve_guidance_doc(&lifecycle, memory_assets, label)
        .map_err(|reason| invalid(&reason))?
        .map(str::to_string);
    let scheduled_pass_prompts = resolve_scheduled_pass_prompts(&lifecycle, memory_assets, label)
        .map_err(|reason| invalid(&reason))?
        .into_iter()
        .map(|(trigger, prompt)| (trigger, prompt.to_string()))
        .collect();
    let manifest = record
        .manifest()
        .clone()
        .try_into()
        .map_err(|error: ExtensionError| invalid(&error))?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, record.raw_toml())?;
    Ok(BundledMemoryProvider {
        package,
        lifecycle,
        guidance,
        scheduled_pass_prompts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
        MEMORY_WRITE_CAPABILITY_ID,
    };
    use ironclaw_extension_registry::{CapabilityVisibility, ExtensionRuntimeV2};

    /// The native provider's bundle must carry its guidance ref through from
    /// the manifest AND resolve to real text. Two halves, because either one
    /// failing silently costs the model its memory guidance with nothing
    /// failing: a manifest that drops the key, or a key that no longer matches
    /// the bundled file's path.
    #[test]
    fn native_bundle_declares_guidance_that_resolves_to_the_bundled_asset() {
        let bundle = native_memory_provider_bundle().expect("native bundle loads");
        assert_eq!(
            bundle
                .lifecycle
                .guidance_doc
                .as_ref()
                .map(|doc| doc.as_str()),
            Some("prompts/memory-guidance.md"),
            "the native manifest must keep declaring its memory guidance"
        );
        // `bundle.guidance` is what the fix generalized (#7185): resolved at
        // bundle-construction time against the native crate's OWN asset table
        // (`ironclaw_memory_native::MEMORY_ASSETS`), not through a
        // host-side match on the native provider's constants.
        let guidance = bundle
            .guidance
            .as_deref()
            .expect("the declared guidance ref must resolve to a bundled asset");
        assert!(
            guidance.starts_with('#'),
            "guidance is appended as its own system-prompt section and must open with a heading"
        );
        assert!(
            guidance.contains("ironclaw.memory.write"),
            "the native guidance must name the tool it tells the model to call"
        );
    }

    /// mem0 deliberately ships no guidance: its recall is search-first, so the
    /// native provider's standing-document advice would be wrong under it.
    /// Absent must mean "append nothing", never "fall back to native's". mem0's
    /// own asset table is empty (`ironclaw_memory_mem0::MEMORY_ASSETS`);
    /// this crate cannot name that crate (memory-provider naming gate), so the
    /// test passes an equivalent empty table directly.
    #[test]
    fn a_provider_without_a_guidance_declaration_resolves_to_none() {
        let bundle = mem0_memory_provider_bundle(&[]).expect("mem0 bundle loads");
        assert!(bundle.lifecycle.guidance_doc.is_none());
        assert!(bundle.guidance.is_none());
    }

    /// FAIL LOUD, not fail-quiet: a declared `guidance_doc` ref that the
    /// bundled provider's own asset table does not carry is a manifest/asset
    /// desync (a rename that touched one and not the other), not something to
    /// drop silently — the model would simply lose its guidance with nothing
    /// failing. (A ref that is not a valid relative asset path never gets this
    /// far — it fails the manifest parse.)
    #[test]
    fn a_guidance_ref_the_providers_own_asset_table_does_not_carry_fails_loud() {
        let descriptor = MemoryDescriptor {
            guidance_doc: Some(
                ironclaw_host_api::capability_profile::CapabilityProfileSchemaRef::new(
                    "prompts/from-a-future-provider.md",
                )
                .expect("valid asset ref"),
            ),
            ..MemoryDescriptor::default()
        };
        let error = resolve_guidance_doc(&descriptor, &[], "acme")
            .expect_err("an unresolved guidance_doc ref must fail loud, not resolve to None");
        assert!(
            error.contains("prompts/from-a-future-provider.md"),
            "{error}"
        );
    }

    /// The native provider declares its own upkeep pass (#7664), and the
    /// declared prompt must resolve to real text at bundle construction. Both
    /// halves again: the manifest must keep declaring the op (nothing else
    /// schedules curation now — the hardwired interval is gone), and the ref
    /// must still match the bundled file.
    #[test]
    fn native_bundle_declares_an_after_turn_pass_whose_prompt_resolves() {
        use ironclaw_extension_contracts::memory::MemoryScheduledOpKind;

        let bundle = native_memory_provider_bundle().expect("native bundle loads");
        let scheduled = bundle
            .lifecycle
            .scheduled_op(MemoryScheduledTrigger::AfterTurn)
            .expect("the native manifest must keep declaring its after-turn upkeep pass");
        let MemoryScheduledOpKind::Pass(pass) = &scheduled.op;
        assert_eq!(pass.prompt.as_str(), "prompts/memory_curation.md");
        let prompt = bundle
            .scheduled_pass_prompts
            .iter()
            .find(|(trigger, _)| *trigger == MemoryScheduledTrigger::AfterTurn)
            .map(|(_, prompt)| prompt.as_str())
            .expect("the declared pass prompt must resolve to a bundled asset");
        assert!(
            prompt.contains("Never invent, infer, or extrapolate"),
            "the anti-fabrication rule is the pass's load-bearing constraint and must reach it"
        );
    }

    /// A provider that schedules nothing resolves no prompts — absent must mean
    /// "nothing is scheduled", never "fall back to another provider's pass".
    #[test]
    fn a_provider_without_scheduled_ops_resolves_no_pass_prompts() {
        let bundle = mem0_memory_provider_bundle(&[]).expect("mem0 bundle loads");
        assert!(bundle.lifecycle.scheduled_ops.is_empty());
        assert!(bundle.scheduled_pass_prompts.is_empty());
    }

    /// Same fail-loud posture as the guidance desync, for the declaration that
    /// carries strictly more authority: a pass prompt that resolved to nothing
    /// would still be dispatched on a schedule, still spend its budget, and
    /// still hold write tools over the user's memory — an empty instruction
    /// running as every user. Refuse to construct the provider instead.
    #[test]
    fn provider_bundle_fails_loud_on_a_scheduled_pass_prompt_desync() {
        const DESYNCED_PASS_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme.desynced.pass"
name = "Acme Desynced Pass"
version = "0.1.0"
description = "Bundled provider fixture with a pass prompt no asset backs."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acme_desynced_pass_provider"

[memory]
lifecycle = ["read_long_term"]

[[memory.scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/missing-pass.md", tools = ["ironclaw.memory.read"], max_model_calls = 4 }

[[tools]]
id = "ironclaw.memory.read"
description = "Read a memory document."
effects = ["read_filesystem"]
default_permission = "allow"
visibility = "model"
origin_gate_matrix = { loop_run = "ungated", product = "forbidden", automation = "forbidden" }
input_schema_ref = "schemas/memory/document-read.input.v1.json"
output_schema_ref = "schemas/memory/document-read.output.v1.json"
"#;
        let error = memory_provider_bundle(
            DESYNCED_PASS_MANIFEST,
            "/system/extensions/acme_desynced_pass",
            "acme",
            &[],
        )
        .expect_err("a pass prompt ref with no matching bundled asset must fail loud");
        assert!(
            error.to_string().contains("prompts/missing-pass.md"),
            "{error}"
        );
    }

    /// The bundle loader is only for `[memory]`-declaring providers; a
    /// bundled manifest that lost its `[memory]` section must fail loud, not
    /// register tools with a silently empty lifecycle.
    #[test]
    fn provider_bundle_fails_loud_without_a_memory_surface() {
        const NO_MEMORY_SURFACE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme.memoryless"
name = "Acme Memoryless"
version = "0.1.0"
description = "Bundled provider fixture without a [memory] surface."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acme_memoryless_provider"
"#;
        let error = memory_provider_bundle(
            NO_MEMORY_SURFACE_MANIFEST,
            "/system/extensions/acme_memoryless",
            "acme",
            &[],
        )
        .expect_err("a bundled memory provider manifest must declare [memory]");
        assert!(error.to_string().contains("[memory]"), "{error}");
    }

    /// A bundled provider manifest that declares a `guidance_doc` its own
    /// asset table doesn't carry must fail bundle construction loud — the
    /// same posture as the missing-`[memory]` case above, exercised through
    /// the full `memory_provider_bundle` path (manifest ref this host has no
    /// matching asset for, threaded end to end).
    #[test]
    fn provider_bundle_fails_loud_on_a_guidance_desync() {
        const DESYNCED_GUIDANCE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme.desynced"
name = "Acme Desynced"
version = "0.1.0"
description = "Bundled provider fixture with a guidance_doc no asset backs."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acme_desynced_provider"

[memory]
lifecycle = ["read_long_term"]
guidance_doc = "prompts/missing.md"
"#;
        let error = memory_provider_bundle(
            DESYNCED_GUIDANCE_MANIFEST,
            "/system/extensions/acme_desynced",
            "acme",
            &[],
        )
        .expect_err("a guidance_doc ref with no matching bundled asset must fail loud");
        assert!(error.to_string().contains("prompts/missing.md"), "{error}");
    }

    #[test]
    fn manifest_parses_as_host_bundled_first_party() {
        let manifest = native_memory_manifest().expect("native memory manifest must parse");
        assert_eq!(manifest.id.as_str(), NATIVE_MEMORY_EXTENSION_ID);
        assert_eq!(manifest.source, ManifestSource::HostBundled);
        match &manifest.runtime {
            ExtensionRuntimeV2::FirstParty { service } => {
                // Bundled TOML alone is not authority: its declared service must
                // match the host-registered native memory provider identity.
                assert_eq!(service, NATIVE_MEMORY_PROVIDER_SERVICE);
            }
            other => panic!("expected first_party runtime, got {other:?}"),
        }
    }

    #[test]
    fn manifest_declares_the_model_visible_memory_tools() {
        let manifest = native_memory_manifest().expect("manifest");
        let ids: Vec<&str> = manifest
            .capabilities
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                crate::PROFILE_SET_CAPABILITY_ID,
            ]
        );
        for capability in &manifest.capabilities {
            assert_eq!(
                capability.visibility,
                CapabilityVisibility::Model,
                "{} must be model-visible",
                capability.id
            );
        }
    }

    #[test]
    fn native_memory_declares_no_host_ports() {
        // The live native provider is filesystem-backed; it declares no storage
        // or audit host ports. The SQL/audit ports remain catalogued vocabulary
        // for the deferred SQL-backed milestone (see ADR 0002), but no live
        // capability requires them.
        let manifest = native_memory_manifest().expect("manifest");
        for capability in &manifest.capabilities {
            assert!(
                capability.required_host_ports.is_empty(),
                "{} must declare no required host ports",
                capability.id
            );
        }
    }

    #[test]
    fn native_memory_package_builds() {
        let package = native_memory_first_party_package().expect("native memory package builds");
        assert_eq!(package.manifest.id.as_str(), NATIVE_MEMORY_EXTENSION_ID);
    }

    #[test]
    fn native_provider_bundle_declares_the_full_lifecycle() {
        use ironclaw_extension_contracts::memory::MemoryLifecycleHook;
        let bundle = native_memory_provider_bundle().expect("native bundle builds");
        assert_eq!(
            bundle.package.manifest.id.as_str(),
            NATIVE_MEMORY_EXTENSION_ID
        );
        for hook in MemoryLifecycleHook::ALL {
            assert!(
                bundle.lifecycle.declares(hook),
                "native must declare {hook:?}"
            );
        }
    }

    #[test]
    fn mem0_provider_bundle_builds_with_its_honest_lifecycle_and_tools() {
        use ironclaw_extension_contracts::memory::MemoryLifecycleHook;
        let bundle = mem0_memory_provider_bundle(&[]).expect("mem0 bundle builds");
        assert_eq!(
            bundle.package.manifest.id.as_str(),
            MEM0_MEMORY_EXTENSION_ID
        );
        assert!(bundle.lifecycle.declares(MemoryLifecycleHook::ReadLongTerm));
        assert!(bundle.lifecycle.declares(MemoryLifecycleHook::ProfileRead));
        assert!(
            !bundle
                .lifecycle
                .declares(MemoryLifecycleHook::ReadShortTerm)
        );
        assert!(
            !bundle
                .lifecycle
                .declares(MemoryLifecycleHook::RecordInteraction)
        );
        let ids: Vec<&str> = bundle
            .package
            .manifest
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                crate::PROFILE_SET_CAPABILITY_ID,
            ]
        );
    }

    #[test]
    fn mem0_backend_manifest_is_a_valid_v3_memory_provider() {
        let record = memory_manifest_record(MEM0_MEMORY_MANIFEST_TOML, None)
            .expect("mem0 backend manifest must parse");
        assert_eq!(record.manifest().id.as_str(), MEM0_MEMORY_EXTENSION_ID);
        match &record.manifest().runtime {
            ExtensionRuntimeV2::FirstParty { service } => {
                assert_eq!(service, MEM0_MEMORY_PROVIDER_SERVICE);
            }
            other => panic!("expected first_party runtime, got {other:?}"),
        }
        // mem0's manifest is the source of truth for its own tool surface:
        // the four document tools, declared under the reserved stable
        // `ironclaw.memory.*` ids so a backend swap never renames the
        // model's tools.
        let ids: Vec<&str> = record
            .manifest()
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                crate::PROFILE_SET_CAPABILITY_ID,
            ]
        );
        for capability in &record.manifest().capabilities {
            assert_eq!(capability.visibility, CapabilityVisibility::Model);
        }
        let memory = record
            .resolved()
            .memory
            .as_ref()
            .expect("mem0 manifest declares the [memory] surface");
        // The lifecycle declaration is honest (F5): mem0 implements the
        // long-term retrieval lane and profile reads, has no thread
        // partitioning (no short-term lane), and does not record
        // interactions — undeclared hooks are never called by the host.
        use ironclaw_extension_contracts::memory::MemoryLifecycleHook;
        assert!(memory.declares(MemoryLifecycleHook::ReadLongTerm));
        assert!(memory.declares(MemoryLifecycleHook::ProfileRead));
        assert!(!memory.declares(MemoryLifecycleHook::ReadShortTerm));
        assert!(!memory.declares(MemoryLifecycleHook::RecordInteraction));
    }
}
