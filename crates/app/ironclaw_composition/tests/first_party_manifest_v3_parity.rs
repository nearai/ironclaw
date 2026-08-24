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

use ironclaw_extension_registry::{
    CapabilitySurfaceDeclV2, ExtensionManifestRecord, ExtensionRuntimeV2, MANIFEST_SCHEMA_VERSION,
    MANIFEST_SCHEMA_VERSION_V3, ManifestSource, default_host_api_contract_registry,
};
use ironclaw_host_api::{
    capability::{RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource},
    host_port::default_host_port_catalog,
};

fn parse(toml: &str) -> ExtensionManifestRecord {
    ExtensionManifestRecord::from_toml(
        toml,
        ManifestSource::HostBundled,
        &default_host_port_catalog().expect("default host port catalog"),
        None,
        &default_host_api_contract_registry().expect("default host api contracts"),
        None,
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
        "{}/../../extensions/packages/{dir}/manifest.toml",
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
        RuntimeCredentialAccountSetup::DeviceLink => "device_link",
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

/// Capability ids a package has gained SINCE its v2 snapshot was frozen, in
/// manifest order, appended after the v2-era set.
///
/// This suite pins the v2->v3 *rewrite*, not the manifest forever: a package
/// that later grows a genuinely new tool would otherwise have to falsify its
/// historical v2 fixture (recording tools that never existed in v2) to stay
/// green, which would destroy the very baseline the gate exists to compare
/// against. Declaring the additions here keeps the original claim intact —
/// every v2-era tool still projects identically, positionally — while making
/// each addition an explicit, reviewable line rather than a relaxed
/// assertion. An addition also has to earn its own per-package test pinning
/// what it declares (`slack_v3_appends_the_remaining_standard_ops`).
///
/// Scopes work the same way: `[auth.<vendor>]` is a union ceiling over the
/// per-tool scope lists, so a new tool needing a new scope necessarily widens
/// it. The additions' scope delta is declared per package and asserted to be
/// exactly what the live union added.
struct PackageAdditions {
    /// Capability ids appended after the v2-era set, in manifest order.
    tool_ids: &'static [&'static str],
    /// Scopes the live auth union has gained, sorted.
    added_scopes: &'static [&'static str],
}

const NO_ADDITIONS: PackageAdditions = PackageAdditions {
    tool_ids: &[],
    added_scopes: &[],
};

/// Slack completed its coverage of the 16 core standard messaging operations
/// after the v2 freeze. Order matches the `[[tools]]` order in
/// `crates/extensions/packages/slack/manifest.toml`.
const SLACK_ADDITIONS: PackageAdditions = PackageAdditions {
    tool_ids: &[
        "slack.edit_message",
        "slack.delete_message",
        "slack.add_reaction",
        "slack.remove_reaction",
        "slack.open_dm",
        "slack.get_message",
        "slack.resolve_user",
        "slack.list_members",
    ],
    // The reaction pair and open_dm are the only additions needing a scope
    // the v2-era grant did not already hold; the four read-side additions
    // reuse the existing read scopes, and edit/delete reuse chat:write.
    added_scopes: &["im:write", "reactions:read", "reactions:write"],
};

/// Google Docs gained semantic operations after the v2 freeze. They reuse the
/// existing read/write scope ceiling and are appended after every legacy tool.
const GOOGLE_DOCS_ADDITIONS: PackageAdditions = PackageAdditions {
    tool_ids: &[
        "google-docs.inspect_document",
        "google-docs.apply_text_edits",
        "google-docs.create_table_with_data",
        "google-docs.verify_document",
    ],
    added_scopes: &[],
};

fn assert_static_projection_parity(dir: &str) {
    assert_projection_parity_with_additions(dir, &NO_ADDITIONS);
}

fn assert_projection_parity_with_additions(dir: &str, additions: &PackageAdditions) {
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
        !kinds(&v2)
            .contains(&ironclaw_extension_contracts::surface::CapabilitySurfaceKind::Channel),
        "{dir}: v2 fixtures cannot attest channel surfaces post-DEL-5"
    );
    let non_channel_kinds = |record: &ExtensionManifestRecord| {
        kinds(record)
            .into_iter()
            .filter(|kind| {
                *kind != ironclaw_extension_contracts::surface::CapabilitySurfaceKind::Channel
            })
            .collect::<Vec<_>>()
    };
    // Non-tool surfaces (auth, ...) must still match exactly and in order.
    // Tool surfaces are compared by count, because a declared addition adds
    // one; with no additions this is the same assertion as before.
    let tool_kind = ironclaw_extension_contracts::surface::CapabilitySurfaceKind::Tool;
    let without_tools = |record: &ExtensionManifestRecord| {
        non_channel_kinds(record)
            .into_iter()
            .filter(|kind| *kind != tool_kind)
            .collect::<Vec<_>>()
    };
    let tool_surfaces = |record: &ExtensionManifestRecord| {
        non_channel_kinds(record)
            .into_iter()
            .filter(|kind| *kind == tool_kind)
            .count()
    };
    assert_eq!(
        without_tools(&v2),
        without_tools(&v3),
        "{dir}: derived non-tool surface kinds"
    );
    assert_eq!(
        tool_surfaces(&v3),
        tool_surfaces(&v2) + additions.tool_ids.len(),
        "{dir}: tool surface count (v2 baseline plus {} declared addition(s))",
        additions.tool_ids.len()
    );

    // Tool-by-tool parity. The v2-era tools are compared positionally, so an
    // addition must be APPENDED — reordering or interleaving one fails here,
    // which is what keeps the historical comparison meaningful.
    let (v2_tools, v3_tools) = (&v2.manifest().capabilities, &v3.manifest().capabilities);
    assert_eq!(
        v3_tools.len(),
        v2_tools.len() + additions.tool_ids.len(),
        "{dir}: tool count"
    );
    let appended = v3_tools[v2_tools.len().min(v3_tools.len())..]
        .iter()
        .map(|capability| capability.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        appended, additions.tool_ids,
        "{dir}: tools appended since the v2 freeze must be declared in PackageAdditions"
    );
    for (a, b) in v2_tools.iter().zip(v3_tools.iter()) {
        let id = a.id.as_str();
        assert_eq!(a.id, b.id, "{dir}: capability id order");
        // Effects are compared modulo `DispatchCapability`: v3 normalizes
        // dispatchability uniformly (it is host plumbing, not authoring
        // vocabulary), while v2 declared it inconsistently (24 of github's
        // 48 tools). It gates nothing downstream; MAN-3's parity list is
        // surfaces / capability ids / scopes / credentials.
        let observable = |effects: &[ironclaw_host_api::capability::EffectKind]| {
            effects
                .iter()
                .copied()
                .filter(|effect| {
                    *effect != ironclaw_host_api::capability::EffectKind::DispatchCapability
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            observable(&a.effects),
            observable(&b.effects),
            "{dir}/{id}: effects"
        );
        assert!(
            b.effects
                .contains(&ironclaw_host_api::capability::EffectKind::DispatchCapability),
            "{dir}/{id}: v3 normalization always includes the dispatch effect"
        );
        assert_eq!(
            a.default_permission, b.default_permission,
            "{dir}/{id}: default_permission"
        );
        assert_eq!(a.visibility, b.visibility, "{dir}/{id}: visibility");
        // A `standard_op`-bound tool's schema refs are host-synthesized
        // (standardized messaging framework, task 9: `standard:messaging/<op>.
        // {input,output}.<version>`), replacing the package-authored refs the
        // frozen v2 baseline recorded — an intentional divergence from the
        // v2->v3 rewrite this suite otherwise pins byte-for-byte, not a
        // projection bug. Assert the synthesized shape instead of baseline
        // equality; every other declared field (effects, permission,
        // visibility, prompt_doc_ref, credentials) still must match the v2
        // baseline exactly, same as any other tool.
        //
        // The output version is read from the op's own contract, not spelled
        // `.v1`: `send_message` graduated to `.output.v2` (the
        // `sent_unverified` evidence branch), and hardcoding a version here
        // would pin the ratchet to whichever ops had graduated the day it was
        // written.
        match b.standard_op {
            Some(op) => {
                assert_eq!(
                    b.input_schema_ref.as_str(),
                    format!("standard:messaging/{}.input.v1", op.op_name()),
                    "{dir}/{id}: standard_op input_schema_ref must be host-synthesized"
                );
                let output_version = op
                    .contract()
                    .unwrap_or_else(|| panic!("{dir}/{id}: bound to a reserved standard op"))
                    .output_schema_version;
                assert_eq!(
                    b.output_schema_ref
                        .as_ref()
                        .map(|schema_ref| schema_ref.as_str()),
                    Some(
                        format!(
                            "standard:messaging/{}.output.{}",
                            op.op_name(),
                            output_version.as_str()
                        )
                        .as_str()
                    ),
                    "{dir}/{id}: standard_op output_schema_ref must be host-synthesized"
                );
            }
            None => {
                assert_eq!(
                    a.input_schema_ref, b.input_schema_ref,
                    "{dir}/{id}: input_schema_ref"
                );
                // Most v3 manifests drop `output_schema_ref` (schemas remain
                // package assets); the dialect regained the field with the
                // redirect-egress tool port, and a v3 declaration must then
                // match the v2 baseline.
                assert!(
                    b.output_schema_ref.is_none() || a.output_schema_ref == b.output_schema_ref,
                    "{dir}/{id}: a declared v3 output_schema_ref must match the v2 baseline"
                );
            }
        }
        assert_eq!(
            a.prompt_doc_ref, b.prompt_doc_ref,
            "{dir}/{id}: prompt_doc_ref"
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
    // With no declared additions this is exact equality, unchanged. With
    // additions, the vendor and setup kind still must match exactly and the
    // scope union may only GROW, by exactly the declared delta — a scope
    // silently disappearing from the ceiling would narrow every existing
    // tool's grant, so the subset direction is asserted too.
    let (v2_auth, v3_auth) = (auth_surface_view(&v2), auth_surface_view(&v3));
    if additions.added_scopes.is_empty() {
        assert_eq!(v2_auth, v3_auth, "{dir}: derived auth surfaces");
    } else {
        assert_eq!(
            v2_auth
                .iter()
                .map(|(vendor, setup, _)| (vendor, setup))
                .collect::<Vec<_>>(),
            v3_auth
                .iter()
                .map(|(vendor, setup, _)| (vendor, setup))
                .collect::<Vec<_>>(),
            "{dir}: derived auth vendors and setup kinds"
        );
        let baseline_scopes = v2_auth
            .iter()
            .flat_map(|(_, _, scopes)| scopes.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        let live_scopes = v3_auth
            .iter()
            .flat_map(|(_, _, scopes)| scopes.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        let dropped = baseline_scopes
            .difference(&live_scopes)
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            dropped.is_empty(),
            "{dir}: the auth scope ceiling may only grow; these v2 scopes are gone: {dropped:?}"
        );
        let added = live_scopes
            .difference(&baseline_scopes)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            added, additions.added_scopes,
            "{dir}: scopes gained since the v2 freeze must be declared in PackageAdditions"
        );
    }

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
        ironclaw_extension_registry::CapabilityVisibility::HostInternal,
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
                    || *effect == ironclaw_host_api::capability::EffectKind::DispatchCapability,
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
#[test]
fn google_docs_v3_projects_identically_with_semantic_additions() {
    assert_projection_parity_with_additions("google-docs", &GOOGLE_DOCS_ADDITIONS);
}
static_parity!(google_drive_v3_projects_identically, "google-drive");
static_parity!(google_sheets_v3_projects_identically, "google-sheets");
static_parity!(google_slides_v3_projects_identically, "google-slides");
/// Slack is the one package with declared post-freeze additions: its v2-era
/// eight tools still project identically, and the eight standard ops appended
/// since are pinned by `slack_v3_appends_the_remaining_standard_ops`.
#[test]
fn slack_v3_projects_identically() {
    assert_projection_parity_with_additions("slack", &SLACK_ADDITIONS);
}
static_parity!(web_access_v3_projects_identically, "web-access");

/// The eight standard messaging operations Slack gained after the v2 freeze.
/// `slack_v3_projects_identically` proves they were APPENDED without
/// disturbing the v2-era eight; this proves each one is a real standard-op
/// binding rather than a bespoke tool wearing a canonical id.
///
/// The registry's own parse-time rules (reserved ops, id shape, absent schema
/// refs, the write effects floor, one binding per op) already fail the
/// manifest at install time — these assertions pin the *projection* those
/// rules produce, which is what the host resolves schemas and gates writes
/// from at dispatch.
#[test]
fn slack_v3_appends_the_remaining_standard_ops() {
    use ironclaw_host_api::messaging::StandardMessagingOp;

    let v3 = parse(&live_asset("slack"));
    let bound: Vec<(&str, StandardMessagingOp)> = v3
        .manifest()
        .capabilities
        .iter()
        .filter_map(|capability| {
            capability
                .standard_op
                .map(|op| (capability.id.as_str(), op))
        })
        .collect();

    // Slack binds every core operation exactly once — the whole point of the
    // change, and the property that keeps the model from meeting a vendor
    // with half a messaging vocabulary.
    assert_eq!(
        bound.len(),
        16,
        "slack must bind all 16 core standard messaging operations, got {bound:?}"
    );
    let core: std::collections::BTreeSet<&str> = StandardMessagingOp::ALL
        .iter()
        .filter(|op| op.contract().is_some())
        .map(|op| op.op_name())
        .collect();
    let slack_ops: std::collections::BTreeSet<&str> =
        bound.iter().map(|(_, op)| op.op_name()).collect();
    assert_eq!(
        slack_ops, core,
        "slack's bound ops must be exactly the core set"
    );

    for (id, op) in &bound {
        // The binding rule fixes a bound tool's id; a drift here would make
        // the guest's capability-id dispatch table unreachable.
        assert_eq!(
            *id,
            format!("slack.{}", op.op_name()),
            "standard op tool id must be slack.<op_name>"
        );

        let capability = v3
            .manifest()
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == *id)
            .expect("capability just enumerated");

        // Host-canonical schemas on both directions: each operation pins the
        // current output version declared by its contract. The output half is
        // what makes a send that cannot produce provider evidence fail instead
        // of silently passing through.
        assert_eq!(
            capability.input_schema_ref.as_str(),
            format!("standard:messaging/{}.input.v1", op.op_name())
        );
        let output_version = op
            .contract()
            .unwrap_or_else(|| panic!("{id}: bound to a reserved standard op"))
            .output_schema_version;
        let expected_output_ref = format!(
            "standard:messaging/{}.output.{}",
            op.op_name(),
            output_version.as_str()
        );
        assert_eq!(
            capability
                .output_schema_ref
                .as_ref()
                .map(|schema_ref| schema_ref.as_str()),
            Some(expected_output_ref.as_str())
        );

        // Write ops must declare external_write (spec §6 rule 4) — this is
        // what routes them through the approval path reads skip.
        if op.is_write() {
            assert!(
                capability
                    .effects
                    .contains(&ironclaw_host_api::capability::EffectKind::ExternalWrite),
                "{id}: write op must declare the external_write effect"
            );
        }

        // Every bound tool keeps a package-owned vendor addendum; without one
        // the model gets the extension-neutral core text with no Slack
        // dialect notes (emoji names, mrkdwn, id shapes).
        let prompt_doc_ref = capability
            .prompt_doc_ref
            .as_ref()
            .unwrap_or_else(|| panic!("{id}: standard op must ship a vendor addendum"));
        assert_eq!(
            prompt_doc_ref.as_str(),
            format!("prompts/slack/{}.md", op.op_name()),
            "{id}: addendum path is derived from the op name"
        );

        // Every tool runs on the user token, never the bot's.
        let handles = capability
            .runtime_credentials
            .iter()
            .map(|credential| credential.handle.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            handles,
            ["slack_user_token"],
            "{id}: standard ops act as the connected user"
        );
    }
}

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
            .filter(|kind| **kind
                == ironclaw_extension_contracts::surface::CapabilitySurfaceKind::Channel)
            .count(),
        1,
        "live slack manifest must declare exactly one channel surface; got {kinds:?}"
    );
}

#[test]
fn slack_v3_declares_only_bounded_file_transfer_egress() {
    use ironclaw_host_api::action::NetworkMethod;

    let v3 = parse(&live_asset("slack"));
    let channel = v3
        .resolved()
        .channel
        .as_ref()
        .expect("slack manifest must declare its channel");
    assert_eq!(channel.egress.len(), 4);

    let api_post = channel
        .egress
        .iter()
        .find(|target| target.host == "slack.com" && target.methods == [NetworkMethod::Post])
        .expect("Slack API POST target");
    assert_eq!(
        api_post.paths,
        [
            "/api/chat.postMessage",
            // Sender-visible-only delivery for the unbound-user connect nudge
            // (#7681). Same `chat:write` bot scope as `chat.postMessage`, so
            // this widens the allowlist without widening the Slack grant.
            "/api/chat.postEphemeral",
            "/api/chat.delete",
            "/api/conversations.open",
            "/api/files.completeUploadExternal",
            "/api/reactions.add",
            "/api/reactions.remove",
        ]
    );
    assert_eq!(api_post.request_body_limit_bytes, Some(256 * 1024));
    assert_eq!(api_post.response_body_limit_bytes, Some(256 * 1024));
    assert!(
        api_post
            .paths
            .iter()
            .all(|path| path != "/api/files.upload")
    );

    let api_get = channel
        .egress
        .iter()
        .find(|target| target.host == "slack.com" && target.methods == [NetworkMethod::Get])
        .expect("Slack API GET target");
    assert_eq!(
        api_get.paths,
        [
            "/api/files.info",
            "/api/files.getUploadURLExternal",
            "/api/conversations.history",
            "/api/conversations.replies",
        ]
    );
    assert_eq!(api_get.request_body_limit_bytes, Some(0));
    assert_eq!(api_get.response_body_limit_bytes, Some(256 * 1024));

    let private_download = channel
        .egress
        .iter()
        .find(|target| target.host == "files.slack.com" && target.methods == [NetworkMethod::Get])
        .expect("Slack private download target");
    assert_eq!(private_download.path_prefixes, ["/files-pri/"]);
    assert_eq!(private_download.request_body_limit_bytes, Some(0));
    assert_eq!(
        private_download.response_body_limit_bytes,
        Some(5 * 1024 * 1024)
    );

    let external_upload = channel
        .egress
        .iter()
        .find(|target| target.host == "files.slack.com" && target.methods == [NetworkMethod::Post])
        .expect("Slack external upload target");
    assert_eq!(external_upload.path_prefixes, ["/upload/"]);
    assert_eq!(external_upload.response_body_limit_bytes, Some(256 * 1024));
    assert_eq!(
        external_upload.request_body_limit_bytes,
        Some(5 * 1024 * 1024)
    );
}

#[test]
fn telegram_v3_declares_only_the_bot_api_and_bounded_file_transfer_paths() {
    let v3 = parse(&live_asset("telegram"));
    let channel = v3
        .resolved()
        .channel
        .as_ref()
        .expect("telegram manifest must declare its channel");
    assert_eq!(channel.egress.len(), 2);

    let post = channel
        .egress
        .iter()
        .find(|target| target.methods == [ironclaw_host_api::action::NetworkMethod::Post])
        .expect("bounded Bot API POST target");
    assert_eq!(post.host, "api.telegram.org");
    assert_eq!(
        post.paths,
        [
            "/bot{telegram_bot_token}/setWebhook",
            "/bot{telegram_bot_token}/deleteWebhook",
            "/bot{telegram_bot_token}/sendMessage",
            "/bot{telegram_bot_token}/deleteMessage",
            "/bot{telegram_bot_token}/setMessageReaction",
            "/bot{telegram_bot_token}/getFile",
            "/bot{telegram_bot_token}/sendDocument",
        ]
    );
    assert_eq!(
        post.request_body_limit_bytes,
        Some(5 * 1024 * 1024 + 64 * 1024)
    );
    // This target also serves sendMessage/deleteMessage, whose responses echo
    // the full Message object (including `reply_to_message` when the adapter
    // threads a reply). A file-sized response cap here failed sends that had
    // already reached the user, so it keeps the host default.
    assert_eq!(post.response_body_limit_bytes, Some(256 * 1024));

    let download = channel
        .egress
        .iter()
        .find(|target| target.methods == [ironclaw_host_api::action::NetworkMethod::Get])
        .expect("bounded Telegram file GET target");
    assert_eq!(download.path_prefixes, ["/file/bot{telegram_bot_token}/"]);
    assert_eq!(download.request_body_limit_bytes, Some(0));
    assert_eq!(download.response_body_limit_bytes, Some(5 * 1024 * 1024));
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
                capability.visibility == ironclaw_extension_registry::CapabilityVisibility::Model
            })
            .map(|capability| capability.id.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["nearai.web_search".to_string()],
        "nearai-mcp: web_search must stay statically pinned"
    );
}
