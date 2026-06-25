use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, InvocationId, ResourceScope, UserId};
use ironclaw_host_runtime::HostRuntimeHttpEgressPort;
use ironclaw_product_workflow::{
    IronhubInstallDeliveryRequest, IronhubInstallDeliveryResult, IronhubLinkError,
    IronhubLinkService, IronhubRegisterRequest, LifecyclePhase,
};

use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::lifecycle::RebornLocalSkillManagementPort;

use super::agent_link::{InstallDelivery, RegisterChallenge, verify_signature};
use super::model::{IronHubCommand, IronHubEntryKind, IronHubInstallOptions};
use super::service::IronHubService;

const MAX_TIMESTAMP_DRIFT_SECS: i64 = 300;

pub(crate) struct RebornIronhubLinkService {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    host_runtime_http_egress: HostRuntimeHttpEgressPort,
    shared_key: String,
}

impl RebornIronhubLinkService {
    pub(crate) fn new(
        skill_management: Arc<RebornLocalSkillManagementPort>,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
        host_runtime_http_egress: HostRuntimeHttpEgressPort,
        shared_key: String,
    ) -> Self {
        Self {
            skill_management,
            extension_management,
            host_runtime_http_egress,
            shared_key,
        }
    }

    fn install_service(&self) -> Result<IronHubService, IronhubLinkError> {
        let scope = ResourceScope::local_default(
            UserId::new("reborn-ironhub-link").map_err(internal)?,
            InvocationId::new(),
        )
        .map_err(internal)?;
        let capability_id = CapabilityId::new("builtin.ironhub_install").map_err(internal)?;
        Ok(IronHubService::new_with_host_egress(
            Arc::clone(&self.skill_management),
            Arc::clone(&self.extension_management),
            self.host_runtime_http_egress.clone(),
            capability_id,
            scope,
        ))
    }
}

fn internal(error: impl std::fmt::Display) -> IronhubLinkError {
    IronhubLinkError::Install {
        reason: error.to_string(),
    }
}

fn timestamp_fresh(ts: u64) -> bool {
    let drift = chrono::Utc::now().timestamp() - ts as i64;
    drift.abs() <= MAX_TIMESTAMP_DRIFT_SECS
}

fn install_kind(kind: Option<&str>) -> Option<IronHubEntryKind> {
    match kind {
        Some("tool") => Some(IronHubEntryKind::Tool),
        Some("skill") => Some(IronHubEntryKind::Skill),
        _ => None,
    }
}

#[async_trait]
impl IronhubLinkService for RebornIronhubLinkService {
    async fn register(&self, request: IronhubRegisterRequest) -> Result<(), IronhubLinkError> {
        if !timestamp_fresh(request.ts) {
            return Err(IronhubLinkError::StaleTimestamp);
        }
        let challenge = RegisterChallenge {
            uid: &request.uid,
            aid: &request.aid,
            ts: request.ts,
            nonce: &request.nonce,
        };
        if verify_signature(&self.shared_key, &challenge.payload(), &request.sig) {
            Ok(())
        } else {
            Err(IronhubLinkError::InvalidSignature)
        }
    }

    async fn deliver_install(
        &self,
        request: IronhubInstallDeliveryRequest,
    ) -> Result<IronhubInstallDeliveryResult, IronhubLinkError> {
        if !timestamp_fresh(request.ts) {
            return Err(IronhubLinkError::StaleTimestamp);
        }
        let delivery = InstallDelivery {
            slug: &request.slug,
            version: &request.version,
            uid: &request.uid,
            aid: &request.aid,
            ts: request.ts,
            nonce: &request.nonce,
            artifact_digest: &request.artifact_digest,
        };
        if !verify_signature(&self.shared_key, &delivery.payload(), &request.sig) {
            return Err(IronhubLinkError::InvalidSignature);
        }

        let options = IronHubInstallOptions {
            kind: install_kind(request.kind.as_deref()),
            force: false,
            acknowledge_unverified: false,
            expected_version: Some(request.version),
            expected_artifact_digest: Some(request.artifact_digest),
            private_manifest_url: request.private_manifest_url,
        };
        let response = self
            .install_service()?
            .execute(IronHubCommand::Install {
                name: request.slug.clone(),
                options,
            })
            .await
            .map_err(internal)?;

        Ok(IronhubInstallDeliveryResult {
            installed: matches!(response.phase, LifecyclePhase::Installed),
            slug: request.slug,
            message: response.message.unwrap_or_default(),
        })
    }
}
