use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle};
use ironclaw_oauth::{OAuthError, OAuthProvider, TokenPersister};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

pub const GOOGLE_CREDENTIAL_NAME: &str = "google_oauth_token";
const REFCOUNT_ROW: &str = "google_oauth_token_refs";
const REFRESH_BUFFER: Duration = Duration::seconds(60);

#[derive(Debug, Clone)]
pub struct GoogleCredential {
    pub access_token: SecretString,
    pub granted_scopes: Vec<String>,
    pub missing_scopes: Vec<String>,
    pub refresh_required: bool,
}

#[derive(Debug, Error)]
pub enum GoogleCredentialError {
    #[error("Google credential is missing")]
    Missing,
    #[error(transparent)]
    OAuth(#[from] OAuthError),
    #[error(transparent)]
    HostApi(#[from] ironclaw_host_api::HostApiError),
    #[error(transparent)]
    Secret(#[from] ironclaw_secrets::SecretStoreError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct GoogleCredentialResolver {
    secrets: Arc<dyn SecretStore>,
    tokens: TokenPersister,
}

impl GoogleCredentialResolver {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            tokens: TokenPersister::new(secrets.clone()),
            secrets,
        }
    }

    pub async fn resolve(
        &self,
        scope: &ResourceScope,
        provider: &dyn OAuthProvider,
        required_scopes: &[String],
    ) -> Result<GoogleCredential, GoogleCredentialError> {
        let Some(access_token) = self
            .tokens
            .load_access_token(scope, provider.credential_name())
            .await?
        else {
            return Err(GoogleCredentialError::Missing);
        };
        let granted_scopes = self
            .tokens
            .load_scopes(scope, provider.credential_name())
            .await?;
        let missing_scopes = provider.detect_scope_mismatch(&granted_scopes, required_scopes);
        let refresh_required = self
            .tokens
            .load_expiry(scope, provider.credential_name())
            .await?
            .is_some_and(|expiry| expiry <= Utc::now() + REFRESH_BUFFER);
        Ok(GoogleCredential {
            access_token,
            granted_scopes,
            missing_scopes,
            refresh_required,
        })
    }

    pub async fn add_ref(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ExtensionId>, GoogleCredentialError> {
        let mut refs = self.load_refs(scope).await?;
        if !refs.contains(extension_id) {
            refs.push(extension_id.clone());
            self.store_refs(scope, &refs).await?;
        }
        Ok(refs)
    }

    pub async fn remove_ref(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ExtensionId>, GoogleCredentialError> {
        let mut refs = self.load_refs(scope).await?;
        refs.retain(|id| id != extension_id);
        if refs.is_empty() {
            self.delete_credential_rows(scope).await?;
        } else {
            self.store_refs(scope, &refs).await?;
        }
        Ok(refs)
    }

    pub async fn load_refs(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ExtensionId>, GoogleCredentialError> {
        let handle = SecretHandle::new(REFCOUNT_ROW)?;
        if self.secrets.metadata(scope, &handle).await?.is_none() {
            return Ok(Vec::new());
        }
        let lease = self.secrets.lease_once(scope, &handle).await?;
        let material = self.secrets.consume(scope, lease.id).await?;
        Ok(serde_json::from_str(material.expose_secret())?)
    }

    async fn store_refs(
        &self,
        scope: &ResourceScope,
        refs: &[ExtensionId],
    ) -> Result<(), GoogleCredentialError> {
        let handle = SecretHandle::new(REFCOUNT_ROW)?;
        let material = SecretMaterial::from(serde_json::to_string(refs)?);
        self.secrets.put(scope.clone(), handle, material).await?;
        Ok(())
    }

    async fn delete_credential_rows(
        &self,
        scope: &ResourceScope,
    ) -> Result<(), GoogleCredentialError> {
        for row in [
            GOOGLE_CREDENTIAL_NAME.to_string(),
            format!("{GOOGLE_CREDENTIAL_NAME}_refresh_token"),
            format!("{GOOGLE_CREDENTIAL_NAME}_scopes"),
            format!("{GOOGLE_CREDENTIAL_NAME}_expiry"),
            REFCOUNT_ROW.to_string(),
        ] {
            self.secrets.delete(scope, &SecretHandle::new(row)?).await?;
        }
        Ok(())
    }
}
