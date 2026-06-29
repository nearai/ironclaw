use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, InvocationId, ResourceScope, UserId};
use ironclaw_host_runtime::HostRuntimeHttpEgressPort;
use ironclaw_product_workflow::{
    IronhubInstallDeliveryRequest, IronhubInstallDeliveryResult, IronhubInstallKind,
    IronhubLinkError, IronhubLinkService, IronhubRegisterRequest, LifecyclePhase,
};

use crate::RebornBuildError;
use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::lifecycle::RebornLocalSkillManagementPort;

use super::agent_link::{IronhubSharedKey, install_payload, register_payload, verify_signature};
use super::model::{IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions};
use super::service::IronHubService;

const MAX_TIMESTAMP_DRIFT_SECS: i64 = 300;
const INSTALL_CAPABILITY_ID: &str = "builtin.ironhub_install";

static SEEN_INSTALL_NONCES: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) struct RebornIronhubLinkService {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    host_runtime_http_egress: HostRuntimeHttpEgressPort,
    shared_key: IronhubSharedKey,
    install_capability: CapabilityId,
}

impl RebornIronhubLinkService {
    pub(crate) fn new(
        skill_management: Arc<RebornLocalSkillManagementPort>,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
        host_runtime_http_egress: HostRuntimeHttpEgressPort,
        shared_key: IronhubSharedKey,
    ) -> Result<Self, RebornBuildError> {
        Ok(Self {
            skill_management,
            extension_management,
            host_runtime_http_egress,
            shared_key,
            install_capability: CapabilityId::new(INSTALL_CAPABILITY_ID).map_err(invalid_config)?,
        })
    }

    fn install_service(&self, user_id: UserId) -> Result<IronHubService, IronhubLinkError> {
        let scope = ResourceScope::local_default(user_id, InvocationId::new()).map_err(internal)?;
        Ok(IronHubService::new_with_host_egress(
            Arc::clone(&self.skill_management),
            Arc::clone(&self.extension_management),
            self.host_runtime_http_egress.clone(),
            self.install_capability.clone(),
            scope,
        ))
    }
}

fn invalid_config(error: impl std::fmt::Display) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!("ironhub link service: {error}"),
    }
}

fn internal(error: impl std::fmt::Display) -> IronhubLinkError {
    IronhubLinkError::Install {
        reason: error.to_string(),
    }
}

fn map_install_error(error: IronHubCommandError) -> IronhubLinkError {
    match error {
        IronHubCommandError::InvalidInput { reason } | IronHubCommandError::Catalog { reason } => {
            IronhubLinkError::InvalidInput { reason }
        }
        other => IronhubLinkError::Install {
            reason: other.to_string(),
        },
    }
}

fn timestamp_fresh(ts: u64) -> bool {
    let drift = chrono::Utc::now().timestamp() - ts as i64;
    drift.abs() <= MAX_TIMESTAMP_DRIFT_SECS
}

fn map_kind(kind: Option<IronhubInstallKind>) -> Option<IronHubEntryKind> {
    kind.map(|kind| match kind {
        IronhubInstallKind::Tool => IronHubEntryKind::Tool,
        IronhubInstallKind::Skill => IronHubEntryKind::Skill,
    })
}

fn reject_replayed_nonce(nonce: &str) -> Result<(), IronhubLinkError> {
    let ttl = Duration::from_secs(MAX_TIMESTAMP_DRIFT_SECS as u64);
    let now = Instant::now();
    let mut seen = SEEN_INSTALL_NONCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    seen.retain(|_, recorded| now.duration_since(*recorded) < ttl);
    if seen.contains_key(nonce) {
        return Err(IronhubLinkError::Replay);
    }
    seen.insert(nonce.to_string(), now);
    Ok(())
}

#[async_trait]
impl IronhubLinkService for RebornIronhubLinkService {
    async fn register(&self, request: IronhubRegisterRequest) -> Result<(), IronhubLinkError> {
        if !timestamp_fresh(request.ts) {
            return Err(IronhubLinkError::StaleTimestamp);
        }
        // replay-ok: register has no local side effect, so an idempotent retry is
        // harmless; single-use is enforced on deliver_install only.
        if verify_signature(
            self.shared_key.as_str(),
            &register_payload(&request),
            &request.sig,
        ) {
            Ok(())
        } else {
            Err(IronhubLinkError::InvalidSignature)
        }
    }

    async fn deliver_install(
        &self,
        user_id: UserId,
        request: IronhubInstallDeliveryRequest,
    ) -> Result<IronhubInstallDeliveryResult, IronhubLinkError> {
        if !timestamp_fresh(request.ts) {
            return Err(IronhubLinkError::StaleTimestamp);
        }
        if !verify_signature(
            self.shared_key.as_str(),
            &install_payload(&request),
            &request.sig,
        ) {
            return Err(IronhubLinkError::InvalidSignature);
        }
        reject_replayed_nonce(&request.nonce)?;

        let options = IronHubInstallOptions {
            kind: map_kind(request.kind),
            force: false,
            acknowledge_unverified: false,
            expected_version: Some(request.version),
            expected_artifact_digest: Some(request.artifact_digest),
            private_manifest_url: request.private_manifest_url,
        };
        let response = self
            .install_service(user_id)?
            .execute(IronHubCommand::Install {
                name: request.slug.clone(),
                options,
            })
            .await
            .map_err(map_install_error)?;

        Ok(IronhubInstallDeliveryResult {
            installed: matches!(response.phase, LifecyclePhase::Installed),
            slug: request.slug,
            message: response.message.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    use crate::RebornBuildInput;
    use crate::factory::build_reborn_services;

    use super::*;

    const SHARED_KEY: &str = "ihub_sk_LinkServiceTestKey0000000000000000000000000";

    fn now_ts() -> u64 {
        chrono::Utc::now().timestamp() as u64
    }

    fn sign(payload: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SHARED_KEY.as_bytes()).expect("hmac key");
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn register_request(ts: u64, sig: String) -> IronhubRegisterRequest {
        IronhubRegisterRequest {
            uid: "user-1".to_string(),
            aid: "aid-1".to_string(),
            ts,
            nonce: "nonce-register".to_string(),
            sig,
        }
    }

    fn install_request(ts: u64, nonce: &str, sig: String) -> IronhubInstallDeliveryRequest {
        IronhubInstallDeliveryRequest {
            slug: "my-skill".to_string(),
            version: "1.0.0".to_string(),
            uid: "user-1".to_string(),
            aid: "aid-1".to_string(),
            ts,
            nonce: nonce.to_string(),
            artifact_digest: "sha256:deadbeef".to_string(),
            sig,
            kind: Some(IronhubInstallKind::Skill),
            private_manifest_url: None,
        }
    }

    async fn build_link_service(root: &std::path::Path) -> RebornIronhubLinkService {
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "ironhub-link-test-owner",
            root.join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let local_runtime = services.local_runtime.expect("local runtime substrate");
        RebornIronhubLinkService::new(
            Arc::clone(&local_runtime.skill_management),
            local_runtime
                .extension_management
                .as_ref()
                .expect("extension management")
                .clone(),
            local_runtime
                .host_runtime_http_egress
                .as_ref()
                .expect("host runtime http egress")
                .clone(),
            IronhubSharedKey::new(SHARED_KEY).expect("shared key"),
        )
        .expect("link service builds")
    }

    #[test]
    fn timestamp_fresh_accepts_now_and_rejects_stale() {
        assert!(timestamp_fresh(now_ts()));
        assert!(!timestamp_fresh(1));
    }

    #[test]
    fn map_kind_maps_each_variant() {
        assert!(matches!(
            map_kind(Some(IronhubInstallKind::Tool)),
            Some(IronHubEntryKind::Tool)
        ));
        assert!(matches!(
            map_kind(Some(IronhubInstallKind::Skill)),
            Some(IronHubEntryKind::Skill)
        ));
        assert!(map_kind(None).is_none());
    }

    #[test]
    fn map_install_error_classifies_invalid_and_catalog_as_invalid_input() {
        assert!(matches!(
            map_install_error(IronHubCommandError::InvalidInput {
                reason: "bad".to_string()
            }),
            IronhubLinkError::InvalidInput { .. }
        ));
        assert!(matches!(
            map_install_error(IronHubCommandError::Catalog {
                reason: "bad".to_string()
            }),
            IronhubLinkError::InvalidInput { .. }
        ));
        assert!(matches!(
            map_install_error(IronHubCommandError::LocalRuntimeUnavailable),
            IronhubLinkError::Install { .. }
        ));
    }

    #[test]
    fn reject_replayed_nonce_rejects_second_use() {
        let nonce = "nonce-replay-unit-unique";
        assert!(reject_replayed_nonce(nonce).is_ok());
        assert!(matches!(
            reject_replayed_nonce(nonce),
            Err(IronhubLinkError::Replay)
        ));
    }

    #[tokio::test]
    async fn register_rejects_stale_timestamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = build_link_service(dir.path()).await;
        let request = register_request(1, "00".to_string());
        assert!(matches!(
            service.register(request).await,
            Err(IronhubLinkError::StaleTimestamp)
        ));
    }

    #[tokio::test]
    async fn register_rejects_invalid_signature() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = build_link_service(dir.path()).await;
        let request = register_request(now_ts(), "00".to_string());
        assert!(matches!(
            service.register(request).await,
            Err(IronhubLinkError::InvalidSignature)
        ));
    }

    #[tokio::test]
    async fn register_accepts_valid_signature() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = build_link_service(dir.path()).await;
        let ts = now_ts();
        let mut request = register_request(ts, String::new());
        request.sig = sign(&register_payload(&request));
        service
            .register(request)
            .await
            .expect("a correctly signed register handshake is accepted");
    }

    #[tokio::test]
    async fn deliver_install_rejects_stale_timestamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = build_link_service(dir.path()).await;
        let request = install_request(1, "nonce-stale", "00".to_string());
        let user_id = UserId::new("user-1").expect("user id");
        assert!(matches!(
            service.deliver_install(user_id, request).await,
            Err(IronhubLinkError::StaleTimestamp)
        ));
    }

    #[tokio::test]
    async fn deliver_install_rejects_invalid_signature() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = build_link_service(dir.path()).await;
        let request = install_request(now_ts(), "nonce-bad-sig", "00".to_string());
        let user_id = UserId::new("user-1").expect("user id");
        assert!(matches!(
            service.deliver_install(user_id, request).await,
            Err(IronhubLinkError::InvalidSignature)
        ));
    }
}
