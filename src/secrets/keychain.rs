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
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SERVICE_NAME: &str = "ironclaw";

/// Account name for the master key.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MASTER_KEY_ACCOUNT: &str = "master_key";

/// Keystore error distinguishing "no key stored" from "store unavailable".
///
/// This distinction is load-bearing for master-key handling: a `NotFound`
/// is a legitimate signal that we should fall back to file or generate a
/// new key, but an `Unavailable` (keychain locked, D-Bus down, permission
/// denied, transient crash) must NEVER be treated as `NotFound` -- doing
/// so would silently rotate the master key and strand previously
/// encrypted secrets.
#[derive(Debug, Clone, thiserror::Error)]
pub enum KeystoreError {
    /// The keystore is reachable and reports that no entry exists.
    #[error("keystore entry not found")]
    NotFound,

    /// The keystore is unavailable, locked, permission-denied, or
    /// otherwise transiently broken. The caller MUST NOT rotate or
    /// regenerate keys in response to this error.
    #[error("keystore unavailable: {reason}")]
    Unavailable { reason: String },
}

impl From<KeystoreError> for SecretError {
    fn from(err: KeystoreError) -> Self {
        SecretError::KeychainError(err.to_string())
    }
}

/// Abstraction over the OS keystore used for master-key storage.
///
/// This trait exists so that `SecretsConfig::resolve()` can be tested
/// with a deterministic stub. Production code uses [`OsKeystore`], which
/// wraps the platform-specific implementation and maps raw errors to
/// [`KeystoreError::NotFound`] vs [`KeystoreError::Unavailable`].
#[async_trait::async_trait]
pub trait Keystore: Send + Sync {
    /// Fetch the master key bytes. `NotFound` means "no entry"; any
    /// other error must surface as `Unavailable`.
    async fn get_master_key(&self) -> Result<Vec<u8>, KeystoreError>;

    /// Store the master key bytes.
    async fn store_master_key(&self, key: &[u8]) -> Result<(), KeystoreError>;
}

/// Production keystore backed by the OS keychain / secret-service.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeystore;

#[async_trait::async_trait]
impl Keystore for OsKeystore {
    async fn get_master_key(&self) -> Result<Vec<u8>, KeystoreError> {
        platform::get_master_key_typed().await
    }

    async fn store_master_key(&self, key: &[u8]) -> Result<(), KeystoreError> {
        platform::store_master_key(key)
            .await
            .map_err(|e| KeystoreError::Unavailable {
                reason: e.to_string(),
            })
    }
}

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

// ============================================================================
// macOS implementation using security-framework
// ============================================================================

#[cfg(target_os = "macos")]
mod platform {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    use super::*;

    /// macOS `errSecItemNotFound` status code.
    /// See <https://developer.apple.com/documentation/security/errsecitemnotfound>.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    /// Typed variant of `get_master_key` that distinguishes NotFound from
    /// transient/unavailable errors.
    pub async fn get_master_key_typed() -> Result<Vec<u8>, KeystoreError> {
        match get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT) {
            Ok(password) => {
                let hex_str =
                    String::from_utf8(password).map_err(|_| KeystoreError::Unavailable {
                        reason: "invalid UTF-8 in keychain entry".to_string(),
                    })?;
                hex_to_bytes(&hex_str).map_err(|e| KeystoreError::Unavailable {
                    reason: e.to_string(),
                })
            }
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Err(KeystoreError::NotFound),
            Err(e) => Err(KeystoreError::Unavailable {
                reason: format!("keychain error (code {}): {}", e.code(), e),
            }),
        }
    }

    /// Store the master key in the macOS Keychain.
    pub async fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        // Convert to hex for storage (keychain prefers strings)
        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();

        set_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT, key_hex.as_bytes())
            .map_err(|e| SecretError::KeychainError(format!("Failed to store in keychain: {}", e)))
    }

    /// Retrieve the master key from the macOS Keychain.
    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        let password = get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|e| {
            SecretError::KeychainError(format!("Failed to get from keychain: {}", e))
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

    /// Typed variant of `get_master_key` that distinguishes NotFound (empty
    /// search result from a reachable secret-service) from transient /
    /// unavailable errors (D-Bus down, collection locked, dbus permission
    /// denied, etc.).
    pub async fn get_master_key_typed() -> Result<Vec<u8>, KeystoreError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| KeystoreError::Unavailable {
                reason: format!("failed to connect to secret service: {e}"),
            })?;

        let items = ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
            .map_err(|e| KeystoreError::Unavailable {
                reason: format!("failed to search secret service: {e}"),
            })?;

        let item = match items.unlocked.first().or(items.locked.first()) {
            Some(item) => item,
            None => return Err(KeystoreError::NotFound),
        };

        if item.is_locked().await.unwrap_or(true) {
            item.unlock()
                .await
                .map_err(|e| KeystoreError::Unavailable {
                    reason: format!("failed to unlock secret: {e}"),
                })?;
        }

        let secret = item
            .get_secret()
            .await
            .map_err(|e| KeystoreError::Unavailable {
                reason: format!("failed to read secret: {e}"),
            })?;

        let hex_str = String::from_utf8(secret).map_err(|_| KeystoreError::Unavailable {
            reason: "invalid UTF-8 in secret-service entry".to_string(),
        })?;

        hex_to_bytes(&hex_str).map_err(|e| KeystoreError::Unavailable {
            reason: e.to_string(),
        })
    }

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

    /// On unsupported platforms there is no OS keystore. Report
    /// `NotFound` so the caller falls through to the file-based
    /// fallback, which is the only persistence path available here.
    pub async fn get_master_key_typed() -> Result<Vec<u8>, KeystoreError> {
        Err(KeystoreError::NotFound)
    }

    pub async fn store_master_key(_key: &[u8]) -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
    }

    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
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
}
