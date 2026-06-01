use std::{fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_auth::{AuthFlowId, AuthProductError, CredentialAccountId};
use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::SecretStore;
use secrecy::SecretString;

/// Boundary for turning Google token material into durable secret handles.
#[async_trait]
pub(super) trait GoogleProviderTokenSink: Send + Sync {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError>;

    async fn store_refreshed_tokens(
        &self,
        request: GoogleProviderRefreshTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError>;

    async fn load_refresh_token(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError>;

    async fn delete_tokens(
        &self,
        scope: &ResourceScope,
        handles: &[SecretHandle],
    ) -> Result<(), AuthProductError>;
}

/// Raw Google token material passed exactly once to the injected storage
/// boundary. This type intentionally does not implement serde.
pub(super) struct GoogleProviderTokenSet {
    pub(super) access_token: SecretString,
    pub(super) refresh_token: Option<SecretString>,
}

impl fmt::Debug for GoogleProviderTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Scoped token-storage request. Raw provider token material must be bound to
/// the already-claimed callback scope and flow before it reaches storage.
pub(super) struct GoogleProviderTokenStorageRequest {
    pub(super) scope: ResourceScope,
    pub(super) flow_id: AuthFlowId,
    pub(super) tokens: GoogleProviderTokenSet,
}

impl fmt::Debug for GoogleProviderTokenStorageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenStorageRequest")
            .field("scope", &self.scope)
            .field("flow_id", &self.flow_id)
            .field("tokens", &self.tokens)
            .finish()
    }
}

/// Scoped token-storage request for account refresh. Raw token material is
/// passed exactly once to storage, and returned values are handles only.
pub(super) struct GoogleProviderRefreshTokenStorageRequest {
    pub(super) scope: ResourceScope,
    pub(super) account_id: CredentialAccountId,
    pub(super) tokens: GoogleProviderTokenSet,
}

impl fmt::Debug for GoogleProviderRefreshTokenStorageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderRefreshTokenStorageRequest")
            .field("scope", &self.scope)
            .field("account_id", &self.account_id)
            .field("tokens", &self.tokens)
            .finish()
    }
}

/// Durable secret handles produced after Google OAuth token material is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GoogleProviderStoredTokens {
    pub(super) access_secret: SecretHandle,
    pub(super) refresh_secret: Option<SecretHandle>,
}

pub(super) struct SecretStoreGoogleTokenSink {
    pub(super) store: Arc<dyn SecretStore>,
}

#[async_trait]
impl GoogleProviderTokenSink for SecretStoreGoogleTokenSink {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
        let access_secret = google_token_handle(&request, "access")?;
        let refresh_handle = request
            .tokens
            .refresh_token
            .as_ref()
            .map(|_| google_token_handle(&request, "refresh"))
            .transpose()?;
        let GoogleProviderTokenStorageRequest {
            scope,
            tokens,
            flow_id: _,
        } = request;
        let GoogleProviderTokenSet {
            access_token,
            refresh_token,
        } = tokens;
        self.store
            .put(scope.clone(), access_secret.clone(), access_token)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;

        let refresh_secret = match refresh_token {
            Some(refresh_token) => {
                let handle = refresh_handle.ok_or(AuthProductError::BackendUnavailable)?;
                if let Err(error) = self
                    .store
                    .put(scope.clone(), handle.clone(), refresh_token)
                    .await
                {
                    if let Err(delete_error) = self.store.delete(&scope, &access_secret).await {
                        tracing::debug!(
                            secret_store_reason = delete_error.stable_reason(),
                            "google oauth callback cleanup failed after refresh token write failure"
                        );
                    }
                    return Err(map_secret_store_error(error));
                }
                Some(handle)
            }
            None => None,
        };

        Ok(GoogleProviderStoredTokens {
            access_secret,
            refresh_secret,
        })
    }

    async fn store_refreshed_tokens(
        &self,
        request: GoogleProviderRefreshTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
        let access_secret = google_refresh_token_handle(&request, "access")?;
        let refresh_handle = request
            .tokens
            .refresh_token
            .as_ref()
            .map(|_| google_refresh_token_handle(&request, "refresh"))
            .transpose()?;
        let GoogleProviderRefreshTokenStorageRequest {
            scope,
            tokens,
            account_id: _,
        } = request;
        let GoogleProviderTokenSet {
            access_token,
            refresh_token,
        } = tokens;
        self.store
            .put(scope.clone(), access_secret.clone(), access_token)
            .await
            .map_err(map_secret_store_error)?;

        let refresh_secret = match refresh_token {
            Some(refresh_token) => {
                let handle = refresh_handle.ok_or(AuthProductError::BackendUnavailable)?;
                if let Err(error) = self
                    .store
                    .put(scope.clone(), handle.clone(), refresh_token)
                    .await
                {
                    return Err(map_secret_store_error(error));
                }
                Some(handle)
            }
            None => None,
        };

        Ok(GoogleProviderStoredTokens {
            access_secret,
            refresh_secret,
        })
    }

    async fn load_refresh_token(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError> {
        let lease = self
            .store
            .lease_once(scope, handle)
            .await
            .map_err(map_refresh_secret_error)?;
        self.store
            .consume(scope, lease.id)
            .await
            .map_err(map_refresh_secret_error)
    }

    async fn delete_tokens(
        &self,
        scope: &ResourceScope,
        handles: &[SecretHandle],
    ) -> Result<(), AuthProductError> {
        let mut first_error = None;
        for handle in handles {
            if let Err(error) = self.store.delete(scope, handle).await
                && first_error.is_none()
            {
                first_error = Some(map_secret_store_error(error));
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn google_token_handle(
    request: &GoogleProviderTokenStorageRequest,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "google-oauth-{token_kind}-{}-{}",
        request.flow_id, request.scope.invocation_id
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

fn google_refresh_token_handle(
    request: &GoogleProviderRefreshTokenStorageRequest,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "google-oauth-refresh-{token_kind}-{}",
        request.account_id
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

fn map_secret_store_error(error: ironclaw_secrets::SecretStoreError) -> AuthProductError {
    tracing::debug!(
        secret_store_reason = error.stable_reason(),
        "google oauth secret store operation failed"
    );
    AuthProductError::BackendUnavailable
}

fn map_refresh_secret_error(error: ironclaw_secrets::SecretStoreError) -> AuthProductError {
    if error.is_unknown_secret()
        || error.is_unknown_lease()
        || error.is_consumed()
        || error.is_revoked()
        || error.is_expired()
    {
        AuthProductError::RefreshFailed
    } else {
        AuthProductError::BackendUnavailable
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, TenantId, UserId};
    use ironclaw_secrets::{
        SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    };
    use secrecy::SecretString;

    use super::{
        GoogleProviderRefreshTokenStorageRequest, GoogleProviderTokenSet, GoogleProviderTokenSink,
        SecretStoreGoogleTokenSink, google_refresh_token_handle,
    };

    fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id,
        }
    }

    fn sample_request(
        invocation_id: InvocationId,
        account_id: ironclaw_auth::CredentialAccountId,
    ) -> GoogleProviderRefreshTokenStorageRequest {
        GoogleProviderRefreshTokenStorageRequest {
            scope: sample_scope(invocation_id),
            account_id,
            tokens: GoogleProviderTokenSet {
                access_token: SecretString::new("access".into()),
                refresh_token: Some(SecretString::new("refresh".into())),
            },
        }
    }

    struct RecordingSecretStore {
        puts: Mutex<Vec<String>>,
        deleted: Mutex<Vec<String>>,
        failing_handles: HashSet<String>,
    }

    impl RecordingSecretStore {
        fn new(failing_handles: impl IntoIterator<Item = impl Into<String>>) -> Self {
            Self {
                puts: Mutex::new(Vec::new()),
                deleted: Mutex::new(Vec::new()),
                failing_handles: failing_handles.into_iter().map(Into::into).collect(),
            }
        }

        fn put_handles(&self) -> Vec<String> {
            self.puts.lock().unwrap().clone()
        }

        fn deleted_handles(&self) -> Vec<String> {
            self.deleted.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SecretStore for RecordingSecretStore {
        async fn put(
            &self,
            scope: ResourceScope,
            handle: SecretHandle,
            _material: SecretMaterial,
        ) -> Result<SecretMetadata, ironclaw_secrets::SecretStoreError> {
            self.puts.lock().unwrap().push(handle.as_str().to_string());
            if self.failing_handles.contains(handle.as_str()) {
                Err(ironclaw_secrets::SecretStoreError::BackendMisconfigured {
                    reason: format!("failed to write {}", handle.as_str()),
                })
            } else {
                Ok(SecretMetadata { scope, handle })
            }
        }

        async fn metadata(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<Option<SecretMetadata>, ironclaw_secrets::SecretStoreError> {
            unreachable!("not used in tests")
        }

        async fn delete(
            &self,
            _scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<bool, ironclaw_secrets::SecretStoreError> {
            self.deleted
                .lock()
                .unwrap()
                .push(handle.as_str().to_string());
            if self.failing_handles.contains(handle.as_str()) {
                Err(ironclaw_secrets::SecretStoreError::BackendMisconfigured {
                    reason: format!("failed to delete {}", handle.as_str()),
                })
            } else {
                Ok(true)
            }
        }

        async fn lease_once(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<SecretLease, ironclaw_secrets::SecretStoreError> {
            unreachable!("not used in tests")
        }

        async fn consume(
            &self,
            _scope: &ResourceScope,
            _lease_id: SecretLeaseId,
        ) -> Result<SecretMaterial, ironclaw_secrets::SecretStoreError> {
            unreachable!("not used in tests")
        }

        async fn revoke(
            &self,
            _scope: &ResourceScope,
            _lease_id: SecretLeaseId,
        ) -> Result<SecretLease, ironclaw_secrets::SecretStoreError> {
            unreachable!("not used in tests")
        }

        async fn leases_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<SecretLease>, ironclaw_secrets::SecretStoreError> {
            unreachable!("not used in tests")
        }
    }

    #[tokio::test]
    async fn google_refresh_token_handles_ignore_invocation_id() {
        let account_id = ironclaw_auth::CredentialAccountId::new();
        let request_a = sample_request(InvocationId::new(), account_id);
        let request_b = sample_request(InvocationId::new(), account_id);

        let access_a = google_refresh_token_handle(&request_a, "access").unwrap();
        let access_b = google_refresh_token_handle(&request_b, "access").unwrap();
        let refresh_a = google_refresh_token_handle(&request_a, "refresh").unwrap();
        let refresh_b = google_refresh_token_handle(&request_b, "refresh").unwrap();

        assert_eq!(access_a, access_b);
        assert_eq!(refresh_a, refresh_b);
    }

    #[tokio::test]
    async fn delete_tokens_attempts_every_handle_before_returning_first_error() {
        let store = Arc::new(RecordingSecretStore::new(["second"]));
        let sink = SecretStoreGoogleTokenSink {
            store: store.clone(),
        };
        let scope = sample_scope(InvocationId::new());
        let handles = vec![
            SecretHandle::new("first").unwrap(),
            SecretHandle::new("second").unwrap(),
            SecretHandle::new("third").unwrap(),
        ];

        let error = sink.delete_tokens(&scope, &handles).await.unwrap_err();

        assert_eq!(error, ironclaw_auth::AuthProductError::BackendUnavailable);
        assert_eq!(
            store.deleted_handles(),
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn store_refreshed_tokens_keeps_access_secret_when_refresh_write_fails() {
        let account_id = ironclaw_auth::CredentialAccountId::new();
        let scope = sample_scope(InvocationId::new());
        let request = GoogleProviderRefreshTokenStorageRequest {
            scope: scope.clone(),
            account_id,
            tokens: GoogleProviderTokenSet {
                access_token: SecretString::new("access".into()),
                refresh_token: Some(SecretString::new("refresh".into())),
            },
        };
        let access_handle = google_refresh_token_handle(&request, "access").unwrap();
        let refresh_handle = google_refresh_token_handle(&request, "refresh").unwrap();
        let store = Arc::new(RecordingSecretStore::new([refresh_handle.as_str()]));
        let sink = SecretStoreGoogleTokenSink {
            store: store.clone(),
        };

        let error = sink.store_refreshed_tokens(request).await.unwrap_err();

        assert_eq!(error, ironclaw_auth::AuthProductError::BackendUnavailable);
        assert_eq!(
            store.put_handles(),
            vec![
                access_handle.as_str().to_string(),
                refresh_handle.as_str().to_string()
            ]
        );
        assert!(store.deleted_handles().is_empty());
    }
}
