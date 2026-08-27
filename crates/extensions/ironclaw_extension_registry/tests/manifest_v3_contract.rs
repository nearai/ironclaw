//! Extension Manifest v3 contract tests (extension-runtime P1, workstream A).
//!
//! v3 is v2 plus explicit `[channel]` and `[auth.*]` sections and an `[mcp]`
//! declaration for proxied servers. Both schemas parse through the single
//! `ExtensionManifestRecord::from_toml` entry point and normalize into the
//! same [`ResolvedExtensionManifest`] (checklist MAN-2).

use std::sync::Arc;

use ironclaw_extension_contracts::{
    channel::ConversationModel, memory::MemoryLifecycleHook, recipe::VendorAuthRecipe,
    surface::CapabilitySurfaceKind,
};
use ironclaw_extension_registry::{
    CapabilityProviderHostApiContract, CapabilitySurfaceDeclV2, CapabilityVisibility,
    ExtensionManifest, ExtensionManifestRecord, ExtensionPackage, ExtensionRuntimeV2,
    HostApiContractRegistry, MANIFEST_SCHEMA_VERSION_V3, ManifestSource,
};
use ironclaw_host_api::{
    capability::{
        CapabilityDescriptor, EffectKind, OriginGatePolicy, PermissionMode,
        RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource,
    },
    host_port::{
        HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostPortCatalog, HostPortCatalogEntry, HostPortId,
    },
    messaging::StandardMessagingOp,
    path::VirtualPath,
};

const ACME_MANIFEST: &str =
    include_str!("../../../../tests/fixtures/extensions/acme-messenger/manifest.toml");

fn contracts() -> HostApiContractRegistry {
    let mut registry = HostApiContractRegistry::new();
    registry
        .register(Arc::new(
            CapabilityProviderHostApiContract::new().expect("contract"),
        ))
        .expect("register capability provider contract");
    registry
}

fn catalog() -> HostPortCatalog {
    HostPortCatalog::new(vec![HostPortCatalogEntry::new(
        HostPortId::new(HOST_RUNTIME_HTTP_EGRESS_PORT_ID).unwrap(),
    )])
    .unwrap()
}

fn parse_v3(toml: &str) -> Result<ExtensionManifestRecord, String> {
    parse_v3_with_source(toml, ManifestSource::HostBundled)
}

fn parse_v3_with_source(
    toml: &str,
    source: ManifestSource,
) -> Result<ExtensionManifestRecord, String> {
    ExtensionManifestRecord::from_toml(toml, source, &catalog(), None, &contracts(), None)
        .map_err(|error| error.to_string())
}

fn acme_record() -> ExtensionManifestRecord {
    parse_v3(ACME_MANIFEST).expect("acme fixture manifest must parse")
}

// arch-exempt: large_file, manifest-v3 cases remain one conformance suite pending fixture split, plan #7477
// Parsing the documented v3 shape
// ---------------------------------------------------------------------------

#[test]
fn acme_fixture_parses_through_the_single_entry_point() {
    let record = acme_record();
    let manifest = record.manifest();
    assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION_V3);
    assert_eq!(manifest.id.as_str(), "acme-messenger");
    assert!(matches!(
        &manifest.runtime,
        ExtensionRuntimeV2::FirstParty { service } if service == "acme-messenger.extension/v1"
    ));

    // send_note plus the 16 core standard messaging ops, normalized into the
    // internal capability model; send_note stays first (bespoke coexistence
    // proof — declared before the standard_op entries in the fixture).
    assert_eq!(manifest.capabilities.len(), 17);
    let tool = &manifest.capabilities[0];
    assert_eq!(tool.id.as_str(), "acme-messenger.send_note");
    assert_eq!(tool.visibility, CapabilityVisibility::Model);
    assert_eq!(tool.default_permission, PermissionMode::Ask);
    // The dispatch effect is an implementation detail the normalizer adds;
    // authors declare only the externally meaningful effects.
    assert_eq!(
        tool.effects,
        vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ]
    );
    // First-party services receive host services through invocation wiring;
    // only sandboxed runtimes (wasm/mcp) derive the egress port from the
    // network effect.
    assert!(tool.required_host_ports.is_empty());
    // The acme fixture declares no output_schema_ref (optional in v3).
    assert!(tool.output_schema_ref.is_none());

    // Credential: vendor + per-tool scopes; the account setup derives from
    // the [auth.acme] recipe's scope ceiling.
    assert_eq!(tool.runtime_credentials.len(), 1);
    let credential = &tool.runtime_credentials[0];
    assert_eq!(credential.handle.as_str(), "acme_user_token");
    assert_eq!(credential.provider_scopes, vec!["notes:write".to_string()]);
    match &credential.source {
        RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } => {
            assert_eq!(provider.as_str(), "acme");
            assert_eq!(
                setup,
                &RuntimeCredentialAccountSetup::OAuth {
                    scopes: vec!["notes:write".to_string()],
                }
            );
        }
        other => panic!("expected product auth account source, got {other:?}"),
    }

    // Standard-op spot check (standardized messaging framework, task 7): the
    // 16 core ops bind alongside send_note; send_message is representative.
    let send_message = manifest
        .capabilities
        .iter()
        .find(|capability| capability.id.as_str() == "acme-messenger.send_message")
        .expect("acme fixture declares send_message");
    assert_eq!(
        send_message.standard_op,
        Some(StandardMessagingOp::SendMessage)
    );
}

#[test]
fn acme_fixture_resolves_channel_and_auth_recipe() {
    let record = acme_record();
    let resolved = record.resolved();

    let channel = resolved.channel.as_ref().expect("channel declared");
    assert_eq!(channel.id, "messages");
    assert_eq!(channel.conversation_model, ConversationModel::Continuous);
    let ingress = channel.ingress.as_ref().expect("ingress declared");
    assert_eq!(
        ingress
            .route_suffix
            .as_ref()
            .expect("webhook ingress declares a route_suffix")
            .as_str(),
        "events"
    );

    assert_eq!(resolved.auth.len(), 1);
    let auth = &resolved.auth[0];
    assert_eq!(auth.vendor.as_str(), "acme");
    let recipe = auth.recipe.as_ref().expect("v3 auth carries a recipe");
    let VendorAuthRecipe::Oauth2Code(recipe) = recipe else {
        panic!("expected oauth2_code recipe");
    };
    assert_eq!(
        recipe.authorization_endpoint.as_str(),
        "https://auth.acme.example/oauth/authorize"
    );

    // The channel surface participates in the derived surface set: one Tool
    // kind per declared capability (send_note plus the 16 standard ops),
    // then Channel, then Auth.
    let kinds: Vec<CapabilitySurfaceKind> = record
        .manifest()
        .capability_surfaces()
        .iter()
        .map(CapabilitySurfaceDeclV2::kind)
        .collect();
    let mut expected_kinds =
        vec![CapabilitySurfaceKind::Tool; record.manifest().capabilities.len()];
    expected_kinds.push(CapabilitySurfaceKind::Channel);
    expected_kinds.push(CapabilitySurfaceKind::Auth);
    assert_eq!(kinds, expected_kinds);
}

#[test]
fn admin_configuration_is_manifest_declared_and_resolved_without_installation_state() {
    let record = parse_v3(ACME_MANIFEST)
        .expect("manifest-declared admin configuration should parse without installation state");
    let [descriptor] = record.resolved().admin_configuration.as_slice() else {
        panic!("expected one resolved admin configuration descriptor");
    };
    assert_eq!(descriptor.group_id.as_str(), "vendor.acme");
    assert_eq!(descriptor.fields.len(), 2);
    assert!(descriptor.fields[0].secret);
    assert!(descriptor.fields[1].secret);
    assert_eq!(
        descriptor.fields[0].description,
        "Issued by the Acme developer console under Bot Settings.",
        "a manifest-declared field description must survive resolution"
    );
    assert!(
        descriptor.fields[1].description.is_empty(),
        "a field without a declared description resolves to empty, not an error"
    );
}

#[test]
fn duplicate_admin_configuration_handles_fail_closed() {
    let original_fields = r#"fields = [
  { handle = "acme_bot_token", label = "Bot token", secret = true, required = true, description = "Issued by the Acme developer console under Bot Settings." },
  { handle = "acme_signing_secret", label = "Signing secret", secret = true, required = true },
]"#;
    assert!(
        ACME_MANIFEST.contains(original_fields),
        "fixture fields block drifted; update this replacement source"
    );
    let toml = ACME_MANIFEST.replace(
        original_fields,
        r#"fields = [
  { handle = "acme_client_id", label = "Client ID", secret = false, required = true },
  { handle = "acme_client_id", label = "Duplicate", secret = true, required = true },
]"#,
    );

    let error = parse_v3(&toml).expect_err("duplicate handles must fail closed");
    assert!(
        error.contains("duplicate") && error.contains("acme_client_id"),
        "{error}"
    );
}

#[test]
fn channel_runtime_configuration_comes_from_admin_configuration_alone() {
    let record = parse_v3(ACME_MANIFEST)
        .expect("one manifest-owned admin schema should configure the channel runtime");
    let [descriptor] = record.resolved().admin_configuration.as_slice() else {
        panic!("expected one resolved admin configuration descriptor");
    };
    assert_eq!(descriptor.group_id.as_str(), "vendor.acme");
    assert_eq!(
        descriptor
            .fields
            .iter()
            .map(|field| field.handle.as_str())
            .collect::<Vec<_>>(),
        vec!["acme_bot_token", "acme_signing_secret"]
    );
    assert!(record.resolved().channel.is_some());
}

#[test]
fn channel_runtime_secret_references_must_be_declared_by_admin_configuration() {
    let original_fields = r#"fields = [
  { handle = "acme_bot_token", label = "Bot token", secret = true, required = true, description = "Issued by the Acme developer console under Bot Settings." },
  { handle = "acme_signing_secret", label = "Signing secret", secret = true, required = true },
]"#;
    assert!(
        ACME_MANIFEST.contains(original_fields),
        "fixture fields block drifted; update this replacement source"
    );
    let toml = ACME_MANIFEST.replace(
        original_fields,
        r#"fields = [
  { handle = "acme_bot_token", label = "Bot token", secret = true, required = true },
]"#,
    );

    let error = parse_v3(&toml).expect_err("undeclared channel secrets must fail closed");
    assert!(
        error.contains("channel ingress verification")
            && error.contains("acme_signing_secret")
            && error.contains("admin_configuration"),
        "{error}"
    );
}

/// An `[admin_configuration]` group is deployment-owned, operator-managed state.
/// Only a host-bundled (first-party) manifest — one compiled into the binary —
/// may declare one. An untrusted, filesystem-discovered, or registry-installed
/// manifest must be rejected at parse: otherwise it could collide with a
/// first-party group id (aborting boot via a descriptor conflict) or register
/// itself as a consumer of a first-party group's non-secret routing.
#[test]
fn admin_configuration_group_is_reserved_to_first_party_manifests() {
    const THIRD_PARTY_ADMIN_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "third-party-admin"
name = "Third Party Admin"
version = "0.1.0"
description = "A third-party manifest that declares a deployment-owned admin group."
trust = "third_party"

[admin_configuration]
group_id = "vendor.rogue"
display_name = "Rogue deployment configuration"
fields = [ { handle = "rogue_secret", label = "Secret", secret = true, required = true } ]

[runtime]
kind = "wasm"
module = "wasm/rogue.wasm"

[[tools]]
id = "third-party-admin.noop"
description = "A no-op tool."
effects = []
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/rogue/noop.input.v1.json"
"#;

    // A host-bundled (first-party) source may declare an admin group.
    parse_v3_with_source(THIRD_PARTY_ADMIN_MANIFEST, ManifestSource::HostBundled)
        .expect("host-bundled manifest may declare [admin_configuration]");

    // Every non-first-party source is rejected at parse — the earliest
    // fail-closed point for the deployment-owned admin surface.
    for source in [
        ManifestSource::InstalledLocal,
        ManifestSource::RegistryInstalled,
    ] {
        let error = parse_v3_with_source(THIRD_PARTY_ADMIN_MANIFEST, source)
            .expect_err("a non-first-party manifest must not declare [admin_configuration]");
        assert!(
            error.contains("admin_configuration")
                && (error.contains("host-bundled") || error.contains("first-party")),
            "{source:?}: {error}"
        );
    }
}

// Each channel runtime reference must resolve to a matching `[admin_configuration]`
// field. The ingress-verification branch is covered above; these cover the
// remaining fail-closed branches (egress credential, egress body credential,
// connection deep-link placeholder, and the secret-flag mismatch).

#[test]
fn undeclared_channel_egress_credential_fails_closed() {
    let toml = ACME_MANIFEST.replace(
        "credential_handle = \"acme_bot_token\"",
        "credential_handle = \"acme_undeclared_egress_token\"",
    );

    let error = parse_v3(&toml).expect_err("an undeclared egress credential must fail closed");
    assert!(
        error.contains("channel egress credential")
            && error.contains("acme_undeclared_egress_token")
            && error.contains("admin_configuration"),
        "{error}"
    );
}

#[test]
fn undeclared_channel_egress_body_credential_fails_closed() {
    let toml = ACME_MANIFEST.replace(
        "methods = [\"post\"]\ncredential_handle = \"acme_bot_token\"",
        "methods = [\"post\"]\ncredential_handle = \"acme_bot_token\"\n\
         body_credentials = [{ handle = \"acme_undeclared_body_secret\", pointer = \"/token\" }]",
    );

    let error = parse_v3(&toml).expect_err("an undeclared egress body credential must fail closed");
    assert!(
        error.contains("channel egress body credential")
            && error.contains("acme_undeclared_body_secret")
            && error.contains("admin_configuration"),
        "{error}"
    );
}

#[test]
fn undeclared_channel_connection_placeholder_fails_closed() {
    // Extend the fixture channel with a generated-code connection whose deep
    // link interpolates a non-`{code}` placeholder that no admin field declares.
    let toml = ACME_MANIFEST.replace(
        "[channel.presentation]\n\
         supports_markdown = true\n\
         supports_threads = false\n",
        "[channel.presentation]\n\
         supports_markdown = true\n\
         supports_threads = false\n\n\
         [channel.connection]\n\
         provider = \"acme\"\n\
         strategy = \"web_generated_code\"\n\
         instructions = \"Pair your Acme account by opening the link.\"\n\
         submit_label = \"Open pairing\"\n\
         error_message = \"Pairing failed.\"\n\
         connection_success_message = \"Acme paired.\"\n\
         deep_link_template = \"https://acme.example/pair?ref={acme_undeclared_ref}&code={code}\"\n\n\
         [channel.connection.notices]\n\
         connect_required = \"Pair first.\"\n\
         paired = \"Paired.\"\n\
         already_paired_same_user = \"Already paired.\"\n\
         already_bound_to_other_user = \"Paired elsewhere.\"\n\
         expired_or_unknown = \"Invalid code.\"\n",
    );
    assert!(
        toml.contains("[channel.connection]"),
        "the connection block must be inserted for this test to exercise the placeholder branch"
    );

    let error = parse_v3(&toml).expect_err("an undeclared connection placeholder must fail closed");
    assert!(
        error.contains("channel connection placeholder")
            && error.contains("acme_undeclared_ref")
            && error.contains("admin_configuration"),
        "{error}"
    );
}

#[test]
fn channel_secret_declared_with_wrong_secret_flag_fails_closed() {
    // The ingress verification requires `acme_signing_secret` as a secret field;
    // declaring it non-secret in [admin_configuration] must fail closed rather
    // than silently expose a signing secret through the non-secret read path.
    let toml = ACME_MANIFEST.replace(
        "{ handle = \"acme_signing_secret\", label = \"Signing secret\", secret = true, required = true },",
        "{ handle = \"acme_signing_secret\", label = \"Signing secret\", secret = false, required = true },",
    );

    let error = parse_v3(&toml)
        .expect_err("a channel secret declared with the wrong flag must fail closed");
    assert!(
        error.contains("channel ingress verification")
            && error.contains("acme_signing_secret")
            && error.contains("secret = true"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// Fail-closed validation (MAN-4, MAN-5)
// ---------------------------------------------------------------------------

#[test]
fn unknown_top_level_fields_fail_closed_with_path_context() {
    let toml = ACME_MANIFEST.replace(
        "trust = \"first_party_requested\"",
        "trust = \"first_party_requested\"\nsurprise = 1",
    );
    let error = parse_v3(&toml).unwrap_err();
    assert!(error.contains("surprise"), "{error}");
}

#[test]
fn non_https_recipe_endpoints_are_rejected() {
    let toml = ACME_MANIFEST.replace(
        "authorization_endpoint = \"https://auth.acme.example/oauth/authorize\"",
        "authorization_endpoint = \"http://auth.acme.example/oauth/authorize\"",
    );
    let error = parse_v3(&toml).unwrap_err();
    assert!(error.contains("https"), "{error}");
}

#[test]
fn reserved_authorize_params_are_rejected() {
    let toml = ACME_MANIFEST.replace(
        "pkce = \"s256\"",
        "pkce = \"s256\"\nextra_authorize_params = { redirect_uri = \"https://evil.example\" }",
    );
    let error = parse_v3(&toml).unwrap_err();
    assert!(error.contains("redirect_uri"), "{error}");
}

#[test]
fn wildcard_or_deep_json_pointers_are_rejected() {
    let wildcard = ACME_MANIFEST.replace(
        "access_token = \"/access_token\"",
        "access_token = \"/tokens/*\"",
    );
    assert!(parse_v3(&wildcard).is_err());

    let deep = ACME_MANIFEST.replace(
        "access_token = \"/access_token\"",
        "access_token = \"/a/b/c/d/e/f/g/h/i\"",
    );
    assert!(parse_v3(&deep).is_err());
}

#[test]
fn wildcard_egress_hosts_are_rejected() {
    let toml = ACME_MANIFEST.replace(
        "host = \"api.acme.example\"\nmethods = [\"post\"]",
        "host = \"*.acme.example\"\nmethods = [\"post\"]",
    );
    let error = parse_v3(&toml).unwrap_err();
    assert!(
        error.contains("wildcard") || error.contains("literal"),
        "{error}"
    );
}

#[test]
fn multi_segment_route_suffixes_are_rejected() {
    let toml = ACME_MANIFEST.replace(
        "route_suffix = \"events\"",
        "route_suffix = \"events/deep\"",
    );
    let error = parse_v3(&toml).unwrap_err();
    assert!(
        error.contains("segment") || error.contains("route_suffix"),
        "{error}"
    );
}

#[test]
fn conversation_model_is_required() {
    let toml = ACME_MANIFEST.replace("conversation_model = \"continuous\"\n", "");
    let error = parse_v3(&toml).unwrap_err();
    assert!(error.contains("conversation_model"), "{error}");
}

#[test]
fn referenced_vendors_require_an_auth_recipe() {
    // Point the tool credential at a vendor with no [auth.*] section.
    let toml = ACME_MANIFEST.replace("vendor = \"acme\"", "vendor = \"zeta\"");
    let error = parse_v3(&toml).unwrap_err();
    assert!(error.contains("zeta"), "{error}");
}

/// A `device_link` recipe must project onto
/// `RuntimeCredentialAccountSetup::DeviceLink` on **both** sides of the
/// manifest — the per-tool credential requirement and the resolved auth
/// surface — because the two are read by different consumers (the auth-gate
/// challenge and the extension-card connect affordance). A vendor whose
/// surface disagreed with its credentials would offer a connect affordance
/// that services a challenge the engine cannot mint. The setup carries no
/// scopes: a linked device holds the account's own authority, which is why
/// `VendorAuthRecipe::scope_ceiling` is empty for this method.
#[test]
fn device_link_recipe_projects_onto_the_device_link_account_setup() {
    let toml = ACME_MANIFEST
        .replace(
            r#"method = "oauth2_code"
display_name = "Acme account"
authorization_endpoint = "https://auth.acme.example/oauth/authorize"
token_endpoint = "https://auth.acme.example/oauth/token"
pkce = "s256"
scopes = ["notes:write"]
client_credentials = { client_id_handle = "acme_oauth_client_id", client_secret_handle = "acme_oauth_client_secret" }

[auth.acme.token_response]
access_token = "/access_token"
scope = { path = "/scope", missing = "fallback_to_requested" }

[auth.acme.identity]
account_id = "/account/id""#,
            r#"method = "device_link"
display_name = "Acme account"
default_mode_label = "Scan a code"
alternate_mode_label = "Use your phone number""#,
        )
        // The tool credential's own `scopes` are a per-tool request, not the
        // recipe ceiling; a device-link vendor grants no scopes at all.
        .replace(r#"scopes = ["notes:write"]"#, "scopes = []");
    assert!(
        !toml.contains("oauth2_code"),
        "the oauth recipe block must actually have been replaced"
    );

    let record = parse_v3(&toml).expect("a device_link recipe is a valid v3 auth section");
    let resolved = record.resolved();

    let auth = resolved
        .auth
        .iter()
        .find(|surface| surface.vendor.as_str() == "acme")
        .expect("acme auth surface");
    assert!(
        matches!(
            auth.recipe.as_ref().expect("recipe"),
            VendorAuthRecipe::DeviceLink(_)
        ),
        "the recipe must round-trip as device_link"
    );
    assert_eq!(auth.setup, RuntimeCredentialAccountSetup::DeviceLink);

    let credential = record
        .manifest()
        .capabilities
        .iter()
        .flat_map(|capability| capability.runtime_credentials.iter())
        .find(|credential| credential.handle.as_str() == "acme_user_token")
        .expect("acme fixture declares the user-token credential");
    match &credential.source {
        RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } => {
            assert_eq!(provider.as_str(), "acme");
            assert_eq!(
                setup,
                &RuntimeCredentialAccountSetup::DeviceLink,
                "the credential requirement must agree with the auth surface"
            );
        }
        other => panic!("expected product auth account source, got {other:?}"),
    }
}

#[test]
fn wildcard_tool_audience_hosts_are_rejected() {
    let toml = ACME_MANIFEST.replace(
        "audience = { scheme = \"https\", host = \"api.acme.example\" }",
        "audience = { scheme = \"https\", host = \"*.acme.example\" }",
    );
    assert!(parse_v3(&toml).is_err());
}

// ---------------------------------------------------------------------------
// [mcp] declarations (MAN-6)
// ---------------------------------------------------------------------------

fn mcp_manifest() -> String {
    format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "zeta"
name = "Zeta"
version = "0.1.0"
description = "Hosted MCP fixture"
trust = "third_party"

[mcp]
server = "https://mcp.zeta.example/mcp"
namespace = "zeta"
max_tools = 64
default_permission = "ask"
effects = ["network", "use_secret"]

[[mcp.credentials]]
handle = "zeta_account"
vendor = "zeta"
scopes = ["read_content"]
injection = {{ type = "header", name = "authorization", prefix = "Bearer " }}

[auth.zeta]
method = "oauth2_code"
display_name = "Zeta account"
authorization_endpoint = "https://auth.zeta.example/authorize"
token_endpoint = "https://auth.zeta.example/token"
scopes = ["read_content"]
client_credentials = {{ client_id_handle = "zeta_client_id" }}

[auth.zeta.token_response]
access_token = "/access_token"
"#
    )
}

/// Same shape as [`mcp_manifest`] but with the id (and matching `[mcp]`
/// namespace) parameterized, for exercising the reserved `mcp-` id
/// namespace.
fn mcp_manifest_with_id(id: &str) -> String {
    format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "{id}"
name = "Zeta"
version = "0.1.0"
description = "Hosted MCP fixture"
trust = "third_party"

[mcp]
server = "https://mcp.zeta.example/mcp"
namespace = "{id}"
max_tools = 64
default_permission = "ask"
effects = ["network", "use_secret"]
"#
    )
}

// ---------------------------------------------------------------------------
// Reserved `mcp-` id namespace (seed for user-registered MCP servers)
// ---------------------------------------------------------------------------

/// A non-`UserRegistered` source declaring an `mcp-`-prefixed id must be
/// rejected — through the public `ExtensionManifestRecord::from_toml` entry
/// point (one of the two import paths the single `parse_v3` arm must cover).
#[test]
fn reserved_mcp_namespace_rejects_non_user_registered_source_via_from_toml() {
    for source in [
        ManifestSource::HostBundled,
        ManifestSource::InstalledLocal,
        ManifestSource::RegistryInstalled,
    ] {
        let error = parse_v3_with_source(&mcp_manifest_with_id("mcp-foo"), source)
            .expect_err("non-user-registered source must not claim the mcp- namespace");
        assert!(
            error.contains("mcp-"),
            "error should name the reserved prefix, got: {error}"
        );
    }
}

/// A `UserRegistered` manifest with an `mcp-` id parses fine.
#[test]
fn reserved_mcp_namespace_allows_user_registered_source_via_from_toml() {
    let record = parse_v3_with_source(
        &mcp_manifest_with_id("mcp-foo"),
        ManifestSource::UserRegistered,
    )
    .expect("user-registered source may declare an mcp- id");
    assert_eq!(record.manifest().id.as_str(), "mcp-foo");
}

/// A `UserRegistered` manifest can never declare first-party/system trust,
/// mcp- id or not.
#[test]
fn user_registered_source_cannot_declare_first_party_trust() {
    let manifest = mcp_manifest_with_id("mcp-foo").replace(
        "trust = \"third_party\"",
        "trust = \"first_party_requested\"",
    );
    let error = parse_v3_with_source(&manifest, ManifestSource::UserRegistered)
        .expect_err("user-registered source must never be granted first-party trust");
    assert!(
        error.contains("trust"),
        "error should mention trust, got: {error}"
    );
}

#[test]
fn mcp_manifest_parses_and_synthesizes_a_host_internal_template() {
    let record = parse_v3(&mcp_manifest()).expect("mcp manifest parses");
    let manifest = record.manifest();
    assert!(matches!(
        &manifest.runtime,
        ExtensionRuntimeV2::Mcp { transport, url: Some(url), command: None, .. }
            if transport == "http" && url == "https://mcp.zeta.example/mcp"
    ));
    // The connection template capability is host-internal: never advertised
    // to the model; discovery replaces it with the server's tools.
    assert_eq!(manifest.capabilities.len(), 1);
    let template = &manifest.capabilities[0];
    assert_eq!(template.id.as_str(), "zeta.mcp_server");
    assert_eq!(template.visibility, CapabilityVisibility::HostInternal);
    assert_eq!(template.runtime_credentials.len(), 1);
    // The [mcp] connection credential's audience is the server host —
    // nothing a server returns can widen egress.
    assert_eq!(
        template.runtime_credentials[0].audience.host_pattern,
        "mcp.zeta.example"
    );

    let resolved = record.resolved();
    let mcp = resolved.mcp.as_ref().expect("resolved mcp declaration");
    assert_eq!(mcp.namespace, "zeta");
    assert_eq!(mcp.max_tools, 64);
}

#[test]
fn mcp_is_mutually_exclusive_with_runtime_and_channel() {
    let with_runtime = mcp_manifest().replace(
        "[mcp]",
        "[runtime]\nkind = \"wasm\"\nmodule = \"wasm/zeta.wasm\"\n\n[mcp]",
    );
    assert!(parse_v3(&with_runtime).is_err());

    let with_channel = format!(
        "{}\n[channel]\nid = \"messages\"\ndisplay_name = \"Zeta\"\nconversation_model = \"continuous\"\n",
        mcp_manifest()
    );
    assert!(parse_v3(&with_channel).is_err());
}

/// Regression contract for the boot-time hosted-MCP tool guarantee: an
/// `[mcp]` manifest may pin static `[[tools]]` that exist without live
/// discovery (bundled fallback, first boot). They inherit the connection
/// template's credential/effect/host-port shape — a static tool declaring
/// its own credentials, effects, or resource_profile is rejected.
#[test]
fn mcp_static_tools_parse_and_inherit_the_connection_template() {
    let with_static_tool = format!(
        "{}\n[[tools]]\nid = \"zeta.search\"\ndescription = \"Search through Zeta.\"\ndefault_permission = \"ask\"\ninput_schema_ref = \"schemas/zeta/search.input.v1.json\"\n",
        mcp_manifest()
    );
    let record = parse_v3(&with_static_tool).expect("mcp manifest with static tool parses");
    let manifest = record.manifest();
    assert_eq!(manifest.capabilities.len(), 2);
    // The host-internal connection template stays first (discovery reads the
    // template from the leading capability).
    let template = &manifest.capabilities[0];
    assert_eq!(template.id.as_str(), "zeta.mcp_server");
    assert_eq!(template.visibility, CapabilityVisibility::HostInternal);
    let static_tool = &manifest.capabilities[1];
    assert_eq!(static_tool.id.as_str(), "zeta.search");
    assert_eq!(static_tool.visibility, CapabilityVisibility::Model);
    // Inherited template shape: same credentials (server-host audience),
    // same effects, same host ports — the discovery template-consistency
    // check must hold for every capability on the package.
    assert_eq!(
        static_tool.runtime_credentials,
        template.runtime_credentials
    );
    assert_eq!(static_tool.effects, template.effects);
    assert_eq!(
        static_tool.required_host_ports,
        template.required_host_ports
    );
    assert_eq!(
        static_tool.runtime_credentials[0].audience.host_pattern,
        "mcp.zeta.example"
    );

    for divergent in [
        "credentials = [{ handle = \"zeta_account\", vendor = \"zeta\", audience = { scheme = \"https\", host = \"mcp.zeta.example\" }, injection = { type = \"header\", name = \"authorization\", prefix = \"Bearer \" } }]",
        "effects = [\"network\"]",
        "resource_profile = { default_estimate = { wall_clock_ms = 5000 } }",
        "network_targets = [{ scheme = \"https\", host_pattern = \"cdn.zeta.example\" }]",
        "output_schema_ref = \"schemas/zeta/search.output.v1.json\"",
        "standard_op = \"send_message\"",
    ] {
        let with_divergent_tool = format!(
            "{}\n[[tools]]\nid = \"zeta.search\"\ndescription = \"Search through Zeta.\"\ndefault_permission = \"ask\"\ninput_schema_ref = \"schemas/zeta/search.input.v1.json\"\n{divergent}\n",
            mcp_manifest()
        );
        assert!(
            parse_v3(&with_divergent_tool).is_err(),
            "static mcp tool declaring `{divergent}` must be rejected"
        );
    }
}

#[test]
fn mcp_requires_server_namespace_and_max_tools() {
    for field in ["server = ", "namespace = ", "max_tools = "] {
        let toml: String = mcp_manifest()
            .lines()
            .filter(|line| !line.starts_with(field))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            parse_v3(&toml).is_err(),
            "expected rejection when `{field}` is missing"
        );
    }
}

#[test]
fn declaring_neither_runtime_nor_mcp_is_rejected() {
    let toml: String = mcp_manifest()
        .replace("[mcp]", "[metadata_ignored_mcp]")
        .lines()
        .filter(|line| {
            !line.starts_with("server = ")
                && !line.starts_with("namespace = ")
                && !line.starts_with("max_tools = ")
                && !line.starts_with("default_permission = ")
                && !line.starts_with("effects = ")
                && !line.starts_with("[metadata_ignored_mcp]")
                && !line.starts_with("[[mcp.credentials]]")
                && !line.starts_with("handle = ")
                && !line.starts_with("vendor = ")
                && !line.starts_with("scopes = [\"read_content\"]")
                && !line.starts_with("injection = ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(parse_v3(&toml).is_err());
}

// ---------------------------------------------------------------------------
// v2 normalization parity (MAN-2, MAN-3 groundwork)
// ---------------------------------------------------------------------------

/// A v2 manifest and its hand-written v3 rewrite resolve to identical
/// surfaces, capability ids, scopes, and credentials.
#[test]
fn v2_and_v3_rewrites_resolve_identically() {
    let v2 = r#"
schema_version = "reborn.extension_manifest.v2"
id = "zephyrite"
name = "Zephyrite"
version = "0.1.0"
description = "test"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/zephyrite_tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "zephyrite.echo"
description = "Echoes input"
effects = ["dispatch_capability", "network", "use_secret"]
runtime_credentials = [
  { handle = "zephyrite_token", source = { type = "product_auth_account", provider = "zephyrite", setup = { kind = "oauth", scopes = ["echo:read"] } }, provider_scopes = ["echo:read"], audience = { scheme = "https", host_pattern = "api.zephyrite.example" }, target = { type = "header", name = "authorization", prefix = "Bearer " }, placeholder_env = "ZEPHYRITE_TOKEN" },
]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zephyrite/echo.input.v1.json"
output_schema_ref = "schemas/zephyrite/echo.output.v1.json"
required_host_ports = ["host.runtime.http_egress"]
"#;
    let v3 = format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "zephyrite"
name = "Zephyrite"
version = "0.1.0"
description = "test"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/zephyrite_tool.wasm"

[[tools]]
id = "zephyrite.echo"
description = "Echoes input"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zephyrite/echo.input.v1.json"

[[tools.credentials]]
handle = "zephyrite_token"
vendor = "zephyrite"
scopes = ["echo:read"]
audience = {{ scheme = "https", host = "api.zephyrite.example" }}
injection = {{ type = "header", name = "authorization", prefix = "Bearer " }}
placeholder_env = "ZEPHYRITE_TOKEN"

[auth.zephyrite]
method = "oauth2_code"
display_name = "Zephyrite account"
authorization_endpoint = "https://auth.zephyrite.example/authorize"
token_endpoint = "https://auth.zephyrite.example/token"
scopes = ["echo:read"]
client_credentials = {{ client_id_handle = "zephyrite_client_id" }}

[auth.zephyrite.token_response]
access_token = "/access_token"
"#
    );

    let v2_record = parse_v3(v2).expect("v2 parses");
    let v3_record = parse_v3(&v3).expect("v3 parses");

    let v2_manifest = v2_record.manifest();
    let v3_manifest = v3_record.manifest();

    // Same capability ids, effects, permissions, ports.
    assert_eq!(
        v2_manifest.capabilities.len(),
        v3_manifest.capabilities.len()
    );
    let (a, b) = (&v2_manifest.capabilities[0], &v3_manifest.capabilities[0]);
    assert_eq!(a.id, b.id);
    assert_eq!(a.effects, b.effects);
    assert_eq!(a.default_permission, b.default_permission);
    assert_eq!(a.required_host_ports, b.required_host_ports);
    assert_eq!(a.input_schema_ref, b.input_schema_ref);
    // Same credentials: handle, vendor, setup scopes, per-tool scopes,
    // audience, injection, and direct process binding.
    assert_eq!(a.runtime_credentials, b.runtime_credentials);

    // Same derived surface kinds (tool + auth).
    let kinds = |manifest: &ironclaw_extension_registry::ExtensionManifestV2| {
        manifest
            .capability_surfaces()
            .iter()
            .map(CapabilitySurfaceDeclV2::kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(v2_manifest), kinds(v3_manifest));

    // The v3 resolved model additionally carries the recipe.
    assert!(v3_record.resolved().auth[0].recipe.is_some());
    assert!(v2_record.resolved().auth[0].recipe.is_none());
    // But the auth surface itself (vendor + setup) is identical.
    assert_eq!(
        v2_record.resolved().auth[0].vendor,
        v3_record.resolved().auth[0].vendor
    );
    assert_eq!(
        v2_record.resolved().auth[0].setup,
        v3_record.resolved().auth[0].setup
    );
}

/// Regression contract for the v3 dialect extension shipped with the
/// redirect-egress tool port: a plain (non-`[mcp]`) `[[tools]]` entry may
/// declare `output_schema_ref` and credential-free `network_targets`, and
/// both thread into the normalized capability (the egress allowlist and the
/// output-schema asset requirement read them from there).
#[test]
fn plain_tools_thread_output_schema_ref_and_network_targets() {
    let toml = format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "zephyrite"
name = "Zephyrite"
version = "0.1.0"
description = "test"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/zephyrite_tool.wasm"

[[tools]]
id = "zephyrite.fetch_log"
description = "Fetches a build log."
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zephyrite/fetch_log.input.v1.json"
output_schema_ref = "schemas/zephyrite/fetch_log.output.v1.json"
network_targets = [{{ scheme = "https", host_pattern = "*.blob.zephyrite.example" }}]
"#
    );
    let record = parse_v3(&toml).expect("plain v3 manifest with dialect fields parses");
    let manifest = record.manifest();
    assert_eq!(manifest.capabilities.len(), 1);
    let tool = &manifest.capabilities[0];
    assert_eq!(
        tool.output_schema_ref.as_ref().map(|r| r.as_str()),
        Some("schemas/zephyrite/fetch_log.output.v1.json")
    );
    assert_eq!(tool.network_targets.len(), 1);
    assert_eq!(
        tool.network_targets[0].scheme,
        Some(ironclaw_host_api::action::NetworkScheme::Https)
    );
    assert_eq!(
        tool.network_targets[0].host_pattern,
        "*.blob.zephyrite.example"
    );
}

// ---------------------------------------------------------------------------
// Resolved record: rebuild without reparse (REC-1/REC-2 groundwork)
// ---------------------------------------------------------------------------

#[test]
fn records_rebuild_from_the_resolved_contract_without_reparsing_toml() {
    let original = acme_record();
    let resolved = original.resolved().clone();

    // The raw source is diagnostics-only: a record rebuilt from the resolved
    // contract must not need to parse it.
    let rebuilt = ExtensionManifestRecord::from_resolved(
        "# raw manifest source unavailable".to_string(),
        ManifestSource::HostBundled,
        resolved,
        None,
    )
    .expect("rebuild from resolved");
    assert_eq!(rebuilt.manifest(), original.manifest());
    assert_eq!(rebuilt.resolved(), original.resolved());
}

#[test]
fn resolved_contract_round_trips_through_serde() {
    let record = acme_record();
    let json = serde_json::to_string(record.resolved()).expect("serialize");
    let back: ironclaw_extension_registry::ResolvedExtensionManifest =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(&back, record.resolved());
}

// ---------------------------------------------------------------------------
// [memory] surface validation (#3537, lifecycle-capability contract)
// ---------------------------------------------------------------------------

/// Minimal well-formed `[memory]` manifest (a provider-only shape: no tools,
/// first_party runtime, the full lifecycle set). Each test perturbs one axis
/// of this baseline.
const MEMORY_PROVIDER_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme.memory"
name = "Acme Memory"
version = "0.1.0"
description = "Test memory provider manifest."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acme_memory_provider"

[memory]
lifecycle = ["read_long_term", "read_short_term", "record_interaction", "profile_read"]
"#;

const FULL_LIFECYCLE_LINE: &str =
    r#"lifecycle = ["read_long_term", "read_short_term", "record_interaction", "profile_read"]"#;

/// Replace the baseline's full-lifecycle line, failing loudly if the needle
/// ever drifts from the baseline — `str::replace` with an unmatched needle is
/// a no-op that would silently run the test against the unmodified manifest.
fn manifest_with_lifecycle_replaced(replacement: &str) -> String {
    assert!(
        MEMORY_PROVIDER_MANIFEST.contains(FULL_LIFECYCLE_LINE),
        "FULL_LIFECYCLE_LINE drifted from MEMORY_PROVIDER_MANIFEST"
    );
    MEMORY_PROVIDER_MANIFEST.replace(FULL_LIFECYCLE_LINE, replacement)
}

#[test]
fn memory_provider_manifest_baseline_parses_with_full_lifecycle() {
    let record = parse_v3(MEMORY_PROVIDER_MANIFEST).expect("memory provider manifest parses");
    let memory = record
        .resolved()
        .memory
        .as_ref()
        .expect("resolved manifest carries the [memory] descriptor");
    for hook in [
        MemoryLifecycleHook::ReadLongTerm,
        MemoryLifecycleHook::ReadShortTerm,
        MemoryLifecycleHook::RecordInteraction,
        MemoryLifecycleHook::ProfileRead,
    ] {
        assert!(
            memory.declares(hook),
            "baseline manifest must declare {hook:?}"
        );
    }
}

#[test]
fn memory_surface_on_non_first_party_runtime_fails_closed() {
    let toml = MEMORY_PROVIDER_MANIFEST.replace(
        "kind = \"first_party\"\nservice = \"acme_memory_provider\"",
        "kind = \"wasm\"\nmodule = \"wasm/acme_memory.wasm\"",
    );
    let error = parse_v3(&toml).expect_err("[memory] on a wasm runtime must fail closed");
    assert!(
        error.contains("[memory]") && error.contains("first_party runtime"),
        "{error}"
    );
}

/// F2 regression: a provider truthfully declaring only the hooks it
/// implements is a legal manifest. Undeclared hooks resolve as not declared.
#[test]
fn memory_surface_accepts_a_lifecycle_subset() {
    let toml =
        manifest_with_lifecycle_replaced(r#"lifecycle = ["read_long_term", "record_interaction"]"#);
    let record = parse_v3(&toml).expect("a lifecycle subset must parse");
    let memory = record
        .resolved()
        .memory
        .as_ref()
        .expect("resolved manifest carries the [memory] descriptor");
    assert!(memory.declares(MemoryLifecycleHook::ReadLongTerm));
    assert!(memory.declares(MemoryLifecycleHook::RecordInteraction));
    assert!(!memory.declares(MemoryLifecycleHook::ReadShortTerm));
    assert!(!memory.declares(MemoryLifecycleHook::ProfileRead));
}

/// A `[memory]` section with an empty lifecycle is a tools-only memory
/// backend: it contributes its declared tools and participates in no
/// host-initiated hook.
#[test]
fn memory_surface_with_empty_lifecycle_is_tools_only() {
    let toml = manifest_with_lifecycle_replaced("lifecycle = []");
    let record = parse_v3(&toml).expect("[memory] with an empty lifecycle must parse");
    let memory = record
        .resolved()
        .memory
        .as_ref()
        .expect("resolved manifest carries the [memory] descriptor");
    assert!(memory.lifecycle.is_empty());
}

/// `lifecycle` may be absent entirely — equivalent to an empty declaration.
#[test]
fn memory_surface_with_absent_lifecycle_is_tools_only() {
    let toml = manifest_with_lifecycle_replaced("");
    let record = parse_v3(&toml).expect("[memory] with no lifecycle key must parse");
    let memory = record
        .resolved()
        .memory
        .as_ref()
        .expect("resolved manifest carries the [memory] descriptor");
    assert!(memory.lifecycle.is_empty());
}

/// Unknown lifecycle tokens fail closed: the vocabulary is exactly
/// `read_long_term | read_short_term | record_interaction | profile_read`.
#[test]
fn memory_surface_rejects_an_unknown_lifecycle_token() {
    let toml = manifest_with_lifecycle_replaced(r#"lifecycle = ["on_boot"]"#);
    parse_v3(&toml).expect_err("an unknown lifecycle token must fail closed");
}

/// A memory-tool declaration under the reserved stable namespace, requesting
/// gating the host must clamp: `ungated` on a write-effect tool that is NOT in
/// the reviewed Ungated allowlist, plus an `ungated` product cell.
const RESERVED_NAMESPACE_WRITE_TOOL: &str = r#"
[[tools]]
id = "ironclaw.memory.write"
description = "Write persistent memory documents."
effects = ["read_filesystem", "write_filesystem"]
default_permission = "allow"
visibility = "model"
origin_gate_matrix = { loop_run = "ungated", product = "ungated", automation = "forbidden" }
input_schema_ref = "schemas/memory/document-write.input.v1.json"
"#;

const RESERVED_NAMESPACE_SEARCH_TOOL: &str = r#"
[[tools]]
id = "ironclaw.memory.search"
description = "Search persistent memory documents."
effects = ["read_filesystem"]
default_permission = "allow"
visibility = "model"
origin_gate_matrix = { loop_run = "ungated", product = "forbidden", automation = "forbidden" }
input_schema_ref = "schemas/memory/search.input.v1.json"
"#;

/// A `[memory]`-declaring manifest may declare tools under the reserved stable
/// `ironclaw.memory.*` namespace even when its own extension id differs, so
/// swapping the bound backend does not rename the model's tools. Trust-safe:
/// `[memory]` requires a first_party runtime, which requires a host-bundled
/// source.
#[test]
fn memory_provider_declares_tools_under_the_reserved_namespace() {
    let toml = format!("{MEMORY_PROVIDER_MANIFEST}{RESERVED_NAMESPACE_SEARCH_TOOL}");
    let record = parse_v3(&toml).expect("a [memory] manifest may declare reserved-namespace tools");
    let ids: Vec<&str> = record
        .manifest()
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str())
        .collect();
    assert_eq!(ids, vec!["ironclaw.memory.search"]);
}

/// Without a `[memory]` surface the reserved namespace stays closed: the
/// ordinary provider-prefix rule rejects a foreign `ironclaw.memory.*` id.
#[test]
fn reserved_memory_namespace_requires_a_memory_surface() {
    let memory_section = format!("[memory]\n{FULL_LIFECYCLE_LINE}\n");
    assert!(
        MEMORY_PROVIDER_MANIFEST.contains(&memory_section),
        "[memory] section drifted from MEMORY_PROVIDER_MANIFEST"
    );
    let toml = format!(
        "{}{RESERVED_NAMESPACE_SEARCH_TOOL}",
        MEMORY_PROVIDER_MANIFEST.replace(&memory_section, "")
    );
    let error = parse_v3(&toml)
        .expect_err("a non-memory manifest must not declare reserved-namespace tools");
    assert!(error.contains("provider-prefixed"), "{error}");
}

/// Requested tool gating is requested, not granted: `ungated` is a reviewed
/// host allowlist decision, so a memory provider requesting `ungated` on a
/// write tool (off-allowlist) — or on the product column — is clamped to
/// `gated_unless_granted`. Declared non-`ungated` cells pass through.
#[test]
fn memory_tool_requesting_ungated_write_is_clamped() {
    let toml = format!("{MEMORY_PROVIDER_MANIFEST}{RESERVED_NAMESPACE_WRITE_TOOL}");
    let record = parse_v3(&toml).expect("the clamped manifest still parses");
    let write = record
        .manifest()
        .capabilities
        .iter()
        .find(|capability| capability.id.as_str() == "ironclaw.memory.write")
        .expect("write tool declared");
    let matrix = write
        .origin_gate_matrix
        .as_ref()
        .expect("write tool carries a matrix");
    assert_eq!(
        matrix.loop_run,
        OriginGatePolicy::GatedUnlessGranted,
        "off-allowlist ungated loop_run must be clamped"
    );
    assert_eq!(
        matrix.product,
        OriginGatePolicy::GatedUnlessGranted,
        "ungated product must be clamped (no reviewed product allowlist)"
    );
    assert_eq!(matrix.automation, OriginGatePolicy::Forbidden);
}

/// The reviewed Ungated allowlist still applies: an allowlisted read-only
/// memory tool keeps its requested `ungated` loop_run.
#[test]
fn allowlisted_memory_read_tool_keeps_ungated_loop_run() {
    let toml = format!("{MEMORY_PROVIDER_MANIFEST}{RESERVED_NAMESPACE_SEARCH_TOOL}");
    let record = parse_v3(&toml).expect("manifest parses");
    let search = record
        .manifest()
        .capabilities
        .iter()
        .find(|capability| capability.id.as_str() == "ironclaw.memory.search")
        .expect("search tool declared");
    let matrix = search
        .origin_gate_matrix
        .as_ref()
        .expect("search tool carries a matrix");
    assert_eq!(matrix.loop_run, OriginGatePolicy::Ungated);
}

// ---------------------------------------------------------------------------
// standard_op binding (standardized messaging framework, task 2)
// ---------------------------------------------------------------------------

/// Baseline v3 manifest with one `[[tools]]` entry bound to the
/// `send_message` standard op: canonical id shape (`<extension>.<op_name>`),
/// no declared schema refs (the host synthesizes them), and the
/// `external_write` effect the write-op rule requires. Individual tests
/// perturb one axis via `.replace(...)`, mirroring `mcp_manifest()` above.
fn zeta_standard_op_manifest() -> String {
    format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "zeta"
name = "Zeta"
version = "0.1.0"
description = "Zeta standard-op fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/zeta.wasm"

[[tools]]
standard_op = "send_message"
id = "zeta.send_message"
description = "Zeta notes."
effects = ["network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"

[[tools.credentials]]
handle = "zeta_user_token"
vendor = "zeta"
scopes = ["chat:write"]
audience = {{ scheme = "https", host = "api.zeta.example" }}
injection = {{ type = "header", name = "authorization", prefix = "Bearer " }}

[auth.zeta]
method = "oauth2_code"
display_name = "Zeta account"
authorization_endpoint = "https://auth.zeta.example/authorize"
token_endpoint = "https://auth.zeta.example/token"
scopes = ["chat:write"]
client_credentials = {{ client_id_handle = "zeta_client_id" }}

[auth.zeta.token_response]
access_token = "/access_token"
"#
    )
}

#[test]
fn standard_op_binding_threads_and_synthesizes_canonical_refs() {
    let record = parse_v3(&zeta_standard_op_manifest()).expect("standard op binding parses");
    let cap = &record.manifest().capabilities[0];
    assert_eq!(cap.standard_op, Some(StandardMessagingOp::SendMessage));
    assert_eq!(
        cap.input_schema_ref.as_str(),
        "standard:messaging/send_message.input.v1"
    );
    // The synthesized ref carries the op's CURRENT schema version, which is
    // not necessarily `.v1`: `send_message` graduated to `.output.v2` (the
    // `sent_unverified` evidence branch). Already-installed bindings keep
    // their `.v1` ref — nothing rewrites a resolved record — but a manifest
    // parsed today pins the current version.
    assert_eq!(
        cap.output_schema_ref.as_ref().map(|r| r.as_str()),
        Some("standard:messaging/send_message.output.v2")
    );
}

#[test]
fn standard_op_reserved_name_is_rejected() {
    let toml = zeta_standard_op_manifest().replace(
        "standard_op = \"send_message\"",
        "standard_op = \"forward_message\"",
    );
    let error = parse_v3(&toml).expect_err("a reserved standard_op must be rejected");
    assert!(error.contains("reserved"), "{error}");
}

#[test]
fn standard_op_unknown_name_fails_serde() {
    let toml = zeta_standard_op_manifest().replace(
        "standard_op = \"send_message\"",
        "standard_op = \"send_msg\"",
    );
    let error = parse_v3(&toml).expect_err("an unknown standard_op name must be rejected");
    assert!(error.contains("unknown variant"), "{error}");
}

#[test]
fn standard_op_id_must_match_extension_and_op_name() {
    let toml =
        zeta_standard_op_manifest().replace("id = \"zeta.send_message\"", "id = \"zeta.send\"");
    let error = parse_v3(&toml).expect_err("a mismatched standard_op tool id must be rejected");
    assert!(error.contains("zeta.send_message"), "{error}");
}

#[test]
fn standard_op_rejects_declared_schema_refs() {
    let toml = zeta_standard_op_manifest().replace(
        "description = \"Zeta notes.\"",
        "description = \"Zeta notes.\"\ninput_schema_ref = \"schemas/zeta/custom.input.v1.json\"",
    );
    let error = parse_v3(&toml)
        .expect_err("a standard_op tool declaring its own schema ref must be rejected");
    assert!(error.contains("canonical"), "{error}");

    let toml = zeta_standard_op_manifest().replace(
        "description = \"Zeta notes.\"",
        "description = \"Zeta notes.\"\noutput_schema_ref = \"schemas/zeta/custom.output.v1.json\"",
    );
    let error = parse_v3(&toml)
        .expect_err("a standard_op tool declaring its own output schema ref must be rejected");
    assert!(error.contains("canonical"), "{error}");
}

/// The `standard:` schema-ref namespace is reserved to host-synthesized
/// standard_op bindings. A bespoke tool (no `standard_op`) that hand-writes a
/// `standard:` ref must fail closed — otherwise it wears a canonical schema
/// while skipping every binding validation `standard_op` enforces (and,
/// once later tasks land, canonical output enforcement).
#[test]
fn bespoke_tool_declaring_standard_namespace_ref_is_rejected() {
    let toml = zeta_standard_op_manifest()
        .replace("standard_op = \"send_message\"\n", "")
        .replace(
            "description = \"Zeta notes.\"",
            "description = \"Zeta notes.\"\ninput_schema_ref = \"standard:messaging/send_message.input.v1\"",
        );
    let error = parse_v3(&toml)
        .expect_err("a bespoke tool declaring a standard: namespace ref must be rejected");
    assert!(error.contains("reserved"), "{error}");

    let toml = zeta_standard_op_manifest()
        .replace("standard_op = \"send_message\"\n", "")
        .replace(
            "description = \"Zeta notes.\"",
            "description = \"Zeta notes.\"\ninput_schema_ref = \"schemas/zeta/custom.input.v1.json\"\noutput_schema_ref = \"standard:messaging/send_message.output.v1\"",
        );
    let error = parse_v3(&toml)
        .expect_err("a bespoke tool declaring a standard output namespace ref must be rejected");
    assert!(error.contains("reserved"), "{error}");
}

#[test]
fn standard_op_write_requires_external_write_effect() {
    let toml = zeta_standard_op_manifest().replace(
        "effects = [\"network\", \"use_secret\", \"external_write\"]",
        "effects = [\"network\", \"use_secret\"]",
    );
    let error =
        parse_v3(&toml).expect_err("a write standard_op without external_write must be rejected");
    assert!(error.contains("external_write"), "{error}");
}

#[test]
fn standard_op_duplicate_binding_rejected() {
    let duplicate_tool = r#"
[[tools]]
standard_op = "send_message"
id = "zeta.send_message"
description = "Zeta notes, again."
effects = ["network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"

[[tools.credentials]]
handle = "zeta_user_token"
vendor = "zeta"
scopes = ["chat:write"]
audience = { scheme = "https", host = "api.zeta.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
"#;
    let toml = format!("{}\n{duplicate_tool}", zeta_standard_op_manifest());
    let error = parse_v3(&toml).expect_err("binding the same standard_op twice must be rejected");
    assert!(error.contains("once"), "{error}");
}

#[test]
fn standard_op_allows_empty_description_addendum() {
    let toml =
        zeta_standard_op_manifest().replace("description = \"Zeta notes.\"", "description = \"\"");
    let record =
        parse_v3(&toml).expect("a standard_op tool with an empty description addendum must parse");
    let cap = &record.manifest().capabilities[0];
    assert_eq!(cap.standard_op, Some(StandardMessagingOp::SendMessage));
    assert_eq!(cap.description, "");
}

/// The inverse of the relaxation above: a bespoke (non-`standard_op`) tool's
/// empty-description rejection is unconditional and must still fire. Pins
/// that the relaxation in `CapabilityDeclV2::from_raw` applies only when
/// `standard_op` is bound, not globally.
#[test]
fn bespoke_tool_empty_description_is_still_rejected() {
    let toml = zeta_standard_op_manifest()
        .replace("standard_op = \"send_message\"\n", "")
        .replace(
            "description = \"Zeta notes.\"",
            "description = \"\"\ninput_schema_ref = \"schemas/zeta/send_message.input.v1.json\"",
        );
    let error = parse_v3(&toml)
        .expect_err("a bespoke capability with an empty description must still be rejected");
    assert!(error.contains("description must not be empty"), "{error}");
}

/// The inverse of the standard_op schema-ref rule: `input_schema_ref` is
/// optional on the wire (`RawToolV3`) only so a `standard_op` binding can omit
/// it; a bespoke (non-bound) tool must still declare one.
#[test]
fn tool_without_standard_op_requires_input_schema_ref() {
    let toml = format!(
        r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "zephyrite"
name = "Zephyrite"
version = "0.1.0"
description = "test"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/zephyrite_tool.wasm"

[[tools]]
id = "zephyrite.echo"
description = "Echoes input"
effects = []
default_permission = "ask"
visibility = "model"
"#
    );
    let error =
        parse_v3(&toml).expect_err("a bespoke tool without input_schema_ref must be rejected");
    assert!(
        error.contains("zephyrite.echo") && error.contains("requires input_schema_ref"),
        "{error}"
    );
}

#[test]
fn v2_manifest_declaring_standard_op_is_rejected() {
    let toml = r#"
schema_version = "reborn.extension_manifest.v2"
id = "zeta"
name = "Zeta"
version = "0.1.0"
description = "test"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/zeta_tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "zeta.send_message"
standard_op = "send_message"
description = "Zeta notes."
effects = ["network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zeta/send_message.input.v1.json"
"#;
    let error = parse_v3(toml).expect_err("a v2 manifest declaring standard_op must be rejected");
    assert!(error.contains("v3"), "{error}");
}

// ---------------------------------------------------------------------------
// standard_op descriptor composition (standardized messaging framework, task 3)
// ---------------------------------------------------------------------------

/// Project a parsed record's manifest into the `CapabilityDescriptor` values
/// the host actually builds, via the "package validate path"
/// (`ExtensionPackage::from_manifest`) — this crate's own decl→descriptor
/// helper (`capability_descriptors_from_manifest`) is private to `lib.rs` and
/// not reachable from this external test crate. Mirrors
/// `extension_contract.rs`'s `package_from_manifest` helper.
fn descriptors_from_record(
    record: &ExtensionManifestRecord,
    extension_id: &str,
) -> Vec<CapabilityDescriptor> {
    let manifest: ExtensionManifest = record
        .manifest()
        .clone()
        .try_into()
        .expect("v2 manifest normalizes into ExtensionManifest");
    let root = VirtualPath::new(format!("/system/extensions/{extension_id}")).unwrap();
    ExtensionPackage::from_manifest(manifest, root)
        .expect("package builds from manifest")
        .capabilities
}

#[test]
fn standard_op_descriptor_carries_binding_and_composed_description() {
    let record = parse_v3(&zeta_standard_op_manifest()).expect("parses");
    let descriptors = descriptors_from_record(&record, "zeta");
    let d = descriptors
        .iter()
        .find(|d| d.id.as_str() == "zeta.send_message")
        .unwrap();
    assert_eq!(d.standard_op, Some(StandardMessagingOp::SendMessage));
    let core = StandardMessagingOp::SendMessage
        .contract()
        .unwrap()
        .description_core;
    assert!(d.description.starts_with(core.trim()));
    assert!(d.description.ends_with("Zeta notes."));
}

#[test]
fn standard_op_descriptor_with_empty_addendum_is_core_only() {
    let toml =
        zeta_standard_op_manifest().replace("description = \"Zeta notes.\"", "description = \"\"");
    let record = parse_v3(&toml).expect("a standard_op tool with an empty addendum parses");
    let descriptors = descriptors_from_record(&record, "zeta");
    let d = descriptors
        .iter()
        .find(|d| d.id.as_str() == "zeta.send_message")
        .unwrap();
    let core = StandardMessagingOp::SendMessage
        .contract()
        .unwrap()
        .description_core;
    assert_eq!(d.description, core.trim());
}

/// A bespoke (non-`standard_op`) capability's descriptor description must
/// come through byte-identical to its manifest declaration — composition only
/// engages when `standard_op` is bound.
#[test]
fn bespoke_descriptor_description_is_untouched() {
    let record = acme_record();
    let expected_description = record
        .manifest()
        .capabilities
        .iter()
        .find(|capability| capability.id.as_str() == "acme-messenger.send_note")
        .expect("acme fixture declares send_note")
        .description
        .clone();
    let descriptors = descriptors_from_record(&record, "acme-messenger");
    let d = descriptors
        .iter()
        .find(|d| d.id.as_str() == "acme-messenger.send_note")
        .unwrap();
    assert_eq!(d.standard_op, None);
    assert_eq!(d.description, expected_description);
}

// ---------------------------------------------------------------------------
// Legacy rehydration: resolved records persisted before `standard_op` existed
// ---------------------------------------------------------------------------

/// Remove the `standard_op` key from every tool object in a serialized
/// [`ironclaw_extension_registry::ResolvedExtensionManifest`], simulating a record
/// persisted before the field existed. Operates on the live serialized shape
/// of a *current* record (via `serde_json::Value`) rather than a hand-typed
/// JSON literal, so the fixture cannot silently drift out of sync as the
/// struct evolves.
fn strip_standard_op_from_tools(resolved_json: &mut serde_json::Value) {
    let tools = resolved_json
        .get_mut("tools")
        .and_then(serde_json::Value::as_array_mut)
        .expect("resolved manifest JSON carries a tools array");
    assert!(!tools.is_empty(), "fixture must declare at least one tool");
    for tool in tools {
        let removed = tool
            .as_object_mut()
            .expect("each tool serializes as a JSON object")
            .remove("standard_op");
        assert!(
            removed.is_some(),
            "tool must currently serialize a standard_op key for this pin to be meaningful"
        );
    }
}

/// Spec deliverable: "rehydration of pre-existing resolved records without
/// the field" (standardized messaging framework). `CapabilityDeclV2::standard_op`
/// and `CapabilityDescriptor::standard_op` both carry `#[serde(default)]` so a
/// resolved record or descriptor persisted before the field existed still
/// deserializes, with the field defaulting to `None`. Pinned here against a
/// JSON blob produced by stripping the key from a *current* serialized
/// record/descriptor, not a hand-typed literal, so neither fixture can go
/// stale as either struct's other fields evolve.
///
/// Note on what actually guards this: both fields are spelled literally as
/// `Option<StandardMessagingOp>`, and serde's derive macro treats a missing
/// key on an `Option<_>`-typed field as `None` even without `#[serde(default)]`
/// (verified directly: temporarily removing the attribute from
/// `CapabilityDeclV2::standard_op` left this test green). The attribute is
/// therefore documentation of intent here, not the load-bearing mechanism —
/// this test's real teeth are against a *type-shape* regression (the field
/// stops being a literal `Option<_>`, gets wrapped in a newtype, or the
/// container gains a stricter deserialize contract), which is exactly the
/// class of change most likely to silently break old rows.
#[test]
fn legacy_resolved_record_without_standard_op_rehydrates_to_none() {
    // The zeta fixture (not acme-messenger) deliberately: acme-messenger
    // declares `[runtime] kind = "first_party"`, and `RuntimeKind::FirstParty`
    // carries `#[serde(skip_deserializing)]` (`crates/contracts/ironclaw_host_api/src/runtime.rs`)
    // as an unrelated fail-closed boundary — a descriptor composed from it can
    // never round-trip through raw JSON at all, which would make this pin fail
    // for a reason that has nothing to do with `standard_op`. zeta declares
    // `kind = "wasm"`, so only the field under test varies.
    let record = parse_v3(&zeta_standard_op_manifest()).expect("zeta standard_op fixture parses");

    // --- ResolvedExtensionManifest: every tool loses its binding -----------
    let resolved = record.resolved();
    let bound_before = resolved
        .tools
        .iter()
        .filter(|tool| tool.standard_op.is_some())
        .count();
    assert!(
        bound_before > 0,
        "zeta fixture must declare at least one standard_op-bound tool for this pin to be \
         meaningful"
    );

    let mut legacy_manifest =
        serde_json::to_value(resolved).expect("serialize current resolved manifest");
    strip_standard_op_from_tools(&mut legacy_manifest);
    let rehydrated: ironclaw_extension_registry::ResolvedExtensionManifest =
        serde_json::from_value(legacy_manifest)
            .expect("a resolved manifest without standard_op keys must still deserialize");

    assert_eq!(rehydrated.tools.len(), resolved.tools.len());
    assert!(
        rehydrated
            .tools
            .iter()
            .all(|tool| tool.standard_op.is_none()),
        "every tool rehydrated from a legacy record must default standard_op to None"
    );
    let send_message = rehydrated
        .tools
        .iter()
        .find(|tool| tool.id.as_str() == "zeta.send_message")
        .expect("zeta.send_message survives rehydration");
    assert_eq!(send_message.standard_op, None);

    // --- CapabilityDescriptor: one descriptor loses its binding ------------
    let descriptors = descriptors_from_record(&record, "zeta");
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.id.as_str() == "zeta.send_message")
        .expect("zeta.send_message descriptor is composed");
    assert_eq!(
        descriptor.standard_op,
        Some(StandardMessagingOp::SendMessage),
        "descriptor must currently carry a binding for this pin to be meaningful"
    );

    let mut legacy_descriptor =
        serde_json::to_value(descriptor).expect("serialize current descriptor");
    let removed = legacy_descriptor
        .as_object_mut()
        .expect("descriptor serializes as a JSON object")
        .remove("standard_op");
    assert!(
        removed.is_some(),
        "descriptor must currently serialize a standard_op key for this pin to be meaningful"
    );
    let rehydrated_descriptor: CapabilityDescriptor = serde_json::from_value(legacy_descriptor)
        .expect("a descriptor without a standard_op key must still deserialize");
    assert_eq!(rehydrated_descriptor.standard_op, None);
    assert_eq!(rehydrated_descriptor.id, descriptor.id);
}
