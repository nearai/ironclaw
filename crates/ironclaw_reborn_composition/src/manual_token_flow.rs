use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthChallenge, AuthFlowKind, AuthFlowManager, AuthFlowRecord, AuthInteractionId,
    AuthInteractionService, AuthProductError, AuthProductScope, CredentialAccountService,
    CredentialAccountStatus, InMemoryAuthProductServices, ManualTokenCompletionInput,
    ManualTokenSetupRequest, NewAuthFlow, SecretSubmitRequest, SecretSubmitResult,
};
use ironclaw_filesystem::RootFilesystem;

use crate::product_auth_durable::FilesystemAuthProductServices;

#[async_trait]
#[doc(hidden)]
pub trait RebornManualTokenFlowService: Send + Sync {
    async fn request_manual_token_flow(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError>;

    async fn submit_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<(SecretSubmitResult, AuthFlowRecord), AuthProductError>;

    async fn abandon_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError>;
}

#[derive(Clone)]
pub(crate) struct PortBackedManualTokenFlowService {
    flow_manager: Arc<dyn AuthFlowManager>,
    interaction_service: Arc<dyn AuthInteractionService>,
    credential_account_service: Arc<dyn CredentialAccountService>,
}

impl PortBackedManualTokenFlowService {
    pub(crate) fn new(
        flow_manager: Arc<dyn AuthFlowManager>,
        interaction_service: Arc<dyn AuthInteractionService>,
        credential_account_service: Arc<dyn CredentialAccountService>,
    ) -> Self {
        Self {
            flow_manager,
            interaction_service,
            credential_account_service,
        }
    }
}

#[async_trait]
impl RebornManualTokenFlowService for PortBackedManualTokenFlowService {
    async fn request_manual_token_flow(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        request_manual_token_flow_with(
            self.flow_manager.as_ref(),
            self.interaction_service.as_ref(),
            request,
        )
        .await
    }

    async fn submit_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<(SecretSubmitResult, AuthFlowRecord), AuthProductError> {
        submit_manual_token_flow_with(
            self.flow_manager.as_ref(),
            self.interaction_service.as_ref(),
            self.credential_account_service.as_ref(),
            scope,
            request,
        )
        .await
    }

    async fn abandon_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        abandon_manual_token_flow_with(
            self.flow_manager.as_ref(),
            self.interaction_service.as_ref(),
            scope,
            interaction_id,
        )
        .await
    }
}

#[async_trait]
impl RebornManualTokenFlowService for InMemoryAuthProductServices {
    async fn request_manual_token_flow(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        request_manual_token_flow_with(self, self, request).await
    }

    async fn submit_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<(SecretSubmitResult, AuthFlowRecord), AuthProductError> {
        submit_manual_token_flow_with(self, self, self, scope, request).await
    }

    async fn abandon_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        abandon_manual_token_flow_with(self, self, scope, interaction_id).await
    }
}

#[async_trait]
impl<F> RebornManualTokenFlowService for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn request_manual_token_flow(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        request_manual_token_flow_with(self, self, request).await
    }

    async fn submit_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<(SecretSubmitResult, AuthFlowRecord), AuthProductError> {
        submit_manual_token_flow_with(self, self, self, scope, request).await
    }

    async fn abandon_manual_token_flow(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        abandon_manual_token_flow_with(self, self, scope, interaction_id).await
    }
}

async fn request_manual_token_flow_with(
    flow_manager: &dyn AuthFlowManager,
    interaction_service: &dyn AuthInteractionService,
    request: ManualTokenSetupRequest,
) -> Result<AuthChallenge, AuthProductError> {
    let flow_scope = request.scope.clone();
    let flow_provider = request.provider.clone();
    let flow_continuation = request.continuation.clone();
    let flow_update_binding = request.update_binding.clone();
    let flow_expires_at = request.expires_at;
    let challenge = interaction_service.request_secret_input(request).await?;
    let AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        label,
        expires_at,
    } = &challenge
    else {
        return Err(AuthProductError::InvalidRequest {
            reason: "manual token setup returned an unexpected challenge".to_string(),
        });
    };
    if let Err(error) = flow_manager
        .create_flow(NewAuthFlow {
            id: None,
            scope: flow_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: flow_provider,
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id: *interaction_id,
                provider: provider.clone(),
                label: label.clone(),
                expires_at: *expires_at,
            },
            continuation: flow_continuation,
            update_binding: flow_update_binding,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: flow_expires_at,
        })
        .await
    {
        if let Err(cleanup_error) = interaction_service
            .abandon_manual_token(&flow_scope, *interaction_id)
            .await
        {
            tracing::warn!(
                interaction_id = %interaction_id,
                error_code = ?error.code(),
                cleanup_error_code = ?cleanup_error.code(),
                "manual-token flow creation failed and interaction cleanup failed"
            );
        }
        return Err(error);
    }
    Ok(challenge)
}

async fn submit_manual_token_flow_with(
    flow_manager: &dyn AuthFlowManager,
    interaction_service: &dyn AuthInteractionService,
    credential_account_service: &dyn CredentialAccountService,
    scope: &AuthProductScope,
    request: SecretSubmitRequest,
) -> Result<(SecretSubmitResult, AuthFlowRecord), AuthProductError> {
    let interaction_id = request.interaction_id;
    let result = interaction_service
        .submit_manual_token(scope, request)
        .await?;
    let completed = match flow_manager
        .complete_manual_token(
            scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: result.account_id,
            },
        )
        .await
    {
        Ok(completed) => completed,
        Err(error) => {
            if let Err(cleanup_error) = flow_manager
                .cancel_manual_token(scope, interaction_id)
                .await
            {
                tracing::warn!(
                    interaction_id = %interaction_id,
                    error_code = ?error.code(),
                    cleanup_error_code = ?cleanup_error.code(),
                    "manual-token flow completion failed and flow cleanup failed"
                );
            }
            if let Err(cleanup_error) = credential_account_service
                .update_status(scope, result.account_id, CredentialAccountStatus::Revoked)
                .await
            {
                tracing::warn!(
                    interaction_id = %interaction_id,
                    account_id = %result.account_id,
                    error_code = ?error.code(),
                    cleanup_error_code = ?cleanup_error.code(),
                    "manual-token flow completion failed and account compensation failed"
                );
            }
            return Err(error);
        }
    };
    Ok((result, completed))
}

async fn abandon_manual_token_flow_with(
    flow_manager: &dyn AuthFlowManager,
    interaction_service: &dyn AuthInteractionService,
    scope: &AuthProductScope,
    interaction_id: AuthInteractionId,
) -> Result<bool, AuthProductError> {
    let interaction_abandoned = interaction_service
        .abandon_manual_token(scope, interaction_id)
        .await?;
    let flow_abandoned = flow_manager
        .cancel_manual_token(scope, interaction_id)
        .await?
        .is_some();
    Ok(interaction_abandoned || flow_abandoned)
}
