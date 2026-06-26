use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, InvocationId, ResourceScope, UserId};
use ironclaw_host_runtime::HostRuntimeHttpEgressPort;
use ironclaw_product_workflow::{
    IronhubInstallDeliveryRequest, IronhubInstallDeliveryResult, IronhubLinkError,
    IronhubLinkService, IronhubRegisterRequest, LifecyclePhase,
};

use crate::RebornBuildError;
use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::lifecycle::RebornLocalSkillManagementPort;

use super::agent_link::{IronhubSharedKey, install_payload, register_payload, verify_signature};
use super::model::{IronHubCommand, IronHubEntryKind, IronHubInstallOptions};
use super::service::IronHubService;

const MAX_TIMESTAMP_DRIFT_SECS: i64 = 300;
const LINK_USER_ID: &str = "reborn-ironhub-link";
const INSTALL_CAPABILITY_ID: &str = "builtin.ironhub_install";

pub(crate) struct RebornIronhubLinkService {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    host_runtime_http_egress: HostRuntimeHttpEgressPort,
    shared_key: IronhubSharedKey,
    link_user_id: UserId,
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
            link_user_id: UserId::new(LINK_USER_ID).map_err(invalid_config)?,
            install_capability: CapabilityId::new(INSTALL_CAPABILITY_ID).map_err(invalid_config)?,
        })
    }

    fn install_service(&self) -> Result<IronHubService, IronhubLinkError> {
        let scope = ResourceScope::local_default(self.link_user_id.clone(), InvocationId::new())
            .map_err(internal)?;
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
        if verify_signature(self.shared_key.as_str(), &register_payload(&request), &request.sig) {
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
        if !verify_signature(self.shared_key.as_str(), &install_payload(&request), &request.sig) {
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
