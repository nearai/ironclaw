use ironclaw_product_adapter_registry::{ProductAdapterManifestDocument, RegistryError};
use ironclaw_product_adapters::{AuthRequirement, ProductCapabilityFlag, ProductSurfaceKind};

const TELEGRAM_MANIFEST: &str = include_str!("fixtures/telegram-v2.product-adapter.toml");

#[test]
fn parses_telegram_v2_manifest_fixture() {
    let manifest = ProductAdapterManifestDocument::from_toml(TELEGRAM_MANIFEST)
        .unwrap()
        .into_manifest()
        .unwrap();

    assert_eq!(manifest.adapter_id().as_str(), "telegram-v2");
    assert_eq!(manifest.version().to_string(), "0.1.0");
    assert_eq!(manifest.surface_kind(), ProductSurfaceKind::ExternalChannel);
    assert_eq!(
        manifest.component_ref().as_str(),
        "file://adapters/telegram-v2.wasm"
    );
    assert!(matches!(
        manifest.auth_requirement(),
        AuthRequirement::SharedSecretHeader { header_name }
            if header_name == "X-Telegram-Bot-Api-Secret-Token"
    ));
    assert!(
        manifest
            .capabilities()
            .contains(ProductCapabilityFlag::ExternalFinalReplyPush)
    );
    assert_eq!(manifest.required_credentials().len(), 1);
    assert_eq!(
        manifest.required_credentials()[0].as_str(),
        "telegram_bot_token"
    );
    assert_eq!(manifest.declared_egress().len(), 1);
    assert_eq!(
        manifest.declared_egress()[0].host.as_str(),
        "api.telegram.org"
    );
    assert_eq!(
        manifest.declared_egress()[0]
            .credential_handle
            .as_ref()
            .unwrap()
            .as_str(),
        "telegram_bot_token"
    );
}

#[test]
fn rejects_unknown_manifest_fields() {
    let raw = r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "file://bad.wasm"
env_adapter_list = "telegram-v2"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]
"#;

    let err = ProductAdapterManifestDocument::from_toml(raw).unwrap_err();
    assert!(matches!(err, RegistryError::ManifestParse { .. }));
}

#[test]
fn rejects_inline_secret_material_in_manifest() {
    let raw = r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "file://bad.wasm"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]

[[required_credentials]]
handle = "bot_token"
secret_value = "123456789:AABBccDDeeFFgg"
"#;

    let err = ProductAdapterManifestDocument::from_toml(raw).unwrap_err();
    assert!(matches!(err, RegistryError::InlineSecretMaterial { .. }));
}

#[test]
fn rejects_uppercase_provider_secret_prefixes() {
    for value in ["SK-ABCDEF", "XOXB-ABCDEF", "GHP_ABCDEFG"] {
        let raw = format!(
            r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "{value}"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]
"#
        );
        let err = ProductAdapterManifestDocument::from_toml(&raw).unwrap_err();
        assert!(
            matches!(err, RegistryError::InlineSecretMaterial { .. }),
            "{value} should be rejected",
        );
    }
}

#[test]
fn rejects_secret_like_values_in_allowed_fields() {
    let raw = r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "https://bot:123456789:AABBccDDeeFFgg@example.com/adapter.wasm"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]
"#;

    let err = ProductAdapterManifestDocument::from_toml(raw).unwrap_err();
    assert!(matches!(err, RegistryError::InlineSecretMaterial { .. }));
}

#[test]
fn rejects_egress_credential_not_declared_as_required() {
    let raw = r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "file://bad.wasm"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]

[[egress]]
host = "api.example.com"
credential_handle = "undeclared_token"
"#;

    let err = ProductAdapterManifestDocument::from_toml(raw)
        .unwrap()
        .into_manifest()
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::UndeclaredEgressCredentialHandle { .. }
    ));
}

// ---------------------------------------------------------------------------
// Auth header/cookie syntactic validation (security/correctness review fix 2).
// `header_name`, `timestamp_header_name`, and `SessionCookie.name` reach
// downstream HTTP machinery as raw strings; they must be RFC 7230/6265 tokens
// so a manifest cannot smuggle CRLF or separators into header/cookie
// interpolation.
// ---------------------------------------------------------------------------

fn auth_doc(auth_block: &str) -> String {
    format!(
        r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "telegram-v2"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "file://adapters/telegram-v2.wasm"

{auth_block}

[capabilities]
flags = ["inbound_messages"]
"#
    )
}

#[test]
fn rejects_shared_secret_header_with_crlf_in_name() {
    let raw = auth_doc(
        "[auth]
kind = \"shared_secret_header\"
header_name = \"X-Foo\\r\\nInjected: x\"",
    );
    let err = ProductAdapterManifestDocument::from_toml(&raw)
        .unwrap()
        .into_manifest()
        .unwrap_err();
    assert!(
        matches!(
            err,
            RegistryError::InvalidValue {
                field: "auth.header_name",
                ..
            }
        ),
        "expected InvalidValue auth.header_name, got: {err:?}"
    );
}

#[test]
fn rejects_request_signature_with_whitespace_in_header_name() {
    let raw = auth_doc(
        "[auth]
kind = \"request_signature\"
header_name = \"X Foo\"
timestamp_header_name = \"X-Timestamp\"",
    );
    let err = ProductAdapterManifestDocument::from_toml(&raw)
        .unwrap()
        .into_manifest()
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::InvalidValue {
            field: "auth.header_name",
            ..
        }
    ));
}

#[test]
fn rejects_request_signature_with_invalid_timestamp_header_name() {
    let raw = auth_doc(
        "[auth]
kind = \"request_signature\"
header_name = \"X-Sig\"
timestamp_header_name = \"X-Time;stamp\"",
    );
    let err = ProductAdapterManifestDocument::from_toml(&raw)
        .unwrap()
        .into_manifest()
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::InvalidValue {
            field: "auth.timestamp_header_name",
            ..
        }
    ));
}

#[test]
fn rejects_session_cookie_with_separator_in_name() {
    let raw = auth_doc(
        "[auth]
kind = \"session_cookie\"
name = \"session=evil\"",
    );
    let err = ProductAdapterManifestDocument::from_toml(&raw)
        .unwrap()
        .into_manifest()
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::InvalidValue {
            field: "auth.name",
            ..
        }
    ));
}

#[test]
fn rejects_empty_header_name() {
    let raw = auth_doc(
        "[auth]
kind = \"shared_secret_header\"
header_name = \"\"",
    );
    let err = ProductAdapterManifestDocument::from_toml(&raw)
        .unwrap()
        .into_manifest()
        .unwrap_err();
    assert!(matches!(
        err,
        RegistryError::InvalidValue {
            field: "auth.header_name",
            ..
        }
    ));
}

#[test]
fn accepts_well_formed_token_header_names() {
    let raw = auth_doc(
        "[auth]
kind = \"request_signature\"
header_name = \"X-Telegram-Bot-Api-Secret-Token\"
timestamp_header_name = \"X-Telegram-Bot-Api-Timestamp\"",
    );
    ProductAdapterManifestDocument::from_toml(&raw)
        .unwrap()
        .into_manifest()
        .expect("valid header names must parse");
}

// ---------------------------------------------------------------------------
// Expanded inline-secret denylist (security/correctness review fix 5).
// ---------------------------------------------------------------------------

fn manifest_with_key(key_block: &str) -> String {
    format!(
        r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "file://bad.wasm"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]

[[required_credentials]]
handle = "bot_token"
{key_block}
"#
    )
}

#[test]
fn rejects_secret_like_table_keys() {
    // Each of these key names is independently expected to trip the
    // inline-secret tripwire regardless of the value attached.
    for key in [
        "client_secret",
        "client-secret",
        "private_key",
        "bearer_token",
        "access_token",
        "refresh_token",
        "auth_token",
        "webhook_secret",
        "oauth_token",
        "id_token",
        "apikey",
        "api_secret",
        "passphrase",
    ] {
        let raw = manifest_with_key(&format!("{key} = \"placeholder\""));
        let err = ProductAdapterManifestDocument::from_toml(&raw).unwrap_err();
        assert!(
            matches!(err, RegistryError::InlineSecretMaterial { .. }),
            "key {key} should trip inline-secret guard, got {err:?}"
        );
    }
}

#[test]
fn rejects_value_shaped_credentials_in_component_ref() {
    // `component_ref` is an allowed field; the value-shape guard catches
    // sneaky vendor credentials parked there.
    let value_cases = [
        "gho_abcdefghijklmnop",
        "ghu_abcdefghijklmnop",
        "ghs_abcdefghijklmnop",
        "ghr_abcdefghijklmnop",
        "xoxa-1234567890-abcdef",
        "xoxp-1234567890-abcdef",
        "xoxs-1234567890-abcdef",
        "xoxe-1234567890-abcdef",
        "AKIASOMEFAKEAWSKEYID",
        "ASIASOMEFAKEAWSKEYID",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.signature",
    ];
    for value in value_cases {
        let raw = format!(
            r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "bad-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "{value}"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]
"#
        );
        let err = ProductAdapterManifestDocument::from_toml(&raw).unwrap_err();
        assert!(
            matches!(err, RegistryError::InlineSecretMaterial { .. }),
            "value {value} should trip inline-secret guard, got {err:?}"
        );
    }
}

#[test]
fn accepts_component_ref_with_aws_like_prefix_but_not_key_shape() {
    let raw = r#"
api_version = "ironclaw.product_adapter_manifest/v1"
kind = "ProductAdapterManifest"
adapter_id = "asian-markets-adapter"
version = "0.1.0"
surface_kind = "external_channel"
component_ref = "asian-markets-adapter"

[auth]
kind = "bearer_token"

[capabilities]
flags = ["inbound_messages"]
"#;

    ProductAdapterManifestDocument::from_toml(raw)
        .unwrap()
        .into_manifest()
        .unwrap();
}
