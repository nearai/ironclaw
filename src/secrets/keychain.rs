//! OS keychain integration for secrets master key storage.
//!
//! Provides platform-specific keychain support:
//! - macOS: security-framework (Keychain Services)
//! - Linux: secret-service (GNOME Keyring, KWallet)
//!
//! # Example
//!
//! ```ignore
//! use ironclaw::secrets::keychain::{store_master_key, get_master_key, delete_master_key};
//!
//! // Generate and store a new master key
//! let key = generate_master_key();
//! store_master_key(&key)?;
//!
//! // Later, retrieve it
//! let key = get_master_key()?;
//! ```

use crate::secrets::SecretError;

/// Service name for keychain entries.
const SERVICE_NAME: &str = "ironclaw";

/// Account name for the master key.
const MASTER_KEY_ACCOUNT: &str = "master_key";

/// Generate a random 32-byte master key.
pub fn generate_master_key() -> Vec<u8> {
    use rand::RngCore;
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Generate a master key as a hex string.
pub fn generate_master_key_hex() -> String {
    let bytes = generate_master_key();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
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

    /// Store the master key in the macOS Keychain.
    pub fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        // Convert to hex for storage (keychain prefers strings)
        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();

        set_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT, key_hex.as_bytes())
            .map_err(|e| SecretError::KeychainError(format!("Failed to store in keychain: {}", e)))
    }

    /// Retrieve the master key from the macOS Keychain.
    pub fn get_master_key() -> Result<Vec<u8>, SecretError> {
        let password = get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|e| {
            SecretError::KeychainError(format!("Failed to get from keychain: {}", e))
        })?;

        // Parse hex string back to bytes
        let hex_str = String::from_utf8(password)
            .map_err(|_| SecretError::KeychainError("Invalid UTF-8 in keychain".to_string()))?;

        hex_to_bytes(&hex_str)
    }

    /// Delete the master key from the macOS Keychain.
    pub fn delete_master_key() -> Result<(), SecretError> {
        delete_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|e| {
            SecretError::KeychainError(format!("Failed to delete from keychain: {}", e))
        })
    }

    /// Check if a master key exists in the keychain.
    pub fn has_master_key() -> bool {
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
    pub fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| SecretError::KeychainError("No tokio runtime available".to_string()))?;

        rt.block_on(async {
            let ss = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|e| {
                    SecretError::KeychainError(format!(
                        "Failed to connect to secret service: {}",
                        e
                    ))
                })?;

            let collection = ss.get_default_collection().await.map_err(|e| {
                SecretError::KeychainError(format!("Failed to get collection: {}", e))
            })?;

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
                .map_err(|e| {
                    SecretError::KeychainError(format!("Failed to create secret: {}", e))
                })?;

            Ok(())
        })
    }

    /// Retrieve the master key from the Linux secret service.
    pub fn get_master_key() -> Result<Vec<u8>, SecretError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| SecretError::KeychainError("No tokio runtime available".to_string()))?;

        rt.block_on(async {
            let ss = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|e| {
                    SecretError::KeychainError(format!(
                        "Failed to connect to secret service: {}",
                        e
                    ))
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
                .ok_or_else(|| SecretError::KeychainError("Master key not found".to_string()))?;

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
        })
    }

    /// Delete the master key from the Linux secret service.
    pub fn delete_master_key() -> Result<(), SecretError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| SecretError::KeychainError("No tokio runtime available".to_string()))?;

        rt.block_on(async {
            let ss = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|e| {
                    SecretError::KeychainError(format!(
                        "Failed to connect to secret service: {}",
                        e
                    ))
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
        })
    }

    /// Check if a master key exists in the secret service.
    pub fn has_master_key() -> bool {
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => return false,
        };

        rt.block_on(async {
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
        })
    }
}

// ============================================================================
// Fallback for unsupported platforms
// ============================================================================

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub fn store_master_key(_key: &[u8]) -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
    }

    pub fn get_master_key() -> Result<Vec<u8>, SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
    }

    pub fn delete_master_key() -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform".to_string(),
        ))
    }

    pub fn has_master_key() -> bool {
        false
    }
}

// Re-export platform-specific functions
pub use platform::{delete_master_key, get_master_key, has_master_key, store_master_key};

/// Parse a hex string to bytes.
fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, SecretError> {
    if hex.len() % 2 != 0 {
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
}
