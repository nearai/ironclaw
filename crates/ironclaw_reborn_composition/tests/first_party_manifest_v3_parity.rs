//! H.7 projection-equality gate (extension-runtime P1, checklist MAN-3).
//!
//! Every bundled first-party package was rewritten from manifest v2 to v3.
//! For each package this suite parses the pre-rewrite v2 snapshot
//! (`tests/fixtures/first_party_v2/<dir>.toml`) and the live asset through
//! the single record entry point and asserts the projections are identical:
//! derived surface kinds, capability ids, per-tool declarations, scopes, and
//! credentials. The two hosted-MCP packages (`notion-mcp`, `nearai-mcp`)
//! intentionally change shape — their placeholder static tools become one
//! `[mcp]` declaration — so they assert the declared ceiling plus the
//! connection template instead of static equality.
//!
//! Per-credential account setups are compared at the *derived surface*
//! level (union of scopes, sorted, deduplicated): v3 derives each
//! credential's setup from the vendor recipe's scope ceiling, which equals
//! v2's surface-level union — the connect-time behavior users see today.

use ironclaw_extensions::{
    CapabilitySurfaceDeclV2, ExtensionManifestRecord, ExtensionRuntimeV2, MANIFEST_SCHEMA_VERSION,
    MANIFEST_SCHEMA_VERSION_V3, ManifestSource,
};
use ironclaw_host_api::{RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource};
use ironclaw_host_runtime::{default_host_api_contract_registry, default_host_port_catalog};

fn parse(toml: &str) -> ExtensionManifestRecord {
    ExtensionManifestRecord::from_toml(
        toml,
        ManifestSource::HostBundled,
        &default_host_port_catalog().expect("default host port catalog"),
        None,
        &default_host_api_contract_registry().expect("default host api contracts"),
    )
    .expect("first-party manifest must parse")
}

fn v2_fixture(dir: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/first_party_v2/{dir}.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

fn live_asset(dir: &str) -> String {
    let path = format!(
        "{}/../ironclaw_first_party_extensions/assets/{dir}/manifest.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

fn setup_kind(setup: &RuntimeCredentialAccountSetup) -> &'static str {
    match setup {
        RuntimeCredentialAccountSetup::ManualToken => "manual_token",
        RuntimeCredentialAccountSetup::OAuth { .. } => "oauth",
        RuntimeCredentialAccountSetup::Retired => "retired",
        RuntimeCredentialAccountSetup::Pairing => "pairing",
    }
}

/// The union-level auth surface view: vendor -> (setup kind, sorted scopes).
fn auth_surface_view(record: &ExtensionManifestRecord) -> Vec<(String, &'static str, Vec<String>)> {
    let mut surfaces: Vec<(String, &'static str, Vec<String>)> = record
        .manifest()
        .capability_surfaces()
        .into_iter()
        .filter_map(|surface| match surface {
            CapabilitySurfaceDeclV2::Auth { provider, setup } => {
                let scopes = match &setup {
                    RuntimeCredentialAccountSetup::OAuth { scopes } => {
                        let mut scopes = scopes.clone();
                        scopes.sort();
                        scopes.dedup();
                        scopes
                    }
                    _ => Vec::new(),
                };
                Some((provider.as_str().to_string(), setup_kind(&setup), scopes))
            }
            _ => None,
        })
        .collect();
    surfaces.sort();
    surfaces
}

fn assert_static_projection_parity(dir: &str) {
    let v2 = parse(&v2_fixture(dir));
    let v3 = parse(&live_asset(dir));

    assert_eq!(
        v2.manifest().schema_version,
        MANIFEST_SCHEMA_VERSION,
        "{dir}: fixture must be the v2 snapshot"
    );
    assert_eq!(
        v3.manifest().schema_version,
        MANIFEST_SCHEMA_VERSION_V3,
        "{dir}: live asset must be rewritten to v3"
    );

    assert_eq!(v2.manifest().id, v3.manifest().id, "{dir}: id");
    assert_eq!(
        v2.manifest().requested_trust,
        v3.manifest().requested_trust,
        "{dir}: trust"
    );
    assert_eq!(
        v2.manifest().runtime,
        v3.manifest().runtime,
        "{dir}: runtime declaration"
    );

    // Derived surface kinds, in order. DEL-5 retired the v2 channel
    // vocabulary (`ironclaw.product_adapter/v1`), so a frozen v2 baseline can
    // no longer attest a channel surface — the channel surface is compared as
    // a v3-only presence pin (`slack_v3_still_declares_the_channel_surface`)
    // and excluded from the byte-order comparison here. Everything else must
    // match exactly, and the v2 baseline must not carry a channel surface at
    // all (its vocabulary no longer parses).
    let kinds = |record: &ExtensionManifestRecord| {
        record
            .manifest()
            .capability_surfaces()
            .iter()
            .map(CapabilitySurfaceDeclV2::kind)
            .collect::<Vec<_>>()
    };
    assert!(
        !kinds(&v2).contains(&ironclaw_host_api::CapabilitySurfaceKind::Channel),
        "{dir}: v2 fixtures cannot attest channel surfaces post-DEL-5"
    );
    let non_channel_kinds = |record: &ExtensionManifestRecord| {
        kinds(record)
            .into_iter()
            .filter(|kind| *kind != ironclaw_host_api::CapabilitySurfaceKind::Channel)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        non_channel_kinds(&v2),
        non_channel_kinds(&v3),
        "{dir}: derived surface kinds"
    );

    // Tool-by-tool parity.
    let (v2_tools, v3_tools) = (&v2.manifest().capabilities, &v3.manifest().capabilities);
    assert_eq!(v2_tools.len(), v3_tools.len(), "{dir}: tool count");
    for (a, b) in v2_tools.iter().zip(v3_tools.iter()) {
        let id = a.id.as_str();
        assert_eq!(a.id, b.id, "{dir}: capability id order");
        // Effects are compared modulo `DispatchCapability`: v3 normalizes
        // dispatchability uniformly (it is host plumbing, not authoring
        // vocabulary), while v2 declared it inconsistently (24 of github's
        // 48 tools). It gates nothing downstream; MAN-3's parity list is
        // surfaces / capability ids / scopes / credentials.
        let observable = |effects: &[ironclaw_host_api::EffectKind]| {
            effects
                .iter()
                .copied()
                .filter(|effect| *effect != ironclaw_host_api::EffectKind::DispatchCapability)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            observable(&a.effects),
            observable(&b.effects),
            "{dir}/{id}: effects"
        );
        assert!(
            b.effects
                .contains(&ironclaw_host_api::EffectKind::DispatchCapability),
            "{dir}/{id}: v3 normalization always includes the dispatch effect"
        );
        assert_eq!(
            a.default_permission, b.default_permission,
            "{dir}/{id}: default_permission"
        );
        assert_eq!(a.visibility, b.visibility, "{dir}/{id}: visibility");
        assert_eq!(
            a.input_schema_ref, b.input_schema_ref,
            "{dir}/{id}: input_schema_ref"
        );
        assert_eq!(
            a.prompt_doc_ref, b.prompt_doc_ref,
            "{dir}/{id}: prompt_doc_ref"
        );
        // Most v3 manifests drop `output_schema_ref` (schemas remain package
        // assets); the dialect regained the field with the redirect-egress
        // tool port, and a v3 declaration must then match the v2 baseline.
        assert!(
            b.output_schema_ref.is_none() || a.output_schema_ref == b.output_schema_ref,
            "{dir}/{id}: a declared v3 output_schema_ref must match the v2 baseline"
        );
        assert_eq!(
            a.required_host_ports, b.required_host_ports,
            "{dir}/{id}: required_host_ports"
        );
        assert_eq!(
            a.resource_profile, b.resource_profile,
            "{dir}/{id}: resource_profile"
        );
        assert_eq!(
            a.runtime_credentials.len(),
            b.runtime_credentials.len(),
            "{dir}/{id}: credential count"
        );
        for (ca, cb) in a
            .runtime_credentials
            .iter()
            .zip(b.runtime_credentials.iter())
        {
            assert_eq!(ca.handle, cb.handle, "{dir}/{id}: credential handle");
            assert_eq!(
                ca.provider_scopes, cb.provider_scopes,
                "{dir}/{id}: provider scopes"
            );
            assert_eq!(ca.audience, cb.audience, "{dir}/{id}: audience");
            assert_eq!(ca.target, cb.target, "{dir}/{id}: injection target");
            assert_eq!(ca.required, cb.required, "{dir}/{id}: required flag");
            match (&ca.source, &cb.source) {
                (
                    RuntimeCredentialRequirementSource::ProductAuthAccount {
                        provider: pa,
                        setup: sa,
                    },
                    RuntimeCredentialRequirementSource::ProductAuthAccount {
                        provider: pb,
                        setup: sb,
                    },
                ) => {
                    assert_eq!(pa, pb, "{dir}/{id}: credential vendor");
                    assert_eq!(
                        setup_kind(sa),
                        setup_kind(sb),
                        "{dir}/{id}: credential setup kind"
                    );
                }
                (a_source, b_source) => {
                    assert_eq!(a_source, b_source, "{dir}/{id}: credential source")
                }
            }
        }
    }

    // Union-level auth surface parity (vendor, setup kind, sorted scopes).
    assert_eq!(
        auth_surface_view(&v2),
        auth_surface_view(&v3),
        "{dir}: derived auth surfaces"
    );

    // v3 records must carry a recipe for every vendor.
    for auth in &v3.resolved().auth {
        assert!(
            auth.recipe.is_some(),
            "{dir}: v3 auth surface for {} must carry a recipe",
            auth.vendor
        );
    }
}

fn assert_hosted_mcp_projection(dir: &str, expected_namespace: &str) {
    let v2 = parse(&v2_fixture(dir));
    let v3 = parse(&live_asset(dir));

    assert_eq!(
        v3.manifest().schema_version,
        MANIFEST_SCHEMA_VERSION_V3,
        "{dir}: live asset must be rewritten to v3"
    );
    assert_eq!(v2.manifest().id, v3.manifest().id, "{dir}: id");

    // The proxied-server declaration replaces placeholder static tools: the
    // server URL is unchanged, and the connection credential matches the v2
    // template credential (same handle, vendor, injection).
    let ExtensionRuntimeV2::Mcp {
        url: Some(v2_url), ..
    } = &v2.manifest().runtime
    else {
        panic!("{dir}: v2 fixture must be a hosted MCP runtime");
    };
    let mcp = v3.resolved().mcp.as_ref().expect("v3 [mcp] declaration");
    assert_eq!(&mcp.server, v2_url, "{dir}: server URL");
    assert_eq!(mcp.namespace, expected_namespace, "{dir}: namespace");
    assert!(
        mcp.max_tools >= v2.manifest().capabilities.len() as u32,
        "{dir}: max_tools ceiling must cover the previous static set"
    );

    let v2_template = &v2.manifest().capabilities[0];
    let v3_template = &v3.manifest().capabilities[0];
    // The connection template leads; any further capabilities are statically
    // pinned tools (guaranteed present without live discovery — the bundled
    // fallback / first-boot set). Each static tool must exist in the v2
    // fixture under the same id with the same schema/prompt refs and
    // visibility, and must inherit the connection template's credentials —
    // v3 may pin fewer static tools than v2 declared (the rest became
    // discovery), but never invent new ones.
    for static_tool in &v3.manifest().capabilities[1..] {
        let id = static_tool.id.as_str();
        let v2_tool = v2
            .manifest()
            .capabilities
            .iter()
            .find(|capability| capability.id == static_tool.id)
            .unwrap_or_else(|| panic!("{dir}/{id}: static v3 tool must exist in the v2 fixture"));
        assert_eq!(
            static_tool.visibility, v2_tool.visibility,
            "{dir}/{id}: static tool visibility"
        );
        assert_eq!(
            static_tool.input_schema_ref, v2_tool.input_schema_ref,
            "{dir}/{id}: static tool input_schema_ref"
        );
        assert_eq!(
            static_tool.prompt_doc_ref, v2_tool.prompt_doc_ref,
            "{dir}/{id}: static tool prompt_doc_ref"
        );
        assert_eq!(
            static_tool.default_permission, v2_tool.default_permission,
            "{dir}/{id}: static tool default_permission"
        );
        assert_eq!(
            static_tool.runtime_credentials, v3_template.runtime_credentials,
            "{dir}/{id}: static tool inherits the connection template credentials"
        );
        assert_eq!(
            static_tool.required_host_ports, v3_template.required_host_ports,
            "{dir}/{id}: static tool inherits the connection template host ports"
        );
    }
    assert_eq!(
        v3_template.visibility,
        ironclaw_extensions::CapabilityVisibility::HostInternal,
        "{dir}: template is host-internal"
    );
    assert_eq!(
        v2_template.runtime_credentials.len(),
        v3_template.runtime_credentials.len(),
        "{dir}: connection credential count"
    );
    for (ca, cb) in v2_template
        .runtime_credentials
        .iter()
        .zip(v3_template.runtime_credentials.iter())
    {
        assert_eq!(ca.handle, cb.handle, "{dir}: connection credential handle");
        assert_eq!(ca.target, cb.target, "{dir}: connection injection");
        match (&ca.source, &cb.source) {
            (
                RuntimeCredentialRequirementSource::ProductAuthAccount { provider: pa, .. },
                RuntimeCredentialRequirementSource::ProductAuthAccount { provider: pb, .. },
            ) => assert_eq!(pa, pb, "{dir}: connection credential vendor"),
            (a_source, b_source) => assert_eq!(a_source, b_source, "{dir}: credential source"),
        }
    }

    // The effect ceiling covers every effect the static placeholders used.
    for capability in &v2.manifest().capabilities {
        for effect in &capability.effects {
            assert!(
                mcp.effects.contains(effect)
                    || *effect == ironclaw_host_api::EffectKind::DispatchCapability,
                "{dir}: ceiling must cover static effect {effect:?}"
            );
        }
    }

    // The tool surface count is intentionally different (placeholders became
    // discovery); auth surface parity still holds at the vendor level.
    let v2_auth = auth_surface_view(&v2);
    let v3_auth = auth_surface_view(&v3);
    assert_eq!(
        v2_auth
            .iter()
            .map(|(vendor, ..)| vendor)
            .collect::<Vec<_>>(),
        v3_auth
            .iter()
            .map(|(vendor, ..)| vendor)
            .collect::<Vec<_>>(),
        "{dir}: auth vendors"
    );
}

macro_rules! static_parity {
    ($name:ident, $dir:literal) => {
        #[test]
        fn $name() {
            assert_static_projection_parity($dir);
        }
    };
}

static_parity!(github_v3_projects_identically, "github");
static_parity!(gmail_v3_projects_identically, "gmail");
static_parity!(google_calendar_v3_projects_identically, "google-calendar");
static_parity!(google_docs_v3_projects_identically, "google-docs");
static_parity!(google_drive_v3_projects_identically, "google-drive");
static_parity!(google_sheets_v3_projects_identically, "google-sheets");
static_parity!(google_slides_v3_projects_identically, "google-slides");
static_parity!(slack_v3_projects_identically, "slack");
static_parity!(web_access_v3_projects_identically, "web-access");

/// DEL-5 removed `ironclaw.product_adapter/v1`, so the v2 slack baseline can
/// no longer carry its channel surface — this presence pin replaces the byte
/// parity for that one surface: the live v3 manifest must keep declaring the
/// Slack channel.
#[test]
fn slack_v3_still_declares_the_channel_surface() {
    let v3 = parse(&live_asset("slack"));
    let kinds = v3
        .manifest()
        .capability_surfaces()
        .iter()
        .map(CapabilitySurfaceDeclV2::kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == ironclaw_host_api::CapabilitySurfaceKind::Channel)
            .count(),
        1,
        "live slack manifest must declare exactly one channel surface; got {kinds:?}"
    );
}

#[test]
fn notion_mcp_v3_declares_the_ceiling() {
    assert_hosted_mcp_projection("notion-mcp", "notion");
}

#[test]
fn nearai_mcp_v3_declares_the_ceiling() {
    assert_hosted_mcp_projection("nearai-mcp", "nearai");
    // Main parity: web_search is statically pinned — model-visible from
    // first boot and on the bundled-manifest fallback, without live MCP
    // discovery (the regression `runtime_nearai_mcp_bootstraps_*` pins at
    // the runtime tier).
    let v3 = parse(&live_asset("nearai-mcp"));
    assert_eq!(
        v3.manifest()
            .capabilities
            .iter()
            .filter(|capability| {
                capability.visibility == ironclaw_extensions::CapabilityVisibility::Model
            })
            .map(|capability| capability.id.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["nearai.web_search".to_string()],
        "nearai-mcp: web_search must stay statically pinned"
    );
}
