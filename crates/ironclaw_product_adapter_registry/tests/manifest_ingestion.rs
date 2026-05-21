use ironclaw_extensions::{MANIFEST_SCHEMA_VERSION, ManifestSource};
use ironclaw_host_api::HostPortCatalog;
use ironclaw_product_adapter_registry::{
    ExtensionManifestRecord, ManifestHash, RegistryError, parse_product_adapter_manifest_record,
    product_adapter_sections,
};
use ironclaw_product_adapters::{AuthRequirement, ProductCapabilityFlag, ProductSurfaceKind};

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
    let adapter = &adapters[0];
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
        RegistryError::UndeclaredEgressCredentialHandle { .. } | RegistryError::Manifest(_)
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
        RegistryError::InvalidValue {
            field: "auth.header_name",
            ..
        } | RegistryError::Manifest(_)
    ));
}
