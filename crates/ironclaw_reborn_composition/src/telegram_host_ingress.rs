//! Telegram Bot API webhook host-ingress bridge.
//!
//! Mirrors the Slack host-ingress bridge but for Telegram's much simpler auth
//! model: inbound updates are authenticated by a shared-secret header
//! (`X-Telegram-Bot-Api-Secret-Token`) that the bot owner configured with
//! `setWebhook`. The host verifies the header through the shared
//! [`SharedSecretHeaderAuth`] verifier and mints `ProtocolAuthEvidence` *before*
//! the Telegram adapter ever parses the update body
//! (`ironclaw_telegram_v2_adapter` rejects unverified evidence).
//!
//! Unlike Slack, a Telegram update body does not reliably identify which bot
//! installation it targets before authentication, so [`auth_candidates`] offers
//! every enabled installation as a candidate; the generic engine resolves each
//! candidate's secret, runs the shared-secret verifier, caps the fan-out, and
//! fails closed on zero or more-than-one match.
//!
//! [`auth_candidates`]: TelegramUpdatesIngressHandler::auth_candidates

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::num::{NonZeroU32, NonZeroU64};
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, HostIngressRouteDeclaration,
    HostIngressTarget, IngressAckMode, IngressAuthBinding, IngressAuthPolicy, IngressAuthScheme,
    IngressCredentialHandle, IngressDrainMode, IngressPolicy, IngressPolicyParts,
    IngressRouteDescriptor, IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope,
    StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{CapabilityId, NetworkMethod, ResourceScope, SecretHandle};
use ironclaw_product_adapters::{AdapterInstallationId, AuthRequirement, ProtocolAuthEvidence};
use ironclaw_secrets::{SecretStore, SecretStoreError};
use ironclaw_wasm_product_adapters::{
    ImmediateAckWorkflowObserver, NativeProductAdapterRunner, RunnerError, SharedSecretHeaderAuth,
    VerificationOutcome, WebhookAuthVerifier, WebhookProcessOutcome,
};

use crate::host_ingress::{
    HostIngressAuthCandidate, HostIngressCapabilityHandler, HostIngressCredentialResolver,
    HostIngressError, HostIngressImmediateResponse, ResolvedIngressSecret,
    UnverifiedHostIngressRequest, VerifiedHostIngressRequest,
};

/// Stable route id for the Telegram updates webhook host-ingress route.
pub const TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID: &str = "telegram.updates";
/// Default mounted path. Preserves the legacy `/webhook/telegram` expectation so
/// a bot whose `setWebhook` already points there keeps working.
pub const TELEGRAM_UPDATES_HOST_INGRESS_PATH: &str = "/webhooks/telegram/updates";
/// Header carrying the per-installation webhook shared secret.
pub const TELEGRAM_WEBHOOK_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";
/// Telegram updates can carry large payloads (e.g. captions, inline data); cap
/// at 1 MiB to bound host memory while comfortably covering real updates.
const TELEGRAM_UPDATES_BODY_LIMIT_BYTES: u64 = 1024 * 1024;
const TELEGRAM_UPDATES_MAX_REQUESTS_PER_WINDOW: u32 = 12_000;
const TELEGRAM_UPDATES_RATE_WINDOW_SECONDS: u32 = 60;

type WorkflowObserver = dyn ImmediateAckWorkflowObserver;

/// One enabled Telegram bot installation reachable through the shared updates
/// webhook route. The `candidate_id`/evidence subject is the adapter
/// installation id, so `handle_verified` can route a verified request back to
/// the runner that owns it.
#[derive(Clone)]
pub struct TelegramHostIngressInstallation {
    adapter_installation_id: AdapterInstallationId,
    credential_handles: Vec<IngressCredentialHandle>,
    runner: Arc<NativeProductAdapterRunner>,
    workflow_observer: Option<Arc<WorkflowObserver>>,
}

impl TelegramHostIngressInstallation {
    pub fn new(
        adapter_installation_id: AdapterInstallationId,
        credential_handles: Vec<IngressCredentialHandle>,
        runner: Arc<NativeProductAdapterRunner>,
    ) -> Result<Self, HostIngressError> {
        if credential_handles.is_empty() {
            return Err(internal(
                "Telegram ingress installation must declare at least one credential handle",
            ));
        }
        Ok(Self {
            adapter_installation_id,
            credential_handles,
            runner,
            workflow_observer: None,
        })
    }

    pub fn with_workflow_observer(mut self, workflow_observer: Arc<WorkflowObserver>) -> Self {
        self.workflow_observer = Some(workflow_observer);
        self
    }

    fn candidate_id(&self) -> &str {
        self.adapter_installation_id.as_str()
    }
}

pub struct TelegramUpdatesIngressHandler {
    installations: Arc<[TelegramHostIngressInstallation]>,
}

impl TelegramUpdatesIngressHandler {
    pub fn new(
        installations: impl IntoIterator<Item = TelegramHostIngressInstallation>,
    ) -> Result<Self, HostIngressError> {
        let installations: Vec<_> = installations.into_iter().collect();
        if installations.is_empty() {
            return Err(internal(
                "Telegram updates ingress handler requires at least one installation",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for installation in &installations {
            if !seen.insert(installation.candidate_id().to_string()) {
                return Err(internal(format!(
                    "duplicate Telegram ingress installation candidate id `{}`",
                    installation.candidate_id()
                )));
            }
        }
        Ok(Self {
            installations: installations.into(),
        })
    }

    pub fn declared_credential_handles(&self) -> Vec<IngressCredentialHandle> {
        let mut seen = std::collections::HashSet::new();
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
    ) -> Result<&TelegramHostIngressInstallation, HostIngressError> {
        let subject = evidence
            .claim()
            .map(|claim| claim.subject())
            .ok_or_else(|| {
                auth_failed(
                    "Telegram ingress request reached handler without verified auth evidence",
                )
            })?;
        self.installations
            .iter()
            .find(|installation| installation.candidate_id() == subject)
            .ok_or_else(|| {
                auth_failed(format!(
                    "Telegram ingress verified candidate `{subject}` has no configured installation"
                ))
            })
    }
}

#[async_trait]
impl HostIngressCapabilityHandler for TelegramUpdatesIngressHandler {
    async fn auth_candidates(
        &self,
        _request: &UnverifiedHostIngressRequest<'_>,
    ) -> Result<Vec<HostIngressAuthCandidate>, HostIngressError> {
        // A Telegram update body does not identify the bot installation before
        // authentication, so every enabled installation is a candidate. The
        // engine caps the fan-out and requires exactly one to verify.
        self.installations
            .iter()
            .map(telegram_shared_secret_auth_candidate)
            .collect()
    }

    async fn handle_verified(
        &self,
        request: VerifiedHostIngressRequest,
    ) -> Result<HostIngressImmediateResponse, HostIngressError> {
        let installation = self.installation_for_evidence(request.auth_evidence())?;
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

pub fn telegram_updates_host_ingress_declaration(
    credential_handles: Vec<IngressCredentialHandle>,
) -> Result<HostIngressRouteDeclaration, HostIngressError> {
    if credential_handles.is_empty() {
        return Err(internal(
            "Telegram updates ingress declaration requires at least one credential handle",
        ));
    }
    let descriptor = IngressRouteDescriptor::new(
        TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID,
        NetworkMethod::Post,
        TELEGRAM_UPDATES_HOST_INGRESS_PATH,
        telegram_updates_ingress_policy()?,
    )
    .map_err(|error| {
        internal(format!(
            "Telegram updates ingress descriptor did not validate: {error}"
        ))
    })?;
    let auth = IngressAuthBinding::new(IngressAuthScheme::SharedSecretHeader, credential_handles)
        .map_err(|error| {
        internal(format!(
            "Telegram updates ingress auth binding did not validate: {error}"
        ))
    })?;
    let capability_id =
        CapabilityId::new(TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID).map_err(|error| {
            internal(format!(
                "Telegram updates ingress capability id did not validate: {error}"
            ))
        })?;
    HostIngressRouteDeclaration::new(
        descriptor,
        HostIngressTarget::ProductAdapterInbound {
            capability_id,
            product_adapter_section: "updates".to_string(),
        },
        vec![auth],
        IngressAckMode::Immediate,
        IngressDrainMode::DrainBeforeRuntimeShutdown,
    )
    .map_err(|error| {
        internal(format!(
            "Telegram updates ingress declaration did not validate: {error}"
        ))
    })
}

fn telegram_shared_secret_auth_candidate(
    installation: &TelegramHostIngressInstallation,
) -> Result<HostIngressAuthCandidate, HostIngressError> {
    let subject = installation.candidate_id().to_string();
    let header_name = TELEGRAM_WEBHOOK_SECRET_HEADER.to_string();
    let verifier_subject = subject.clone();
    HostIngressAuthCandidate::new(
        subject,
        AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_WEBHOOK_SECRET_HEADER.to_string(),
        },
        installation.credential_handles.clone(),
        move |request, secret| {
            verify_telegram_shared_secret(request, secret, &header_name, &verifier_subject)
        },
    )
}

fn verify_telegram_shared_secret(
    request: &UnverifiedHostIngressRequest<'_>,
    secret: &ResolvedIngressSecret,
    header_name: &str,
    subject: &str,
) -> Result<bool, HostIngressError> {
    let verifier = SharedSecretHeaderAuth {
        header_name: header_name.to_string(),
        expected_secret: secret.expose_secret().to_string(),
        subject: subject.to_string(),
    };
    match verifier.verify(request.headers(), request.body()) {
        VerificationOutcome::Verified { .. } => Ok(true),
        VerificationOutcome::Failed { failure } => Err(auth_failed(format!(
            "Telegram shared-secret header verification failed: {failure}"
        ))),
    }
}

fn telegram_updates_ingress_policy() -> Result<IngressPolicy, HostIngressError> {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::SharedSecretHeader],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: nonzero_u64(TELEGRAM_UPDATES_BODY_LIMIT_BYTES, "body_limit")?,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::Global,
            max_requests: nonzero_u32(TELEGRAM_UPDATES_MAX_REQUESTS_PER_WINDOW, "max_requests")?,
            window_seconds: nonzero_u32(TELEGRAM_UPDATES_RATE_WINDOW_SECONDS, "window_seconds")?,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .map_err(|error| {
        internal(format!(
            "Telegram updates ingress policy did not validate: {error}"
        ))
    })
}

fn map_runner_error(error: RunnerError) -> HostIngressError {
    match &error {
        RunnerError::AuthenticationFailed { failure } => {
            auth_failed(format!("Telegram runner authentication failed: {failure}"))
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
            reason: format!("Telegram adapter rejected inbound payload: {adapter_error}"),
        },
    }
}

fn nonzero_u64(value: u64, field: &'static str) -> Result<NonZeroU64, HostIngressError> {
    NonZeroU64::new(value)
        .ok_or_else(|| internal(format!("Telegram updates ingress {field} must be non-zero")))
}

fn nonzero_u32(value: u32, field: &'static str) -> Result<NonZeroU32, HostIngressError> {
    NonZeroU32::new(value)
        .ok_or_else(|| internal(format!("Telegram updates ingress {field} must be non-zero")))
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

/// A binding from a host-ingress auth candidate + credential handle to a
/// concrete secret-store scope/handle. Protocol-agnostic; serve wiring injects
/// bindings derived from enabled `ExtensionInstallation::credential_bindings()`.
#[derive(Debug, Clone)]
pub struct ExtensionInstallationIngressCredentialBinding {
    pub candidate_id: String,
    pub ingress_credential_handle: IngressCredentialHandle,
    pub secret_scope: ResourceScope,
    pub secret_handle: SecretHandle,
}

/// Resolves host-ingress credential handles to leased secret material through
/// the durable secret store, keyed by candidate id + credential handle.
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
                    "host ingress credential binding candidate id must not be empty",
                ));
            }
            let key = (
                binding.candidate_id.clone(),
                binding.ingress_credential_handle.clone(),
            );
            if !seen.insert(key) {
                return Err(internal(
                    "duplicate host ingress credential binding for candidate and handle",
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
                    "missing host ingress credential binding for candidate `{}` handle `{}`",
                    candidate.candidate_id(),
                    handle
                ))
            })?;
        let lease = self
            .secret_store
            .lease_once(&binding.secret_scope, &binding.secret_handle)
            .await
            .map_err(|error| map_secret_store_error("lease host ingress secret", error))?;
        let material = self
            .secret_store
            .consume(&binding.secret_scope, lease.id)
            .await
            .map_err(|error| map_secret_store_error("consume host ingress secret", error))?;
        Ok(ResolvedIngressSecret::new(material))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    use super::*;

    fn headers_with_secret(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-telegram-bot-api-secret-token"),
            HeaderValue::from_str(value).expect("valid header value"),
        );
        headers
    }

    fn handle() -> IngressCredentialHandle {
        IngressCredentialHandle::new("telegram_webhook_secret").expect("valid handle")
    }

    #[test]
    fn telegram_updates_declaration_has_expected_shape() {
        let declaration =
            telegram_updates_host_ingress_declaration(vec![handle()]).expect("declaration builds");

        assert_eq!(declaration.route().route_id().as_str(), "telegram.updates");
        assert_eq!(declaration.route().method(), NetworkMethod::Post);
        // Must byte-match the registry `telegram_updates` profile path + the
        // bundled manifest, or the projected mount and the handler declaration
        // would diverge.
        assert_eq!(
            declaration.route().route_pattern().as_str(),
            "/webhooks/telegram/updates"
        );
        assert_eq!(
            declaration.route().route_pattern().as_str(),
            TELEGRAM_UPDATES_HOST_INGRESS_PATH
        );
        assert_eq!(declaration.ack(), IngressAckMode::Immediate);
        assert_eq!(
            declaration.drain(),
            IngressDrainMode::DrainBeforeRuntimeShutdown
        );
        match declaration.target() {
            HostIngressTarget::ProductAdapterInbound {
                product_adapter_section,
                ..
            } => assert_eq!(product_adapter_section, "updates"),
            other => panic!("unexpected ingress target: {other:?}"),
        }
        assert_eq!(declaration.auth().len(), 1);
        assert_eq!(
            declaration.auth()[0].verifier(),
            IngressAuthScheme::SharedSecretHeader
        );
    }

    #[test]
    fn telegram_updates_declaration_requires_credential_handle() {
        let error = telegram_updates_host_ingress_declaration(Vec::new())
            .expect_err("declaration without credential handles must reject");
        assert!(matches!(error, HostIngressError::Internal { .. }));
    }

    #[test]
    fn verify_telegram_shared_secret_accepts_matching_secret() {
        let headers = headers_with_secret("s3cr3t-token");
        let request = UnverifiedHostIngressRequest::new(&headers, b"{\"update_id\":1}");
        let secret = ResolvedIngressSecret::from_plaintext("s3cr3t-token");

        let verified = verify_telegram_shared_secret(
            &request,
            &secret,
            TELEGRAM_WEBHOOK_SECRET_HEADER,
            "telegram-default",
        )
        .expect("matching secret verifies");

        assert!(verified);
    }

    #[test]
    fn verify_telegram_shared_secret_rejects_wrong_secret() {
        let headers = headers_with_secret("attacker-supplied");
        let request = UnverifiedHostIngressRequest::new(&headers, b"{}");
        let secret = ResolvedIngressSecret::from_plaintext("real-secret");

        let error = verify_telegram_shared_secret(
            &request,
            &secret,
            TELEGRAM_WEBHOOK_SECRET_HEADER,
            "telegram-default",
        )
        .expect_err("wrong secret must fail closed");

        assert!(matches!(
            error,
            HostIngressError::AuthenticationFailed { .. }
        ));
    }

    #[test]
    fn verify_telegram_shared_secret_rejects_missing_header() {
        let headers = HeaderMap::new();
        let request = UnverifiedHostIngressRequest::new(&headers, b"{}");
        let secret = ResolvedIngressSecret::from_plaintext("real-secret");

        let error = verify_telegram_shared_secret(
            &request,
            &secret,
            TELEGRAM_WEBHOOK_SECRET_HEADER,
            "telegram-default",
        )
        .expect_err("missing header must fail closed");

        assert!(matches!(
            error,
            HostIngressError::AuthenticationFailed { .. }
        ));
    }
}
