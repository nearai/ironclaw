use std::error::Error as _;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    ExtensionId, RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement, UserId,
};
use ironclaw_product_workflow::{
    AccountConnectionStatusError, AccountConnectionStatusSource, ChannelConnectionRequirement,
    ExtensionAccountSetupDescriptor, ExtensionAccountSetupError, ExtensionAccountSetupRegistry,
    RebornChannelConnectStrategy,
};

fn extension_id(value: &str) -> ExtensionId {
    ExtensionId::new(value).expect("valid extension id")
}

fn user_id(value: &str) -> UserId {
    UserId::new(value).expect("valid user id")
}

fn descriptor(extension: &str) -> ExtensionAccountSetupDescriptor {
    let extension_id = extension_id(extension);
    ExtensionAccountSetupDescriptor {
        extension_id: extension_id.clone(),
        auth_requirement: RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new(extension)
                .expect("valid provider id"),
            setup: RuntimeCredentialAccountSetup::Pairing,
            requester_extension: extension_id,
            provider_scopes: Vec::new(),
        },
        connection_requirement: ChannelConnectionRequirement {
            channel: extension.to_string(),
            strategy: RebornChannelConnectStrategy::WebGeneratedCode,
            instructions: "Connect the account.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect".to_string(),
            error_message: "Connection failed.".to_string(),
        },
        activation_success_message: "Account setup is ready.".to_string(),
    }
}

#[derive(Debug)]
struct PerUserStatusSource;

#[async_trait]
impl AccountConnectionStatusSource for PerUserStatusSource {
    async fn connected(&self, user_id: &UserId) -> Result<bool, AccountConnectionStatusError> {
        Ok(user_id.as_str() == "connected-user")
    }
}

#[derive(Debug)]
struct FailingStatusSource;

#[async_trait]
impl AccountConnectionStatusSource for FailingStatusSource {
    async fn connected(&self, _user_id: &UserId) -> Result<bool, AccountConnectionStatusError> {
        Err(AccountConnectionStatusError::new("backend diagnostic"))
    }
}

#[tokio::test]
async fn extension_account_setup_undeclared_extension_needs_no_requirement() {
    let registry = ExtensionAccountSetupRegistry::default();

    let missing = registry
        .missing_requirement(&extension_id("undeclared"), &user_id("caller"))
        .await
        .expect("undeclared extensions are not account-gated");

    assert_eq!(missing, None);
}

#[tokio::test]
async fn extension_account_setup_declared_but_unconnected_host_fails_closed() {
    let registry = ExtensionAccountSetupRegistry::default();
    let extension_id = extension_id("paired-channel");
    assert!(registry.declare(descriptor(extension_id.as_str())));

    let error = registry
        .missing_requirement(&extension_id, &user_id("caller"))
        .await
        .expect_err("a declared setup without its host must fail closed");

    assert_eq!(
        error,
        ExtensionAccountSetupError::HostUnavailable { extension_id }
    );
}

#[tokio::test]
async fn extension_account_setup_returns_requirement_only_for_disconnected_users() {
    let registry = ExtensionAccountSetupRegistry::default();
    let extension_id = extension_id("paired-channel");
    let declared = descriptor(extension_id.as_str());
    let expected_requirement = declared.auth_requirement.clone();
    assert!(registry.declare(declared));
    assert!(registry.connect(&extension_id, Arc::new(PerUserStatusSource)));

    let connected = registry
        .missing_requirement(&extension_id, &user_id("connected-user"))
        .await
        .expect("connected status lookup");
    let disconnected = registry
        .missing_requirement(&extension_id, &user_id("disconnected-user"))
        .await
        .expect("disconnected status lookup");

    assert_eq!(connected, None);
    assert_eq!(disconnected, Some(expected_requirement));
}

#[test]
fn extension_account_setup_declaration_is_immutable_and_unique() {
    let registry = ExtensionAccountSetupRegistry::default();
    let extension_id = extension_id("paired-channel");
    let original = descriptor(extension_id.as_str());
    let mut replacement = original.clone();
    replacement.activation_success_message = "replacement".to_string();

    assert!(registry.declare(original.clone()));
    assert!(!registry.declare(replacement));
    assert_eq!(registry.descriptor(&extension_id), Some(original));
}

#[test]
fn extension_account_setup_connection_is_single_assignment() {
    let registry = ExtensionAccountSetupRegistry::default();
    let declared_extension_id = extension_id("paired-channel");
    assert!(registry.declare(descriptor(declared_extension_id.as_str())));

    assert!(registry.connect(&declared_extension_id, Arc::new(PerUserStatusSource)));
    assert!(!registry.connect(&declared_extension_id, Arc::new(FailingStatusSource)));
    assert!(!registry.connect(&extension_id("undeclared"), Arc::new(PerUserStatusSource)));
}

#[tokio::test]
async fn extension_account_setup_status_outage_is_sanitized() {
    let registry = ExtensionAccountSetupRegistry::default();
    let extension_id = extension_id("paired-channel");
    assert!(registry.declare(descriptor(extension_id.as_str())));
    assert!(registry.connect(&extension_id, Arc::new(FailingStatusSource)));

    let error = registry
        .missing_requirement(&extension_id, &user_id("caller"))
        .await
        .expect_err("status outages must not look disconnected");

    assert!(matches!(
        &error,
        ExtensionAccountSetupError::StatusUnavailable {
            extension_id: actual_extension_id,
            ..
        } if actual_extension_id == &extension_id
    ));
    assert!(!error.to_string().contains("backend diagnostic"));
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("account connection status read failed: backend diagnostic")
    );
}
