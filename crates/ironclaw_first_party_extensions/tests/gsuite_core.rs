mod support;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, CredentialAccount,
    CredentialAccountChoiceRequest, CredentialAccountLabel, CredentialAccountListPage,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountOwnerScope,
    CredentialAccountProjection, CredentialAccountSelectionRequest, CredentialAccountStatus,
    CredentialOwnership, CredentialRecoveryProjection, CredentialRecoveryRequest,
    CredentialRefreshReport, CredentialRefreshRequest, GOOGLE_CALENDAR_READONLY_SCOPE,
    GOOGLE_GMAIL_MODIFY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
    InMemoryAuthProductServices, NewCredentialAccount,
};
use ironclaw_first_party_extensions::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
    CALENDAR_GET_EVENT_CAPABILITY_ID, CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
    CALENDAR_LIST_EVENTS_CAPABILITY_ID, CALENDAR_SET_REMINDER_CAPABILITY_ID,
    CALENDAR_UPDATE_EVENT_CAPABILITY_ID, GMAIL_CREATE_DRAFT_CAPABILITY_ID,
    GMAIL_GET_MESSAGE_CAPABILITY_ID, GMAIL_LIST_MESSAGES_CAPABILITY_ID,
    GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GMAIL_TRASH_MESSAGE_CAPABILITY_ID, GSUITE_OUTPUT_BYTES_LIMIT, GSUITE_PROVIDER_SCOPES,
    GSUITE_REQUEST_BODY_LIMIT, GSUITE_RESPONSE_BODY_LIMIT, GsuiteCredentialDispatchReason,
    GsuiteDispatchRequest, GsuiteExecutor, google_provider_id, gsuite_package_specs,
    gsuite_resource_profile,
};
use ironclaw_host_api::{
    ExtensionId, NetworkMethod, NetworkScheme, RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED,
    RuntimeDispatchErrorKind, RuntimeHttpEgressError, SecretHandle,
};
use serde_json::json;
use support::*;

#[test]
fn gsuite_packages_declare_calendar_and_gmail_capabilities() {
    let packages = gsuite_package_specs();
    let ids = packages
        .iter()
        .map(|package| package.extension_id.to_string())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["google-calendar", "gmail"]);
    let capability_count = packages
        .iter()
        .map(|package| package.capabilities.len())
        .sum::<usize>();
    assert_eq!(capability_count, 15);
}

#[test]
fn google_provider_id_returns_valid_provider() {
    assert_eq!(google_provider_id().unwrap().as_str(), "google");
}

#[test]
fn gsuite_provider_scope_union_covers_every_capability_scope() {
    for package in gsuite_package_specs() {
        for capability in package.capabilities {
            for scope in capability.required_scopes {
                assert!(
                    GSUITE_PROVIDER_SCOPES.contains(scope),
                    "{} is missing required scope {scope} from the shared provider scope union",
                    capability.id
                );
            }
        }
    }
}

#[tokio::test]
async fn calendar_handler_integration_tests() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![
            provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE),
            provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE),
        ],
    )
    .await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        json!({"calendar_id":"primary","time_min":"2026-05-21T00:00:00Z"}),
        egress.clone(),
    )
    .await;
    dispatch_ok(
        auth,
        scope,
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        json!({"calendar_id":"primary","event":{"summary":"Review"}}),
        egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.contains("/calendars/primary/events"));
    assert_eq!(requests[1].method, NetworkMethod::Post);
    assert!(requests[1].url.contains("/calendars/primary/events"));
    assert!(!requests[1].url.contains("timeMin"));
    assert!(!requests[1].url.contains("pageToken"));
    assert!(!requests[1].url.contains("maxResults"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[1].body).unwrap()["summary"],
        "Review"
    );
}

#[tokio::test]
async fn calendar_create_event_does_not_forward_list_query_fields() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    dispatch_ok(
        auth,
        scope,
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "time_min": "2026-05-21T00:00:00Z",
            "time_max": "2026-05-22T00:00:00Z",
            "page_token": "next",
            "max_results": 2500,
            "event": { "summary": "Review" }
        }),
        egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://www.googleapis.com/calendar/v3/calendars/primary/events"
    );
}

#[tokio::test]
async fn gsuite_handler_refreshes_expired_google_token_once_and_retries() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    ironclaw_auth::CredentialAccountService::create_account(
        auth.as_ref(),
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-old-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-old-refresh").unwrap()),
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(
            401,
            json!({"error":{"status":"UNAUTHENTICATED","message":"expired"}}),
        ),
        RecordingEgress::json(json!({"id":"sent-after-refresh"})),
    ]));

    let output = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(output["status"], 200);
    assert_eq!(egress.requests().len(), 2);
    let refreshed = ironclaw_auth::CredentialAccountService::select_unique_configured_account(
        auth.as_ref(),
        ironclaw_auth::CredentialAccountSelectionRequest::new(
            auth_scope(&scope),
            google_provider_id().unwrap(),
        )
        .for_extension(ExtensionId::new("gmail").unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(refreshed.status, CredentialAccountStatus::Configured);
}

#[tokio::test]
async fn gsuite_handler_refresh_retries_with_the_same_account_after_account_selection_changes() {
    let scope = scope();
    let seed_auth = InMemoryAuthProductServices::new();
    let initial_account = ironclaw_auth::CredentialAccountService::create_account(
        &seed_auth,
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-old-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-old-refresh").unwrap()),
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();
    let alternate_account = ironclaw_auth::CredentialAccountService::create_account(
        &seed_auth,
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("personal google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-other-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-other-refresh").unwrap()),
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();
    let auth = Arc::new(AccountSwitchingAuthService::new(
        initial_account.clone(),
        alternate_account.clone(),
    ));
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(
            401,
            json!({"error":{"status":"UNAUTHENTICATED","message":"expired"}}),
        ),
        RecordingEgress::json(json!({"id":"sent-after-refresh"})),
    ]));
    let capability_id = capability_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);

    let output = GsuiteExecutor::new(auth.clone(), auth.clone(), noop_credential_stager())
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message": { "raw": "base64url-rfc822" } }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap()
        .output;

    assert_eq!(output["status"], 200);
    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].credential_injections[0].handle,
        SecretHandle::new("google-old-access").unwrap()
    );
    assert_eq!(
        requests[1].credential_injections[0].handle,
        SecretHandle::new("google-refreshed-access").unwrap()
    );
    let state = auth.state.lock().expect("auth state");
    assert_eq!(state.refresh_calls, 1);
    assert_eq!(state.initial_account.id, initial_account.id);
    assert_eq!(state.alternate_account.id, alternate_account.id);
}

#[tokio::test]
async fn gsuite_handler_does_not_refresh_on_non_401_unauthenticated_response() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    ironclaw_auth::CredentialAccountService::create_account(
        auth.as_ref(),
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-old-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-old-refresh").unwrap()),
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(
            403,
            json!({"error":{"status":"UNAUTHENTICATED","message":"expired"}}),
        ),
    ]));

    let output = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(output["status"], 403);
    assert_eq!(output["body"]["error"]["status"], "UNAUTHENTICATED");
    assert_eq!(egress.requests().len(), 1);
    let account = ironclaw_auth::CredentialAccountService::select_unique_configured_account(
        auth.as_ref(),
        ironclaw_auth::CredentialAccountSelectionRequest::new(
            auth_scope(&scope),
            google_provider_id().unwrap(),
        )
        .for_extension(ExtensionId::new("gmail").unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(account.status, CredentialAccountStatus::Configured);
}

#[tokio::test]
async fn gsuite_handler_errors_when_refresh_retry_is_still_auth_expired() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    ironclaw_auth::CredentialAccountService::create_account(
        auth.as_ref(),
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-old-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-old-refresh").unwrap()),
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(
            401,
            json!({"error":{"status":"UNAUTHENTICATED","message":"expired"}}),
        ),
        RecordingEgress::json_status(
            401,
            json!({"error":{"status":"UNAUTHENTICATED","message":"still expired"}}),
        ),
    ]));
    let capability_id = capability_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);

    let error = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager())
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message": { "raw": "base64url-rfc822" } }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .expect_err("retry auth expiry should fail");

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(
        error.is_auth_required(),
        "refreshed credential rejected by Google must trigger auth reauthorization; got {error:?}"
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    let refreshed_access = requests[1].credential_injections[0].handle.clone();
    assert_eq!(
        error.reason(),
        Some(&GsuiteCredentialDispatchReason::AuthRequired {
            required_secrets: vec![refreshed_access.clone()]
        })
    );
    assert_eq!(error.auth_requirement(), Some(vec![refreshed_access]));
    assert_eq!(
        error.usage().map(|usage| usage.network_egress_bytes),
        Some(246)
    );
}

#[tokio::test]
async fn gsuite_handler_stage_credential_failure_on_refresh_retry_returns_auth_required() {
    // stage_credential is called twice: once after initial resolve (line 87 in handlers.rs)
    // and once after the 401-triggered token refresh (line 112).  This test verifies the
    // second call's failure (CredentialStageError::AuthRequired) is correctly projected
    // to GsuiteDispatchError::auth_requirement() -> Some([handle]).
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let access_handle = SecretHandle::new("google-access-token").unwrap();
    ironclaw_auth::CredentialAccountService::create_account(
        auth.as_ref(),
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_handle.clone()),
            refresh_secret: Some(SecretHandle::new("google-refresh-token").unwrap()),
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();
    // Egress: 401 on first call triggers refresh; second call succeeds at egress level
    // but the post-refresh stager fails with AuthRequired before egress is reached.
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(
            401,
            json!({"error":{"status":"UNAUTHENTICATED","message":"expired"}}),
        ),
        // second egress response is never consumed — stager fails first
    ]));

    let error = dispatch_error_with_stager(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
        FailOnNthCallStager::fail_on_second_call(),
    )
    .await;

    // stage_credential failure on the refresh path must surface as auth_required.
    assert!(
        error.is_auth_required(),
        "post-refresh staging failure must produce auth_required; got: {error:?}"
    );
    // Only one egress request was made (the 401-triggering initial request).
    assert_eq!(
        egress.requests().len(),
        1,
        "exactly one egress request must have been made before stage_credential failure"
    );
}

struct AccountSwitchingAuthService {
    state: Mutex<AccountSwitchingAuthState>,
}

struct AccountSwitchingAuthState {
    initial_account: CredentialAccount,
    alternate_account: CredentialAccount,
    select_unique_calls: usize,
    refresh_calls: usize,
}

impl AccountSwitchingAuthService {
    fn new(initial_account: CredentialAccount, alternate_account: CredentialAccount) -> Self {
        Self {
            state: Mutex::new(AccountSwitchingAuthState {
                initial_account,
                alternate_account,
                select_unique_calls: 0,
                refresh_calls: 0,
            }),
        }
    }
}

#[async_trait]
impl ironclaw_auth::CredentialAccountRecordSource for AccountSwitchingAuthService {
    async fn accounts_for_owner(
        &self,
        scope: &AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        let owner = CredentialAccountOwnerScope::from_scope(scope);
        let state = self.state.lock().expect("auth state");
        let accounts = if state.refresh_calls == 0 {
            vec![state.initial_account.clone()]
        } else {
            vec![
                state.initial_account.clone(),
                state.alternate_account.clone(),
            ]
        };
        Ok(accounts
            .into_iter()
            .filter(|account| owner.matches(account))
            .collect())
    }
}

#[async_trait]
impl ironclaw_auth::CredentialAccountService for AccountSwitchingAuthService {
    async fn create_account(
        &self,
        _request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn get_account(
        &self,
        request: CredentialAccountLookupRequest,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        let state = self.state.lock().expect("auth state");
        Ok(if request.account_id == state.initial_account.id {
            Some(state.initial_account.clone())
        } else if request.account_id == state.alternate_account.id {
            Some(state.alternate_account.clone())
        } else {
            None
        })
    }

    async fn list_accounts(
        &self,
        _request: CredentialAccountListRequest,
    ) -> Result<CredentialAccountListPage, AuthProductError> {
        let state = self.state.lock().expect("auth state");
        Ok(CredentialAccountListPage {
            accounts: vec![
                state.initial_account.projection(),
                state.alternate_account.projection(),
            ],
            next_cursor: None,
        })
    }

    async fn update_status(
        &self,
        _scope: &AuthProductScope,
        account_id: ironclaw_auth::CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError> {
        let mut state = self.state.lock().expect("auth state");
        let account = if account_id == state.initial_account.id {
            &mut state.initial_account
        } else if account_id == state.alternate_account.id {
            &mut state.alternate_account
        } else {
            return Err(AuthProductError::CredentialMissing);
        };
        account.status = status;
        Ok(account.clone())
    }

    async fn select_unique_configured_account(
        &self,
        _request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        let mut state = self.state.lock().expect("auth state");
        state.select_unique_calls += 1;
        Ok(if state.select_unique_calls == 1 {
            state.initial_account.projection()
        } else {
            state.alternate_account.projection()
        })
    }

    async fn project_credential_recovery(
        &self,
        _request: CredentialRecoveryRequest,
    ) -> Result<CredentialRecoveryProjection, AuthProductError> {
        let state = self.state.lock().expect("auth state");
        Ok(CredentialRecoveryProjection::configured(
            google_provider_id().unwrap(),
            state.initial_account.projection(),
        ))
    }

    async fn select_configured_account(
        &self,
        _request: CredentialAccountChoiceRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn refresh_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        let mut state = self.state.lock().expect("auth state");
        state.refresh_calls += 1;
        if request.account_id != state.initial_account.id {
            return Err(AuthProductError::CredentialMissing);
        }
        state.initial_account.access_secret =
            Some(SecretHandle::new("google-refreshed-access").unwrap());
        let account = state.initial_account.clone();
        Ok(CredentialRefreshReport {
            account: account.projection(),
            recovery: CredentialRecoveryProjection::configured(
                google_provider_id().unwrap(),
                account.projection(),
            ),
            refreshed: true,
        })
    }
}

#[tokio::test]
async fn gsuite_handler_rejects_oversized_request_body_before_egress() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    let error = dispatch_error(
        auth,
        scope,
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        json!({
            "event": {
                "summary": "Review",
                "description": "x".repeat(GSUITE_REQUEST_BODY_LIMIT + 1)
            }
        }),
        egress.clone(),
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gmail_handler_integration_tests() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_LIST_MESSAGES_CAPABILITY_ID,
        json!({"query":"is:unread","max_results":10}),
        egress.clone(),
    )
    .await;
    dispatch_ok(
        auth,
        scope,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({"message_id":"msg-1"}),
        egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.contains("/users/me/messages"));
    assert!(requests[0].url.contains("q=is%3Aunread"));
    assert_eq!(requests[1].method, NetworkMethod::Get);
    assert!(
        requests[1]
            .url
            .ends_with("/users/me/messages/msg-1?format=full")
    );
}

#[tokio::test]
async fn gsuite_handler_uses_selected_credential_handle_for_runtime_egress() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let executor = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager());
    let capability_id = capability_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::permissive_success());

    let result = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message": { "raw": "base64url-rfc822" } }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    assert_eq!(result.output["status"], 200);
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability_id, capability_id);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
    assert!(
        !requests[0]
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
    );
    assert_eq!(requests[0].credential_injections.len(), 1);
    assert_eq!(
        requests[0].credential_injections[0].handle,
        SecretHandle::new("google-access-token").unwrap()
    );
}

#[tokio::test]
async fn gsuite_handler_applies_google_api_network_policy() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let executor = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager());
    let capability_id = capability_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::permissive_success());

    executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message": { "raw": "base64url-rfc822" } }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    let policy = &requests[0].network_policy;
    assert!(policy.deny_private_ip_ranges);
    assert_eq!(policy.max_egress_bytes, Some(10 * 1024 * 1024));
    let allowed = policy
        .allowed_targets
        .iter()
        .map(|target| (target.scheme, target.host_pattern.as_str()))
        .collect::<Vec<_>>();
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "www.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "gmail.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "calendar.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "oauth2.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "accounts.google.com")));
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_scope_is_missing() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let executor = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager());
    let capability_id = capability_id(GMAIL_TRASH_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::permissive_success());

    let error = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message_id": "msg-1" }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::Client
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handler_allows_reply_to_message_with_send_scope_only() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    dispatch_ok(
        auth,
        scope,
        GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822", "threadId": "thread-1" } }),
        egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
}

#[test]
fn gsuite_package_specs_include_core_capabilities() {
    let capability_ids = gsuite_package_specs()
        .iter()
        .flat_map(|package| {
            package
                .capabilities
                .iter()
                .map(|capability| format!("{}.{}", package.extension_id, capability.short_name))
        })
        .collect::<Vec<_>>();

    for id in [
        CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        CALENDAR_GET_EVENT_CAPABILITY_ID,
        CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
        CALENDAR_DELETE_EVENT_CAPABILITY_ID,
        CALENDAR_ADD_ATTENDEES_CAPABILITY_ID,
        CALENDAR_SET_REMINDER_CAPABILITY_ID,
        GMAIL_LIST_MESSAGES_CAPABILITY_ID,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        GMAIL_CREATE_DRAFT_CAPABILITY_ID,
        GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
        GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
    ] {
        assert!(
            capability_ids.contains(&id.to_string()),
            "missing capability spec for {id}"
        );
    }
    assert!(AuthProviderId::new("google").is_ok());
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_account_is_not_configured() {
    let scope = scope();
    let auth = auth_with_google_account_status(
        &scope,
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::PendingSetup,
        true,
    )
    .await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::Client
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_access_secret_is_missing() {
    let scope = scope();
    let auth = auth_with_google_account_status(
        &scope,
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::Configured,
        false,
    )
    .await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::Client
    );
    assert_eq!(
        error.reason(),
        Some(&GsuiteCredentialDispatchReason::MissingAccessSecret)
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handler_rejects_missing_required_input() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)]).await;

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({}),
        Arc::new(RecordingEgress::permissive_success()),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode
    );
}

#[tokio::test]
async fn calendar_id_default_does_not_swallow_invalid_type() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE)])
            .await;

    let error = dispatch_error(
        auth,
        scope,
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        json!({ "calendar_id": false }),
        Arc::new(RecordingEgress::permissive_success()),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode
    );
}

#[tokio::test]
async fn gsuite_handler_rejects_malformed_egress_response() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::malformed_json(),
    ]));

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress,
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);
}

#[tokio::test]
async fn gsuite_handler_maps_runtime_egress_errors() {
    let cases = [
        (
            RuntimeHttpEgressError::Credential {
                reason: "missing".to_string(),
            },
            RuntimeDispatchErrorKind::Client,
        ),
        (
            RuntimeHttpEgressError::Request {
                reason: "denied".to_string(),
                request_bytes: 11,
                response_bytes: 0,
            },
            RuntimeDispatchErrorKind::InputEncode,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "offline".to_string(),
                request_bytes: 12,
                response_bytes: 0,
            },
            RuntimeDispatchErrorKind::NetworkDenied,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "bad response".to_string(),
                request_bytes: 13,
                response_bytes: 1,
            },
            RuntimeDispatchErrorKind::OutputDecode,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED.to_string(),
                request_bytes: 14,
                response_bytes: 1024,
            },
            RuntimeDispatchErrorKind::OutputTooLarge,
        ),
    ];

    for (error, expected_kind) in cases {
        let scope = scope();
        let auth =
            auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
        let request_bytes = error.request_bytes();
        let egress = Arc::new(RecordingEgress::with_errors(vec![error]));

        let error = dispatch_error(
            auth,
            scope,
            GMAIL_SEND_MESSAGE_CAPABILITY_ID,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            egress,
        )
        .await;

        assert_eq!(error.kind(), expected_kind);
        if request_bytes > 0 {
            assert_eq!(
                error.usage().map(|usage| usage.network_egress_bytes),
                Some(request_bytes)
            );
        }
    }
}

#[tokio::test]
async fn gsuite_handler_maps_panicking_runtime_egress_to_backend() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::with_responses(Vec::new()));

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress,
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Backend);
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_google_account_is_missing_or_ambiguous() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let egress = Arc::new(RecordingEgress::permissive_success());

    let error = dispatch_error(
        auth.clone(),
        scope.clone(),
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(egress.requests().is_empty());

    ironclaw_auth::CredentialAccountService::create_account(
        auth.as_ref(),
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("hidden google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(ExtensionId::new("other-extension").unwrap()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("hidden-access").unwrap()),
            refresh_secret: None,
            scopes: vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        },
    )
    .await
    .unwrap();

    let error = dispatch_error(
        auth.clone(),
        scope.clone(),
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(egress.requests().is_empty());

    add_google_account(
        &auth,
        &scope,
        "work google",
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::Configured,
        true,
    )
    .await;
    add_google_account(
        &auth,
        &scope,
        "personal google",
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::Configured,
        true,
    )
    .await;
    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_for_non_configured_account_states() {
    let scope = scope();
    let egress = Arc::new(RecordingEgress::permissive_success());
    let cases = [
        CredentialAccountStatus::Inactive,
        CredentialAccountStatus::Expired,
        CredentialAccountStatus::RefreshFailed,
        CredentialAccountStatus::Revoked,
    ];

    for status in cases {
        let auth = auth_with_google_account_status(
            &scope,
            vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
            status,
            true,
        )
        .await;
        let error = dispatch_error(
            auth,
            scope.clone(),
            GMAIL_SEND_MESSAGE_CAPABILITY_ID,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            egress.clone(),
        )
        .await;

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
        assert!(egress.requests().is_empty());
    }
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_missing_scopes() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-1" }),
        egress.clone(),
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(matches!(
        error.reason(),
        Some(GsuiteCredentialDispatchReason::MissingScopes { missing_scopes })
            if missing_scopes.as_slice() == [provider_scope(GOOGLE_GMAIL_MODIFY_SCOPE)]
    ));
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handlers_build_expected_requests_for_each_capability() {
    let scope = scope();
    let all_scopes = vec![
        provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE),
        provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE),
        provider_scope(GOOGLE_GMAIL_READONLY_SCOPE),
        provider_scope(GOOGLE_GMAIL_SEND_SCOPE),
        provider_scope(GOOGLE_GMAIL_MODIFY_SCOPE),
    ];
    let auth = auth_with_google_account(&scope, all_scopes).await;
    let egress = Arc::new(RecordingEgress::permissive_success());

    let cases = [
        (
            CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
            json!({}),
            NetworkMethod::Get,
            "/users/me/calendarList",
        ),
        (
            CALENDAR_LIST_EVENTS_CAPABILITY_ID,
            json!({"calendar_id":"primary","time_min":"2026-05-21T00:00:00Z","max_results":10}),
            NetworkMethod::Get,
            "/calendars/primary/events",
        ),
        (
            CALENDAR_GET_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1"}),
            NetworkMethod::Get,
            "/events/evt-1",
        ),
        (
            CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
            json!({"timeMin":"2026-05-21T00:00:00Z","timeMax":"2026-05-22T00:00:00Z"}),
            NetworkMethod::Post,
            "/freeBusy",
        ),
        (
            CALENDAR_CREATE_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event":{"summary":"Review"}}),
            NetworkMethod::Post,
            "/calendars/primary/events",
        ),
        (
            CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1","event":{"summary":"Updated"}}),
            NetworkMethod::Patch,
            "/events/evt-1",
        ),
        (
            CALENDAR_DELETE_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1"}),
            NetworkMethod::Delete,
            "/events/evt-1",
        ),
        (
            CALENDAR_SET_REMINDER_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1","reminders":{"useDefault":false}}),
            NetworkMethod::Patch,
            "/events/evt-1",
        ),
        (
            GMAIL_LIST_MESSAGES_CAPABILITY_ID,
            json!({"query":"is:unread","label_ids":["INBOX"],"max_results":10}),
            NetworkMethod::Get,
            "/users/me/messages",
        ),
        (
            GMAIL_GET_MESSAGE_CAPABILITY_ID,
            json!({"message_id":"msg-1"}),
            NetworkMethod::Get,
            "/users/me/messages/msg-1?format=full",
        ),
        (
            GMAIL_SEND_MESSAGE_CAPABILITY_ID,
            json!({"message":{"raw":"base64url-rfc822"}}),
            NetworkMethod::Post,
            "/users/me/messages/send",
        ),
        (
            GMAIL_CREATE_DRAFT_CAPABILITY_ID,
            json!({"draft":{"message":{"raw":"base64url-rfc822"}}}),
            NetworkMethod::Post,
            "/users/me/drafts",
        ),
        (
            GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
            json!({"message":{"raw":"base64url-rfc822","threadId":"thread-1"}}),
            NetworkMethod::Post,
            "/users/me/messages/send",
        ),
        (
            GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
            json!({"message_id":"msg-1"}),
            NetworkMethod::Post,
            "/users/me/messages/msg-1/trash",
        ),
    ];

    for (capability, input, _, _) in &cases {
        dispatch_ok(
            auth.clone(),
            scope.clone(),
            capability,
            input.clone(),
            egress.clone(),
        )
        .await;
    }
    let add_egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json(json!({"attendees":[{"email":"existing@example.com"}]})),
        RecordingEgress::json(json!({"id":"evt-1"})),
    ]));
    dispatch_ok(
        auth,
        scope.clone(),
        CALENDAR_ADD_ATTENDEES_CAPABILITY_ID,
        json!({
            "calendar_id":"primary",
            "event_id":"evt-1",
            "attendees":[{"email":"new@example.com"}]
        }),
        add_egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), cases.len());
    for ((_, _, method, url_fragment), request) in cases.iter().zip(requests.iter()) {
        assert_eq!(&request.method, method);
        assert!(
            request.url.contains(url_fragment),
            "{} did not contain {url_fragment}",
            request.url
        );
        assert_eq!(request.credential_injections.len(), 1);
    }
    assert!(requests[8].url.contains("labelIds=INBOX"));

    let add_requests = add_egress.requests();
    assert_eq!(add_requests.len(), 2);
    let add_get = &add_requests[0];
    let add_patch = &add_requests[1];
    assert_eq!(add_get.method, NetworkMethod::Get);
    assert!(add_get.url.contains("/events/evt-1"));
    assert_eq!(add_patch.method, NetworkMethod::Patch);
    assert!(add_patch.url.contains("/events/evt-1"));
    let patch_body: serde_json::Value = serde_json::from_slice(&add_patch.body).unwrap();
    let attendees = patch_body["attendees"].as_array().unwrap();
    assert!(
        attendees
            .iter()
            .any(|attendee| attendee["email"] == "existing@example.com")
    );
    assert!(
        attendees
            .iter()
            .any(|attendee| attendee["email"] == "new@example.com")
    );
}

#[test]
fn gsuite_resource_profile_allows_wrapped_response_headroom() {
    let profile = gsuite_resource_profile();
    let output_limit = profile.default_estimate.output_bytes.unwrap_or_default();

    assert!(output_limit > GSUITE_RESPONSE_BODY_LIMIT);
    assert_eq!(output_limit, GSUITE_OUTPUT_BYTES_LIMIT);
    assert_eq!(
        profile
            .hard_ceiling
            .as_ref()
            .and_then(|ceiling| ceiling.max_output_bytes),
        Some(GSUITE_OUTPUT_BYTES_LIMIT)
    );
}

#[tokio::test]
async fn add_attendees_refreshes_expired_get_and_retries_patch() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    ironclaw_auth::CredentialAccountService::create_account(
        auth.as_ref(),
        NewCredentialAccount {
            scope: auth_scope(&scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-old-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-old-refresh").unwrap()),
            scopes: vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
        },
    )
    .await
    .unwrap();
    let capability_id = capability_id(CALENDAR_ADD_ATTENDEES_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(
            401,
            json!({"error":{"status":"UNAUTHENTICATED","message":"expired"}}),
        ),
        RecordingEgress::json_with_request_bytes(
            json!({
                "attendees":[{"email":"existing@example.com"}],
                "etag":"retry-get-etag"
            }),
            101,
        ),
        RecordingEgress::json_with_request_bytes(json!({"id":"evt-1","updated":true}), 211),
    ]));

    let result = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager())
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({
                "calendar_id": "primary",
                "event_id": "evt-1",
                "attendees": [{"email": "new@example.com"}]
            }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    assert_eq!(result.output["status"], 200);
    assert_eq!(result.output["body"]["id"], "evt-1");
    assert_eq!(result.output["body"]["updated"], true);
    assert_eq!(result.output["redaction_applied"], true);

    let requests = egress.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.ends_with("/calendars/primary/events/evt-1"));
    assert_eq!(requests[1].method, NetworkMethod::Get);
    assert!(requests[1].url.ends_with("/calendars/primary/events/evt-1"));
    assert_eq!(requests[2].method, NetworkMethod::Patch);
    assert!(requests[2].url.ends_with("/calendars/primary/events/evt-1"));
    assert_eq!(
        requests[2]
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("if-match"))
            .map(|(_, value)| value.as_str()),
        Some("retry-get-etag")
    );
    let patch_body: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert_eq!(
        patch_body["attendees"],
        json!([
            {"email":"existing@example.com"},
            {"email":"new@example.com"}
        ])
    );
    assert_eq!(result.usage.network_egress_bytes, 435);
}

#[tokio::test]
async fn add_attendees_reports_both_google_api_requests() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let executor = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager());
    let capability_id = capability_id(CALENDAR_ADD_ATTENDEES_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_with_request_bytes(
            json!({"attendees":[{"email":"existing@example.com"}]}),
            101,
        ),
        RecordingEgress::json_with_request_bytes(json!({"id":"evt-1"}), 211),
    ]));

    let result = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({
                "calendar_id": "primary",
                "event_id": "evt-1",
                "attendees": [{"email": "new@example.com"}]
            }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    assert_eq!(egress.requests().len(), 2);
    assert_eq!(result.usage.network_egress_bytes, 312);
}

#[tokio::test]
async fn add_attendees_reports_failed_initial_get_network_usage() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let executor = GsuiteExecutor::new(auth.clone(), auth, noop_credential_stager());
    let capability_id = capability_id(CALENDAR_ADD_ATTENDEES_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::with_errors(vec![
        RuntimeHttpEgressError::Network {
            reason: "calendar offline".to_string(),
            request_bytes: 101,
            response_bytes: 0,
        },
    ]));

    let error = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({
                "calendar_id": "primary",
                "event_id": "evt-1",
                "attendees": [{"email": "new@example.com"}]
            }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .expect_err("initial GET failure should report egress usage");

    assert_eq!(egress.requests().len(), 1);
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    assert_eq!(
        error.usage().map(|usage| usage.network_egress_bytes),
        Some(101)
    );
}

#[tokio::test]
async fn add_attendees_restages_credential_before_patch() {
    // P2 regression: the staged-obligation store is one-shot.
    // execute_add_attendees makes GET then PATCH; without restaging, the PATCH
    // fires with no credential. We use FailOnNthCallStager to verify the second
    // staging call happens — if it doesn't happen, the dispatch succeeds instead
    // of returning AuthRequired.
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let capability_id = capability_id(CALENDAR_ADD_ATTENDEES_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_with_request_bytes(
            json!({
                "attendees": [{"email": "existing@example.com"}],
                "etag": "test-etag"
            }),
            50,
        ),
        // PATCH response — would succeed, but stager fails before we get here
        RecordingEgress::json_with_request_bytes(json!({"id": "evt-1", "updated": true}), 80),
    ]));

    // Stager succeeds on first call (for GET), fails AuthRequired on second call (for PATCH).
    // Without the P2 fix, the second staging call never happens and dispatch succeeds.
    let error = GsuiteExecutor::new(
        auth.clone(),
        auth,
        FailOnNthCallStager::fail_on_second_call(),
    )
    .dispatch(GsuiteDispatchRequest {
        capability_id: &capability_id,
        scope: &scope,
        input: &json!({
            "calendar_id": "primary",
            "event_id": "evt-1",
            "attendees": [{"email": "new@example.com"}]
        }),
        runtime_http_egress: egress.clone(),
    })
    .await
    .expect_err("second staging should fail with AuthRequired before PATCH fires");

    assert!(
        error.is_auth_required(),
        "stager auth-required on PATCH restage must surface as AuthRequired; got {error:?}"
    );
    // Only the GET fired; PATCH was blocked by staging failure
    assert_eq!(
        egress.requests().len(),
        1,
        "PATCH must not fire after staging failure; egress count should be 1 (GET only)"
    );
    assert_eq!(
        error.usage().map(|u| u.network_egress_bytes),
        Some(50),
        "GET network bytes must be reported even when PATCH staging fails"
    );
}
