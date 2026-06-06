//! WebUI route composition for Slack personal binding pairing-code redemption.

use std::num::{NonZeroU32, NonZeroU64};

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
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use serde::{Deserialize, Serialize};

use crate::slack_personal_binding::SlackPersonalBindingPrincipal;
use crate::slack_personal_binding_pairing::{
    SlackPersonalBindingPairingCode, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingService,
};

pub const WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH: &str =
    "/api/webchat/v2/extensions/pairing/redeem";

const SLACK_PERSONAL_BINDING_PAIRING_REDEEM_ROUTE_ID: &str = "webui.v2.extensions.pairing.redeem";
const SLACK_PERSONAL_BINDING_PAIRING_BODY_LIMIT_BYTES: NonZeroU64 =
    NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const SLACK_PERSONAL_BINDING_PAIRING_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(20).unwrap(); // safety: 20 is non-zero.
const SLACK_PERSONAL_BINDING_PAIRING_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.

#[derive(Clone, Debug)]
pub struct SlackPersonalBindingPairingRouteConfig {
    pairing_service: SlackPersonalBindingPairingService,
}

impl SlackPersonalBindingPairingRouteConfig {
    pub fn new(pairing_service: SlackPersonalBindingPairingService) -> Self {
        Self { pairing_service }
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
    let binding = config
        .pairing_service
        .redeem_challenge(
            SlackPersonalBindingPrincipal {
                tenant_id: caller.tenant_id,
                user_id: caller.user_id,
            },
            code,
        )
        .await?;
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
            Self::BadRequest => (StatusCode::BAD_REQUEST, "Invalid or expired pairing code."),
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
            })
            .body(Body::from(body))
            .unwrap()
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).unwrap()
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
