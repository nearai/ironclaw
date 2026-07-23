use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
#[cfg(test)]
pub(crate) use ironclaw_extensions::ExtensionRemovalChannelId;
pub(crate) use ironclaw_extensions::{
    ExtensionRemovalCleanupAdapterId, ExtensionRemovalCleanupBinding,
    ExtensionRemovalCleanupRequirement,
};
use ironclaw_host_api::{ProductSurfaceError, ResourceScope, UserId};
use ironclaw_product::ProductWorkflowError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtensionRemovalCleanupContext {
    pub(crate) scope: ResourceScope,
    pub(crate) authenticated_actor: UserId,
}

impl ExtensionRemovalCleanupContext {
    pub(crate) fn new(scope: ResourceScope, authenticated_actor: UserId) -> Self {
        Self {
            scope,
            authenticated_actor,
        }
    }
}

#[async_trait]
pub(crate) trait ExtensionRemovalCleanupAdapter: Send + Sync {
    fn adapter_id(&self) -> ExtensionRemovalCleanupAdapterId;

    async fn cleanup(
        &self,
        context: &ExtensionRemovalCleanupContext,
        binding: &ExtensionRemovalCleanupBinding,
    ) -> Result<(), ProductSurfaceError>;
}

pub(crate) struct ExtensionRemovalCleanupRegistry {
    adapters: BTreeMap<ExtensionRemovalCleanupAdapterId, Arc<dyn ExtensionRemovalCleanupAdapter>>,
}

impl std::fmt::Debug for ExtensionRemovalCleanupRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtensionRemovalCleanupRegistry")
            .field("adapter_ids", &self.adapters.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ExtensionRemovalCleanupRegistry {
    pub(crate) fn empty() -> Self {
        Self {
            adapters: BTreeMap::new(),
        }
    }

    pub(crate) fn try_from_adapters(
        adapters: Vec<Arc<dyn ExtensionRemovalCleanupAdapter>>,
    ) -> Result<Self, ProductWorkflowError> {
        let mut by_id = BTreeMap::new();
        for adapter in adapters {
            let adapter_id = adapter.adapter_id();
            if by_id.insert(adapter_id.clone(), adapter).is_some() {
                return Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: format!(
                        "duplicate extension removal cleanup adapter: {}",
                        adapter_id.as_str()
                    ),
                });
            }
        }
        Ok(Self { adapters: by_id })
    }

    pub(crate) async fn cleanup_requirements(
        &self,
        requirements: &[ExtensionRemovalCleanupRequirement],
        context: &ExtensionRemovalCleanupContext,
    ) -> Result<(), ProductWorkflowError> {
        let mut ordered_requirements = requirements.iter().collect::<Vec<_>>();
        ordered_requirements.sort();
        for requirement in ordered_requirements {
            let adapter = self.adapters.get(&requirement.adapter_id).ok_or_else(|| {
                ProductWorkflowError::Transient {
                    reason: format!(
                        "required extension removal cleanup adapter is unavailable: {}",
                        requirement.adapter_id.as_str()
                    ),
                }
            })?;
            adapter
                .cleanup(context, &requirement.binding)
                .await
                .map_err(|error| ProductWorkflowError::Transient {
                    reason: format!(
                        "extension removal cleanup adapter {} failed: {:?}",
                        requirement.adapter_id.as_str(),
                        error.code
                    ),
                })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{
        AgentId, InvocationId, ProductSurfaceError, ProjectId, ResourceScope, TenantId, UserId,
    };
    use ironclaw_product::ProductWorkflowError;

    use super::*;

    #[test]
    fn cleanup_ids_reject_empty_or_untrusted_syntax() {
        assert!(ExtensionRemovalCleanupAdapterId::new("").is_err());
        assert!(ExtensionRemovalCleanupAdapterId::new("slack personal").is_err());
        assert!(ExtensionRemovalChannelId::new("Slack/../../secret").is_err());

        assert_eq!(
            ExtensionRemovalCleanupAdapterId::new("slack.personal_connection")
                .expect("valid cleanup adapter id")
                .as_str(),
            "slack.personal_connection"
        );
        assert_eq!(
            ExtensionRemovalChannelId::new("slack")
                .expect("valid channel id")
                .as_str(),
            "slack"
        );
    }

    #[derive(Clone)]
    struct RecordingAdapter {
        id: ExtensionRemovalCleanupAdapterId,
        calls: Arc<Mutex<Vec<String>>>,
        failure_detail: Option<&'static str>,
    }

    #[async_trait]
    impl ExtensionRemovalCleanupAdapter for RecordingAdapter {
        fn adapter_id(&self) -> ExtensionRemovalCleanupAdapterId {
            self.id.clone()
        }

        async fn cleanup(
            &self,
            _context: &ExtensionRemovalCleanupContext,
            binding: &ExtensionRemovalCleanupBinding,
        ) -> Result<(), ProductSurfaceError> {
            if let Some(detail) = self.failure_detail {
                return Err(ProductSurfaceError::internal_from(detail));
            }
            let ExtensionRemovalCleanupBinding::ChannelConnection { channel } = binding;
            self.calls
                .lock()
                .expect("recording cleanup lock")
                .push(format!("{}:{}", self.id.as_str(), channel.as_str()));
            Ok(())
        }
    }

    fn adapter(
        id: &str,
        calls: Arc<Mutex<Vec<String>>>,
    ) -> Arc<dyn ExtensionRemovalCleanupAdapter> {
        Arc::new(RecordingAdapter {
            id: ExtensionRemovalCleanupAdapterId::new(id).expect("valid adapter id"),
            calls,
            failure_detail: None,
        })
    }

    fn requirement(adapter_id: &str, channel: &str) -> ExtensionRemovalCleanupRequirement {
        ExtensionRemovalCleanupRequirement::channel_connection(
            ExtensionRemovalCleanupAdapterId::new(adapter_id).expect("valid adapter id"),
            ExtensionRemovalChannelId::new(channel).expect("valid channel id"),
        )
    }

    fn cleanup_context() -> ExtensionRemovalCleanupContext {
        ExtensionRemovalCleanupContext::new(
            ResourceScope {
                tenant_id: TenantId::new("tenant-a").expect("tenant id"),
                user_id: UserId::new("scope-user").expect("scope user id"),
                agent_id: Some(AgentId::new("agent-a").expect("agent id")),
                project_id: Some(ProjectId::new("project-a").expect("project id")),
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            UserId::new("authenticated-user").expect("authenticated actor id"),
        )
    }

    #[tokio::test]
    async fn registry_dispatches_requirements_in_deterministic_order() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let registry = ExtensionRemovalCleanupRegistry::try_from_adapters(vec![
            adapter("zeta.cleanup", Arc::clone(&calls)),
            adapter("alpha.cleanup", Arc::clone(&calls)),
        ])
        .expect("unique adapters");
        let requirements = vec![
            requirement("zeta.cleanup", "zeta"),
            requirement("alpha.cleanup", "alpha"),
        ];

        registry
            .cleanup_requirements(&requirements, &cleanup_context())
            .await
            .expect("cleanup succeeds");

        assert_eq!(
            *calls.lock().expect("recording cleanup lock"),
            vec!["alpha.cleanup:alpha", "zeta.cleanup:zeta"]
        );
    }

    #[test]
    fn registry_rejects_duplicate_adapter_ids() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let error = ExtensionRemovalCleanupRegistry::try_from_adapters(vec![
            adapter("duplicate.cleanup", Arc::clone(&calls)),
            adapter("duplicate.cleanup", calls),
        ])
        .expect_err("duplicate adapter ids must fail construction");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { reason }
                if reason.contains("duplicate extension removal cleanup adapter")
        ));
    }

    #[tokio::test]
    async fn registry_fails_closed_for_unknown_required_adapter() {
        let registry = ExtensionRemovalCleanupRegistry::try_from_adapters(Vec::new())
            .expect("empty registry is valid");

        let error = registry
            .cleanup_requirements(
                &[requirement("missing.cleanup", "slack")],
                &cleanup_context(),
            )
            .await
            .expect_err("missing required adapter must fail closed");

        assert!(matches!(
            error,
            ProductWorkflowError::Transient { reason }
                if reason.contains("required extension removal cleanup adapter is unavailable")
                    && reason.contains("missing.cleanup")
        ));
    }

    #[tokio::test]
    async fn registry_sanitizes_adapter_failures() {
        let secret_detail = "opaque-backend-detail: /private/credential-store";
        let failing: Arc<dyn ExtensionRemovalCleanupAdapter> = Arc::new(RecordingAdapter {
            id: ExtensionRemovalCleanupAdapterId::new("failing.cleanup").expect("valid adapter id"),
            calls: Arc::new(Mutex::new(Vec::new())),
            failure_detail: Some(secret_detail),
        });
        let registry = ExtensionRemovalCleanupRegistry::try_from_adapters(vec![failing])
            .expect("unique adapter");

        let error = registry
            .cleanup_requirements(
                &[requirement("failing.cleanup", "slack")],
                &cleanup_context(),
            )
            .await
            .expect_err("adapter failure must fail cleanup");

        let ProductWorkflowError::Transient { reason } = error else {
            panic!("adapter failure should be retryable");
        };
        assert!(reason.contains("failing.cleanup"));
        assert!(reason.contains("Internal"));
        assert!(!reason.contains(secret_detail));
        assert!(!reason.contains("credential-store"));
    }
}
