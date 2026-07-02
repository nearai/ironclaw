//! WebUI route composition for Slack personal binding pairing-code redemption.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_product_workflow::{
    ChannelConnectionResumeScope, ChannelConnectionResumeService, ResumeChannelConnectionRequest,
    WebUiAuthenticatedCaller,
};
use serde::{Deserialize, Serialize};

use crate::slack_personal_binding::SlackPersonalBindingPrincipal;
use crate::slack_personal_binding_pairing::{
    SlackPersonalBindingPairingCode, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingService,
};

pub const WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH: &str =
    "/api/webchat/v2/extensions/pairing/redeem";

/// Canonical connectable-channel id the Slack activation gate keys on. The
/// browser may send the `slack`/`slack_v2`/`slack-v2` aliases, but the parked
/// `ChannelPairing` gate always carries `slack`, so resume is driven by this
/// canonical id regardless of the alias the caller used.
const SLACK_CONNECTION_CHANNEL: &str = "slack";

const SLACK_PERSONAL_BINDING_PAIRING_REDEEM_ROUTE_ID: &str = "webui.v2.extensions.pairing.redeem";
const SLACK_PERSONAL_BINDING_PAIRING_BODY_LIMIT_BYTES: NonZeroU64 =
    NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const SLACK_PERSONAL_BINDING_PAIRING_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(20).unwrap(); // safety: 20 is non-zero.
const SLACK_PERSONAL_BINDING_PAIRING_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.

#[derive(Clone)]
pub struct SlackPersonalBindingPairingRouteConfig {
    pairing_service: SlackPersonalBindingPairingService,
    // arch-exempt: optional_arc, resume is an additive follow-up to a successful
    // pairing bind — the redeem binds the identity with or without it, and the
    // public serve-composition test / unit helpers mount the route without a
    // turn-state-backed resume. Production Slack host-beta always wires it. See
    // docs/plans/2026-07-02-channel-connection-gate.md.
    channel_connection_resume: Option<Arc<dyn ChannelConnectionResumeService>>,
}

impl SlackPersonalBindingPairingRouteConfig {
    pub fn new(pairing_service: SlackPersonalBindingPairingService) -> Self {
        Self {
            pairing_service,
            channel_connection_resume: None,
        }
    }

    /// Wire the channel-connection gate resume so redeeming a pairing code
    /// continues every run the caller has parked on this channel's connection
    /// gate (V2 of the channel-connection gate; supersedes the browser's fake
    /// "Slack is connected, continue" message).
    pub fn with_channel_connection_resume(
        mut self,
        resume: Arc<dyn ChannelConnectionResumeService>,
    ) -> Self {
        self.channel_connection_resume = Some(resume);
        self
    }
}

pub(crate) struct SlackPersonalBindingPairingRouteMount {
    pub(crate) protected: Router,
    pub(crate) descriptors: Vec<IngressRouteDescriptor>,
}

pub(crate) fn slack_personal_binding_pairing_route_mount(
    config: SlackPersonalBindingPairingRouteConfig,
) -> SlackPersonalBindingPairingRouteMount {
    SlackPersonalBindingPairingRouteMount {
        protected: Router::new()
            .route(
                WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH,
                post(slack_personal_binding_pairing_redeem_handler),
            )
            .with_state(config),
        descriptors: slack_personal_binding_pairing_route_descriptors(),
    }
}

pub(crate) fn slack_personal_binding_pairing_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            SLACK_PERSONAL_BINDING_PAIRING_REDEEM_ROUTE_ID,
            NetworkMethod::Post,
            WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH,
            redeem_policy(),
        )
        .expect("Slack personal binding pairing route descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
    ]
}

fn redeem_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: SLACK_PERSONAL_BINDING_PAIRING_BODY_LIMIT_BYTES,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: SLACK_PERSONAL_BINDING_PAIRING_MAX_REQUESTS,
            window_seconds: SLACK_PERSONAL_BINDING_PAIRING_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("Slack personal binding pairing policy must validate") // safety: policy fields are typed static literals with non-zero limits.
}

#[derive(Debug, Deserialize)]
struct SlackPersonalBindingPairingRedeemRequest {
    channel: String,
    code: String,
}

#[derive(Debug, Serialize)]
pub struct SlackPersonalBindingPairingRedeemResponse {
    pub provider: String,
    pub provider_user_id: String,
}

async fn slack_personal_binding_pairing_redeem_handler(
    State(config): State<SlackPersonalBindingPairingRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackPersonalBindingPairingRedeemRequest>,
) -> Result<Json<SlackPersonalBindingPairingRedeemResponse>, SlackPersonalBindingPairingRouteError>
{
    validate_pairing_channel(&request.channel)?;
    let code = SlackPersonalBindingPairingCode::new(request.code)?;
    // Bind the caller's channel identity FIRST: the resume below re-dispatches
    // each parked `extension_activate`, which re-checks the per-caller channel
    // connection — so the binding must already be durable before resume runs.
    let binding = config
        .pairing_service
        .redeem_challenge(
            SlackPersonalBindingPrincipal {
                tenant_id: caller.tenant_id.clone(),
                user_id: caller.user_id.clone(),
            },
            code,
        )
        .await?;
    // Continue every run this caller has parked on the Slack connection gate.
    // A redeem with nothing parked is valid (the resume returns no runs); a
    // backend resume fault is surfaced rather than masked.
    if let Some(resume) = &config.channel_connection_resume {
        resume
            .resume_channel_connection(ResumeChannelConnectionRequest {
                scope: ChannelConnectionResumeScope {
                    tenant_id: caller.tenant_id,
                    user_id: caller.user_id,
                },
                channel: SLACK_CONNECTION_CHANNEL.to_string(),
            })
            .await
            .map_err(SlackPersonalBindingPairingRouteError::from_resume)?;
    }
    Ok(Json(SlackPersonalBindingPairingRedeemResponse {
        provider: binding.provider.to_string(),
        provider_user_id: binding.provider_user_id.to_string(),
    }))
}

fn validate_pairing_channel(channel: &str) -> Result<(), SlackPersonalBindingPairingRouteError> {
    match channel.trim().to_ascii_lowercase().as_str() {
        "slack" | "slack_v2" | "slack-v2" => Ok(()),
        _ => Err(SlackPersonalBindingPairingRouteError::BadRequest),
    }
}

#[derive(Debug)]
enum SlackPersonalBindingPairingRouteError {
    BadRequest,
    Unavailable,
}

impl SlackPersonalBindingPairingRouteError {
    /// Map a channel-connection resume failure. The identity was already bound
    /// durably at this point, so this is a follow-up fault: surface it as a
    /// retryable Unavailable rather than dropping the error (error-handling.md:
    /// fail loud, no silent `.ok()?`).
    fn from_resume(error: ironclaw_product_workflow::ProductWorkflowError) -> Self {
        tracing::warn!(
            %error,
            "channel-connection resume after Slack pairing redeem failed"
        );
        Self::Unavailable
    }
}

impl From<SlackPersonalBindingPairingError> for SlackPersonalBindingPairingRouteError {
    fn from(error: SlackPersonalBindingPairingError) -> Self {
        match error {
            SlackPersonalBindingPairingError::InvalidCode { .. }
            | SlackPersonalBindingPairingError::ChallengeNotFound => Self::BadRequest,
            SlackPersonalBindingPairingError::Binding(binding_error) => match binding_error {
                crate::slack_personal_binding::SlackPersonalUserBindingError::UnknownInstallation {
                    ..
                }
                | crate::slack_personal_binding::SlackPersonalUserBindingError::InstallationNotTenantScoped {
                    ..
                }
                | crate::slack_personal_binding::SlackPersonalUserBindingError::SlackInstallationContextMismatch {
                    ..
                }
                | crate::slack_personal_binding::SlackPersonalUserBindingError::InvalidSlackId {
                    ..
                } => Self::BadRequest,
                crate::slack_personal_binding::SlackPersonalUserBindingError::BindingStore(_) => {
                    Self::Unavailable
                }
            },
            SlackPersonalBindingPairingError::Backend(_) => Self::Unavailable,
        }
    }
}

impl IntoResponse for SlackPersonalBindingPairingRouteError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest => (
                StatusCode::BAD_REQUEST,
                "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Slack pairing service is unavailable.",
            ),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_product_workflow::{ProductWorkflowError, ResumeChannelConnectionResponse};
    use tower::ServiceExt;

    use super::*;
    use crate::slack_personal_binding::{
        RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
        SlackPersonalBindingInstallation, SlackPersonalUserBindingService,
    };
    use crate::slack_personal_binding_pairing::{
        IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallenge,
        SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingNotification,
        SlackPersonalBindingPairingNotifier,
    };
    use crate::slack_serve::{SlackInstallationSelector, SlackUserId};

    #[tokio::test]
    async fn redeem_route_binds_code_to_authenticated_caller() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let mount = route_mount(
            binding_store.clone(),
            Arc::new(StaticChallengeStore::found()),
        );
        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"slack","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(binding_store.bound_user_ids(), vec!["user:alice"]);
    }

    #[tokio::test]
    async fn redeem_route_maps_invalid_code_to_bad_request() {
        let mount = route_mount(
            Arc::new(RecordingBindingStore::default()),
            Arc::new(StaticChallengeStore::found()),
        );

        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"slack","code":"abc123"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // Deliverable: an invalid/expired code must steer the user back to
        // `/pair` (the only self-service recovery surface). The web pairing
        // card renders this JSON `error` body verbatim, so the `/pair`
        // instruction has to live in the route response, not only in the
        // descriptor/i18n fallback copy.
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let message = body["error"].as_str().unwrap();
        assert_eq!(
            message, "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.",
            "invalid-code redeem error must match the caller-facing recovery copy"
        );
    }

    #[tokio::test]
    async fn redeem_route_maps_unknown_code_to_bad_request() {
        let mount = route_mount(
            Arc::new(RecordingBindingStore::default()),
            Arc::new(StaticChallengeStore::missing()),
        );

        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"slack","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn redeem_route_maps_foreign_tenant_code_to_opaque_bad_request() {
        let mount = route_mount(
            Arc::new(RecordingBindingStore::default()),
            Arc::new(StaticChallengeStore::found()),
        );

        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-b",
                r#"{"channel":"slack","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn redeem_route_maps_binding_store_error_to_unavailable() {
        let binding_store = Arc::new(RecordingBindingStore::with_error(
            RebornUserIdentityBindingError::Backend("store down".into()),
        ));
        let mount = route_mount(binding_store, Arc::new(StaticChallengeStore::found()));

        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"slack","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn redeem_route_rejects_unsupported_channels_before_binding() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let mount = route_mount(
            binding_store.clone(),
            Arc::new(StaticChallengeStore::found()),
        );

        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"discord","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(binding_store.bound_user_ids().is_empty());
    }

    fn route_mount(
        binding_store: Arc<RecordingBindingStore>,
        challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
    ) -> SlackPersonalBindingPairingRouteMount {
        let pairing = SlackPersonalBindingPairingService::new(
            SlackPersonalUserBindingService::new(
                [SlackPersonalBindingInstallation {
                    tenant_id: TenantId::new("tenant-a").unwrap(),
                    installation_id: installation("install-a"),
                    selector: SlackInstallationSelector::app_team("A-app", "T-team"),
                }],
                binding_store,
            ),
            challenge_store,
            Arc::new(NoopNotifier),
        );
        slack_personal_binding_pairing_route_mount(SlackPersonalBindingPairingRouteConfig::new(
            pairing,
        ))
    }

    fn redeem_request(tenant_id: &str, body: &'static str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
            .header("content-type", "application/json")
            .extension(WebUiAuthenticatedCaller {
                tenant_id: TenantId::new(tenant_id).unwrap(),
                user_id: UserId::new("user:alice").unwrap(),
                agent_id: None,
                project_id: None,
                operator_webui_config: false,
            })
            .body(Body::from(body))
            .unwrap()
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).unwrap()
    }

    /// Records each resume request plus the binding-store state captured at call
    /// time, so a test can assert the identity was already bound before resume
    /// ran (bind-then-resume ordering).
    struct RecordingResume {
        binding_store: Arc<RecordingBindingStore>,
        calls: Mutex<Vec<RecordedResume>>,
    }

    struct RecordedResume {
        channel: String,
        tenant_id: TenantId,
        user_id: UserId,
        bound_user_ids_at_call: Vec<String>,
    }

    impl RecordingResume {
        fn new(binding_store: Arc<RecordingBindingStore>) -> Self {
            Self {
                binding_store,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> std::sync::MutexGuard<'_, Vec<RecordedResume>> {
            self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl ChannelConnectionResumeService for RecordingResume {
        async fn resume_channel_connection(
            &self,
            request: ResumeChannelConnectionRequest,
        ) -> Result<ResumeChannelConnectionResponse, ProductWorkflowError> {
            self.calls().push(RecordedResume {
                channel: request.channel,
                tenant_id: request.scope.tenant_id,
                user_id: request.scope.user_id,
                bound_user_ids_at_call: self.binding_store.bound_user_ids(),
            });
            Ok(ResumeChannelConnectionResponse {
                resumed_runs: Vec::new(),
            })
        }
    }

    fn route_mount_with_resume(
        binding_store: Arc<RecordingBindingStore>,
        challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
        resume: Arc<RecordingResume>,
    ) -> SlackPersonalBindingPairingRouteMount {
        let pairing = SlackPersonalBindingPairingService::new(
            SlackPersonalUserBindingService::new(
                [SlackPersonalBindingInstallation {
                    tenant_id: TenantId::new("tenant-a").unwrap(),
                    installation_id: installation("install-a"),
                    selector: SlackInstallationSelector::app_team("A-app", "T-team"),
                }],
                binding_store,
            ),
            challenge_store,
            Arc::new(NoopNotifier),
        );
        slack_personal_binding_pairing_route_mount(
            SlackPersonalBindingPairingRouteConfig::new(pairing)
                .with_channel_connection_resume(resume),
        )
    }

    #[tokio::test]
    async fn redeem_binds_then_resumes_channel_connection() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let resume = Arc::new(RecordingResume::new(binding_store.clone()));
        let mount = route_mount_with_resume(
            binding_store.clone(),
            Arc::new(StaticChallengeStore::found()),
            resume.clone(),
        );

        // The browser sent the `slack_v2` alias; resume must still target the
        // canonical `slack` channel the activation gate keys on.
        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"slack_v2","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(binding_store.bound_user_ids(), vec!["user:alice"]);

        let calls = resume.calls();
        assert_eq!(calls.len(), 1, "a successful redeem resumes exactly once");
        assert_eq!(
            calls[0].channel, "slack",
            "resume targets the canonical slack channel, not the wire alias"
        );
        assert_eq!(calls[0].user_id.as_str(), "user:alice");
        assert_eq!(calls[0].tenant_id.as_str(), "tenant-a");
        assert_eq!(
            calls[0].bound_user_ids_at_call,
            vec!["user:alice".to_string()],
            "identity must be bound before the resume runs"
        );
    }

    #[tokio::test]
    async fn redeem_failure_does_not_resume() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let resume = Arc::new(RecordingResume::new(binding_store.clone()));
        let mount = route_mount_with_resume(
            binding_store,
            Arc::new(StaticChallengeStore::missing()),
            resume.clone(),
        );

        let response = mount
            .protected
            .oneshot(redeem_request(
                "tenant-a",
                r#"{"channel":"slack","code":"abc12345"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            resume.calls().is_empty(),
            "resume must not run when the pairing bind fails"
        );
    }

    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
        error: Option<RebornUserIdentityBindingError>,
    }

    impl RecordingBindingStore {
        fn with_error(error: RebornUserIdentityBindingError) -> Self {
            Self {
                bindings: Mutex::new(Vec::new()),
                error: Some(error),
            }
        }

        fn bound_user_ids(&self) -> Vec<String> {
            self.bindings
                .lock()
                .unwrap()
                .iter()
                .map(|binding| binding.user_id.to_string())
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings.lock().unwrap().push(binding);
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
    }

    struct StaticChallengeStore {
        challenge: Option<SlackPersonalBindingPairingChallenge>,
    }

    impl StaticChallengeStore {
        fn found() -> Self {
            Self {
                challenge: Some(SlackPersonalBindingPairingChallenge {
                    installation_id: installation("install-a"),
                    slack_user_id: SlackUserId::new("U123"),
                    setup_revision: None,
                }),
            }
        }

        fn missing() -> Self {
            Self { challenge: None }
        }
    }

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingChallengeStore for StaticChallengeStore {
        async fn issue_challenge(
            &self,
            challenge: SlackPersonalBindingPairingChallenge,
        ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Ok(IssuedSlackPersonalBindingPairingChallenge {
                code: SlackPersonalBindingPairingCode::new("ABC12345").unwrap(),
                challenge,
            })
        }

        async fn get_challenge(
            &self,
            code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            if code.as_str() != "ABC12345" {
                return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
            }
            self.challenge
                .clone()
                .ok_or(SlackPersonalBindingPairingError::ChallengeNotFound)
        }

        async fn consume_challenge(
            &self,
            code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            self.get_challenge(code).await
        }
    }

    struct NoopNotifier;

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingNotifier for NoopNotifier {
        async fn send_pairing_challenge(
            &self,
            _notification: SlackPersonalBindingPairingNotification,
        ) -> Result<(), SlackPersonalBindingPairingError> {
            Ok(())
        }
    }
}
