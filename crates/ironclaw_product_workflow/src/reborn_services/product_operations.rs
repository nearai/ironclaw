use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProductOperationHandler {
    OperatorSetupRun,
    LlmProviderUpsert,
    LlmProviderDelete,
    LlmActiveSet,
    ExtensionImport,
    ExtensionSetupSubmit,
    ProjectUpdate,
    ProjectDelete,
    ProjectMemberAdd,
    ProjectMemberUpdate,
    ProjectMemberRemove,
    ThreadDelete,
    AutomationPause,
    AutomationResume,
    AutomationRename,
    AutomationDelete,
    AdminUserUpdate,
    AdminUserSetStatus,
    AdminUserSetRole,
    AdminUserDelete,
    AdminUserPutSecret,
    AdminUserDeleteSecret,
}

impl ProductOperationHandler {
    pub(super) fn parse(capability: &CapabilityId) -> Option<Self> {
        match capability.as_str() {
            OPERATOR_SETUP_RUN_CAPABILITY_ID => Some(Self::OperatorSetupRun),
            LLM_PROVIDER_UPSERT_CAPABILITY_ID => Some(Self::LlmProviderUpsert),
            LLM_PROVIDER_DELETE_CAPABILITY_ID => Some(Self::LlmProviderDelete),
            LLM_ACTIVE_SET_CAPABILITY_ID => Some(Self::LlmActiveSet),
            EXTENSION_IMPORT_CAPABILITY_ID => Some(Self::ExtensionImport),
            EXTENSION_SETUP_SUBMIT_CAPABILITY_ID => Some(Self::ExtensionSetupSubmit),
            PROJECT_UPDATE_CAPABILITY_ID => Some(Self::ProjectUpdate),
            PROJECT_DELETE_CAPABILITY_ID => Some(Self::ProjectDelete),
            PROJECT_MEMBER_ADD_CAPABILITY_ID => Some(Self::ProjectMemberAdd),
            PROJECT_MEMBER_UPDATE_CAPABILITY_ID => Some(Self::ProjectMemberUpdate),
            PROJECT_MEMBER_REMOVE_CAPABILITY_ID => Some(Self::ProjectMemberRemove),
            THREAD_DELETE_CAPABILITY_ID => Some(Self::ThreadDelete),
            AUTOMATION_PAUSE_CAPABILITY_ID => Some(Self::AutomationPause),
            AUTOMATION_RESUME_CAPABILITY_ID => Some(Self::AutomationResume),
            AUTOMATION_RENAME_CAPABILITY_ID => Some(Self::AutomationRename),
            AUTOMATION_DELETE_CAPABILITY_ID => Some(Self::AutomationDelete),
            ADMIN_USER_UPDATE_CAPABILITY_ID => Some(Self::AdminUserUpdate),
            ADMIN_USER_SET_STATUS_CAPABILITY_ID => Some(Self::AdminUserSetStatus),
            ADMIN_USER_SET_ROLE_CAPABILITY_ID => Some(Self::AdminUserSetRole),
            ADMIN_USER_DELETE_CAPABILITY_ID => Some(Self::AdminUserDelete),
            ADMIN_USER_PUT_SECRET_CAPABILITY_ID => Some(Self::AdminUserPutSecret),
            ADMIN_USER_DELETE_SECRET_CAPABILITY_ID => Some(Self::AdminUserDeleteSecret),
            _ => None,
        }
    }

    pub(super) const fn success_summary(self) -> &'static str {
        match self {
            Self::OperatorSetupRun => "operator setup updated",
            Self::LlmProviderUpsert => "llm provider updated",
            Self::LlmProviderDelete => "llm provider deleted",
            Self::LlmActiveSet => "llm active provider updated",
            Self::ExtensionImport => "extension imported",
            Self::ExtensionSetupSubmit => "extension setup updated",
            Self::ProjectUpdate => "project updated",
            Self::ProjectDelete => "project deleted",
            Self::ProjectMemberAdd => "project member added",
            Self::ProjectMemberUpdate => "project member updated",
            Self::ProjectMemberRemove => "project member removed",
            Self::ThreadDelete => "thread deleted",
            Self::AutomationPause => "automation paused",
            Self::AutomationResume => "automation resumed",
            Self::AutomationRename => "automation renamed",
            Self::AutomationDelete => "automation deleted",
            Self::AdminUserUpdate => "admin user updated",
            Self::AdminUserSetStatus => "admin user status updated",
            Self::AdminUserSetRole => "admin user role updated",
            Self::AdminUserDelete => "admin user deleted",
            Self::AdminUserPutSecret => "admin user protected value updated",
            Self::AdminUserDeleteSecret => "admin user protected value deleted",
        }
    }

    pub(super) async fn invoke<I, V>(
        self,
        services: &RebornServices<I, V>,
        caller: WebUiAuthenticatedCaller,
        input: ProductCapabilityInput,
    ) -> Result<(), RebornServicesError>
    where
        I: ProductCapabilityInvoker + Clone + 'static,
        V: RebornViewProvider + Clone + 'static,
    {
        match self {
            Self::OperatorSetupRun => {
                services
                    .invoke_operator_setup_run(caller, input.into_json()?)
                    .await
            }
            Self::LlmProviderUpsert => {
                let ProductCapabilityInput::LlmProviderUpsert(request) = input else {
                    return Err(product_capability_input_error("input"));
                };
                services.invoke_llm_provider_upsert(caller, request).await
            }
            Self::LlmProviderDelete => {
                services
                    .invoke_llm_provider_delete(caller, input.into_json()?)
                    .await
            }
            Self::LlmActiveSet => {
                services
                    .invoke_llm_active_set(caller, input.into_json()?)
                    .await
            }
            Self::ExtensionImport => {
                extensions::import_extension_capability(
                    services.lifecycle_facade.as_ref(),
                    caller,
                    input.into_json()?,
                )
                .await
            }
            Self::ExtensionSetupSubmit => {
                lifecycle_setup::submit_extension_setup_capability(
                    services.lifecycle_facade.as_ref(),
                    services.extension_credentials.as_deref(),
                    services.channel_config_facade.as_deref(),
                    caller,
                    input.into_json()?,
                )
                .await
            }
            Self::ProjectUpdate => {
                let request = product_command_input(input.into_json()?)?;
                services.update_project(caller, request).await?;
                Ok(())
            }
            Self::ProjectDelete => {
                let request = product_command_input(input.into_json()?)?;
                services.delete_project(caller, request).await?;
                Ok(())
            }
            Self::ProjectMemberAdd => {
                let request = product_command_input(input.into_json()?)?;
                services.add_project_member(caller, request).await?;
                Ok(())
            }
            Self::ProjectMemberUpdate => {
                let request = product_command_input(input.into_json()?)?;
                services.update_project_member_role(caller, request).await?;
                Ok(())
            }
            Self::ProjectMemberRemove => {
                let request = product_command_input(input.into_json()?)?;
                services.remove_project_member(caller, request).await?;
                Ok(())
            }
            Self::ThreadDelete => {
                let request = product_command_input(input.into_json()?)?;
                services.delete_thread(caller, request).await?;
                Ok(())
            }
            Self::AutomationPause => {
                let request: RebornAutomationRequest = product_command_input(input.into_json()?)?;
                services
                    .pause_automation(caller, request.automation_id)
                    .await?;
                Ok(())
            }
            Self::AutomationResume => {
                let request: RebornAutomationRequest = product_command_input(input.into_json()?)?;
                services
                    .resume_automation(caller, request.automation_id)
                    .await?;
                Ok(())
            }
            Self::AutomationRename => {
                let request: RebornRenameAutomationProductRequest =
                    product_command_input(input.into_json()?)?;
                services
                    .rename_automation(
                        caller,
                        request.automation_id,
                        WebUiRenameAutomationRequest { name: request.name },
                    )
                    .await?;
                Ok(())
            }
            Self::AutomationDelete => {
                let request: RebornAutomationRequest = product_command_input(input.into_json()?)?;
                services
                    .delete_automation(caller, request.automation_id)
                    .await?;
                Ok(())
            }
            Self::AdminUserUpdate => {
                let request: RebornAdminUpdateUserProductRequest =
                    product_command_input(input.into_json()?)?;
                services
                    .update_admin_user(
                        caller,
                        request.user_id,
                        RebornAdminUpdateUserRequest {
                            display_name: request.display_name,
                            metadata: request.metadata,
                        },
                    )
                    .await?;
                Ok(())
            }
            Self::AdminUserSetStatus => {
                let request: RebornAdminSetStatusProductRequest =
                    product_command_input(input.into_json()?)?;
                services
                    .set_admin_user_status(
                        caller,
                        request.user_id,
                        RebornAdminSetStatusRequest {
                            status: request.status,
                        },
                    )
                    .await?;
                Ok(())
            }
            Self::AdminUserSetRole => {
                let request: RebornAdminSetRoleProductRequest =
                    product_command_input(input.into_json()?)?;
                services
                    .set_admin_user_role(
                        caller,
                        request.user_id,
                        RebornAdminSetRoleRequest { role: request.role },
                    )
                    .await?;
                Ok(())
            }
            Self::AdminUserDelete => {
                let request: RebornAdminUserRequest = product_command_input(input.into_json()?)?;
                services.delete_admin_user(caller, request.user_id).await?;
                Ok(())
            }
            Self::AdminUserPutSecret => {
                let request: RebornAdminPutSecretProductRequest =
                    product_command_input(input.into_json()?)?;
                let handle = product_secret_handle(request.handle)?;
                services
                    .put_admin_user_secret(
                        caller,
                        request.user_id,
                        handle,
                        RebornAdminPutSecretRequest {
                            value: request.value,
                        },
                    )
                    .await?;
                Ok(())
            }
            Self::AdminUserDeleteSecret => {
                let request: RebornAdminDeleteSecretProductRequest =
                    product_command_input(input.into_json()?)?;
                let handle = product_secret_handle(request.handle)?;
                services
                    .delete_admin_user_secret(caller, request.user_id, handle)
                    .await?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_product_workflow_capability_operations() {
        for id in [
            OPERATOR_SETUP_RUN_CAPABILITY_ID,
            LLM_PROVIDER_UPSERT_CAPABILITY_ID,
            LLM_PROVIDER_DELETE_CAPABILITY_ID,
            LLM_ACTIVE_SET_CAPABILITY_ID,
            EXTENSION_IMPORT_CAPABILITY_ID,
            EXTENSION_SETUP_SUBMIT_CAPABILITY_ID,
            PROJECT_UPDATE_CAPABILITY_ID,
            PROJECT_DELETE_CAPABILITY_ID,
            PROJECT_MEMBER_ADD_CAPABILITY_ID,
            PROJECT_MEMBER_UPDATE_CAPABILITY_ID,
            PROJECT_MEMBER_REMOVE_CAPABILITY_ID,
            THREAD_DELETE_CAPABILITY_ID,
            AUTOMATION_PAUSE_CAPABILITY_ID,
            AUTOMATION_RESUME_CAPABILITY_ID,
            AUTOMATION_RENAME_CAPABILITY_ID,
            AUTOMATION_DELETE_CAPABILITY_ID,
            ADMIN_USER_UPDATE_CAPABILITY_ID,
            ADMIN_USER_SET_STATUS_CAPABILITY_ID,
            ADMIN_USER_SET_ROLE_CAPABILITY_ID,
            ADMIN_USER_DELETE_CAPABILITY_ID,
            ADMIN_USER_PUT_SECRET_CAPABILITY_ID,
            ADMIN_USER_DELETE_SECRET_CAPABILITY_ID,
        ] {
            let capability = CapabilityId::new(id).expect("valid capability id");
            assert!(
                ProductOperationHandler::parse(&capability).is_some(),
                "{id} must be registry-backed"
            );
        }
    }

    #[test]
    fn runtime_backed_capabilities_stay_out_of_product_operation_registry() {
        for id in [
            EXTENSION_INSTALL_CAPABILITY_ID,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            EXTENSION_REMOVE_CAPABILITY_ID,
            SKILL_INSTALL_CAPABILITY_ID,
            SKILL_UPDATE_CAPABILITY_ID,
            SKILL_REMOVE_CAPABILITY_ID,
            SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
        ] {
            let capability = CapabilityId::new(id).expect("valid capability id");
            assert!(
                ProductOperationHandler::parse(&capability).is_none(),
                "{id} should delegate to the runtime first-party invoker"
            );
        }
    }
}
