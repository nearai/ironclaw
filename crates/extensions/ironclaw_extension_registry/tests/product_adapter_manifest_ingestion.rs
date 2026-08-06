use ironclaw_extension_contracts::product_adapter_section::ProductAdapterSectionError;
use ironclaw_extension_contracts::surface::CapabilitySurfaceKind;
use ironclaw_extension_registry::host_api::product_adapter::{
    RegistryError, parse_product_adapter_manifest_record, product_adapter_sections,
};
use ironclaw_extension_registry::{
    CapabilitySurfaceDeclV2, ExtensionManifestRecord, MANIFEST_SCHEMA_VERSION, ManifestHash,
    ManifestSource,
};
use ironclaw_host_api::host_port::HostPortCatalog;
use ironclaw_host_api::product_adapter::{
    AuthRequirement, ProductCapabilityFlag, ProductSurfaceKind,
};

fn manifest(extra: &str) -> String {
    format!(
        r#"
schema_version = "{schema}"
id = "telegram-v2"
name = "Telegram"
version = "0.1.0"
description = "Telegram product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/telegram-v2.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "shared_secret_header"
header_name = "X-Telegram-Bot-Api-Secret-Token"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages", "external_final_reply_push"]

[[product_adapter.inbound.required_credentials]]
handle = "telegram_bot_token"

[[product_adapter.inbound.egress]]
host = "api.telegram.org"
credential_handle = "telegram_bot_token"

{extra}
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    )
}

fn parse(raw: &str) -> Result<ExtensionManifestRecord, RegistryError> {
    parse_product_adapter_manifest_record(
        raw,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        Some(ManifestHash::new("sha256:abc123").unwrap()),
    )
}

#[test]
fn parses_product_adapter_host_api_section_from_extension_manifest_v2() {
    let record = parse(&manifest("")).unwrap();

    assert_eq!(record.extension_id().as_str(), "telegram-v2");
    let adapters = product_adapter_sections(&record).unwrap();
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].section().as_str(), "product_adapter.inbound");
    let adapter = adapters[0].resolved();
    assert_eq!(adapter.adapter_id().as_str(), "telegram-v2/inbound");
    assert_eq!(adapter.surface_kind(), ProductSurfaceKind::ExternalChannel);
    assert!(matches!(
        adapter.auth_requirement(),
        AuthRequirement::SharedSecretHeader { header_name }
            if header_name == "X-Telegram-Bot-Api-Secret-Token"
    ));
    assert!(
        adapter
            .capabilities()
            .contains(ProductCapabilityFlag::InboundMessages)
    );
    assert_eq!(
        adapter.required_credentials()[0].as_str(),
        "telegram_bot_token"
    );
    assert_eq!(
        adapter.declared_egress()[0].host.as_str(),
        "api.telegram.org"
    );
}

#[test]
fn rejects_unreferenced_product_adapter_section() {
    let raw = manifest(
        r#"
[product_adapter.stale]
surface_kind = "external_channel"
"#,
    );

    let err = parse(&raw).unwrap_err();
    assert!(matches!(err, RegistryError::Manifest(_)));
}

#[test]
fn rejects_inline_secret_material_in_product_adapter_section() {
    let raw = manifest(
        r#"
[[product_adapter.inbound.required_credentials]]
handle = "other_token"
secret_value = "123456789:AABBccDDeeFFgg"
"#,
    );

    let err = parse(&raw).unwrap_err();
    assert!(matches!(
        err,
        RegistryError::InlineSecretMaterial { .. } | RegistryError::Manifest(_)
    ));
}

/// The inline-secret guard's vendor token prefixes are a safety **denylist**,
/// deliberately a **superset** of the bundled package inventory — so it cannot
/// be sourced from the packages, which is what CHECKLIST WS5's `product` narrows
/// row asked for under "slack/telegram token heuristics → the packages that own
/// them". This test is the mechanical half of that refutation: it drives the
/// real ingestion path with shapes belonging to **no** installed package, and
/// each must still be rejected. Delete a prefix to "de-hardcode the vendor
/// names" and this goes red rather than the guard quietly narrowing.
///
/// **It also closes a real coverage hole.** The test above it writes its secret
/// under the key `secret_value`, which `is_secret_key_name` rejects on the *key*
/// alone — so before this test the entire value-shape half of the guard, every
/// prefix in it, had no coverage at all. Every case here uses `handle`, a key
/// the name rule does not match, so only `looks_like_inline_secret` can reject.
///
/// The sibling disposition, same shape and same reason, is
/// `tool_payload_redaction_profile_is_a_safety_denylist_not_inventory_routing`
/// in `ironclaw_trace_commons`. The `PATH_TERM_COLLISIONS` carve-out in
/// `crates/ironclaw_architecture_tests/tests/reborn_extension_specificity.rs`
/// records why the names stay here.
#[test]
fn inline_secret_guard_is_a_safety_denylist_not_package_inventory() {
    for (label, secret) in [
        // Three shapes DO belong to bundled packages…
        ("slack bot token", "xoxb-1111-2222-abcdefghijklmnop"),
        (
            "telegram bot token",
            "123456789:AABBccDDeeFFggHHiiJJkkLLmmNNooPP",
        ),
        ("github pat", "ghp_0123456789abcdefghijklmnopqrstuvwxyz"),
        // …and these belong to NONE. Sourcing the set from the package
        // inventory would drop every row below and stop rejecting them.
        (
            "openai/anthropic api key",
            "sk-0123456789abcdefghijklmnopqrst",
        ),
        ("aws access key id", "AKIAIOSFODNN7EXAMPLE"),
        (
            "json web token",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln",
        ),
        ("pem private key", "-----BEGIN PRIVATE KEY-----"),
        ("uri userinfo", "https://user:hunter2@example.com/hook"),
    ] {
        let raw = manifest(&format!(
            r#"
[[product_adapter.inbound.required_credentials]]
handle = "{secret}"
"#
        ));

        // The guard's rejection reaches the caller wrapped by the manifest
        // parser (`Manifest(HostApiSectionRejected { reason })`), so match on
        // the rendered reason and on the field it names. Matching the outer
        // variant alone would accept a schema rejection and mask a deleted
        // prefix — which is the exact regression this test exists to catch.
        let error = parse(&raw).err().unwrap_or_else(|| {
            panic!(
                "{label}: accepted inline secret material. The guard is a safety denylist \
                 and must stay a SUPERSET of the bundled package inventory; if this shape \
                 was dropped to \"de-hardcode the vendor names\", restore it."
            )
        });
        let rendered = error.to_string();
        assert!(
            rendered.contains("inline secret material is not allowed in manifest field")
                && rendered.contains("required_credentials[1].handle"),
            "{label}: the manifest was rejected, but not by the inline-secret guard on the \
             `handle` value ({rendered}). A schema or key-name rejection would mask a \
             deleted prefix — this case must fail on `looks_like_inline_secret`."
        );
    }

    // Negative control: a plain handle must still parse, so the cases above are
    // proving the guard fires rather than proving everything fails.
    parse(&manifest(
        r#"
[[product_adapter.inbound.required_credentials]]
handle = "other_token"
"#,
    ))
    .expect("a non-secret-looking handle must still parse");
}

#[test]
fn rejects_egress_credential_not_declared_as_required() {
    let raw = manifest(
        r#"
[[product_adapter.inbound.egress]]
host = "api.example.com"
credential_handle = "undeclared_token"
"#,
    );

    let err = parse(&raw).unwrap_err();
    assert!(matches!(
        err,
        RegistryError::Section(ProductAdapterSectionError::UndeclaredEgressCredentialHandle { .. })
            | RegistryError::Manifest(_)
    ));
}

#[test]
fn rejects_auth_header_injection_shape() {
    let raw = manifest("").replace(
        "header_name = \"X-Telegram-Bot-Api-Secret-Token\"",
        "header_name = \"X-Foo\\r\\nInjected: x\"",
    );

    let err = parse(&raw).unwrap_err();
    assert!(matches!(
        err,
        RegistryError::Section(ProductAdapterSectionError::InvalidValue {
            field: "auth.header_name",
            ..
        }) | RegistryError::Manifest(_)
    ));
}

/// A valid public-webhook host-ingress route declaration in manifest wire form.
/// `{cred}` lets a test swap the verifying credential handle; `{scheme}` lets a
/// test swap the auth scheme to exercise host_api's fail-closed floor.
fn host_ingress_fragment(cred: &str, scheme: &str) -> String {
    format!(
        r#"
[[product_adapter.inbound.host_ingress]]
credential_handles = ["{cred}"]
descriptor = {{ route_id = "telegram.updates", method = "post", route_pattern = "/webhooks/telegram/updates", policy = {{ listener_class = "public_webhook", auth = {{ type = "required", schemes = ["{scheme}"] }}, scope_source = "host_resolved", body_limit = {{ type = "limited", max_bytes = 262144 }}, rate_limit = {{ type = "limited", scope = "global", max_requests = 600, window_seconds = 60 }}, cors = "not_applicable", websocket_origin = "not_applicable", streaming = "none", audit = "public_callback", effect_path = {{ type = "product_surface" }} }} }}
"#
    )
}

#[test]
fn parses_host_ingress_route_from_manifest() {
    let record = parse(&manifest(&host_ingress_fragment(
        "telegram_bot_token",
        "webhook_signature",
    )))
    .unwrap();
    let adapters = product_adapter_sections(&record).unwrap();
    let routes = adapters[0].resolved().host_ingress();
    assert_eq!(routes.len(), 1);
    assert_eq!(
        routes[0].descriptor().route_id().as_str(),
        "telegram.updates"
    );
    assert_eq!(
        routes[0].descriptor().route_pattern().as_str(),
        "/webhooks/telegram/updates"
    );
    assert_eq!(
        routes[0].credential_handles()[0].as_str(),
        "telegram_bot_token"
    );
}

#[test]
fn rejects_host_ingress_public_webhook_without_webhook_signature() {
    // host_api's own fail-closed floor: a `public_webhook` listener MUST
    // require `webhook_signature`. Declaring `bearer_token` instead must be
    // rejected while projecting the manifest — the manifest layer cannot
    // weaken the descriptor's built-in verification requirement.
    let raw = manifest(&host_ingress_fragment("telegram_bot_token", "bearer_token"));
    let err = parse(&raw).unwrap_err();
    // The invalid descriptor is rejected while deserializing the section, which
    // may surface either as a stringified `Manifest` error (via the host-api
    // contract validator) or as a typed `ManifestSectionParse` (via the final
    // projection) depending on which projection runs first — accept both so the
    // test pins the fail-closed behavior, not the error-routing path.
    assert!(
        matches!(
            err,
            RegistryError::Manifest(_) | RegistryError::ManifestSectionParse { .. }
        ),
        "expected the host_api listener/auth invariant to reject, got {err:?}"
    );
}

#[test]
fn rejects_host_ingress_credential_handle_not_declared_as_required() {
    // Ingress credential coherence over the wire: a route may only be verified
    // by a credential the section declares in `required_credentials`.
    let raw = manifest(&host_ingress_fragment(
        "undeclared_token",
        "webhook_signature",
    ));
    let err = parse(&raw).unwrap_err();
    assert!(
        matches!(
            err,
            RegistryError::Section(
                ProductAdapterSectionError::UndeclaredIngressCredentialHandle { .. }
            ) | RegistryError::Manifest(_)
        ),
        "got {err:?}"
    );
}

#[test]
fn rejects_real_derived_adapter_id_that_exceeds_limit() {
    let extension_id = "a".repeat(128);
    let subsection = "b".repeat(128);
    let section = format!("product_adapter.{subsection}");
    let raw = format!(
        r#"
schema_version = "{schema}"
id = "{extension_id}"
name = "Long ProductAdapter"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/long.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "{section}"

[{section}]
surface_kind = "external_channel"

[{section}.auth]
kind = "bearer_token"

[{section}.capabilities]
flags = ["inbound_messages"]
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );

    let err = parse(&raw).unwrap_err();
    assert!(
        err.to_string().contains("invalid adapter_id"),
        "expected adapter_id validation error, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Capability-surface projection through the real product-adapter contract
// ---------------------------------------------------------------------------

#[test]
fn external_channel_section_projects_a_channel_capability_surface() {
    let record = parse(&manifest("")).unwrap();
    let surfaces = record.manifest().capability_surfaces();
    let channels: Vec<_> = surfaces
        .iter()
        .filter(|surface| surface.kind() == CapabilitySurfaceKind::Channel)
        .collect();
    assert_eq!(channels.len(), 1, "{surfaces:?}");
    match channels[0] {
        CapabilitySurfaceDeclV2::HostApiSection {
            kind,
            host_api,
            section,
        } => {
            assert_eq!(*kind, CapabilitySurfaceKind::Channel);
            assert_eq!(host_api.as_str(), "ironclaw.product_adapter/v1");
            assert_eq!(section.as_str(), "product_adapter.inbound");
        }
        other => panic!("expected a host-API channel surface, got {other:?}"),
    }
    // An adapter-only manifest declares no tools: the wasm runtime kind must
    // not leak a tool (or any other) surface into the product taxonomy.
    assert!(
        surfaces
            .iter()
            .all(|surface| surface.kind() == CapabilitySurfaceKind::Channel),
        "{surfaces:?}"
    );
}

#[test]
fn host_native_product_surface_kinds_project_no_channel_surface() {
    // `web` / `cli` / `synchronous_api` product-adapter sections describe
    // host-native surfaces; they are not extension channel surfaces.
    let toml = manifest("").replace(
        r#"surface_kind = "external_channel""#,
        r#"surface_kind = "web""#,
    );
    let record = parse(&toml).unwrap();
    let surfaces = record.manifest().capability_surfaces();
    assert!(
        surfaces
            .iter()
            .all(|surface| surface.kind() != CapabilitySurfaceKind::Channel),
        "{surfaces:?}"
    );
}
