use std::sync::Arc;

use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornServices as ProductRebornServices, RebornServicesApi,
    RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
};

use crate::{
    RebornBuildError, RebornProductAuthServices, RebornReadiness, RebornRuntime,
    RebornWebuiAutomationFacade,
    lifecycle::RebornLocalLifecycleFacade,
    outbound_preferences::{EmptyOutboundDeliveryTargetInventory, RebornOutboundPreferencesFacade},
    webui_extension_credentials::ProductAuthExtensionCredentialSetup,
};

/// WebUI-facing Reborn service bundle for host composition.
///
/// This bundle deliberately exposes facade-shaped product handles consumed
/// by WebChat v2 and the optional product-auth OAuth routes. HTTP
/// routing, auth middleware, static assets, and SSE transport stay in the
/// WebUI crate (or, when the `webui-v2-beta` feature is on, the
/// [`crate::webui_serve`] module in this crate); lower runtime handles stay
/// behind the existing Reborn runtime / composition services.
#[derive(Clone)]
pub struct RebornWebuiBundle {
    pub api: Arc<dyn RebornServicesApi>,
    pub product_auth: Option<Arc<RebornProductAuthServices>>,
    pub readiness: RebornReadiness,
}

impl std::fmt::Debug for RebornWebuiBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornWebuiBundle")
            .field("api", &"Arc<dyn RebornServicesApi>")
            .field("product_auth", &self.product_auth.is_some())
            .field("readiness", &self.readiness)
            .finish()
    }
}

/// Compose the WebUI-facing product facade from an already-built Reborn runtime.
///
/// This function does not create a second turn coordinator, thread service,
/// host runtime or route server. It reuses the runtime's existing task-level
/// composition and attaches the runtime-owned projection stream unless the
/// caller supplies a custom stream.
pub fn build_webui_services(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    build_webui_services_with_connectable_channels(runtime, event_stream, None)
}

pub(crate) fn build_webui_services_with_connectable_channels(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    connectable_channels: Option<Arc<dyn ConnectableChannelsProductFacade>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let services = runtime.services();
    let automation_facade = services
        .host_runtime
        .as_ref()
        .map(|host_runtime| Arc::new(RebornWebuiAutomationFacade::new(Arc::clone(host_runtime))));

    let mut api = ProductRebornServices::new(
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    )
    .with_approval_interactions(runtime.webui_approval_interaction_service())
    .with_auth_interactions(runtime.webui_auth_interaction_service());
    if let Some(skill_activation_source) = runtime.webui_skill_activation_source() {
        let activation_recorder = Arc::clone(&skill_activation_source);
        let activation_clearer = skill_activation_source;
        api = api.with_skill_activation_hooks(
            move |scope, accepted_message_ref, message| {
                activation_recorder
                    .record_user_message(scope.clone(), accepted_message_ref.clone(), message)
                    .map_err(|_| RebornServicesError {
                        code: RebornServicesErrorCode::Internal,
                        kind: RebornServicesErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
            move |scope, accepted_message_ref| {
                activation_clearer
                    .clear_accepted_message(scope, accepted_message_ref)
                    .map_err(|_| RebornServicesError {
                        code: RebornServicesErrorCode::Internal,
                        kind: RebornServicesErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
        );
    }
    if let Some(local_runtime) = &services.local_runtime {
        let mut lifecycle_facade =
            RebornLocalLifecycleFacade::new(local_runtime.skill_management.clone());
        if let Some(extension_management) = &local_runtime.extension_management {
            lifecycle_facade =
                lifecycle_facade.with_extension_management(extension_management.clone());
        }
        if let Some(runtime_http_egress) = &local_runtime.runtime_http_egress {
            lifecycle_facade =
                lifecycle_facade.with_runtime_http_egress(runtime_http_egress.clone());
        }
        api = api.with_lifecycle_product_facade(Arc::new(lifecycle_facade));
    }
    if let Some(product_auth) = &services.product_auth {
        api = api.with_extension_credentials(Arc::new(ProductAuthExtensionCredentialSetup::new(
            Arc::clone(product_auth),
        )));
    }
    if let Some(automation_facade) = automation_facade {
        api = api.with_automation_product_facade(automation_facade);
    }
    if let Some(local_runtime) = &services.local_runtime {
        // GitHub PR #4537 carry-forward tracks the Slack delivery target
        // bridge; until then this product-neutral facade exposes no targets.
        api = api.with_outbound_preferences_facade(Arc::new(RebornOutboundPreferencesFacade::new(
            Arc::clone(&local_runtime.outbound_preferences),
            Arc::new(EmptyOutboundDeliveryTargetInventory),
        )));
    }
    if let Some(connectable_channels) = connectable_channels {
        api = api.with_connectable_channels_facade(connectable_channels);
    }
    api = api.with_event_stream(event_stream.unwrap_or_else(|| runtime.webui_event_stream()));

    // Compose the operator LLM-config settings service when the runtime was
    // assembled with a boot config. The secret store stays private to this
    // crate; the service is the only facade-shaped handle that leaves.
    #[cfg(feature = "root-llm-provider")]
    if let Some(boot) = runtime.webui_boot_config() {
        let keys = crate::LlmKeyStore::new(runtime.services().secret_store());
        let mut llm_config = crate::RebornLlmConfigService::new(boot.clone(), keys);
        if let Some(reload) = runtime.webui_llm_reload_trigger() {
            llm_config = llm_config.with_reload_trigger(reload);
        }
        if let Some(session) = runtime.webui_llm_session() {
            llm_config = llm_config.with_nearai_session(session);
        }
        if let Some(states) = runtime.webui_nearai_login_states() {
            llm_config = llm_config.with_nearai_login_states(states);
        }
        api = api.with_llm_config_service(Arc::new(llm_config));
    }

    Ok(RebornWebuiBundle {
        api: Arc::new(api),
        product_auth: services.product_auth.clone(),
        readiness: services.readiness,
    })
}
