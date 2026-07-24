use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProductCommandHandler {
    CreateThread,
    SubmitTurn,
    CancelRun,
    ResolveGate,
    RetryRun,
    ProjectCreate,
    ProjectFsRead,
    FsRead,
    AttachmentRead,
    TraceAccountLoginLink,
    TraceHoldAuthorize,
    OperatorConfigSetKey,
    OperatorServiceLifecycle,
    LlmTestConnection,
    LlmListModels,
    LlmNearAiLogin,
    LlmNearAiWalletLogin,
    LlmCodexLogin,
    AdminUserCreate,
    AdminUserDeleteSecret,
    AutomationPause,
    AutomationResume,
    AutomationRename,
    AutomationDelete,
}

impl ProductCommandHandler {
    pub(super) fn parse(capability: &CapabilityId) -> Option<Self> {
        match capability.as_str() {
            CREATE_THREAD_COMMAND_ID => Some(Self::CreateThread),
            SUBMIT_TURN_COMMAND_ID => Some(Self::SubmitTurn),
            CANCEL_RUN_COMMAND_ID => Some(Self::CancelRun),
            RESOLVE_GATE_COMMAND_ID => Some(Self::ResolveGate),
            RETRY_RUN_COMMAND_ID => Some(Self::RetryRun),
            PROJECT_CREATE_COMMAND_ID => Some(Self::ProjectCreate),
            PROJECT_FS_READ_COMMAND_ID => Some(Self::ProjectFsRead),
            FS_READ_COMMAND_ID => Some(Self::FsRead),
            ATTACHMENT_READ_COMMAND_ID => Some(Self::AttachmentRead),
            TRACE_ACCOUNT_LOGIN_LINK_COMMAND_ID => Some(Self::TraceAccountLoginLink),
            TRACE_HOLD_AUTHORIZE_COMMAND_ID => Some(Self::TraceHoldAuthorize),
            OPERATOR_CONFIG_SET_KEY_COMMAND_ID => Some(Self::OperatorConfigSetKey),
            OPERATOR_SERVICE_LIFECYCLE_COMMAND_ID => Some(Self::OperatorServiceLifecycle),
            LLM_TEST_CONNECTION_COMMAND_ID => Some(Self::LlmTestConnection),
            LLM_LIST_MODELS_COMMAND_ID => Some(Self::LlmListModels),
            LLM_NEARAI_LOGIN_COMMAND_ID => Some(Self::LlmNearAiLogin),
            LLM_NEARAI_WALLET_LOGIN_COMMAND_ID => Some(Self::LlmNearAiWalletLogin),
            LLM_CODEX_LOGIN_COMMAND_ID => Some(Self::LlmCodexLogin),
            ADMIN_USER_CREATE_COMMAND_ID => Some(Self::AdminUserCreate),
            ADMIN_USER_DELETE_SECRET_COMMAND_ID => Some(Self::AdminUserDeleteSecret),
            AUTOMATION_PAUSE_COMMAND_ID => Some(Self::AutomationPause),
            AUTOMATION_RESUME_COMMAND_ID => Some(Self::AutomationResume),
            AUTOMATION_RENAME_COMMAND_ID => Some(Self::AutomationRename),
            AUTOMATION_DELETE_COMMAND_ID => Some(Self::AutomationDelete),
            _ => None,
        }
    }

    pub(super) async fn invoke<I, V>(
        self,
        services: &RebornServices<I, V>,
        caller: ProductSurfaceCaller,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ProductSurfaceError>
    where
        I: ProductCapabilityInvoker + Clone + 'static,
        V: RebornViewProvider + Clone + 'static,
    {
        match self {
            Self::CreateThread => command_output(
                services
                    .create_thread(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::SubmitTurn => command_output(
                services
                    .submit_turn(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::CancelRun => command_output(
                services
                    .cancel_run(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::ResolveGate => command_output(
                services
                    .resolve_gate(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::RetryRun => command_output(
                services
                    .retry_run(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::ProjectCreate => command_output(
                services
                    .create_project(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::ProjectFsRead => command_output(
                services
                    .read_project_file(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::FsRead => command_output(
                services
                    .read_fs_file(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::AttachmentRead => command_output(
                services
                    .read_attachment(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::TraceAccountLoginLink => {
                let _: EmptyProductCommandInput = product_command_input(input)?;
                command_output(services.trace_account_login_link(caller).await?)
            }
            Self::TraceHoldAuthorize => command_output(
                services
                    .authorize_trace_hold(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::OperatorConfigSetKey => {
                let request: RebornOperatorConfigSetProductRequest = product_command_input(input)?;
                command_output(
                    services
                        .set_operator_config_key(
                            caller,
                            request.key,
                            RebornOperatorConfigSetRequest {
                                value: request.value,
                            },
                        )
                        .await?,
                )
            }
            Self::OperatorServiceLifecycle => command_output(
                services
                    .run_operator_service_lifecycle(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::LlmTestConnection => command_output(
                services
                    .test_llm_connection(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::LlmListModels => command_output(
                services
                    .list_llm_models(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::LlmNearAiLogin => command_output(
                services
                    .start_nearai_login(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::LlmNearAiWalletLogin => command_output(
                services
                    .complete_nearai_wallet_login(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::LlmCodexLogin => {
                let _: EmptyProductCommandInput = product_command_input(input)?;
                command_output(services.start_codex_login(caller).await?)
            }
            Self::AdminUserCreate => command_output(
                services
                    .create_admin_user(caller, product_command_input(input)?)
                    .await?,
            ),
            Self::AdminUserDeleteSecret => {
                let request: RebornAdminDeleteSecretProductRequest = product_command_input(input)?;
                let handle = product_secret_handle(request.handle)?;
                command_output(
                    services
                        .delete_admin_user_secret(caller, request.user_id, handle)
                        .await?,
                )
            }
            Self::AutomationPause => {
                let request: RebornAutomationRequest = product_command_input(input)?;
                command_output(
                    services
                        .pause_automation(caller, request.automation_id)
                        .await?,
                )
            }
            Self::AutomationResume => {
                let request: RebornAutomationRequest = product_command_input(input)?;
                command_output(
                    services
                        .resume_automation(caller, request.automation_id)
                        .await?,
                )
            }
            Self::AutomationRename => {
                let request: RebornRenameAutomationProductRequest = product_command_input(input)?;
                command_output(
                    services
                        .rename_automation(
                            caller,
                            request.automation_id,
                            ProductRenameAutomationRequest { name: request.name },
                        )
                        .await?,
                )
            }
            Self::AutomationDelete => {
                let request: RebornAutomationRequest = product_command_input(input)?;
                command_output(
                    services
                        .delete_automation(caller, request.automation_id)
                        .await?,
                )
            }
        }
    }
}

fn command_output<T: Serialize>(output: T) -> Result<serde_json::Value, ProductSurfaceError> {
    serde_json::to_value(output).map_err(ProductSurfaceError::internal_from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProductCapabilityHandler {
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

impl ProductCapabilityHandler {
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
        caller: ProductSurfaceCaller,
        input: serde_json::Value,
    ) -> Result<(), ProductSurfaceError>
    where
        I: ProductCapabilityInvoker + Clone + 'static,
        V: RebornViewProvider + Clone + 'static,
    {
        match self {
            Self::OperatorSetupRun => services.invoke_operator_setup_run(caller, input).await,
            Self::LlmProviderUpsert => {
                let request = product_command_input(input)?;
                services.invoke_llm_provider_upsert(caller, request).await
            }
            Self::LlmProviderDelete => services.invoke_llm_provider_delete(caller, input).await,
            Self::LlmActiveSet => services.invoke_llm_active_set(caller, input).await,
            Self::ExtensionImport => {
                extensions::import_extension_capability(
                    services.lifecycle_facade.as_ref(),
                    caller,
                    input,
                )
                .await
            }
            Self::ExtensionSetupSubmit => {
                lifecycle_setup::submit_extension_setup_capability(
                    services.lifecycle_facade.as_ref(),
                    services.extension_credentials.as_deref(),
                    caller,
                    input,
                )
                .await
            }
            Self::ProjectUpdate => {
                let request = product_command_input(input)?;
                services.update_project(caller, request).await?;
                Ok(())
            }
            Self::ProjectDelete => {
                let request = product_command_input(input)?;
                services.delete_project(caller, request).await?;
                Ok(())
            }
            Self::ProjectMemberAdd => {
                let request = product_command_input(input)?;
                services.add_project_member(caller, request).await?;
                Ok(())
            }
            Self::ProjectMemberUpdate => {
                let request = product_command_input(input)?;
                services.update_project_member_role(caller, request).await?;
                Ok(())
            }
            Self::ProjectMemberRemove => {
                let request = product_command_input(input)?;
                services.remove_project_member(caller, request).await?;
                Ok(())
            }
            Self::ThreadDelete => {
                let request = product_command_input(input)?;
                services.delete_thread(caller, request).await?;
                Ok(())
            }
            Self::AutomationPause => {
                let request: RebornAutomationRequest = product_command_input(input)?;
                services
                    .pause_automation(caller, request.automation_id)
                    .await?;
                Ok(())
            }
            Self::AutomationResume => {
                let request: RebornAutomationRequest = product_command_input(input)?;
                services
                    .resume_automation(caller, request.automation_id)
                    .await?;
                Ok(())
            }
            Self::AutomationRename => {
                let request: RebornRenameAutomationProductRequest = product_command_input(input)?;
                services
                    .rename_automation(
                        caller,
                        request.automation_id,
                        ProductRenameAutomationRequest { name: request.name },
                    )
                    .await?;
                Ok(())
            }
            Self::AutomationDelete => {
                let request: RebornAutomationRequest = product_command_input(input)?;
                services
                    .delete_automation(caller, request.automation_id)
                    .await?;
                Ok(())
            }
            Self::AdminUserUpdate => {
                let request: RebornAdminUpdateUserProductRequest = product_command_input(input)?;
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
                let request: RebornAdminSetStatusProductRequest = product_command_input(input)?;
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
                let request: RebornAdminSetRoleProductRequest = product_command_input(input)?;
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
                let request: RebornAdminUserRequest = product_command_input(input)?;
                services.delete_admin_user(caller, request.user_id).await?;
                Ok(())
            }
            Self::AdminUserPutSecret => {
                let request: RebornAdminPutSecretProductRequest = product_command_input(input)?;
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
                let request: RebornAdminDeleteSecretProductRequest = product_command_input(input)?;
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
                ProductCapabilityHandler::parse(&capability).is_some(),
                "{id} must be registry-backed"
            );
        }
    }

    #[test]
    fn runtime_backed_capabilities_stay_out_of_product_operation_registry() {
        for id in [
            EXTENSION_INSTALL_CAPABILITY_ID,
            EXTENSION_REMOVE_CAPABILITY_ID,
            SKILL_INSTALL_CAPABILITY_ID,
            SKILL_UPDATE_CAPABILITY_ID,
            SKILL_REMOVE_CAPABILITY_ID,
            SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
        ] {
            let capability = CapabilityId::new(id).expect("valid capability id");
            assert!(
                ProductCapabilityHandler::parse(&capability).is_none(),
                "{id} should delegate to the runtime first-party invoker"
            );
        }
    }
}
