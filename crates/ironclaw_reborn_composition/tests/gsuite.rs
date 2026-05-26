use std::sync::{Arc, Mutex};

use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountService,
    CredentialAccountStatus, CredentialOwnership, GOOGLE_GMAIL_SEND_SCOPE,
    InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
};
use ironclaw_extensions::{ExtensionRuntime, ManifestSource};
use ironclaw_first_party_extensions::{
    CALENDAR_LIST_CALENDARS_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID, google_provider_id,
    gsuite_package_specs,
};
use ironclaw_host_api::{
    CapabilityId, InvocationId, ResourceScope, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle,
    TrustClass, UserId,
};
use ironclaw_host_runtime::FirstPartyCapabilityRequest;
use ironclaw_reborn_composition::{
    bundled_gsuite_extension_packages, bundled_gsuite_first_party_handlers,
};
use serde_json::json;

#[derive(Default)]
struct RecordingEgress {
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
}

impl RecordingEgress {
    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().expect("egress lock").clone()
    }
}

impl RuntimeHttpEgress for RecordingEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().expect("egress lock").push(request);
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: br#"{"id":"sent-message"}"#.to_vec(),
            saved_body: None,
            request_bytes: 123,
            response_bytes: 21,
            redaction_applied: true,
        })
    }
}

fn scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap()
}

fn auth_scope(scope: &ResourceScope) -> AuthProductScope {
    AuthProductScope::new(scope.clone(), AuthSurface::Api)
}

fn cap_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn asset_manifest(extension_id: &str) -> ironclaw_extensions::ExtensionManifest {
    let manifest_toml = match extension_id {
        "google-calendar" => {
            include_str!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/manifest.toml"
            )
        }
        "gmail" => include_str!("../../ironclaw_first_party_extensions/assets/gmail/manifest.toml"),
        other => panic!("unknown GSuite asset manifest {other}"),
    };
    ironclaw_extensions::ExtensionManifest::parse(
        manifest_toml,
        ManifestSource::HostBundled,
        &ironclaw_host_api::HostPortCatalog::empty(),
    )
    .unwrap()
}

async fn auth_with_google_account(scope: &ResourceScope) -> Arc<InMemoryAuthProductServices> {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    auth.create_account(NewCredentialAccount {
        scope: auth_scope(scope),
        provider: google_provider_id().unwrap(),
        label: CredentialAccountLabel::new("work google").unwrap(),
        status: CredentialAccountStatus::Configured,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: Some(SecretHandle::new("google-access-token").unwrap()),
        refresh_secret: None,
        scopes: vec![ProviderScope::new(GOOGLE_GMAIL_SEND_SCOPE).unwrap()],
    })
    .await
    .unwrap();
    auth
}

#[test]
fn bundled_gsuite_packages_are_host_bundled_but_not_registered_by_default() {
    let packages = bundled_gsuite_extension_packages().unwrap();

    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].id.as_str(), "google-calendar");
    assert_eq!(packages[1].id.as_str(), "gmail");
    for package in &packages {
        assert_eq!(package.manifest.source, ManifestSource::HostBundled);
        assert!(matches!(
            package.manifest.runtime,
            ExtensionRuntime::FirstParty { .. }
        ));
        assert_eq!(
            package.manifest.descriptor_trust_default,
            TrustClass::Sandbox
        );
    }
    let capability_count = packages
        .iter()
        .map(|package| package.capabilities.len())
        .sum::<usize>();
    assert_eq!(capability_count, 15);
}

#[test]
fn bundled_gsuite_asset_manifests_match_package_specs() {
    for spec in gsuite_package_specs() {
        let manifest = asset_manifest(spec.extension_id);

        assert_eq!(manifest.id.as_str(), spec.extension_id);
        assert!(matches!(
            manifest.runtime,
            ExtensionRuntime::FirstParty { ref service } if service == spec.service
        ));
        let actual = manifest
            .capabilities
            .iter()
            .map(|capability| {
                (
                    capability.id.as_str().to_string(),
                    capability.effects.clone(),
                    capability.default_permission,
                    capability.input_schema_ref.as_str().to_string(),
                    capability.output_schema_ref.as_str().to_string(),
                    capability
                        .prompt_doc_ref
                        .as_ref()
                        .map(|prompt| prompt.as_str().to_string()),
                )
            })
            .collect::<Vec<_>>();
        let expected = spec
            .capabilities
            .iter()
            .map(|capability| {
                (
                    capability.id.to_string(),
                    capability.effects.to_vec(),
                    capability.default_permission,
                    format!(
                        "schemas/{}/{}.input.v1.json",
                        spec.schema_prefix, capability.short_name
                    ),
                    format!(
                        "schemas/{}/{}.output.v1.json",
                        spec.schema_prefix, capability.short_name
                    ),
                    Some(format!(
                        "prompts/{}/{}.md",
                        spec.schema_prefix, capability.short_name
                    )),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn bundled_gsuite_handlers_register_and_forward_runtime_egress() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let registry = bundled_gsuite_first_party_handlers(auth).unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());
    let egress_port: Arc<dyn RuntimeHttpEgress> = egress.clone();
    let handler = registry.get(&capability_id).expect("handler registered");

    let output = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id.clone(),
            scope.clone(),
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(egress_port),
        ))
        .await
        .unwrap()
        .output;

    assert_eq!(output["status"], 200);
    assert!(registry.contains_handler(&cap_id(CALENDAR_LIST_CALENDARS_CAPABILITY_ID)));
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability_id, capability_id);
    assert_eq!(requests[0].scope, scope);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
}

#[tokio::test]
async fn bundled_gsuite_handlers_register_all_gsuite_capabilities() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let registry = bundled_gsuite_first_party_handlers(auth).unwrap();
    let expected_capability_ids = gsuite_package_specs()
        .iter()
        .flat_map(|package| {
            package.capabilities.iter().map(move |capability| {
                format!("{}.{}", package.extension_id, capability.short_name)
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(expected_capability_ids.len(), 15);
    for capability_id in expected_capability_ids {
        assert!(
            registry.contains_handler(&cap_id(&capability_id)),
            "missing handler for {capability_id}"
        );
    }
}

#[tokio::test]
async fn bundled_gsuite_handler_fails_closed_without_runtime_egress() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let registry = bundled_gsuite_first_party_handlers(auth).unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let handler = registry.get(&capability_id).expect("handler registered");

    let error = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            None,
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
}
