//! Credential injection for WASM HTTP requests.
//!
//! Injects secrets into HTTP requests at the host boundary.
//! WASM tools NEVER see the actual credential values.
//!
//! # Injection Flow
//!
//! ```text
//! WASM requests HTTP ──► Host receives request ──► Match credentials by host
//!                                                        │
//!                                    ┌───────────────────┘
//!                                    ▼
//!                        Decrypt secret from store
//!                                    │
//!                                    ▼
//!                        Inject into request:
//!                        ├─► Authorization header (Bearer/Basic)
//!                        ├─► Custom header (X-API-Key, etc.)
//!                        └─► Query parameter
//!                                    │
//!                                    ▼
//!                        Execute HTTP request
//! ```

use std::collections::HashMap;

use crate::secrets::{
    CredentialLocation, CredentialMapping, DecryptedSecret, SecretError, SecretsStore,
};

/// Error during credential injection.
#[derive(Debug, Clone, thiserror::Error)]
pub enum InjectionError {
    #[error("Secret not found: {0}")]
    SecretNotFound(String),

    #[error("Secret access denied: {0}")]
    AccessDenied(String),

    #[error("Secret has expired: {0}")]
    SecretExpired(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("No matching credential for host: {0}")]
    NoMatchingCredential(String),
}

impl From<SecretError> for InjectionError {
    fn from(e: SecretError) -> Self {
        match e {
            SecretError::NotFound(name) => InjectionError::SecretNotFound(name),
            SecretError::Expired => InjectionError::SecretExpired("unknown".to_string()),
            SecretError::AccessDenied => InjectionError::AccessDenied("unknown".to_string()),
            SecretError::DecryptionFailed(msg) => InjectionError::DecryptionFailed(msg),
            _ => InjectionError::DecryptionFailed(e.to_string()),
        }
    }
}

/// Result of credential injection.
#[derive(Debug)]
pub struct InjectedCredentials {
    /// Headers to add to the request.
    pub headers: HashMap<String, String>,
    /// Query parameters to add.
    pub query_params: HashMap<String, String>,
}

impl InjectedCredentials {
    pub fn empty() -> Self {
        Self {
            headers: HashMap::new(),
            query_params: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.query_params.is_empty()
    }
}

/// Injects credentials into HTTP requests.
pub struct CredentialInjector {
    mappings: HashMap<String, CredentialMapping>,
    allowed_secrets: Vec<String>,
}

impl CredentialInjector {
    /// Create a new injector with the given mappings.
    pub fn new(mappings: HashMap<String, CredentialMapping>, allowed_secrets: Vec<String>) -> Self {
        Self {
            mappings,
            allowed_secrets,
        }
    }

    /// Find credentials that should be injected for a given host.
    pub fn find_credentials_for_host(&self, host: &str) -> Vec<&CredentialMapping> {
        self.mappings
            .values()
            .filter(|mapping| {
                mapping
                    .host_patterns
                    .iter()
                    .any(|pattern| host_matches_pattern(host, pattern))
            })
            .collect()
    }

    /// Inject credentials for an HTTP request.
    ///
    /// Returns the headers and query params to add to the request.
    pub async fn inject(
        &self,
        user_id: &str,
        host: &str,
        store: &dyn SecretsStore,
    ) -> Result<InjectedCredentials, InjectionError> {
        let matching_mappings = self.find_credentials_for_host(host);

        if matching_mappings.is_empty() {
            // No credentials needed for this host
            return Ok(InjectedCredentials::empty());
        }

        let mut result = InjectedCredentials::empty();

        for mapping in matching_mappings {
            // Check if secret is in allowed list
            if !self.is_secret_allowed(&mapping.secret_name) {
                return Err(InjectionError::AccessDenied(mapping.secret_name.clone()));
            }

            // Get the decrypted secret
            let secret = store
                .get_decrypted(user_id, &mapping.secret_name)
                .await
                .map_err(|e| match e {
                    SecretError::NotFound(name) => InjectionError::SecretNotFound(name),
                    SecretError::Expired => {
                        InjectionError::SecretExpired(mapping.secret_name.clone())
                    }
                    _ => InjectionError::DecryptionFailed(e.to_string()),
                })?;

            // Inject based on location
            inject_credential(&mut result, &mapping.location, &secret);
        }

        Ok(result)
    }

    /// Check if a secret name is in the allowed list.
    fn is_secret_allowed(&self, name: &str) -> bool {
        for pattern in &self.allowed_secrets {
            if pattern == name {
                return true;
            }
            if let Some(prefix) = pattern.strip_suffix('*')
                && name.starts_with(prefix)
            {
                return true;
            }
        }
        false
    }
}

/// Inject a single credential into the result.
pub(crate) fn inject_credential(
    result: &mut InjectedCredentials,
    location: &CredentialLocation,
    secret: &DecryptedSecret,
) {
    match location {
        CredentialLocation::AuthorizationBearer => {
            result.headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", secret.expose()),
            );
        }
        CredentialLocation::AuthorizationBasic { username } => {
            let credentials = format!("{}:{}", username, secret.expose());
            let encoded = base64_encode(credentials.as_bytes());
            result
                .headers
                .insert("Authorization".to_string(), format!("Basic {}", encoded));
        }
        CredentialLocation::Header { name, prefix } => {
            let value = match prefix {
                Some(p) => format!("{}{}", p, secret.expose()),
                None => secret.expose().to_string(),
            };
            result.headers.insert(name.clone(), value);
        }
        CredentialLocation::QueryParam { name } => {
            result
                .query_params
                .insert(name.clone(), secret.expose().to_string());
        }
        CredentialLocation::UrlPath { .. } => {
            // URL placeholder replacement is handled by channel/tool wrappers
            // that substitute {PLACEHOLDER} values in templated strings.
        }
    }
}

/// Check if a host matches a pattern (supports wildcards).
pub(crate) fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    if pattern == host {
        return true;
    }

    // Support wildcard: *.example.com matches sub.example.com
    if let Some(suffix) = pattern.strip_prefix("*.")
        && host.ends_with(suffix)
        && host.len() > suffix.len()
    {
        let prefix = &host[..host.len() - suffix.len()];
        if prefix.ends_with('.') || prefix.is_empty() {
            return true;
        }
    }

    false
}

/// Simple base64 encoding (avoids extra dependency).
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::new();
    let mut i = 0;

    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };

        result.push(ALPHABET[(b0 >> 2) as usize] as char);
        result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if i + 1 < input.len() {
            result.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < input.len() {
            result.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }

    result
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use secrecy::SecretString;

    use crate::secrets::{
        CreateSecretParams, CredentialLocation, CredentialMapping, InMemorySecretsStore,
        SecretsCrypto, SecretsStore,
    };
    use crate::tools::wasm::credential_injector::{
        CredentialInjector, base64_encode, host_matches_pattern,
    };

    fn test_store() -> InMemorySecretsStore {
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        InMemorySecretsStore::new(crypto)
    }

    #[test]
    fn test_host_matches_exact() {
        assert!(host_matches_pattern("api.openai.com", "api.openai.com"));
        assert!(!host_matches_pattern("api.openai.com", "other.com"));
    }

    #[test]
    fn test_host_matches_wildcard() {
        assert!(host_matches_pattern("api.example.com", "*.example.com"));
        assert!(host_matches_pattern("sub.api.example.com", "*.example.com"));
        assert!(!host_matches_pattern("example.com", "*.example.com"));
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[tokio::test]
    async fn test_inject_bearer() {
        let store = test_store();
        store
            .create("user1", CreateSecretParams::new("openai_key", "sk-test123"))
            .await
            .unwrap();

        let mut mappings = HashMap::new();
        mappings.insert(
            "openai".to_string(),
            CredentialMapping {
                secret_name: "openai_key".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["api.openai.com".to_string()],
            },
        );

        let injector = CredentialInjector::new(mappings, vec!["openai_key".to_string()]);
        let result = injector
            .inject("user1", "api.openai.com", &store)
            .await
            .unwrap();

        assert_eq!(
            result.headers.get("Authorization"),
            Some(&"Bearer sk-test123".to_string())
        );
    }

    #[tokio::test]
    async fn test_inject_custom_header() {
        let store = test_store();
        store
            .create("user1", CreateSecretParams::new("api_key", "secret123"))
            .await
            .unwrap();

        let mut mappings = HashMap::new();
        mappings.insert(
            "custom".to_string(),
            CredentialMapping {
                secret_name: "api_key".to_string(),
                location: CredentialLocation::Header {
                    name: "X-API-Key".to_string(),
                    prefix: None,
                },
                host_patterns: vec!["*.example.com".to_string()],
            },
        );

        let injector = CredentialInjector::new(mappings, vec!["api_key".to_string()]);
        let result = injector
            .inject("user1", "api.example.com", &store)
            .await
            .unwrap();

        assert_eq!(
            result.headers.get("X-API-Key"),
            Some(&"secret123".to_string())
        );
    }

    #[tokio::test]
    async fn test_inject_basic_auth() {
        let store = test_store();
        store
            .create("user1", CreateSecretParams::new("password", "mypassword"))
            .await
            .unwrap();

        let mut mappings = HashMap::new();
        mappings.insert(
            "basic".to_string(),
            CredentialMapping {
                secret_name: "password".to_string(),
                location: CredentialLocation::AuthorizationBasic {
                    username: "myuser".to_string(),
                },
                host_patterns: vec!["api.service.com".to_string()],
            },
        );

        let injector = CredentialInjector::new(mappings, vec!["password".to_string()]);
        let result = injector
            .inject("user1", "api.service.com", &store)
            .await
            .unwrap();

        // myuser:mypassword base64 encoded
        let expected = format!("Basic {}", base64_encode(b"myuser:mypassword"));
        assert_eq!(result.headers.get("Authorization"), Some(&expected));
    }

    #[tokio::test]
    async fn test_no_credentials_for_host() {
        let store = test_store();

        let injector = CredentialInjector::new(HashMap::new(), vec![]);
        let result = injector
            .inject("user1", "unknown.com", &store)
            .await
            .unwrap();

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_access_denied_for_secret() {
        let store = test_store();
        store
            .create("user1", CreateSecretParams::new("secret_key", "value"))
            .await
            .unwrap();

        let mut mappings = HashMap::new();
        mappings.insert(
            "test".to_string(),
            CredentialMapping {
                secret_name: "secret_key".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["api.test.com".to_string()],
            },
        );

        // Empty allowed list = nothing allowed
        let injector = CredentialInjector::new(mappings, vec![]);
        let result = injector.inject("user1", "api.test.com", &store).await;

        assert!(result.is_err());
    }
}
