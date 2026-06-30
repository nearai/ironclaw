//! Slack personal binding by pairing challenge.
//!
//! This is the Reborn equivalent of v1 Slack pairing: a signed Slack event for
//! an unbound actor issues a short code, and the authenticated WebUI user
//! redeems that code to bind their Reborn user to the Slack actor.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductWorkflowError,
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};
use thiserror::Error;

use crate::slack_actor_identity::RebornUserIdentityLookup;
use crate::slack_outbound_targets::{
    SlackPersonalDmTarget, SlackPersonalDmTargetError, SlackPersonalDmTargetProvisioner,
};
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, SlackPersonalBindingPrincipal, SlackPersonalUserBindingError,
    SlackPersonalUserBindingService,
};
use crate::slack_serve::SlackUserId;

const SLACK_PAIRING_CODE_MIN_LEN: usize = 8;
const SLACK_PAIRING_CODE_MAX_LEN: usize = 32;
const SLACK_PAIRING_CHALLENGE_DEDUP_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlackPersonalBindingPairingCode(String);

impl SlackPersonalBindingPairingCode {
    pub fn new(value: impl Into<String>) -> Result<Self, SlackPersonalBindingPairingError> {
        let value = value.into();
        let normalized = value.trim().to_ascii_uppercase();
        if normalized.is_empty() {
            return Err(SlackPersonalBindingPairingError::InvalidCode {
                reason: "pairing code is required",
            });
        }
        if normalized.len() < SLACK_PAIRING_CODE_MIN_LEN
            || normalized.len() > SLACK_PAIRING_CODE_MAX_LEN
            || !normalized.chars().all(|ch| ch.is_ascii_alphanumeric())
        {
            return Err(SlackPersonalBindingPairingError::InvalidCode {
                reason: "pairing code must be 8-32 ASCII letters or digits",
            });
        }
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SlackPersonalBindingPairingCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalBindingPairingChallenge {
    pub installation_id: AdapterInstallationId,
    pub slack_user_id: SlackUserId,
    pub setup_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedSlackPersonalBindingPairingChallenge {
    pub code: SlackPersonalBindingPairingCode,
    pub challenge: SlackPersonalBindingPairingChallenge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalBindingPairingNotification {
    pub installation_id: AdapterInstallationId,
    pub slack_user_id: SlackUserId,
    pub code: SlackPersonalBindingPairingCode,
}

#[derive(Debug, Error)]
pub enum SlackPersonalBindingPairingError {
    #[error("invalid slack personal binding pairing code: {reason}")]
    InvalidCode { reason: &'static str },
    #[error("slack personal binding pairing challenge was not found or expired")]
    ChallengeNotFound,
    #[error("slack personal binding pairing backend unavailable: {0}")]
    Backend(String),
    #[error(transparent)]
    Binding(#[from] SlackPersonalUserBindingError),
}

#[async_trait::async_trait]
pub trait SlackPersonalBindingPairingChallengeStore: Send + Sync {
    async fn issue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>;

    async fn get_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>;

    async fn consume_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>;
}

#[async_trait::async_trait]
pub trait SlackPersonalBindingPairingNotifier: Send + Sync {
    async fn send_pairing_challenge(
        &self,
        notification: SlackPersonalBindingPairingNotification,
    ) -> Result<(), SlackPersonalBindingPairingError>;
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalDmTargetProvisioning: Send + Sync + std::fmt::Debug {
    async fn provision_for_user(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError>;
}

#[async_trait::async_trait]
impl SlackPersonalDmTargetProvisioning for SlackPersonalDmTargetProvisioner {
    async fn provision_for_user(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        SlackPersonalDmTargetProvisioner::provision_for_user(self, user_id, slack_user_id).await
    }
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalUserBinder: Send + Sync + std::fmt::Debug {
    async fn validate_installation_actor(
        &self,
        principal: &SlackPersonalBindingPrincipal,
        installation_id: &AdapterInstallationId,
        slack_user_id: &SlackUserId,
    ) -> Result<(), SlackPersonalUserBindingError>;

    async fn bind_installation_actor(
        &self,
        principal: SlackPersonalBindingPrincipal,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError>;
}

#[async_trait::async_trait]
impl SlackPersonalUserBinder for SlackPersonalUserBindingService {
    async fn validate_installation_actor(
        &self,
        principal: &SlackPersonalBindingPrincipal,
        installation_id: &AdapterInstallationId,
        slack_user_id: &SlackUserId,
    ) -> Result<(), SlackPersonalUserBindingError> {
        SlackPersonalUserBindingService::validate_installation_actor(
            self,
            principal,
            installation_id,
            slack_user_id,
        )
    }

    async fn bind_installation_actor(
        &self,
        principal: SlackPersonalBindingPrincipal,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        SlackPersonalUserBindingService::bind_installation_actor(
            self,
            principal,
            installation_id,
            slack_user_id,
        )
        .await
    }
}

#[derive(Clone)]
pub struct SlackPersonalBindingPairingService {
    binding_service: Arc<dyn SlackPersonalUserBinder>,
    challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
    notifier: Arc<dyn SlackPersonalBindingPairingNotifier>,
    dm_provisioner: Option<Arc<dyn SlackPersonalDmTargetProvisioning>>,
}

impl SlackPersonalBindingPairingService {
    pub fn new(
        binding_service: SlackPersonalUserBindingService,
        challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
        notifier: Arc<dyn SlackPersonalBindingPairingNotifier>,
    ) -> Self {
        Self::new_with_binder(Arc::new(binding_service), challenge_store, notifier)
    }

    pub(crate) fn new_with_binder(
        binding_service: Arc<dyn SlackPersonalUserBinder>,
        challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
        notifier: Arc<dyn SlackPersonalBindingPairingNotifier>,
    ) -> Self {
        Self {
            binding_service,
            challenge_store,
            notifier,
            dm_provisioner: None,
        }
    }

    /// Attach a DM provisioner that is called after successful pairing-code
    /// redemption.  Provisioning runs in a background task: failure is logged
    /// and never blocks or fails the redemption itself.
    pub(crate) fn with_dm_provisioner(
        mut self,
        provisioner: Arc<dyn SlackPersonalDmTargetProvisioning>,
    ) -> Self {
        self.dm_provisioner = Some(provisioner);
        self
    }

    pub async fn issue_challenge(
        &self,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let issued = self
            .challenge_store
            .issue_challenge(SlackPersonalBindingPairingChallenge {
                installation_id,
                slack_user_id,
                setup_revision: None,
            })
            .await?;
        self.notifier
            .send_pairing_challenge(SlackPersonalBindingPairingNotification {
                installation_id: issued.challenge.installation_id.clone(),
                slack_user_id: issued.challenge.slack_user_id.clone(),
                code: issued.code.clone(),
            })
            .await?;
        Ok(issued)
    }

    pub async fn redeem_challenge(
        &self,
        principal: SlackPersonalBindingPrincipal,
        code: SlackPersonalBindingPairingCode,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalBindingPairingError> {
        let preview = self.challenge_store.get_challenge(&code).await?;
        self.binding_service
            .validate_installation_actor(
                &principal,
                &preview.installation_id,
                &preview.slack_user_id,
            )
            .await
            .map_err(SlackPersonalBindingPairingError::Binding)?;
        let challenge = self.challenge_store.consume_challenge(&code).await?;
        let slack_user_id = challenge.slack_user_id.clone();
        let binding = self
            .binding_service
            .bind_installation_actor(
                principal,
                challenge.installation_id,
                challenge.slack_user_id,
            )
            .await
            .map_err(SlackPersonalBindingPairingError::Binding)?;
        if let Some(provisioner) = self.dm_provisioner.clone() {
            let user_id = binding.user_id.clone();
            tokio::spawn(async move {
                match provisioner.provision_for_user(user_id, slack_user_id).await {
                    Ok(_) => {
                        tracing::debug!("Slack personal DM target provisioned after pairing");
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "Slack personal DM target provisioning failed after pairing; \
                             will retry on next pairing event"
                        );
                    }
                }
            });
        }
        Ok(binding)
    }
}

impl std::fmt::Debug for SlackPersonalBindingPairingService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackPersonalBindingPairingService")
            .field("binding_service", &self.binding_service)
            .field(
                "challenge_store",
                &"Arc<dyn SlackPersonalBindingPairingChallengeStore>",
            )
            .field("notifier", &"Arc<dyn SlackPersonalBindingPairingNotifier>")
            .field(
                "dm_provisioner",
                if self.dm_provisioner.is_some() {
                    &"Some(SlackPersonalDmTargetProvisioner)"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct SlackPairingActorResolver {
    lookup: Arc<dyn RebornUserIdentityLookup>,
    pairing: SlackPersonalBindingPairingService,
    pending_challenge_cache: Arc<Mutex<HashMap<SlackPairingChallengeCacheKey, Instant>>>,
    challenge_dedup_ttl: Duration,
}

impl SlackPairingActorResolver {
    pub fn new(
        lookup: Arc<dyn RebornUserIdentityLookup>,
        pairing: SlackPersonalBindingPairingService,
    ) -> Self {
        Self {
            lookup,
            pairing,
            pending_challenge_cache: Arc::new(Mutex::new(HashMap::new())),
            challenge_dedup_ttl: SLACK_PAIRING_CHALLENGE_DEDUP_TTL,
        }
    }

    fn reserve_pairing_challenge(
        &self,
        key: SlackPairingChallengeCacheKey,
    ) -> Result<bool, ProductWorkflowError> {
        let mut cache = self.pending_challenge_cache.lock().map_err(|_| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: "slack pairing challenge cache lock poisoned".into(),
            }
        })?;
        let now = Instant::now();
        cache.retain(|_, expires_at| *expires_at > now);
        if cache.contains_key(&key) {
            return Ok(false);
        }
        cache.insert(key, now + self.challenge_dedup_ttl);
        Ok(true)
    }

    fn clear_pairing_challenge_reservation(
        &self,
        key: &SlackPairingChallengeCacheKey,
    ) -> Result<(), ProductWorkflowError> {
        self.pending_challenge_cache
            .lock()
            .map_err(|_| ProductWorkflowError::BindingResolutionFailed {
                reason: "slack pairing challenge cache lock poisoned".into(),
            })?
            .remove(key);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SlackPairingChallengeCacheKey {
    installation_id: AdapterInstallationId,
    slack_user_id: SlackUserId,
}

impl std::fmt::Debug for SlackPairingActorResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SlackPairingActorResolver(..)")
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for SlackPairingActorResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.adapter_id.as_str() != SLACK_V2_ADAPTER_ID
            || request.external_actor_ref.kind() != SLACK_USER_ACTOR_KIND
        {
            return Ok(None);
        }

        let provider_user_id = crate::slack_actor_identity::slack_user_identity_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        let resolved = self
            .lookup
            .resolve_user_identity(
                crate::slack_actor_identity::SLACK_IDENTITY_PROVIDER,
                &provider_user_id,
            )
            .await
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            })?;
        if resolved.is_some() {
            return Ok(resolved);
        }

        let key = SlackPairingChallengeCacheKey {
            installation_id: request.installation_id,
            slack_user_id: SlackUserId::new(request.external_actor_ref.id()),
        };
        if !self.reserve_pairing_challenge(key.clone())? {
            return Ok(None);
        }

        if let Err(error) = self
            .pairing
            .issue_challenge(key.installation_id.clone(), key.slack_user_id.clone())
            .await
        {
            self.clear_pairing_challenge_reservation(&key)?;
            return Err(ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            });
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_product_adapters::{ExternalActorRef, ProductAdapterId};

    use super::*;
    use crate::slack_actor_identity::RebornUserIdentityLookupError;
    use crate::slack_personal_binding::{
        RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
        RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
        SlackPersonalBindingInstallation, SlackPersonalUserBindingService,
    };
    use crate::slack_serve::SlackInstallationSelector;

    #[tokio::test]
    async fn redeem_challenge_binds_authenticated_user_to_slack_actor() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = SlackPersonalBindingPairingService::new(
            binding_service(store.clone()),
            Arc::new(StaticChallengeStore::new(
                "ABC12345",
                SlackPersonalBindingPairingChallenge {
                    installation_id: installation("install-a"),
                    slack_user_id: SlackUserId::new("U123"),
                    setup_revision: None,
                },
            )),
            Arc::new(RecordingNotifier::default()),
        );

        let binding = service
            .redeem_challenge(principal("tenant-a", "user:alice"), code("abc12345"))
            .await
            .expect("redeem challenge");

        assert_eq!(binding.user_id, user("user:alice"));
        assert_eq!(
            store.bindings(),
            vec![RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("slack").unwrap(),
                provider_user_id: RebornIdentityProviderUserId::new("install-a:U123").unwrap(),
                user_id: user("user:alice"),
            }]
        );
    }

    #[tokio::test]
    async fn redeem_challenge_rejects_foreign_tenant_without_consuming_code() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let challenge_store = Arc::new(StaticChallengeStore::new(
            "ABC12345",
            SlackPersonalBindingPairingChallenge {
                installation_id: installation("install-a"),
                slack_user_id: SlackUserId::new("U123"),
                setup_revision: None,
            },
        ));
        let service = SlackPersonalBindingPairingService::new(
            binding_service(binding_store.clone()),
            challenge_store.clone(),
            Arc::new(RecordingNotifier::default()),
        );

        let error = service
            .redeem_challenge(principal("tenant-b", "user:eve"), code("abc12345"))
            .await
            .expect_err("foreign tenant is rejected");

        assert!(matches!(
            error,
            SlackPersonalBindingPairingError::Binding(
                SlackPersonalUserBindingError::UnknownInstallation { .. }
            )
        ));
        assert_eq!(challenge_store.consumes(), 0);
        assert_eq!(
            binding_store.bindings(),
            Vec::<RebornUserIdentityBinding>::new()
        );
    }

    #[tokio::test]
    async fn redeem_challenge_returns_challenge_not_found_for_unknown_code() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let service = SlackPersonalBindingPairingService::new(
            binding_service(binding_store.clone()),
            Arc::new(StaticIssueStore::new("PAIR4242")),
            Arc::new(RecordingNotifier::default()),
        );

        let error = service
            .redeem_challenge(principal("tenant-a", "user:alice"), code("unknown1"))
            .await
            .expect_err("unknown code is rejected");

        assert!(matches!(
            error,
            SlackPersonalBindingPairingError::ChallengeNotFound
        ));
        assert_eq!(
            binding_store.bindings(),
            Vec::<RebornUserIdentityBinding>::new()
        );
    }

    #[test]
    fn pairing_code_rejects_empty_oversize_short_and_non_alphanumeric() {
        assert!(SlackPersonalBindingPairingCode::new("").is_err());
        assert!(SlackPersonalBindingPairingCode::new("A".repeat(33)).is_err());
        assert!(SlackPersonalBindingPairingCode::new("ABC-1234").is_err());
        assert!(SlackPersonalBindingPairingCode::new("ABC123").is_err());
        assert_eq!(
            SlackPersonalBindingPairingCode::new(" abc12345 ")
                .expect("valid code")
                .as_str(),
            "ABC12345"
        );
    }

    #[tokio::test]
    async fn issue_challenge_propagates_notifier_error() {
        let service = SlackPersonalBindingPairingService::new(
            binding_service(Arc::new(RecordingBindingStore::default())),
            Arc::new(StaticIssueStore::new("PAIR4242")),
            Arc::new(FailingNotifier),
        );

        let error = service
            .issue_challenge(installation("install-a"), SlackUserId::new("U123"))
            .await
            .expect_err("notifier error is propagated");

        assert!(matches!(
            error,
            SlackPersonalBindingPairingError::Backend(_)
        ));
    }

    #[tokio::test]
    async fn resolver_sends_pairing_challenge_for_unknown_slack_actor() {
        let notifier = Arc::new(RecordingNotifier::default());
        let pairing = SlackPersonalBindingPairingService::new(
            binding_service(Arc::new(RecordingBindingStore::default())),
            Arc::new(StaticIssueStore::new("PAIR4242")),
            notifier.clone(),
        );
        let resolver = SlackPairingActorResolver::new(Arc::new(EmptyLookup), pairing);

        let resolved = resolver
            .resolve_product_actor_user(actor_request(
                "slack_v2",
                "install-a",
                "slack_user",
                "U123",
            ))
            .await
            .expect("resolution completes");

        assert_eq!(resolved, None);
        assert_eq!(
            notifier.notifications(),
            vec![SlackPersonalBindingPairingNotification {
                installation_id: installation("install-a"),
                slack_user_id: SlackUserId::new("U123"),
                code: code("PAIR4242"),
            }]
        );
    }

    #[tokio::test]
    async fn resolver_suppresses_duplicate_pairing_challenges_during_cooldown() {
        let notifier = Arc::new(RecordingNotifier::default());
        let pairing = SlackPersonalBindingPairingService::new(
            binding_service(Arc::new(RecordingBindingStore::default())),
            Arc::new(StaticIssueStore::new("PAIR4242")),
            notifier.clone(),
        );
        let resolver = SlackPairingActorResolver::new(Arc::new(EmptyLookup), pairing);
        let request = actor_request("slack_v2", "install-a", "slack_user", "U123");

        resolver
            .resolve_product_actor_user(request.clone())
            .await
            .expect("first resolution completes");
        resolver
            .resolve_product_actor_user(request)
            .await
            .expect("second resolution completes");

        assert_eq!(notifier.notifications().len(), 1);
    }

    #[tokio::test]
    async fn resolver_skips_non_slack_shapes_and_existing_binding_without_issuing() {
        let notifier = Arc::new(RecordingNotifier::default());
        let pairing = SlackPersonalBindingPairingService::new(
            binding_service(Arc::new(RecordingBindingStore::default())),
            Arc::new(StaticIssueStore::new("PAIR4242")),
            notifier.clone(),
        );
        let resolver = SlackPairingActorResolver::new(
            Arc::new(StaticLookup::new(Some(user("user:alice")))),
            pairing,
        );

        assert_eq!(
            resolver
                .resolve_product_actor_user(actor_request(
                    "github",
                    "install-a",
                    "slack_user",
                    "U123"
                ))
                .await
                .expect("wrong adapter returns none"),
            None
        );
        assert_eq!(
            resolver
                .resolve_product_actor_user(actor_request(
                    "slack_v2",
                    "install-a",
                    "github_user",
                    "U123"
                ))
                .await
                .expect("wrong actor kind returns none"),
            None
        );
        assert_eq!(
            resolver
                .resolve_product_actor_user(actor_request(
                    "slack_v2",
                    "install-a",
                    "slack_user",
                    "U123"
                ))
                .await
                .expect("existing binding returns user"),
            Some(user("user:alice"))
        );
        assert!(notifier.notifications().is_empty());
    }

    #[tokio::test]
    async fn resolver_propagates_lookup_and_issue_errors() {
        let pairing = SlackPersonalBindingPairingService::new(
            binding_service(Arc::new(RecordingBindingStore::default())),
            Arc::new(StaticIssueStore::new("PAIR4242")),
            Arc::new(RecordingNotifier::default()),
        );
        let resolver = SlackPairingActorResolver::new(Arc::new(FailingLookup), pairing);

        let lookup_error = resolver
            .resolve_product_actor_user(actor_request(
                "slack_v2",
                "install-a",
                "slack_user",
                "U123",
            ))
            .await
            .expect_err("lookup error propagates");
        assert!(matches!(
            lookup_error,
            ProductWorkflowError::BindingResolutionFailed { .. }
        ));

        let failing_pairing = SlackPersonalBindingPairingService::new(
            binding_service(Arc::new(RecordingBindingStore::default())),
            Arc::new(FailingIssueStore),
            Arc::new(RecordingNotifier::default()),
        );
        let resolver = SlackPairingActorResolver::new(Arc::new(EmptyLookup), failing_pairing);
        let issue_error = resolver
            .resolve_product_actor_user(actor_request(
                "slack_v2",
                "install-a",
                "slack_user",
                "U123",
            ))
            .await
            .expect_err("issue error propagates");
        assert!(matches!(
            issue_error,
            ProductWorkflowError::BindingResolutionFailed { .. }
        ));
    }

    fn binding_service(
        store: Arc<dyn RebornUserIdentityBindingStore>,
    ) -> SlackPersonalUserBindingService {
        SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: TenantId::new("tenant-a").unwrap(),
                installation_id: installation("install-a"),
                selector: SlackInstallationSelector::app_team("A-app", "T-team"),
            }],
            store,
        )
    }

    fn principal(tenant_id: &str, user_id: &str) -> SlackPersonalBindingPrincipal {
        SlackPersonalBindingPrincipal {
            tenant_id: TenantId::new(tenant_id).unwrap(),
            user_id: user(user_id),
        }
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).unwrap()
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }

    fn code(value: &str) -> SlackPersonalBindingPairingCode {
        SlackPersonalBindingPairingCode::new(value).unwrap()
    }

    fn actor_request(
        adapter_id: &str,
        installation_id: &str,
        actor_kind: &str,
        actor_id: &str,
    ) -> ProductActorUserResolutionRequest {
        ProductActorUserResolutionRequest::new(
            ProductAdapterId::new(adapter_id).unwrap(),
            installation(installation_id),
            ExternalActorRef::new(actor_kind, actor_id, None::<String>).unwrap(),
        )
    }

    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
    }

    impl RecordingBindingStore {
        fn bindings(&self) -> Vec<RebornUserIdentityBinding> {
            self.bindings.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings.lock().unwrap().push(binding);
            Ok(())
        }
    }

    struct StaticChallengeStore {
        code: SlackPersonalBindingPairingCode,
        challenge: SlackPersonalBindingPairingChallenge,
        consumes: Mutex<usize>,
    }

    impl StaticChallengeStore {
        fn new(code: &str, challenge: SlackPersonalBindingPairingChallenge) -> Self {
            Self {
                code: super::tests::code(code),
                challenge,
                consumes: Mutex::new(0),
            }
        }

        fn consumes(&self) -> usize {
            *self.consumes.lock().unwrap()
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
                code: self.code.clone(),
                challenge,
            })
        }

        async fn get_challenge(
            &self,
            code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            if code != &self.code {
                return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
            }
            Ok(self.challenge.clone())
        }

        async fn consume_challenge(
            &self,
            code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            if code != &self.code {
                return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
            }
            *self.consumes.lock().unwrap() += 1;
            Ok(self.challenge.clone())
        }
    }

    struct StaticIssueStore {
        code: SlackPersonalBindingPairingCode,
    }

    impl StaticIssueStore {
        fn new(code: &str) -> Self {
            Self {
                code: super::tests::code(code),
            }
        }
    }

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingChallengeStore for StaticIssueStore {
        async fn issue_challenge(
            &self,
            challenge: SlackPersonalBindingPairingChallenge,
        ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Ok(IssuedSlackPersonalBindingPairingChallenge {
                code: self.code.clone(),
                challenge,
            })
        }

        async fn get_challenge(
            &self,
            _code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        }

        async fn consume_challenge(
            &self,
            _code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        }
    }

    #[derive(Default)]
    struct RecordingNotifier {
        notifications: Mutex<Vec<SlackPersonalBindingPairingNotification>>,
    }

    impl RecordingNotifier {
        fn notifications(&self) -> Vec<SlackPersonalBindingPairingNotification> {
            self.notifications.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingNotifier for RecordingNotifier {
        async fn send_pairing_challenge(
            &self,
            notification: SlackPersonalBindingPairingNotification,
        ) -> Result<(), SlackPersonalBindingPairingError> {
            self.notifications.lock().unwrap().push(notification);
            Ok(())
        }
    }

    struct FailingNotifier;

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingNotifier for FailingNotifier {
        async fn send_pairing_challenge(
            &self,
            _notification: SlackPersonalBindingPairingNotification,
        ) -> Result<(), SlackPersonalBindingPairingError> {
            Err(SlackPersonalBindingPairingError::Backend(
                "notifier down".into(),
            ))
        }
    }

    struct FailingIssueStore;

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingChallengeStore for FailingIssueStore {
        async fn issue_challenge(
            &self,
            _challenge: SlackPersonalBindingPairingChallenge,
        ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Err(SlackPersonalBindingPairingError::Backend(
                "store down".into(),
            ))
        }

        async fn get_challenge(
            &self,
            _code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        }

        async fn consume_challenge(
            &self,
            _code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        }
    }

    struct EmptyLookup;

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for EmptyLookup {
        async fn resolve_user_identity(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            Ok(None)
        }
    }

    struct StaticLookup {
        user_id: Option<UserId>,
    }

    impl StaticLookup {
        fn new(user_id: Option<UserId>) -> Self {
            Self { user_id }
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for StaticLookup {
        async fn resolve_user_identity(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            Ok(self.user_id.clone())
        }
    }

    struct FailingLookup;

    #[async_trait::async_trait]
    impl RebornUserIdentityLookup for FailingLookup {
        async fn resolve_user_identity(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            Err(RebornUserIdentityLookupError::Backend("lookup down".into()))
        }
    }
}
