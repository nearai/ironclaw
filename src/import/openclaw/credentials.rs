//! OpenClaw credential import with secure handling.

use std::sync::Arc;

use secrecy::ExposeSecret;
use secrecy::SecretString;

use crate::import::ImportError;
use crate::secrets::{CreateSecretParams, SecretsStore};

/// Import credentials from OpenClaw configuration into IronClaw.
///
/// Credentials are never logged or printed. This function uses the
/// secrets store's upsert semantics to safely re-import without duplicates.
pub async fn import_credentials(
    secrets: &Arc<dyn SecretsStore>,
    credentials: Vec<(String, SecretString)>,
    user_id: &str,
    dry_run: bool,
) -> Result<usize, ImportError> {
    let mut imported = 0;

    for (name, value) in credentials {
        if !dry_run {
            let params = CreateSecretParams::new(name, value.expose_secret().to_string());

            match secrets.create(user_id, params).await {
                Ok(_) => imported += 1,
                Err(e) => {
                    // Log error without revealing secret
                    tracing::warn!("Failed to import credential: {}", e);
                }
            }
        } else {
            imported += 1;
        }
    }

    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_string_not_logged() {
        let secret = SecretString::new("super-secret-key".to_string().into_boxed_str());
        let debug_output = format!("{:?}", secret);

        // Verify that the actual secret is not in the debug output
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_create_secret_params_normalized() {
        let params = CreateSecretParams::new("MY_API_KEY", "value123");
        // Secret names should be normalized to lowercase
        assert_eq!(params.name, "my_api_key");
    }
}
