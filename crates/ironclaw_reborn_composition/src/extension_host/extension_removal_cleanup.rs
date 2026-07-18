use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(any(
    feature = "slack-v2-host-beta",
    feature = "telegram-v2-host-beta",
    test
))]
use std::sync::OnceLock;

use async_trait::async_trait;
#[cfg(any(
    feature = "slack-v2-host-beta",
    feature = "telegram-v2-host-beta",
    test
))]
pub(crate) use ironclaw_extensions::ExtensionRemovalChannelId;
pub(crate) use ironclaw_extensions::{
    ExtensionRemovalCleanupAdapterId, ExtensionRemovalCleanupBinding,
    ExtensionRemovalCleanupRequirement,
};
use ironclaw_host_api::{ResourceScope, UserId};
#[cfg(any(
    feature = "slack-v2-host-beta",
    feature = "telegram-v2-host-beta",
    test
))]
use ironclaw_product_workflow::{ChannelConnectionFacade, WebUiAuthenticatedCaller};
use ironclaw_product_workflow::{ProductWorkflowError, RebornServicesError};

#[cfg(any(feature = "slack-v2-host-beta", test))]
pub(crate) const SLACK_PERSONAL_CONNECTION_CLEANUP_ADAPTER_ID: &str = "slack.personal_connection";
#[cfg(any(feature = "slack-v2-host-beta", test))]
pub(crate) const SLACK_EXTENSION_REMOVAL_CHANNEL_ID: &str = "slack";
#[cfg(any(feature = "telegram-v2-host-beta", test))]
pub(crate) const TELEGRAM_PAIRING_CONNECTION_CLEANUP_ADAPTER_ID: &str =
    "telegram.pairing_connection";
#[cfg(any(feature = "telegram-v2-host-beta", test))]
pub(crate) const TELEGRAM_EXTENSION_REMOVAL_CHANNEL_ID: &str = "telegram";

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
    ) -> Result<(), RebornServicesError>;
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

#[cfg(any(feature = "slack-v2-host-beta", test))]
pub(crate) struct SlackPersonalConnectionCleanupAdapter {
    adapter_id: ExtensionRemovalCleanupAdapterId,
    channel_connection: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>>,
}

#[cfg(any(feature = "slack-v2-host-beta", test))]
impl SlackPersonalConnectionCleanupAdapter {
    pub(crate) fn new(
        channel_connection: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>>,
    ) -> Result<Self, ProductWorkflowError> {
        let adapter_id =
            ExtensionRemovalCleanupAdapterId::new(SLACK_PERSONAL_CONNECTION_CLEANUP_ADAPTER_ID)
                .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                    reason: error.to_string(),
                })?;
        Ok(Self {
            adapter_id,
            channel_connection,
        })
    }
}

#[async_trait]
#[cfg(any(feature = "slack-v2-host-beta", test))]
impl ExtensionRemovalCleanupAdapter for SlackPersonalConnectionCleanupAdapter {
    fn adapter_id(&self) -> ExtensionRemovalCleanupAdapterId {
        self.adapter_id.clone()
    }

    async fn cleanup(
        &self,
        context: &ExtensionRemovalCleanupContext,
        binding: &ExtensionRemovalCleanupBinding,
    ) -> Result<(), RebornServicesError> {
        let ExtensionRemovalCleanupBinding::ChannelConnection { channel } = binding;
        if channel.as_str() != SLACK_EXTENSION_REMOVAL_CHANNEL_ID {
            return Err(RebornServicesError::internal_from(
                "Slack extension removal cleanup received an unsupported binding",
            ));
        }
        let channel_connection = self.channel_connection.get().ok_or_else(|| {
            RebornServicesError::internal_from(
                "Slack extension removal cleanup facade is unavailable",
            )
        })?;
        let caller = WebUiAuthenticatedCaller::new(
            context.scope.tenant_id.clone(),
            context.authenticated_actor.clone(),
            context.scope.agent_id.clone(),
            context.scope.project_id.clone(),
        );
        channel_connection
            .disconnect_channel_for_caller(caller, channel.as_str())
            .await
    }
}

/// Removing the `telegram` extension unpairs the removing user: cleanup
/// routes through the shared channel-connection facade slot (filled by the
/// telegram host mounts, or the composite when Slack is also enabled), whose
/// `disconnect_channel_for_caller("telegram")` deletes the identity binding
/// and DM delivery target and invalidates any pending pairing code. Only the
/// removing user is affected; an unfilled slot fails the removal closed
/// (never a silent skip).
#[cfg(any(feature = "telegram-v2-host-beta", test))]
pub(crate) struct TelegramPairingConnectionCleanupAdapter {
    adapter_id: ExtensionRemovalCleanupAdapterId,
    channel_connection: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>>,
}

#[cfg(any(feature = "telegram-v2-host-beta", test))]
impl TelegramPairingConnectionCleanupAdapter {
    pub(crate) fn new(
        channel_connection: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>>,
    ) -> Result<Self, ProductWorkflowError> {
        let adapter_id =
            ExtensionRemovalCleanupAdapterId::new(TELEGRAM_PAIRING_CONNECTION_CLEANUP_ADAPTER_ID)
                .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                reason: error.to_string(),
            })?;
        Ok(Self {
            adapter_id,
            channel_connection,
        })
    }
}

#[async_trait]
#[cfg(any(feature = "telegram-v2-host-beta", test))]
impl ExtensionRemovalCleanupAdapter for TelegramPairingConnectionCleanupAdapter {
    fn adapter_id(&self) -> ExtensionRemovalCleanupAdapterId {
        self.adapter_id.clone()
    }

    async fn cleanup(
        &self,
        context: &ExtensionRemovalCleanupContext,
        binding: &ExtensionRemovalCleanupBinding,
    ) -> Result<(), RebornServicesError> {
        let ExtensionRemovalCleanupBinding::ChannelConnection { channel } = binding;
        if channel.as_str() != TELEGRAM_EXTENSION_REMOVAL_CHANNEL_ID {
            return Err(RebornServicesError::internal_from(
                "Telegram extension removal cleanup received an unsupported binding",
            ));
        }
        let channel_connection = self.channel_connection.get().ok_or_else(|| {
            RebornServicesError::internal_from(
                "Telegram extension removal cleanup facade is unavailable",
            )
        })?;
        let caller = WebUiAuthenticatedCaller::new(
            context.scope.tenant_id.clone(),
            context.authenticated_actor.clone(),
            context.scope.agent_id.clone(),
            context.scope.project_id.clone(),
        );
        channel_connection
            .disconnect_channel_for_caller(caller, channel.as_str())
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
    use ironclaw_product_workflow::{
        ChannelConnectionFacade, ProductWorkflowError, RebornServicesError,
        WebUiAuthenticatedCaller,
    };

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
        ) -> Result<(), RebornServicesError> {
            if let Some(detail) = self.failure_detail {
                return Err(RebornServicesError::internal_from(detail));
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

    #[derive(Default)]
    struct RecordingChannelConnectionFacade {
        discovery_calls: Mutex<usize>,
        disconnect_calls: Mutex<Vec<(WebUiAuthenticatedCaller, String)>>,
    }

    #[async_trait]
    impl ChannelConnectionFacade for RecordingChannelConnectionFacade {
        async fn caller_channel_connections(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<HashMap<String, bool>, RebornServicesError> {
            *self.discovery_calls.lock().expect("discovery call lock") += 1;
            Ok(HashMap::from([("slack".to_string(), true)]))
        }

        async fn disconnect_channel_for_caller(
            &self,
            caller: WebUiAuthenticatedCaller,
            channel: &str,
        ) -> Result<(), RebornServicesError> {
            self.disconnect_calls
                .lock()
                .expect("disconnect call lock")
                .push((caller, channel.to_string()));
            Ok(())
        }
    }

    fn slack_registry(facade: Arc<dyn ChannelConnectionFacade>) -> ExtensionRemovalCleanupRegistry {
        let slot = Arc::new(OnceLock::new());
        assert!(slot.set(facade).is_ok(), "facade slot starts empty");
        let adapter: Arc<dyn ExtensionRemovalCleanupAdapter> = Arc::new(
            SlackPersonalConnectionCleanupAdapter::new(slot).expect("valid Slack cleanup adapter"),
        );
        ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter])
            .expect("Slack adapter is unique")
    }

    #[tokio::test]
    async fn slack_adapter_rejects_non_slack_channel_binding() {
        let facade: Arc<dyn ChannelConnectionFacade> =
            Arc::new(RecordingChannelConnectionFacade::default());
        let registry = slack_registry(facade);

        let error = registry
            .cleanup_requirements(
                &[requirement("slack.personal_connection", "telegram")],
                &cleanup_context(),
            )
            .await
            .expect_err("Slack adapter must reject a foreign channel binding");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    #[tokio::test]
    async fn slack_adapter_fails_closed_when_late_bound_facade_is_unset() {
        let adapter: Arc<dyn ExtensionRemovalCleanupAdapter> = Arc::new(
            SlackPersonalConnectionCleanupAdapter::new(Arc::new(OnceLock::new()))
                .expect("valid Slack cleanup adapter"),
        );
        let registry = ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter])
            .expect("Slack adapter is unique");

        let error = registry
            .cleanup_requirements(
                &[requirement("slack.personal_connection", "slack")],
                &cleanup_context(),
            )
            .await
            .expect_err("an unwired required cleanup facade must fail closed");

        let ProductWorkflowError::Transient { reason } = error else {
            panic!("missing late-bound facade must remain retryable");
        };
        assert!(reason.contains("slack.personal_connection"));
    }

    #[tokio::test]
    async fn slack_adapter_derives_caller_and_disconnects_without_status_discovery() {
        let facade = Arc::new(RecordingChannelConnectionFacade::default());
        let facade_trait: Arc<dyn ChannelConnectionFacade> = facade.clone();
        let registry = slack_registry(facade_trait);

        registry
            .cleanup_requirements(
                &[requirement("slack.personal_connection", "slack")],
                &cleanup_context(),
            )
            .await
            .expect("Slack cleanup succeeds");

        assert_eq!(
            *facade.discovery_calls.lock().expect("discovery call lock"),
            0,
            "cleanup must never inspect connection status"
        );
        let disconnects = facade
            .disconnect_calls
            .lock()
            .expect("disconnect call lock");
        assert_eq!(disconnects.len(), 1);
        let (caller, channel) = &disconnects[0];
        assert_eq!(channel, "slack");
        assert_eq!(caller.tenant_id.as_str(), "tenant-a");
        assert_eq!(caller.user_id.as_str(), "authenticated-user");
        assert_eq!(
            caller.agent_id.as_ref().map(AgentId::as_str),
            Some("agent-a")
        );
        assert_eq!(
            caller.project_id.as_ref().map(ProjectId::as_str),
            Some("project-a")
        );
        assert!(!caller.operator_webui_config);
    }
}

#[cfg(test)]
mod telegram_cleanup_tests {
    use std::sync::{Arc, Mutex, OnceLock};

    use async_trait::async_trait;
    use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId};
    use ironclaw_product_workflow::{
        ChannelConnectionFacade, RebornServicesError, WebUiAuthenticatedCaller,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingConnectionFacade {
        disconnects: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl ChannelConnectionFacade for RecordingConnectionFacade {
        async fn caller_channel_connections(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<std::collections::HashMap<String, bool>, RebornServicesError> {
            Ok(std::collections::HashMap::from([(
                "telegram".to_string(),
                true,
            )]))
        }

        async fn disconnect_channel_for_caller(
            &self,
            caller: WebUiAuthenticatedCaller,
            channel: &str,
        ) -> Result<(), RebornServicesError> {
            self.disconnects
                .lock()
                .expect("disconnect lock")
                .push((caller.user_id.as_str().to_string(), channel.to_string()));
            Ok(())
        }
    }

    fn context_for(user: &str) -> ExtensionRemovalCleanupContext {
        ExtensionRemovalCleanupContext::new(
            ResourceScope {
                tenant_id: TenantId::new("tenant-a").expect("tenant"),
                user_id: UserId::new(user).expect("user"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            UserId::new(user).expect("user"),
        )
    }

    fn telegram_requirement() -> ExtensionRemovalCleanupRequirement {
        ExtensionRemovalCleanupRequirement::channel_connection(
            ExtensionRemovalCleanupAdapterId::new(TELEGRAM_PAIRING_CONNECTION_CLEANUP_ADAPTER_ID)
                .expect("adapter id"),
            ExtensionRemovalChannelId::new(TELEGRAM_EXTENSION_REMOVAL_CHANNEL_ID)
                .expect("channel id"),
        )
    }

    #[tokio::test]
    async fn telegram_removal_cleanup_disconnects_the_removing_user() {
        let facade = Arc::new(RecordingConnectionFacade::default());
        let slot: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>> = Arc::new(OnceLock::new());
        slot.set(Arc::clone(&facade) as Arc<dyn ChannelConnectionFacade>)
            .ok()
            .expect("slot fills once");
        let registry = ExtensionRemovalCleanupRegistry::try_from_adapters(vec![Arc::new(
            TelegramPairingConnectionCleanupAdapter::new(slot).expect("adapter builds"),
        )])
        .expect("registry builds");

        registry
            .cleanup_requirements(&[telegram_requirement()], &context_for("ben"))
            .await
            .expect("cleanup succeeds");

        assert_eq!(
            facade.disconnects.lock().expect("disconnect lock").clone(),
            vec![("ben".to_string(), "telegram".to_string())],
            "removal unpairs exactly the removing user on the telegram channel"
        );
    }

    #[tokio::test]
    async fn telegram_removal_cleanup_rejects_foreign_channel_bindings() {
        let facade = Arc::new(RecordingConnectionFacade::default());
        let slot: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>> = Arc::new(OnceLock::new());
        slot.set(Arc::clone(&facade) as Arc<dyn ChannelConnectionFacade>)
            .ok()
            .expect("slot fills once");
        let adapter = TelegramPairingConnectionCleanupAdapter::new(slot).expect("adapter builds");

        adapter
            .cleanup(
                &context_for("ben"),
                &ExtensionRemovalCleanupBinding::ChannelConnection {
                    channel: ExtensionRemovalChannelId::new("slack").expect("channel id"),
                },
            )
            .await
            .expect_err("foreign channel bindings must be rejected");
        assert!(
            facade
                .disconnects
                .lock()
                .expect("disconnect lock")
                .is_empty(),
            "a foreign binding must never trigger a disconnect"
        );
    }

    #[tokio::test]
    async fn telegram_removal_cleanup_fails_closed_when_facade_slot_is_unfilled() {
        let slot: Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>> = Arc::new(OnceLock::new());
        let registry = ExtensionRemovalCleanupRegistry::try_from_adapters(vec![Arc::new(
            TelegramPairingConnectionCleanupAdapter::new(slot).expect("adapter builds"),
        )])
        .expect("registry builds");

        registry
            .cleanup_requirements(&[telegram_requirement()], &context_for("ben"))
            .await
            .expect_err("unfilled facade slot must fail the removal closed");
    }
}
