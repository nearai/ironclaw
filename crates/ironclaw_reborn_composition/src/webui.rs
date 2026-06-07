use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornServices as ProductRebornServices, RebornServicesApi,
    RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSkillActionResponse, RebornSkillContentResponse, RebornSkillInfo,
    RebornSkillListResponse, RebornSkillSearchResponse, RebornSkillSourceKind, SkillsProductFacade,
};

use crate::{
    RebornBuildError, RebornProductAuthServices, RebornReadiness, RebornRuntime,
    RebornWebuiAutomationFacade,
    lifecycle::{
        RebornLocalLifecycleFacade, RebornLocalSkillManagementError, RebornLocalSkillManagementPort,
    },
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
        api = api.with_skills_product_facade(Arc::new(LocalSkillsProductFacade::new(
            local_runtime.skill_management.clone(),
        )));
    }
    if let Some(product_auth) = &services.product_auth {
        api = api.with_extension_credentials(Arc::new(ProductAuthExtensionCredentialSetup::new(
            Arc::clone(product_auth),
        )));
    }
    if let Some(automation_facade) = automation_facade {
        api = api.with_automation_product_facade(automation_facade);
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

struct LocalSkillsProductFacade {
    skill_management: Arc<RebornLocalSkillManagementPort>,
}

impl LocalSkillsProductFacade {
    fn new(skill_management: Arc<RebornLocalSkillManagementPort>) -> Self {
        Self { skill_management }
    }
}

#[async_trait]
impl SkillsProductFacade for LocalSkillsProductFacade {
    async fn list_skills(
        &self,
        _caller: ironclaw_product_workflow::WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        let skills = self
            .skill_management
            .list()
            .await
            .map_err(map_skill_management_error)?;
        Ok(skill_list_response(skills))
    }

    async fn search_skills(
        &self,
        _caller: ironclaw_product_workflow::WebUiAuthenticatedCaller,
        query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        let result = self
            .skill_management
            .search(&query, 50)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillSearchResponse {
            catalog: Vec::new(),
            installed: result.skills.into_iter().map(skill_info).collect(),
            registry_url: String::new(),
            catalog_error: result
                .truncated
                .then(|| "Skill search results were truncated".to_string()),
        })
    }

    async fn install_skill(
        &self,
        _caller: ironclaw_product_workflow::WebUiAuthenticatedCaller,
        name: String,
        content: Option<String>,
        url: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        if url.is_some() && content.is_none() {
            return Err(invalid_skill_request());
        }
        let content = content.ok_or_else(invalid_skill_request)?;
        let installed = self
            .skill_management
            .install(Some(&name), &content)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillActionResponse {
            success: true,
            message: format!("Skill '{}' installed", installed.name),
        })
    }

    async fn read_skill_content(
        &self,
        _caller: ironclaw_product_workflow::WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        let content = self
            .skill_management
            .read_content(&name)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillContentResponse {
            name: content.name,
            content: content.content,
        })
    }

    async fn update_skill(
        &self,
        _caller: ironclaw_product_workflow::WebUiAuthenticatedCaller,
        name: String,
        content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        let updated = self
            .skill_management
            .update(&name, &content)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillActionResponse {
            success: true,
            message: format!("Skill '{}' updated", updated.name),
        })
    }

    async fn remove_skill(
        &self,
        _caller: ironclaw_product_workflow::WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        let removed = self
            .skill_management
            .remove(&name)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillActionResponse {
            success: true,
            message: format!("Skill '{}' removed", removed.name),
        })
    }
}

fn skill_list_response(skills: Vec<ironclaw_skills::SkillSummary>) -> RebornSkillListResponse {
    let skills: Vec<_> = skills.into_iter().map(skill_info).collect();
    RebornSkillListResponse {
        count: skills.len(),
        skills,
    }
}

fn skill_info(skill: ironclaw_skills::SkillSummary) -> RebornSkillInfo {
    let source_kind = match skill.source {
        ironclaw_skills::ManagedSkillSource::System => RebornSkillSourceKind::System,
        ironclaw_skills::ManagedSkillSource::User => RebornSkillSourceKind::User,
        ironclaw_skills::ManagedSkillSource::Installed => RebornSkillSourceKind::Installed,
    };
    let can_manage = source_kind != RebornSkillSourceKind::System;
    RebornSkillInfo {
        name: skill.name.clone(),
        description: skill.description,
        version: skill.version,
        trust: if source_kind == RebornSkillSourceKind::Installed {
            "Installed".to_string()
        } else {
            "Trusted".to_string()
        },
        source: skill.source.as_str().to_string(),
        source_kind,
        keywords: skill.keywords,
        usage_hint: Some(format!(
            "Type `/{}` in chat to force-activate this skill.",
            skill.name
        )),
        setup_hint: None,
        bundle_path: None,
        install_source_url: None,
        has_requirements: false,
        has_scripts: false,
        can_edit: can_manage,
        can_delete: can_manage,
    }
}

fn map_skill_management_error(error: RebornLocalSkillManagementError) -> RebornServicesError {
    match error {
        RebornLocalSkillManagementError::InvalidContext { .. } => internal_skill_error(),
        RebornLocalSkillManagementError::Skill(error) => match error.kind() {
            ironclaw_skills::SkillManagementErrorKind::NotFound => RebornServicesError {
                code: RebornServicesErrorCode::NotFound,
                kind: RebornServicesErrorKind::NotFound,
                status_code: 404,
                retryable: false,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::Conflict => RebornServicesError {
                code: RebornServicesErrorCode::Conflict,
                kind: RebornServicesErrorKind::Conflict,
                status_code: 409,
                retryable: false,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::Resource => RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::FilesystemDenied => RebornServicesError {
                code: RebornServicesErrorCode::Forbidden,
                kind: RebornServicesErrorKind::ParticipantDenied,
                status_code: 403,
                retryable: false,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::InvalidInput
            | ironclaw_skills::SkillManagementErrorKind::InvalidSkill => invalid_skill_request(),
        },
    }
}

fn invalid_skill_request() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::InvalidRequest,
        kind: RebornServicesErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn internal_skill_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}
