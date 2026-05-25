//! OS keychain integration for secrets master key storage.
//!
//! Provides platform-specific keychain support:
//! - macOS: security-framework (Keychain Services)
//! - Linux: secret-service (GNOME Keyring, KWallet)
//!
//! # Example
//!
//! ```ignore
//! use ironclaw_secrets::keychain::{store_master_key, get_master_key, delete_master_key};
//!
//! // Generate and store a new master key
//! let key = generate_master_key();
//! store_master_key(&key)?;
//!
//! // Later, retrieve it
//! let key = get_master_key()?;
//! ```

use crate::SecretError;

/// Environment variable checked before the OS keychain.
pub const SECRETS_MASTER_KEY_ENV: &str = "SECRETS_MASTER_KEY";
const MASTER_KEY_BYTES: usize = 32;
const MASTER_KEY_HEX_CHARS: usize = MASTER_KEY_BYTES * 2;

/// Service name for keychain entries.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SERVICE_NAME: &str = "ironclaw";

/// Account name for the master key.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MASTER_KEY_ACCOUNT: &str = "master_key";

/// Generate a random 32-byte master key.
pub fn generate_master_key() -> Vec<u8> {
    use rand::RngCore;
    use rand::rngs::OsRng;
    let mut key = vec![0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

/// Generate a master key as a hex string.
pub fn generate_master_key_hex() -> String {
    let bytes = generate_master_key();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Resolve an existing master key from the environment or OS keychain.
///
/// This mirrors the v1 lookup order without auto-generating new material:
/// an explicitly set `SECRETS_MASTER_KEY` wins, otherwise the OS keychain is
/// consulted. Empty environment values are ignored. Callers that require
/// production durability should fail closed when this returns `None`.
pub async fn resolve_master_key() -> Result<Option<String>, SecretError> {
    let env_key = std::env::var(SECRETS_MASTER_KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    if env_key.is_some() {
        return resolve_master_key_from_sources(env_key, Ok(None));
    }

    resolve_master_key_from_sources(None, resolve_keychain_master_key().await)
}

/// Resolve an existing master key and wrap it for secret-store consumers.
pub async fn resolve_master_key_material() -> Result<Option<crate::SecretMaterial>, SecretError> {
    Ok(resolve_master_key().await?.map(crate::SecretMaterial::from))
}

fn resolve_master_key_from_sources(
    env_key: Option<String>,
    keychain_key: Result<Option<Vec<u8>>, SecretError>,
) -> Result<Option<String>, SecretError> {
    let env_key = env_key.filter(|value| !value.trim().is_empty());
    if let Some(env_key) = env_key {
        return validate_master_key_hex(&env_key).map(Some);
    }

    let keychain_key = match keychain_key {
        Ok(keychain_key) => keychain_key,
        Err(SecretError::NotFound(_)) => None,
        Err(error) => return Err(error),
    };
    if let Some(keychain_key) = keychain_key {
        validate_master_key_bytes(&keychain_key)?;
        return Ok(Some(bytes_to_hex(&keychain_key)));
    }

    Ok(None)
}

async fn resolve_keychain_master_key() -> Result<Option<Vec<u8>>, SecretError> {
    let keychain_key = match get_master_key().await {
        Ok(keychain_key) => keychain_key,
        Err(SecretError::NotFound(_)) => return Ok(None),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to resolve secrets master key from OS keychain"
            );
            return Err(SecretError::KeychainError(
                "failed to resolve secret master key from keychain".to_string(),
            ));
        }
    };
    if let Err(error) = validate_master_key_bytes(&keychain_key) {
        tracing::warn!(
            actual_len = keychain_key.len(),
            expected_len = MASTER_KEY_BYTES,
            "resolved secrets master key from OS keychain has invalid length"
        );
        return Err(error);
    }
    Ok(Some(keychain_key))
}

fn validate_master_key_hex(hex_key: &str) -> Result<String, SecretError> {
    if hex_key.len() != MASTER_KEY_HEX_CHARS {
        return Err(SecretError::InvalidMasterKey);
    }
    let bytes = hex_to_bytes(hex_key).map_err(|_| SecretError::InvalidMasterKey)?;
    validate_master_key_bytes(&bytes)?;
    Ok(bytes_to_hex(&bytes))
}

fn validate_master_key_bytes(bytes: &[u8]) -> Result<(), SecretError> {
    if bytes.len() == MASTER_KEY_BYTES {
        Ok(())
    } else {
        Err(SecretError::InvalidMasterKey)
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ============================================================================
// macOS implementation using security-framework
// ============================================================================

#[cfg(target_os = "macos")]
mod platform {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    use super::*;

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    /// Store the master key in the macOS Keychain.
    pub async fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        // Convert to hex for storage (keychain prefers strings)
        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();

        set_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT, key_hex.as_bytes())
            .map_err(|e| SecretError::KeychainError(format!("Failed to store in keychain: {}", e)))
    }

    /// Retrieve the master key from the macOS Keychain.
    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        let password = get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|error| {
            if error.code() == ERR_SEC_ITEM_NOT_FOUND {
                SecretError::NotFound("master key".to_string())
            } else {
                SecretError::KeychainError(format!("Failed to get from keychain: {error}"))
            }
        })?;

        // Parse hex string back to bytes
        let hex_str = String::from_utf8(password)
            .map_err(|_| SecretError::KeychainError("Invalid UTF-8 in keychain".to_string()))?;

        hex_to_bytes(&hex_str)
    }

    /// Delete the master key from the macOS Keychain.
    pub async fn delete_master_key() -> Result<(), SecretError> {
        delete_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|e| {
            SecretError::KeychainError(format!("Failed to delete from keychain: {}", e))
        })
    }

    /// Check if a master key exists in the keychain.
    pub async fn has_master_key() -> bool {
        get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).is_ok()
    }
}

// ============================================================================
// Linux implementation using secret-service
// ============================================================================

#[cfg(target_os = "linux")]
mod platform {
    use secret_service::{EncryptionType, SecretService};

    use super::*;

    /// Store the master key in the Linux secret service (GNOME Keyring, KWallet).
    pub async fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to get collection: {}", e)))?;

        // Unlock if needed
        if collection.is_locked().await.unwrap_or(true) {
            collection.unlock().await.map_err(|e| {
                SecretError::KeychainError(format!("Failed to unlock collection: {}", e))
            })?;
        }

        // Convert to hex for storage
        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();

        collection
            .create_item(
                &format!("{} master key", SERVICE_NAME),
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
                key_hex.as_bytes(),
                true, // Replace if exists
                "text/plain",
            )
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to create secret: {}", e)))?;

        Ok(())
    }

    /// Retrieve the master key from the Linux secret service.
    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let items = ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to search: {}", e)))?;

        let item = items
            .unlocked
            .first()
            .or(items.locked.first())
            .ok_or_else(|| SecretError::NotFound("master key".to_string()))?;

        // Unlock if needed
        if item.is_locked().await.unwrap_or(true) {
            item.unlock()
                .await
                .map_err(|e| SecretError::KeychainError(format!("Failed to unlock: {}", e)))?;
        }

        let secret = item
            .get_secret()
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to get secret: {}", e)))?;

        let hex_str = String::from_utf8(secret)
            .map_err(|_| SecretError::KeychainError("Invalid UTF-8 in secret".to_string()))?;

        hex_to_bytes(&hex_str)
    }

    /// Delete the master key from the Linux secret service.
    pub async fn delete_master_key() -> Result<(), SecretError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let items = ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to search: {}", e)))?;

        for item in items.unlocked.iter().chain(items.locked.iter()) {
            item.delete()
                .await
                .map_err(|e| SecretError::KeychainError(format!("Failed to delete: {}", e)))?;
        }

        Ok(())
    }

    /// Check if a master key exists in the secret service.
    pub async fn has_master_key() -> bool {
        let ss = match SecretService::connect(EncryptionType::Dh).await {
            Ok(ss) => ss,
            Err(_) => return false,
        };

        let items = match ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
        {
            Ok(items) => items,
            Err(_) => return false,
        };

        !items.unlocked.is_empty() || !items.locked.is_empty()
    }
}

// ============================================================================
// Fallback for unsupported platforms
// ============================================================================

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub async fn store_master_key(_key: &[u8]) -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
    }

    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        Err(SecretError::NotFound("master key".to_string()))
    }

    pub async fn delete_master_key() -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform".to_string(),
        ))
    }

    pub async fn has_master_key() -> bool {
        false
    }
}

// Re-export platform-specific functions
pub use platform::{delete_master_key, get_master_key, has_master_key, store_master_key};

/// Parse a hex string to bytes.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, SecretError> {
    if !hex.len().is_multiple_of(2) {
        return Err(SecretError::KeychainError(
            "Invalid hex string length".to_string(),
        ));
    }

    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| SecretError::KeychainError("Invalid hex character".to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_master_key() {
        let key = generate_master_key();
        assert_eq!(key.len(), 32);

        // Should be different each time
        let key2 = generate_master_key();
        assert_ne!(key, key2);
    }

    #[test]
    fn test_generate_master_key_hex() {
        let hex = generate_master_key_hex();
        assert_eq!(hex.len(), 64); // 32 bytes * 2 hex chars
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hex_to_bytes() {
        let result = hex_to_bytes("deadbeef").unwrap();
        assert_eq!(result, vec![0xde, 0xad, 0xbe, 0xef]);

        let result = hex_to_bytes("00ff").unwrap();
        assert_eq!(result, vec![0x00, 0xff]);
    }

    #[test]
    fn test_hex_to_bytes_invalid() {
        assert!(hex_to_bytes("abc").is_err()); // Odd length
        assert!(hex_to_bytes("gg").is_err()); // Invalid chars
    }

    #[test]
    fn resolve_master_key_prefers_env_key() {
        let env = Some("aa".repeat(32));
        let keychain = Ok(Some(vec![0xbb; 32]));

        let resolved = resolve_master_key_from_sources(env, keychain)
            .unwrap()
            .unwrap();

        assert_eq!(resolved, "aa".repeat(32));
    }

    #[test]
    fn resolve_master_key_uses_keychain_when_env_is_absent() {
        let resolved = resolve_master_key_from_sources(None, Ok(Some(vec![0xbb; 32])))
            .unwrap()
            .unwrap();

        assert_eq!(resolved, "bb".repeat(32));
    }

    #[test]
    fn resolve_master_key_ignores_blank_env_key() {
        let resolved =
            resolve_master_key_from_sources(Some(" \t ".to_string()), Ok(Some(vec![0xab; 32])))
                .expect("keychain resolver should succeed")
                .expect("keychain key should be used");

        assert_eq!(resolved, "ab".repeat(32));
    }

    #[test]
    fn resolve_master_key_returns_none_without_env_or_keychain() {
        assert_eq!(
            resolve_master_key_from_sources(None, Ok(None)).unwrap(),
            None
        );
    }

    #[test]
    fn resolve_master_key_returns_none_when_keychain_reports_not_found() {
        assert_eq!(
            resolve_master_key_from_sources(
                None,
                Err(SecretError::NotFound("master key".to_string()))
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn resolve_master_key_rejects_non_hex_env_key() {
        assert!(matches!(
            resolve_master_key_from_sources(
                Some("correct horse battery staple pad!!".to_string()),
                Ok(None)
            ),
            Err(SecretError::InvalidMasterKey)
        ));
    }

    #[test]
    fn resolve_master_key_rejects_short_keychain_key() {
        assert!(matches!(
            resolve_master_key_from_sources(None, Ok(Some(vec![0xab; 31]))),
            Err(SecretError::InvalidMasterKey)
        ));
    }
}
