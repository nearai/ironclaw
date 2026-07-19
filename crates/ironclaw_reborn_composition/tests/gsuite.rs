use std::sync::{Arc, Mutex};

use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountService,
    CredentialAccountStatus, CredentialOwnership, GOOGLE_GMAIL_SEND_SCOPE,
    InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
};
use ironclaw_extensions::{ExtensionRuntime, ManifestSource};
use ironclaw_first_party_extensions::{
    CALENDAR_LIST_CALENDARS_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GsuiteCredentialStageError, GsuiteCredentialStageRequest, GsuiteCredentialStager,
    google_provider_id, gsuite_package_specs,
};
use ironclaw_host_api::{
    CapabilityId, DispatchFailureDetail, InvocationId, ResourceScope,
    RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource, RuntimeCredentialSource,
    RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, SecretHandle, TrustClass, UserId,
};
use ironclaw_host_runtime::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};
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

#[derive(Default)]
struct RecordingCredentialStager {
    staged: Mutex<Vec<StageRecord>>,
    fail_auth_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StageRecord {
    source_scope: ResourceScope,
    target_scope: ResourceScope,
    access_secret: SecretHandle,
}

impl RecordingCredentialStager {
    fn auth_required() -> Self {
        Self {
            staged: Mutex::new(Vec::new()),
            fail_auth_required: true,
        }
    }

    fn staged(&self) -> Vec<SecretHandle> {
        self.records()
            .into_iter()
            .map(|record| record.access_secret)
            .collect()
    }

    fn records(&self) -> Vec<StageRecord> {
        self.staged.lock().expect("stager lock").clone()
    }
}

#[async_trait::async_trait]
impl GsuiteCredentialStager for RecordingCredentialStager {
    async fn stage(
        &self,
        request: GsuiteCredentialStageRequest<'_>,
    ) -> Result<(), GsuiteCredentialStageError> {
        self.staged.lock().expect("stager lock").push(StageRecord {
            source_scope: request.source_scope.clone(),
            target_scope: request.target_scope.clone(),
            access_secret: request.access_secret.clone(),
        });
        if self.fail_auth_required {
            Err(GsuiteCredentialStageError::AuthRequired)
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingEgress {
    async fn execute(
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

fn asset_schema(path: &str) -> serde_json::Value {
    let schema_json = match path {
        "google-calendar/create_event.input.v1.json" => include_str!(
            "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/create_event.input.v1.json"
        ),
        "google-calendar/list_events.input.v1.json" => include_str!(
            "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_events.input.v1.json"
        ),
        "gmail/list_messages.input.v1.json" => include_str!(
            "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/list_messages.input.v1.json"
        ),
        "gmail/send_message.input.v1.json" => include_str!(
            "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/send_message.input.v1.json"
        ),
        other => panic!("unknown GSuite asset schema {other}"),
    };
    serde_json::from_str(schema_json).unwrap()
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

#[tokio::test]
async fn bundled_gsuite_input_schemas_reject_reviewed_shape_regressions() {
    let create_event = asset_schema("google-calendar/create_event.input.v1.json");
    let create_event_properties = create_event["properties"].as_object().unwrap();
    assert!(create_event_properties.contains_key("calendar_id"));
    assert!(create_event_properties.contains_key("event"));
    assert!(
        !create_event_properties.contains_key("time_min"),
        "create_event schema must not accept list_events query parameters"
    );
    assert!(!create_event_properties.contains_key("time_max"));
    assert!(!create_event_properties.contains_key("page_token"));
    assert!(!create_event_properties.contains_key("max_results"));

    let list_events = asset_schema("google-calendar/list_events.input.v1.json");
    assert_eq!(
        list_events["properties"]["max_results"]["oneOf"][1]["pattern"],
        "^(?:[1-9][0-9]{0,2}|1[0-9]{3}|2[0-4][0-9]{2}|2500)$"
    );
    assert!(list_events["properties"].get("calendar_ids").is_some());
    assert!(
        list_events["properties"]
            .get("include_all_calendars")
            .is_some()
    );
    assert!(list_events["properties"].get("page_tokens").is_some());
    assert_eq!(
        list_events["properties"]["page_tokens"]["propertyNames"]["minLength"],
        json!(1)
    );
    assert!(list_events["properties"].get("query").is_some());
    let list_events_schema_rules = list_events["allOf"]
        .as_array()
        .expect("list_events schema should reject incompatible selector and paging modes");
    assert!(
        list_events_schema_rules
            .iter()
            .any(|rule| { rule["not"]["required"] == json!(["calendar_id", "calendar_ids"]) })
    );
    assert!(list_events_schema_rules.iter().any(|rule| {
        rule["not"]["required"] == json!(["calendar_id", "include_all_calendars"])
            && rule["not"]["properties"]["include_all_calendars"]["const"] == json!(true)
    }));
    assert!(list_events_schema_rules.iter().any(|rule| {
        rule["not"]["required"] == json!(["calendar_ids", "include_all_calendars"])
            && rule["not"]["properties"]["include_all_calendars"]["const"] == json!(true)
    }));
    assert!(
        list_events_schema_rules
            .iter()
            .any(|rule| { rule["not"]["required"] == json!(["page_token", "calendar_ids"]) })
    );
    assert!(list_events_schema_rules.iter().any(|rule| {
        rule["not"]["required"] == json!(["page_token", "include_all_calendars"])
            && rule["not"]["properties"]["include_all_calendars"]["const"] == json!(true)
    }));
    assert!(
        list_events_schema_rules
            .iter()
            .any(|rule| { rule["not"]["required"] == json!(["page_token", "page_tokens"]) })
    );
    assert!(list_events_schema_rules.iter().any(|rule| {
        let Some(any_of) = rule["anyOf"].as_array() else {
            return false;
        };
        any_of
            .iter()
            .any(|branch| branch["not"]["required"] == json!(["page_tokens"]))
            && any_of
                .iter()
                .any(|branch| branch["required"] == json!(["calendar_ids"]))
            && any_of.iter().any(|branch| {
                branch["required"] == json!(["include_all_calendars"])
                    && branch["properties"]["include_all_calendars"]["const"] == json!(true)
            })
    }));

    let list_messages = asset_schema("gmail/list_messages.input.v1.json");
    assert_eq!(
        list_messages["properties"]["max_results"]["oneOf"][1]["pattern"],
        "^(?:[1-9][0-9]{0,1}|[1-4][0-9]{2}|500)$"
    );

    let send_message = asset_schema("gmail/send_message.input.v1.json");
    let send_message_properties = send_message["properties"]["message"]["properties"]
        .as_object()
        .unwrap();
    assert!(send_message_properties.contains_key("raw"));
    assert!(send_message_properties.contains_key("to"));
    assert!(send_message_properties.contains_key("subject"));
    assert!(send_message_properties.contains_key("body"));
    assert!(
        send_message["properties"]["message"]["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|shape| shape["required"] == serde_json::json!(["to", "body"])),
        "send_message schema must advertise structured email input"
    );
}

#[tokio::test]
async fn bundled_gsuite_packages_are_host_bundled_but_not_registered_by_default() {
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

#[tokio::test]
async fn bundled_gsuite_asset_manifests_match_package_specs() {
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
                    capability
                        .runtime_credentials
                        .iter()
                        .map(|credential| {
                            let RuntimeCredentialRequirementSource::ProductAuthAccount {
                                provider,
                                setup:
                                    RuntimeCredentialAccountSetup::OAuth {
                                        scopes: setup_scopes,
                                    },
                            } = &credential.source
                            else {
                                panic!(
                                    "GSuite capability {} must use product-auth OAuth credentials",
                                    capability.id.as_str()
                                );
                            };
                            (
                                credential.handle.as_str().to_string(),
                                provider.as_str().to_string(),
                                setup_scopes.clone(),
                                credential.provider_scopes.clone(),
                                credential.audience.host_pattern.clone(),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let expected = spec
            .capabilities
            .iter()
            .map(|capability| {
                let required_scopes = capability
                    .required_scopes
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect::<Vec<_>>();
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
                    vec![(
                        spec.credential_handle.to_string(),
                        ironclaw_auth::GOOGLE_PROVIDER_ID.to_string(),
                        required_scopes.clone(),
                        required_scopes,
                        spec.credential_host_pattern.to_string(),
                    )],
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
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        Arc::new(RecordingCredentialStager::default()),
        true,
    )
    .unwrap();
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
async fn bundled_gsuite_handlers_stage_selected_account_secret_before_egress() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let stager = Arc::new(RecordingCredentialStager::default());
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        stager.clone() as Arc<dyn GsuiteCredentialStager>,
        true,
    )
    .unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());
    let egress_port: Arc<dyn RuntimeHttpEgress> = egress.clone();
    let handler = registry.get(&capability_id).expect("handler registered");

    handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(egress_port),
        ))
        .await
        .unwrap();

    assert_eq!(
        stager.staged(),
        vec![SecretHandle::new("google-access-token").unwrap()]
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert!(matches!(
        requests[0].credential_injections[0].source,
        RuntimeCredentialSource::StagedObligation { ref capability_id }
            if capability_id.as_str() == GMAIL_SEND_MESSAGE_CAPABILITY_ID
    ));
}

#[tokio::test]
async fn bundled_gsuite_handlers_stage_oauth_account_secret_from_account_scope() {
    let mut account_scope = scope();
    account_scope.invocation_id = InvocationId::new();
    let mut runtime_scope = account_scope.clone();
    runtime_scope.invocation_id = InvocationId::new();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    auth.create_account(NewCredentialAccount {
        scope: AuthProductScope::new(account_scope.clone(), AuthSurface::Callback),
        provider: google_provider_id().unwrap(),
        label: CredentialAccountLabel::new("work google").unwrap(),
        status: CredentialAccountStatus::Configured,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: Some(SecretHandle::new("google-oauth-access-token").unwrap()),
        refresh_secret: None,
        scopes: vec![ProviderScope::new(GOOGLE_GMAIL_SEND_SCOPE).unwrap()],
    })
    .await
    .unwrap();
    let stager = Arc::new(RecordingCredentialStager::default());
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        stager.clone() as Arc<dyn GsuiteCredentialStager>,
        true,
    )
    .unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());
    let egress_port: Arc<dyn RuntimeHttpEgress> = egress.clone();
    let handler = registry.get(&capability_id).expect("handler registered");

    handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            runtime_scope.clone(),
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(egress_port),
        ))
        .await
        .unwrap();

    assert_eq!(
        stager.records(),
        vec![StageRecord {
            source_scope: account_scope,
            target_scope: runtime_scope,
            access_secret: SecretHandle::new("google-oauth-access-token").unwrap(),
        }]
    );
}

#[tokio::test]
async fn bundled_gsuite_handlers_project_staging_auth_failures_as_auth_required() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let stager = Arc::new(RecordingCredentialStager::auth_required());
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        stager as Arc<dyn GsuiteCredentialStager>,
        true,
    )
    .unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());
    let egress_port: Arc<dyn RuntimeHttpEgress> = egress.clone();
    let handler = registry.get(&capability_id).expect("handler registered");

    let error = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(egress_port),
        ))
        .await
        .unwrap_err();

    assert!(
        error.is_auth_required(),
        "staging auth failure should be auth required"
    );
    assert_eq!(
        error
            .required_secrets()
            .expect("staging auth failure should surface required secrets"),
        &vec![SecretHandle::new("google-access-token").unwrap()]
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn bundled_gsuite_handlers_register_all_gsuite_capabilities() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        Arc::new(RecordingCredentialStager::default()),
        true,
    )
    .unwrap();
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
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        Arc::new(RecordingCredentialStager::default()),
        true,
    )
    .unwrap();
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

    assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::NetworkDenied));
}

// ---------------------------------------------------------------------------
// T5: gsuite_error integration — tests that staging errors are projected
// correctly into FirstPartyCapabilityError through handler dispatch.
// ---------------------------------------------------------------------------

struct BackendStager;

#[async_trait::async_trait]
impl GsuiteCredentialStager for BackendStager {
    async fn stage(
        &self,
        _request: GsuiteCredentialStageRequest<'_>,
    ) -> Result<(), GsuiteCredentialStageError> {
        Err(GsuiteCredentialStageError::Backend)
    }
}

#[tokio::test]
async fn bundled_gsuite_handler_projects_stage_auth_required_to_first_party_auth_required() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        Arc::new(RecordingCredentialStager::auth_required()),
        true,
    )
    .unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let handler = registry.get(&capability_id).expect("handler registered");

    let error = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(Arc::new(RecordingEgress::default()) as Arc<dyn RuntimeHttpEgress>),
        ))
        .await
        .unwrap_err();

    assert!(
        error.is_auth_required(),
        "stage AuthRequired must project to FirstPartyCapabilityError::AuthRequired; got {error:?}"
    );
    assert_eq!(
        error.kind(),
        None,
        "AuthRequired variant must have no dispatch kind"
    );
    let credential_requirements = error
        .credential_requirements()
        .expect("stage AuthRequired should surface OAuth requirements");
    assert_eq!(credential_requirements.len(), 1);
    let requirement = &credential_requirements[0];
    assert_eq!(
        requirement.provider.as_str(),
        ironclaw_auth::GOOGLE_PROVIDER_ID
    );
    assert_eq!(requirement.requester_extension.as_str(), "gmail");
    assert_eq!(
        requirement.provider_scopes,
        vec![GOOGLE_GMAIL_SEND_SCOPE.to_string()]
    );
}

#[tokio::test]
async fn bundled_gsuite_handler_projects_stage_backend_to_first_party_dispatch_backend() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let registry =
        bundled_gsuite_first_party_handlers(auth.clone(), auth, Arc::new(BackendStager), true)
            .unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let handler = registry.get(&capability_id).expect("handler registered");

    let error = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(Arc::new(RecordingEgress::default()) as Arc<dyn RuntimeHttpEgress>),
        ))
        .await
        .unwrap_err();

    assert!(
        !error.is_auth_required(),
        "stage Backend must NOT project to AuthRequired; got {error:?}"
    );
    assert_eq!(
        error.kind(),
        Some(RuntimeDispatchErrorKind::Backend),
        "stage Backend must produce Dispatch {{ kind: Backend }}"
    );
    // `BackendAuth`-reasoned dispatch errors (Google account resolved, but
    // the provider itself rejected the request) must read as "configured but
    // rejected" — distinct from both `AuthRequired` (asserted above) and the
    // not-configured pre-dispatch tool result asserted below, which has no
    // account to resolve in the first place.
    let FirstPartyCapabilityError::Dispatch { detail, .. } = error else {
        panic!("expected Dispatch variant");
    };
    let detail = detail.expect("BackendAuth dispatch error must carry diagnostic detail");
    let DispatchFailureDetail::Diagnostic { text } = *detail else {
        panic!("expected Diagnostic detail");
    };
    assert!(text.contains("rejected"), "text: {text}");
    assert!(
        text.contains("config set google.client_secret"),
        "text: {text}"
    );
}

#[tokio::test]
async fn bundled_gsuite_handler_returns_not_configured_tool_result_when_no_google_oauth_backend() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let registry = bundled_gsuite_first_party_handlers(
        auth.clone(),
        auth,
        Arc::new(RecordingCredentialStager::default()),
        false,
    )
    .unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let handler = registry.get(&capability_id).expect("handler registered");

    let error = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            // No egress supplied: the not-configured check must short-circuit
            // before dispatch ever requires runtime HTTP egress.
            None,
        ))
        .await
        .unwrap_err();

    assert!(
        !error.is_auth_required(),
        "not-configured must be a tool-result error, not the AuthRequired gate; got {error:?}"
    );
    assert_eq!(
        error.kind(),
        Some(RuntimeDispatchErrorKind::OperationFailed)
    );
    let FirstPartyCapabilityError::Dispatch { detail, .. } = error else {
        panic!("expected Dispatch variant");
    };
    let detail = detail.expect("not-configured error must carry diagnostic detail");
    let DispatchFailureDetail::Diagnostic { text } = *detail else {
        panic!("expected Diagnostic detail");
    };
    assert!(text.contains("not configured"), "text: {text}");
    assert!(text.contains("config set google.client_id"), "text: {text}");
    assert!(
        text.contains("config set google.client_secret"),
        "text: {text}"
    );
    assert!(
        text.contains("config set google.redirect_uri"),
        "text: {text}"
    );
    assert!(text.contains("console.cloud.google.com"), "text: {text}");
    assert!(text.contains("ironclaw service restart"), "text: {text}");
    assert_eq!(
        text.matches("service restart").count(),
        1,
        "the restart instruction must appear exactly once (the remediation text must not \
         embed its own restart step on top of the appended apply_step_text): {text}"
    );
    assert!(!text.contains("restarts automatically"), "text: {text}");
}
