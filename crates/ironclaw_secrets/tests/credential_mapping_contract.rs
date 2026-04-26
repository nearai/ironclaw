use ironclaw_host_api::SecretHandle;
use ironclaw_secrets::{CredentialLocation, CredentialMapping};

#[test]
fn credential_mapping_bearer_constructor_carries_metadata_only() {
    let mapping =
        CredentialMapping::bearer(SecretHandle::new("github_token").unwrap(), "api.github.com");

    assert_eq!(mapping.handle.as_str(), "github_token");
    assert_eq!(mapping.host_patterns, vec!["api.github.com".to_string()]);
    assert!(matches!(
        mapping.location,
        CredentialLocation::AuthorizationBearer
    ));
    assert!(!format!("{mapping:?}").contains("ghp_"));
}

#[test]
fn credential_mapping_header_constructor_carries_no_material() {
    let mapping = CredentialMapping::header(
        SecretHandle::new("api_key").unwrap(),
        "X-API-Key",
        "*.example.test",
    );

    assert_eq!(mapping.handle.as_str(), "api_key");
    assert_eq!(mapping.host_patterns, vec!["*.example.test".to_string()]);
    assert!(matches!(
        mapping.location,
        CredentialLocation::Header { ref name, prefix: None } if name == "X-API-Key"
    ));
}
