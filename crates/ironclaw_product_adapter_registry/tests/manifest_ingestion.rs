use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionManifestRecord, ExtensionManifestV2, HostApiContractRegistry, HostApiId,
    HostApiManifestContract, HostApiRefV2, MANIFEST_SCHEMA_VERSION, ManifestSectionPath,
    ManifestSource,
};
use ironclaw_host_api::HostPortCatalog;
use ironclaw_product_adapter_registry::{
    ManifestHash, ProductAdapterHostApiContract, RegistryError,
    parse_product_adapter_manifest_record, product_adapter_sections,
};
use ironclaw_product_adapters::{AuthRequirement, ProductCapabilityFlag, ProductSurfaceKind};

const TELEGRAM_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/telegram/manifest.toml");
const SLACK_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/slack/manifest.toml");

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

#[test]
fn real_first_party_product_adapter_manifests_project_cleanly() {
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(Arc::new(ProductAdapterHostApiContract::new().unwrap()))
        .unwrap();
    contracts
        .register(Arc::new(FakeHostIngressContract {
            id: HostApiId::new("ironclaw.host_ingress/v1").unwrap(),
        }))
        .unwrap();

    for raw in [TELEGRAM_MANIFEST, SLACK_MANIFEST] {
        ExtensionManifestV2::parse_with_host_api_contracts(
            raw,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .unwrap();
        assert_eq!(product_adapter_sections_from_manifest(raw, &contracts), 1);
    }
}

fn product_adapter_sections_from_manifest(raw: &str, contracts: &HostApiContractRegistry) -> usize {
    let record = ExtensionManifestRecord::from_toml_with_contracts(
        raw,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        Some(ManifestHash::new("sha256:real-manifest-test").unwrap()),
        contracts,
    )
    .unwrap();
    product_adapter_sections(&record).unwrap().len()
}

struct FakeHostIngressContract {
    id: HostApiId,
}

impl HostApiManifestContract for FakeHostIngressContract {
    fn id(&self) -> &HostApiId {
        &self.id
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool {
        section
            .as_str()
            .strip_prefix("host_ingress")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }

    fn validate_section(
        &self,
        _host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), String> {
        // Post-transport-discriminator shape: route_id lives under
        // [host_ingress.*.transport], not at the section top level.
        section
            .as_table()
            .and_then(|table| table.get("transport"))
            .and_then(toml::Value::as_table)
            .and_then(|transport| transport.get("route_id"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "host_ingress transport.route_id is required".to_string())?;
        Ok(())
    }
}
