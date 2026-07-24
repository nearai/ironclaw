//! Extension Manifest v3 contract tests (extension-runtime P1, workstream A).
//!
//! v3 is v2 plus explicit `[channel]` and `[auth.*]` sections and an `[mcp]`
//! declaration for proxied servers. Both schemas parse through the single
//! `ExtensionManifestRecord::from_toml` entry point and normalize into the
//! same [`ResolvedExtensionManifest`] (checklist MAN-2).

use std::sync::Arc;

use ironclaw_extensions::{
    CapabilityProviderHostApiContract, CapabilitySurfaceDeclV2, CapabilityVisibility,
    ExtensionManifestRecord, ExtensionRuntimeV2, HostApiContractRegistry,
    MANIFEST_SCHEMA_VERSION_V3, ManifestSource,
};
use ironclaw_host_api::{
    CapabilitySurfaceKind, ConversationModel, EffectKind, HOST_RUNTIME_HTTP_EGRESS_PORT_ID,
    HostPortCatalog, HostPortCatalogEntry, HostPortId, PermissionMode,
    RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource, VendorAuthRecipe,
};

const ACME_MANIFEST: &str =
    include_str!("../../../tests/fixtures/extensions/acme-messenger/manifest.toml");

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
    ExtensionManifestRecord::from_toml(toml, source, &catalog(), None, &contracts())
        .map_err(|error| error.to_string())
}

fn acme_record() -> ExtensionManifestRecord {
    parse_v3(ACME_MANIFEST).expect("acme fixture manifest must parse")
}

// ---------------------------------------------------------------------------
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

    // One declared tool, normalized into the internal capability model.
    assert_eq!(manifest.capabilities.len(), 1);
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
}

#[test]
fn acme_fixture_resolves_channel_and_auth_recipe() {
    let record = acme_record();
    let resolved = record.resolved();

    let channel = resolved.channel.as_ref().expect("channel declared");
    assert_eq!(channel.id, "messages");
    assert_eq!(channel.conversation_model, ConversationModel::Continuous);
    let ingress = channel.ingress.as_ref().expect("ingress declared");
    assert_eq!(ingress.route_suffix.as_str(), "events");

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

    // The channel surface participates in the derived surface set.
    let kinds: Vec<CapabilitySurfaceKind> = record
        .manifest()
        .capability_surfaces()
        .iter()
        .map(CapabilitySurfaceDeclV2::kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            CapabilitySurfaceKind::Tool,
            CapabilitySurfaceKind::Channel,
            CapabilitySurfaceKind::Auth,
        ]
    );
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
}

#[test]
fn duplicate_admin_configuration_handles_fail_closed() {
    let toml = ACME_MANIFEST.replace(
        r#"fields = [
  { handle = "acme_bot_token", label = "Bot token", secret = true, required = true },
  { handle = "acme_signing_secret", label = "Signing secret", secret = true, required = true },
]"#,
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
    let toml = ACME_MANIFEST.replace(
        r#"fields = [
  { handle = "acme_bot_token", label = "Bot token", secret = true, required = true },
  { handle = "acme_signing_secret", label = "Signing secret", secret = true, required = true },
]"#,
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
         supports_threads = false\n\
         max_message_chars = 4000\n",
        "[channel.presentation]\n\
         supports_markdown = true\n\
         supports_threads = false\n\
         max_message_chars = 4000\n\n\
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
  { handle = "zephyrite_token", source = { type = "product_auth_account", provider = "zephyrite", setup = { kind = "oauth", scopes = ["echo:read"] } }, provider_scopes = ["echo:read"], audience = { scheme = "https", host_pattern = "api.zephyrite.example" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
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
    // audience, injection.
    assert_eq!(a.runtime_credentials, b.runtime_credentials);

    // Same derived surface kinds (tool + auth).
    let kinds = |manifest: &ironclaw_extensions::ExtensionManifestV2| {
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
        Some(ironclaw_host_api::NetworkScheme::Https)
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
    let back: ironclaw_extensions::ResolvedExtensionManifest =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(&back, record.resolved());
}
