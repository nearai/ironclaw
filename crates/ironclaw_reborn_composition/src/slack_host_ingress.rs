use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::num::{NonZeroU32, NonZeroU64};
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::HeaderMap;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, HostIngressRouteDeclaration,
    HostIngressTarget, IngressAckMode, IngressAuthBinding, IngressAuthPolicy, IngressAuthScheme,
    IngressAuthSchemeName, IngressCredentialHandle, IngressDrainMode, IngressPolicy,
    IngressPolicyParts, IngressRouteDescriptor, IngressScopeSource, ListenerClass, RateLimitPolicy,
    RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{CapabilityId, NetworkMethod, ResourceScope, SecretHandle};
use ironclaw_product_adapters::{ProtocolAuthEvidence, mark_request_signature_verified};
use ironclaw_secrets::{SecretStore, SecretStoreError};
use ironclaw_slack_v2_adapter::slack_request_signature_auth_requirement;
use ironclaw_wasm_product_adapters::{
    HmacWebhookAuth, ImmediateAckWorkflowObserver, NativeProductAdapterRunner, RunnerError,
    VerificationOutcome, WebhookAuthVerifier, WebhookProcessOutcome,
};

use crate::host_ingress::{
    HostIngressAuthCandidate, HostIngressCapabilityHandler, HostIngressCredentialResolver,
    HostIngressError, HostIngressImmediateResponse, HostIngressRegistration, ResolvedIngressSecret,
    UnverifiedHostIngressRequest, VerifiedHostIngressRequest,
};
use crate::slack_serve::{
    ResolvedSlackIngress, SlackEventsWebhookDispatcher, SlackIngressError,
    SlackInstallationRateLimitConfig, SlackInstallationRateLimiter, SlackInstallationRecord,
    SlackInstallationResolver, SlackInstallationSelector, StaticSlackInstallationResolver,
};

pub const SLACK_EVENTS_HOST_INGRESS_ROUTE_ID: &str = "slack.events";
pub const SLACK_EVENTS_HOST_INGRESS_PATH: &str = crate::slack_serve::SLACK_EVENTS_PATH;

const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";
const SLACK_EVENTS_BODY_LIMIT_BYTES: u64 = 1024 * 1024;

type WorkflowObserver = dyn ImmediateAckWorkflowObserver;

#[derive(Debug, Clone)]
pub struct ExtensionInstallationIngressCredentialBinding {
    pub candidate_id: String,
    pub ingress_credential_handle: IngressCredentialHandle,
    pub secret_scope: ResourceScope,
    pub secret_handle: SecretHandle,
}

/// Serve wiring injects bindings derived from enabled `ExtensionInstallation::credential_bindings()`.
pub struct ExtensionInstallationIngressCredentialResolver {
    secret_store: Arc<dyn SecretStore>,
    bindings_by_candidate: HashMap<String, Vec<ExtensionInstallationIngressCredentialBinding>>,
}

impl ExtensionInstallationIngressCredentialResolver {
    pub fn new(
        secret_store: Arc<dyn SecretStore>,
        bindings: impl IntoIterator<Item = ExtensionInstallationIngressCredentialBinding>,
    ) -> Result<Self, HostIngressError> {
        let mut bindings_by_candidate: HashMap<String, Vec<_>> = HashMap::new();
        let mut seen = HashSet::new();
        for binding in bindings {
            if binding.candidate_id.trim().is_empty() {
                return Err(internal(
                    "Slack ingress credential binding candidate id must not be empty",
                ));
            }
            let key = (
                binding.candidate_id.clone(),
                binding.ingress_credential_handle.clone(),
            );
            if !seen.insert(key) {
                return Err(internal(
                    "duplicate Slack ingress credential binding for candidate and handle",
                ));
            }
            bindings_by_candidate
                .entry(binding.candidate_id.clone())
                .or_default()
                .push(binding);
        }
        Ok(Self {
            secret_store,
            bindings_by_candidate,
        })
    }
}

#[async_trait]
impl HostIngressCredentialResolver for ExtensionInstallationIngressCredentialResolver {
    async fn resolve_ingress_secret(
        &self,
        candidate: &HostIngressAuthCandidate,
        handle: &IngressCredentialHandle,
    ) -> Result<ResolvedIngressSecret, HostIngressError> {
        let binding = self
            .bindings_by_candidate
            .get(candidate.candidate_id())
            .and_then(|bindings| {
                bindings
                    .iter()
                    .find(|binding| binding.ingress_credential_handle == *handle)
            })
            .ok_or_else(|| {
                auth_failed(format!(
                    "missing Slack ingress credential binding for candidate `{}` handle `{}`",
                    candidate.candidate_id(),
                    handle
                ))
            })?;

        let lease = self
            .secret_store
            .lease_once(&binding.secret_scope, &binding.secret_handle)
            .await
            .map_err(|error| map_secret_store_error("lease Slack ingress secret", error))?;
        let material = self
            .secret_store
            .consume(&binding.secret_scope, lease.id)
            .await
            .map_err(|error| map_secret_store_error("consume Slack ingress secret", error))?;
        Ok(ResolvedIngressSecret::new(material))
    }
}

#[derive(Clone)]
pub struct SlackHostIngressInstallation {
    tenant_id: ironclaw_host_api::TenantId,
    adapter_installation_id: ironclaw_product_adapters::AdapterInstallationId,
    selector: SlackInstallationSelector,
    credential_handles: Vec<IngressCredentialHandle>,
    runner: Arc<NativeProductAdapterRunner>,
    workflow_observer: Option<Arc<WorkflowObserver>>,
}

impl SlackHostIngressInstallation {
    pub fn new(
        tenant_id: ironclaw_host_api::TenantId,
        adapter_installation_id: ironclaw_product_adapters::AdapterInstallationId,
        selector: SlackInstallationSelector,
        credential_handles: Vec<IngressCredentialHandle>,
        runner: Arc<NativeProductAdapterRunner>,
    ) -> Result<Self, HostIngressError> {
        if credential_handles.is_empty() {
            return Err(internal(
                "Slack ingress installation must declare at least one credential handle",
            ));
        }
        Ok(Self {
            tenant_id,
            adapter_installation_id,
            selector,
            credential_handles,
            runner,
            workflow_observer: None,
        })
    }

    pub fn with_workflow_observer(mut self, workflow_observer: Arc<WorkflowObserver>) -> Self {
        self.workflow_observer = Some(workflow_observer);
        self
    }

    async fn resolve_legacy_metadata(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ResolvedSlackIngress, SlackIngressError> {
        let resolver =
            StaticSlackInstallationResolver::new([self.legacy_record_for_candidate_selection()]);
        resolver.resolve_ingress(headers, body).await
    }

    fn legacy_record_for_candidate_selection(&self) -> SlackInstallationRecord {
        SlackInstallationRecord::new(
            self.tenant_id.clone(),
            self.adapter_installation_id.clone(),
            self.selector.clone(),
            Arc::new(CandidateSelectionDispatcher {
                subject: self.adapter_installation_id.as_str().to_string(),
            }),
        )
    }
}

pub struct SlackEventsIngressHandler {
    installations: Arc<[SlackHostIngressInstallation]>,
    installation_rate_limiter: SlackInstallationRateLimiter,
}

impl SlackEventsIngressHandler {
    pub fn new(
        installations: impl IntoIterator<Item = SlackHostIngressInstallation>,
    ) -> Result<Self, HostIngressError> {
        let installations: Vec<_> = installations.into_iter().collect();
        let mut seen = HashSet::new();
        for installation in &installations {
            let candidate_id = installation.adapter_installation_id.as_str();
            if !seen.insert(candidate_id.to_string()) {
                return Err(internal(format!(
                    "duplicate Slack ingress installation candidate id `{}`",
                    candidate_id
                )));
            }
        }
        Ok(Self {
            installations: installations.into(),
            installation_rate_limiter: SlackInstallationRateLimiter::new(
                SlackInstallationRateLimitConfig::default(),
            ),
        })
    }

    pub fn declared_credential_handles(&self) -> Vec<IngressCredentialHandle> {
        let mut seen = HashSet::new();
        let mut handles = Vec::new();
        for installation in self.installations.iter() {
            for handle in &installation.credential_handles {
                if seen.insert(handle.clone()) {
                    handles.push(handle.clone());
                }
            }
        }
        handles
    }

    fn installation_for_evidence(
        &self,
        evidence: &ProtocolAuthEvidence,
    ) -> Result<&SlackHostIngressInstallation, HostIngressError> {
        let subject = evidence
            .claim()
            .map(|claim| claim.subject())
            .ok_or_else(|| {
                auth_failed("Slack ingress request reached handler without verified auth evidence")
            })?;
        self.installations
            .iter()
            .find(|installation| installation.adapter_installation_id.as_str() == subject)
            .ok_or_else(|| {
                auth_failed(format!(
                    "Slack ingress verified candidate `{subject}` has no configured installation"
                ))
            })
    }
}

#[async_trait]
impl HostIngressCapabilityHandler for SlackEventsIngressHandler {
    async fn auth_candidates(
        &self,
        request: &UnverifiedHostIngressRequest<'_>,
    ) -> Result<Vec<HostIngressAuthCandidate>, HostIngressError> {
        let mut candidates = Vec::new();
        for (index, installation) in self.installations.iter().enumerate() {
            match installation
                .resolve_legacy_metadata(request.headers(), request.body())
                .await
            {
                Ok(ResolvedSlackIngress::UrlVerification { .. })
                | Ok(ResolvedSlackIngress::Event { .. }) => {
                    candidates.push(slack_hmac_auth_candidate(installation)?);
                }
                Err(SlackIngressError::InstallationNotFound) => {}
                Err(SlackIngressError::Envelope(_)) if index == 0 => {
                    candidates.push(slack_hmac_auth_candidate(installation)?);
                    break;
                }
                Err(SlackIngressError::Envelope(_)) => break,
                Err(error) => return Err(map_slack_ingress_error(error)),
            }
        }
        Ok(candidates)
    }

    async fn handle_verified(
        &self,
        request: VerifiedHostIngressRequest,
    ) -> Result<HostIngressImmediateResponse, HostIngressError> {
        let installation = self.installation_for_evidence(request.auth_evidence())?;
        let ingress = installation
            .resolve_legacy_metadata(request.headers(), request.body())
            .await
            .map_err(map_slack_ingress_error)?;
        self.installation_rate_limiter
            .check(ingress.installation())
            .map_err(map_slack_ingress_error)?;

        match ingress {
            ResolvedSlackIngress::UrlVerification { challenge, .. } => {
                Ok(HostIngressImmediateResponse::ok_body(challenge))
            }
            ResolvedSlackIngress::Event { .. } => {
                let outcome = installation
                    .runner
                    .process_verified_webhook_immediate_ack_with_observer(
                        request.body(),
                        request.auth_evidence(),
                        installation.workflow_observer.clone(),
                    )
                    .await
                    .map_err(map_runner_error)?;
                match outcome {
                    WebhookProcessOutcome::AcceptedForAsyncDispatch
                    | WebhookProcessOutcome::Acknowledged { .. } => {
                        Ok(HostIngressImmediateResponse::accepted())
                    }
                }
            }
        }
    }

    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let drains = self
                .installations
                .iter()
                .map(|installation| installation.runner.drain_immediate_ack_tasks());
            futures::future::join_all(drains).await;
        })
    }
}

pub fn slack_events_host_ingress_registrations(
    handler: Arc<SlackEventsIngressHandler>,
) -> Result<Vec<HostIngressRegistration>, HostIngressError> {
    let declaration = slack_events_host_ingress_declaration(handler.declared_credential_handles())?;
    let handler: Arc<dyn HostIngressCapabilityHandler> = handler;
    Ok(vec![HostIngressRegistration {
        declaration,
        handler,
    }])
}

pub fn slack_events_host_ingress_declaration(
    credential_handles: Vec<IngressCredentialHandle>,
) -> Result<HostIngressRouteDeclaration, HostIngressError> {
    if credential_handles.is_empty() {
        return Err(internal(
            "Slack events ingress declaration requires at least one credential handle",
        ));
    }
    let descriptor = IngressRouteDescriptor::new(
        SLACK_EVENTS_HOST_INGRESS_ROUTE_ID,
        NetworkMethod::Post,
        SLACK_EVENTS_HOST_INGRESS_PATH,
        slack_events_ingress_policy()?,
    )
    .map_err(|error| {
        internal(format!(
            "Slack events ingress descriptor did not validate: {error}"
        ))
    })?;
    let auth = IngressAuthBinding::new(slack_hmac_auth_scheme()?, credential_handles).map_err(
        |error| {
            internal(format!(
                "Slack events ingress auth binding did not validate: {error}"
            ))
        },
    )?;
    let capability_id = CapabilityId::new(SLACK_EVENTS_HOST_INGRESS_ROUTE_ID).map_err(|error| {
        internal(format!(
            "Slack events ingress capability id did not validate: {error}"
        ))
    })?;
    HostIngressRouteDeclaration::new(
        descriptor,
        HostIngressTarget::ProductAdapterInbound {
            capability_id,
            product_adapter_section: "product_adapter.inbound".to_string(),
        },
        vec![auth],
        IngressAckMode::Immediate,
        IngressDrainMode::DrainBeforeRuntimeShutdown,
    )
    .map_err(|error| {
        internal(format!(
            "Slack events ingress declaration did not validate: {error}"
        ))
    })
}

fn slack_hmac_auth_candidate(
    installation: &SlackHostIngressInstallation,
) -> Result<HostIngressAuthCandidate, HostIngressError> {
    let subject = installation.adapter_installation_id.as_str().to_string();
    HostIngressAuthCandidate::new(
        subject.clone(),
        slack_request_signature_auth_requirement(),
        installation.credential_handles.clone(),
        move |request, secret| verify_slack_hmac_candidate(request, secret, &subject),
    )
}

fn verify_slack_hmac_candidate(
    request: &UnverifiedHostIngressRequest<'_>,
    secret: &ResolvedIngressSecret,
    subject: &str,
) -> Result<bool, HostIngressError> {
    let verifier = HmacWebhookAuth::new(
        SLACK_SIGNATURE_HEADER,
        SLACK_TIMESTAMP_HEADER,
        secret.as_bytes().to_vec(),
        subject,
    );
    match verifier.verify(request.headers(), request.body()) {
        VerificationOutcome::Verified { .. } => Ok(true),
        VerificationOutcome::Failed { failure } => Err(auth_failed(format!(
            "Slack v0 HMAC verification failed: {failure}"
        ))),
    }
}

fn slack_events_ingress_policy() -> Result<IngressPolicy, HostIngressError> {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::WebhookSignature],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: nonzero_u64(SLACK_EVENTS_BODY_LIMIT_BYTES, "body_limit")?,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::Global,
            max_requests: nonzero_u32(12_000, "max_requests")?,
            window_seconds: nonzero_u32(60, "window_seconds")?,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .map_err(|error| {
        internal(format!(
            "Slack events ingress policy did not validate: {error}"
        ))
    })
}

fn internal(reason: impl Into<String>) -> HostIngressError {
    HostIngressError::Internal {
        reason: reason.into(),
    }
}

fn auth_failed(reason: impl Into<String>) -> HostIngressError {
    HostIngressError::AuthenticationFailed {
        reason: reason.into(),
    }
}

fn slack_hmac_auth_scheme() -> Result<IngressAuthSchemeName, HostIngressError> {
    IngressAuthSchemeName::new("slack_v0_hmac").map_err(|error| {
        internal(format!(
            "Slack events ingress auth scheme did not validate: {error}"
        ))
    })
}

fn nonzero_u64(value: u64, field: &'static str) -> Result<NonZeroU64, HostIngressError> {
    NonZeroU64::new(value)
        .ok_or_else(|| internal(format!("Slack events ingress {field} must be non-zero")))
}

fn nonzero_u32(value: u32, field: &'static str) -> Result<NonZeroU32, HostIngressError> {
    NonZeroU32::new(value)
        .ok_or_else(|| internal(format!("Slack events ingress {field} must be non-zero")))
}

fn map_secret_store_error(action: &'static str, error: SecretStoreError) -> HostIngressError {
    let reason = format!("{action}: {error}");
    match error {
        SecretStoreError::UnknownSecret { .. }
        | SecretStoreError::UnknownLease { .. }
        | SecretStoreError::LeaseConsumed { .. }
        | SecretStoreError::LeaseRevoked { .. }
        | SecretStoreError::LeaseExpired { .. }
        | SecretStoreError::SecretExpired => HostIngressError::AuthenticationFailed { reason },
        SecretStoreError::BackendMisconfigured { .. } => internal(reason),
        SecretStoreError::StoreUnavailable { .. } => {
            HostIngressError::TemporarilyUnavailable { reason }
        }
    }
}

fn map_slack_ingress_error(error: SlackIngressError) -> HostIngressError {
    match &error {
        SlackIngressError::Runner(error) => map_runner_error(error.clone()),
        SlackIngressError::Envelope(error) => HostIngressError::MalformedPayload {
            reason: error.to_string(),
        },
        SlackIngressError::InstallationNotFound | SlackIngressError::AmbiguousInstallation => {
            auth_failed(error.to_string())
        }
        SlackIngressError::InstallationRateLimited { .. } => HostIngressError::Capacity {
            reason: error.to_string(),
        },
    }
}

fn map_runner_error(error: RunnerError) -> HostIngressError {
    match &error {
        RunnerError::AuthenticationFailed { failure } => {
            auth_failed(format!("Slack runner authentication failed: {failure}"))
        }
        RunnerError::TooManyInFlight { .. } => HostIngressError::Capacity {
            reason: error.to_string(),
        },
        RunnerError::Adapter(adapter_error) if adapter_error.is_retryable() => {
            HostIngressError::TemporarilyUnavailable {
                reason: error.to_string(),
            }
        }
        RunnerError::WorkflowTimeout { .. }
        | RunnerError::WorkflowJoinFailed
        | RunnerError::WorkflowPanicked
        | RunnerError::AdapterPanicked => HostIngressError::TemporarilyUnavailable {
            reason: error.to_string(),
        },
        RunnerError::Adapter(adapter_error) => HostIngressError::MalformedPayload {
            reason: format!("Slack adapter rejected inbound payload: {adapter_error}"),
        },
    }
}

struct CandidateSelectionDispatcher {
    subject: String,
}

impl SlackEventsWebhookDispatcher for CandidateSelectionDispatcher {
    fn verify_webhook_auth(
        &self,
        _headers: &HeaderMap,
        _body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError> {
        Ok(mark_request_signature_verified(
            SLACK_SIGNATURE_HEADER,
            Some(SLACK_TIMESTAMP_HEADER.to_string()),
            self.subject.clone(),
        ))
    }

    fn process_verified_webhook_immediate_ack<'a>(
        &'a self,
        _body: &'a [u8],
        _evidence: &'a ProtocolAuthEvidence,
        _observer: Option<Arc<WorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        Box::pin(async { Err(RunnerError::AdapterPanicked) })
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::{Method, StatusCode};
    use hmac::{Hmac, Mac};
    use ironclaw_host_api::{InvocationId, TenantId, UserId};
    use ironclaw_product_adapters::EgressCredentialHandle;
    use ironclaw_product_adapters::identity::{AdapterInstallationId, ProductAdapterId};
    use ironclaw_product_adapters::{
        ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductWorkflow,
    };
    use ironclaw_secrets::{InMemorySecretStore, SecretMaterial};
    use ironclaw_slack_v2_adapter::{SlackV2Adapter, SlackV2AdapterConfig};
    use sha2::Sha256;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use crate::host_ingress::public_ingress_route_mount;

    use super::*;

    const SIGNING_SECRET: &str = "test-signing-secret";
    const EVENT_BODY: &str = r#"{"type":"event_callback","team_id":"T123","api_app_id":"A123","event_id":"Ev1","event":{"type":"message","channel":"D123","user":"U123","text":"hello","channel_type":"im"},"authorizations":[{"team_id":"T123","user_id":"Ubot"}]}"#;

    struct TestMount(
        crate::webui_serve::PublicRouteMount,
        Arc<AtomicUsize>,
        Option<Arc<Notify>>,
        Option<Arc<AtomicUsize>>,
    );

    struct RecordingWorkflow {
        count: Arc<AtomicUsize>,
        entered: Option<Arc<AtomicUsize>>,
        release: Option<Arc<Notify>>,
    }

    #[async_trait]
    impl ProductWorkflow for RecordingWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = &self.entered {
                entered.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(release) = &self.release {
                release.notified().await;
            }
            Ok(ProductInboundAck::NoOp)
        }
    }

    async fn test_mount(blocking_workflow: bool) -> TestMount {
        let workflow_count = Arc::new(AtomicUsize::new(0));
        let release = blocking_workflow.then(|| Arc::new(Notify::new()));
        let entered = blocking_workflow.then(|| Arc::new(AtomicUsize::new(0)));
        let installation_id =
            AdapterInstallationId::new("install_alpha").expect("valid installation id");
        let adapter = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
            adapter_id: ProductAdapterId::new("slack_v2").expect("valid adapter id"),
            installation_id: installation_id.clone(),
            egress_credential_handle: EgressCredentialHandle::new("slack_bot_token")
                .expect("valid egress credential handle"),
            auth_requirement: slack_request_signature_auth_requirement(),
        }));
        let workflow = Arc::new(RecordingWorkflow {
            count: Arc::clone(&workflow_count),
            entered: entered.clone(),
            release: release.clone(),
        });
        let runner = Arc::new(NativeProductAdapterRunner::with_config(
            adapter,
            workflow,
            ironclaw_wasm_product_adapters::WebhookAuth::Hmac(HmacWebhookAuth::new(
                SLACK_SIGNATURE_HEADER,
                SLACK_TIMESTAMP_HEADER,
                SIGNING_SECRET.as_bytes().to_vec(),
                installation_id.as_str(),
            )),
            ironclaw_wasm_product_adapters::NativeProductAdapterRunnerConfig::new(
                Duration::from_secs(1),
                std::num::NonZeroUsize::new(2).expect("non-zero max in-flight"),
            ),
        ));
        let credential_handle =
            IngressCredentialHandle::new("slack_signing_secret").expect("valid ingress handle");
        let installation = SlackHostIngressInstallation::new(
            TenantId::new("tenant").expect("valid tenant id"),
            installation_id.clone(),
            SlackInstallationSelector::team("T123"),
            vec![credential_handle.clone()],
            runner,
        )
        .expect("valid Slack ingress installation");
        let handler = Arc::new(
            SlackEventsIngressHandler::new([installation])
                .expect("valid Slack events ingress handler"),
        );
        let secret_scope = ResourceScope::local_default(
            UserId::new("user").expect("valid user id"),
            InvocationId::new(),
        )
        .expect("valid local default scope");
        let secret_handle = SecretHandle::new("slack_signing_secret").expect("valid secret handle");
        let secret_store = Arc::new(InMemorySecretStore::new());
        secret_store
            .put(
                secret_scope.clone(),
                secret_handle.clone(),
                SecretMaterial::from(SIGNING_SECRET.to_string()),
            )
            .await
            .expect("seed signing secret");
        let resolver = Arc::new(
            ExtensionInstallationIngressCredentialResolver::new(
                secret_store,
                [ExtensionInstallationIngressCredentialBinding {
                    candidate_id: installation_id.as_str().to_string(),
                    ingress_credential_handle: credential_handle,
                    secret_scope,
                    secret_handle,
                }],
            )
            .expect("valid ingress credential resolver"),
        );
        let registrations =
            slack_events_host_ingress_registrations(handler).expect("Slack ingress registrations");
        let mount = public_ingress_route_mount(registrations, resolver)
            .expect("public ingress mount should build");
        TestMount(mount, workflow_count, release, entered)
    }

    async fn post_to_mount(
        mount: &crate::webui_serve::PublicRouteMount,
        body: &'static str,
        signing_secret: &str,
    ) -> axum::response::Response {
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let request = Request::builder()
            .method(Method::POST)
            .uri(SLACK_EVENTS_HOST_INGRESS_PATH)
            .header(SLACK_TIMESTAMP_HEADER, timestamp.as_str())
            .header(
                SLACK_SIGNATURE_HEADER,
                slack_signature(signing_secret, &timestamp, body),
            )
            .body(Body::from(body))
            .expect("valid request");
        mount
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("route response")
    }

    fn slack_signature(signing_secret: &str, timestamp: &str, body: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes())
            .expect("HMAC accepts arbitrary non-empty key lengths");
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body.as_bytes());
        let hex = b"0123456789abcdef";
        let mut encoded = String::with_capacity(64);
        for byte in mac.finalize().into_bytes() {
            encoded.push(hex[(byte >> 4) as usize] as char);
            encoded.push(hex[(byte & 0x0f) as usize] as char);
        }
        format!("v0={encoded}")
    }

    #[tokio::test]
    async fn slack_host_ingress_url_verification_echoes_challenge() {
        let test = test_mount(false).await;
        let body = r#"{"type":"url_verification","challenge":"challenge-token","team_id":"T123"}"#;

        let response = post_to_mount(&test.0, body, SIGNING_SECRET).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("response body");
        assert_eq!(body.as_ref(), b"challenge-token");
        assert_eq!(test.1.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slack_host_ingress_valid_signed_dm_event_dispatches_runner() {
        let test = test_mount(false).await;

        let response = post_to_mount(&test.0, EVENT_BODY, SIGNING_SECRET).await;
        assert_eq!(response.status(), StatusCode::OK);
        test.0
            .drain
            .as_ref()
            .expect("mount has drain")
            .drain()
            .await;
        assert_eq!(test.1.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn slack_host_ingress_forged_signature_returns_401_without_dispatch() {
        let test = test_mount(false).await;

        let response = post_to_mount(&test.0, EVENT_BODY, "wrong-secret").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(test.1.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slack_host_ingress_drain_waits_for_in_flight_runner_work() {
        let test = test_mount(true).await;
        let entered = test.3.clone().expect("blocking workflow entered counter");
        let release = test.2.clone().expect("blocking workflow release");

        let response = post_to_mount(&test.0, EVENT_BODY, SIGNING_SECRET).await;
        assert_eq!(response.status(), StatusCode::OK);
        while entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let drain = test.0.drain.clone().expect("mount has drain");
        let drain_task = tokio::spawn(async move {
            drain.drain().await;
        });
        tokio::task::yield_now().await;
        assert!(!drain_task.is_finished());
        release.notify_waiters();
        drain_task.await.expect("drain task should finish");
        assert_eq!(test.1.load(Ordering::SeqCst), 1);
    }
}
