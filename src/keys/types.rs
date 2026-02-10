//! Core types for NEAR key management.
//!
//! Types for account IDs, public keys, access key permissions, network selection,
//! and key metadata. All types validate on construction to prevent invalid states.
//!
//! SECURITY: Debug impls on key-related types MUST redact secret material.

use std::fmt;
use std::str::FromStr;

use borsh::BorshSerialize;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::keys::KeyError;

/// NEAR account ID with validation.
///
/// Rules: 2-64 chars, lowercase alphanumeric + `.`, `-`, `_`.
/// No leading/trailing separators, no consecutive separators.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NearAccountId(String);

impl NearAccountId {
    pub fn new(id: &str) -> Result<Self, KeyError> {
        Self::validate(id)?;
        Ok(Self(id.to_string()))
    }

    fn validate(id: &str) -> Result<(), KeyError> {
        if id.len() < 2 || id.len() > 64 {
            return Err(KeyError::InvalidAccountId {
                reason: format!("account ID must be 2-64 characters, got {}", id.len()),
            });
        }

        let bytes = id.as_bytes();

        // No leading/trailing separators
        if matches!(bytes[0], b'.' | b'-' | b'_') {
            return Err(KeyError::InvalidAccountId {
                reason: "account ID must not start with a separator".to_string(),
            });
        }
        if matches!(bytes[bytes.len() - 1], b'.' | b'-' | b'_') {
            return Err(KeyError::InvalidAccountId {
                reason: "account ID must not end with a separator".to_string(),
            });
        }

        for ch in id.chars() {
            if !matches!(ch, 'a'..='z' | '0'..='9' | '.' | '-' | '_') {
                return Err(KeyError::InvalidAccountId {
                    reason: format!("invalid character '{}' in account ID", ch),
                });
            }
        }

        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NearAccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for NearAccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NearAccountId({})", self.0)
    }
}

impl FromStr for NearAccountId {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl BorshSerialize for NearAccountId {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // NEAR protocol serializes account IDs as length-prefixed UTF-8 strings.
        BorshSerialize::serialize(&self.0, writer)
    }
}

/// Key type discriminant for borsh serialization (matches NEAR protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    Ed25519 = 0,
}

impl BorshSerialize for KeyType {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&(*self as u8), writer)
    }
}

/// NEAR public key with format parsing.
///
/// Parses the NEAR format: `ed25519:<base58-encoded-32-bytes>`
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearPublicKey {
    pub key_type: KeyType,
    pub data: [u8; 32],
}

impl NearPublicKey {
    /// Parse from NEAR format string: `ed25519:<base58>`
    pub fn from_near_format(s: &str) -> Result<Self, KeyError> {
        let s = s.trim();
        let data_str = s
            .strip_prefix("ed25519:")
            .ok_or_else(|| KeyError::InvalidKeyFormat {
                reason: "public key must start with 'ed25519:'".to_string(),
            })?;

        let bytes = bs58::decode(data_str)
            .into_vec()
            .map_err(|e| KeyError::InvalidKeyFormat {
                reason: format!("invalid base58 in public key: {}", e),
            })?;

        if bytes.len() != 32 {
            return Err(KeyError::InvalidKeyFormat {
                reason: format!("ed25519 public key must be 32 bytes, got {}", bytes.len()),
            });
        }

        let mut data = [0u8; 32];
        data.copy_from_slice(&bytes);

        Ok(Self {
            key_type: KeyType::Ed25519,
            data,
        })
    }

    /// Format as NEAR string: `ed25519:<base58>`
    pub fn to_near_format(&self) -> String {
        format!("ed25519:{}", bs58::encode(&self.data).into_string())
    }

    /// Raw 32-byte key data.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.data
    }
}

impl fmt::Display for NearPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_near_format())
    }
}

impl fmt::Debug for NearPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let encoded = bs58::encode(&self.data).into_string();
        let preview = if encoded.len() > 8 {
            &encoded[..8]
        } else {
            &encoded
        };
        write!(f, "NearPublicKey(ed25519:{}...)", preview)
    }
}

impl BorshSerialize for NearPublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // NEAR protocol: key_type byte + 32 bytes of key data
        BorshSerialize::serialize(&self.key_type, writer)?;
        writer.write_all(&self.data)?;
        Ok(())
    }
}

/// Access key permission level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessKeyPermission {
    FullAccess,
    FunctionCall {
        /// Max NEAR that can be spent (None = unlimited within key's scope).
        allowance: Option<u128>,
        /// Contract this key is scoped to.
        receiver_id: String,
        /// Allowed method names (empty = all methods on the contract).
        method_names: Vec<String>,
    },
}

impl fmt::Display for AccessKeyPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessKeyPermission::FullAccess => write!(f, "FullAccess"),
            AccessKeyPermission::FunctionCall {
                receiver_id,
                method_names,
                allowance,
            } => {
                write!(f, "FunctionCall({}", receiver_id)?;
                if !method_names.is_empty() {
                    write!(f, "::{}", method_names.join(","))?;
                }
                if let Some(a) = allowance {
                    write!(f, ", allowance={})", format_yocto(*a))?;
                } else {
                    write!(f, ")")?;
                }
                Ok(())
            }
        }
    }
}

/// NEAR network configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NearNetwork {
    Mainnet,
    Testnet,
    Custom(String),
}

impl NearNetwork {
    pub fn rpc_url(&self) -> &str {
        match self {
            NearNetwork::Mainnet => "https://rpc.mainnet.near.org",
            NearNetwork::Testnet => "https://rpc.testnet.near.org",
            NearNetwork::Custom(url) => url.as_str(),
        }
    }
}

impl fmt::Display for NearNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NearNetwork::Mainnet => write!(f, "mainnet"),
            NearNetwork::Testnet => write!(f, "testnet"),
            NearNetwork::Custom(url) => write!(f, "custom({})", url),
        }
    }
}

impl FromStr for NearNetwork {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mainnet" => Ok(NearNetwork::Mainnet),
            "testnet" => Ok(NearNetwork::Testnet),
            url if url.starts_with("http") => Ok(NearNetwork::Custom(url.to_string())),
            other => Err(KeyError::InvalidKeyFormat {
                reason: format!(
                    "unknown network '{}', expected mainnet, testnet, or an RPC URL",
                    other
                ),
            }),
        }
    }
}

/// Metadata for a stored key (public info only, no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMetadata {
    pub label: String,
    pub account_id: String,
    pub public_key: String,
    pub permission: AccessKeyPermission,
    pub network: NearNetwork,
    pub created_at: DateTime<Utc>,
    /// Cached nonce for transaction building (avoids extra RPC round-trip).
    pub cached_nonce: Option<u64>,
}

/// Top-level structure for ~/.ironclaw/keys.json
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct KeyStore {
    pub keys: std::collections::HashMap<String, KeyMetadata>,
    pub last_backup_at: Option<DateTime<Utc>>,
}

/// Format yoctoNEAR as human-readable NEAR amount.
pub fn format_yocto(yocto: u128) -> String {
    const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;
    const ONE_MILLI_NEAR: u128 = ONE_NEAR / 1000;
    if yocto == 0 {
        return "0 NEAR".to_string();
    }
    if yocto >= ONE_MILLI_NEAR {
        let whole = yocto / ONE_NEAR;
        let frac = (yocto % ONE_NEAR) / ONE_MILLI_NEAR; // 3 decimal places
        if frac == 0 {
            format!("{} NEAR", whole)
        } else {
            format!("{}.{:03} NEAR", whole, frac)
        }
    } else {
        format!("{} yoctoNEAR", yocto)
    }
}

/// Parse a NEAR amount string into yoctoNEAR.
///
/// Accepts: "1", "0.5", "1.5 NEAR", "100000 yoctoNEAR"
pub fn parse_near_amount(s: &str) -> Result<u128, KeyError> {
    const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

    let s = s.trim();

    // Check for explicit yoctoNEAR suffix
    if let Some(yocto_str) = s
        .strip_suffix("yoctoNEAR")
        .or_else(|| s.strip_suffix("yocto"))
    {
        return yocto_str
            .trim()
            .parse::<u128>()
            .map_err(|e| KeyError::InvalidKeyFormat {
                reason: format!("invalid yoctoNEAR amount: {}", e),
            });
    }

    // Strip optional "NEAR" suffix
    let amount_str = s
        .strip_suffix("NEAR")
        .or_else(|| s.strip_suffix("near"))
        .unwrap_or(s)
        .trim();

    // Parse as decimal NEAR
    if let Some((whole_str, frac_str)) = amount_str.split_once('.') {
        let whole: u128 = whole_str.parse().map_err(|e| KeyError::InvalidKeyFormat {
            reason: format!("invalid NEAR amount: {}", e),
        })?;

        // Pad or truncate fractional part to 24 digits
        let mut frac_padded = frac_str.to_string();
        if frac_padded.len() > 24 {
            frac_padded.truncate(24);
        }
        while frac_padded.len() < 24 {
            frac_padded.push('0');
        }

        let frac: u128 = frac_padded
            .parse()
            .map_err(|e| KeyError::InvalidKeyFormat {
                reason: format!("invalid NEAR fractional amount: {}", e),
            })?;

        Ok(whole * ONE_NEAR + frac)
    } else {
        let whole: u128 = amount_str.parse().map_err(|e| KeyError::InvalidKeyFormat {
            reason: format!("invalid NEAR amount: {}", e),
        })?;
        Ok(whole * ONE_NEAR)
    }
}

#[cfg(test)]
mod tests {
    use crate::keys::types::{
        AccessKeyPermission, KeyType, NearAccountId, NearNetwork, NearPublicKey, format_yocto,
        parse_near_amount,
    };

    // -- NearAccountId tests --

    #[test]
    fn test_valid_account_ids() {
        assert!(NearAccountId::new("alice.near").is_ok());
        assert!(NearAccountId::new("bob.testnet").is_ok());
        assert!(NearAccountId::new("system").is_ok());
        assert!(NearAccountId::new("ab").is_ok()); // minimum 2 chars
        assert!(NearAccountId::new("a0").is_ok());
        assert!(NearAccountId::new("alice-bob.near").is_ok());
        assert!(NearAccountId::new("alice_bob.near").is_ok());
        // 64 chars max
        let long_id = "a".repeat(64);
        assert!(NearAccountId::new(&long_id).is_ok());
    }

    #[test]
    fn test_invalid_account_ids() {
        // Too short
        assert!(NearAccountId::new("a").is_err());
        // Too long
        assert!(NearAccountId::new(&"a".repeat(65)).is_err());
        // Uppercase
        assert!(NearAccountId::new("Alice.near").is_err());
        // Leading separator
        assert!(NearAccountId::new(".alice").is_err());
        assert!(NearAccountId::new("-alice").is_err());
        // Trailing separator
        assert!(NearAccountId::new("alice.").is_err());
        // Invalid chars
        assert!(NearAccountId::new("alice@near").is_err());
        assert!(NearAccountId::new("alice near").is_err());
    }

    #[test]
    fn test_account_id_display() {
        let id = NearAccountId::new("alice.near").unwrap();
        assert_eq!(id.to_string(), "alice.near");
        assert_eq!(id.as_str(), "alice.near");
    }

    #[test]
    fn test_account_id_from_str() {
        let id: NearAccountId = "bob.testnet".parse().unwrap();
        assert_eq!(id.as_str(), "bob.testnet");
    }

    // -- NearPublicKey tests --

    #[test]
    fn test_public_key_roundtrip() {
        let key_str = "ed25519:6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp";
        let key = NearPublicKey::from_near_format(key_str).unwrap();
        assert_eq!(key.key_type, KeyType::Ed25519);
        assert_eq!(key.to_near_format(), key_str);
    }

    #[test]
    fn test_public_key_invalid_prefix() {
        assert!(NearPublicKey::from_near_format("secp256k1:abc").is_err());
        assert!(NearPublicKey::from_near_format("abc123").is_err());
    }

    #[test]
    fn test_public_key_invalid_base58() {
        assert!(NearPublicKey::from_near_format("ed25519:not-valid-base58!!!").is_err());
    }

    #[test]
    fn test_public_key_wrong_length() {
        // Too short (only 16 bytes encoded)
        assert!(NearPublicKey::from_near_format("ed25519:3gZNbFLLDt").is_err());
    }

    #[test]
    fn test_public_key_debug_redacts() {
        let key_str = "ed25519:6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp";
        let key = NearPublicKey::from_near_format(key_str).unwrap();
        let debug = format!("{:?}", key);
        // Should show first 8 chars of base58, not the whole thing
        assert!(debug.contains("..."));
        assert!(!debug.contains("6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp"));
    }

    // -- AccessKeyPermission tests --

    #[test]
    fn test_permission_display() {
        assert_eq!(AccessKeyPermission::FullAccess.to_string(), "FullAccess");

        let fc = AccessKeyPermission::FunctionCall {
            allowance: None,
            receiver_id: "intents.near".to_string(),
            method_names: vec![],
        };
        assert_eq!(fc.to_string(), "FunctionCall(intents.near)");

        let fc_methods = AccessKeyPermission::FunctionCall {
            allowance: Some(1_000_000_000_000_000_000_000_000),
            receiver_id: "contract.near".to_string(),
            method_names: vec!["deposit".to_string(), "withdraw".to_string()],
        };
        assert!(fc_methods.to_string().contains("deposit,withdraw"));
        assert!(fc_methods.to_string().contains("1 NEAR"));
    }

    // -- NearNetwork tests --

    #[test]
    fn test_network_rpc_urls() {
        assert_eq!(
            NearNetwork::Mainnet.rpc_url(),
            "https://rpc.mainnet.near.org"
        );
        assert_eq!(
            NearNetwork::Testnet.rpc_url(),
            "https://rpc.testnet.near.org"
        );
        let custom = NearNetwork::Custom("https://custom.rpc.dev".to_string());
        assert_eq!(custom.rpc_url(), "https://custom.rpc.dev");
    }

    #[test]
    fn test_network_from_str() {
        assert_eq!(
            "mainnet".parse::<NearNetwork>().unwrap(),
            NearNetwork::Mainnet
        );
        assert_eq!(
            "testnet".parse::<NearNetwork>().unwrap(),
            NearNetwork::Testnet
        );
        assert_eq!(
            "https://custom.rpc".parse::<NearNetwork>().unwrap(),
            NearNetwork::Custom("https://custom.rpc".to_string())
        );
        assert!("garbage".parse::<NearNetwork>().is_err());
    }

    // -- NEAR amount formatting/parsing --

    #[test]
    fn test_format_yocto() {
        assert_eq!(format_yocto(0), "0 NEAR");
        assert_eq!(format_yocto(1_000_000_000_000_000_000_000_000), "1 NEAR");
        assert_eq!(
            format_yocto(5_500_000_000_000_000_000_000_000),
            "5.500 NEAR"
        );
        assert_eq!(format_yocto(1), "1 yoctoNEAR");
        assert_eq!(format_yocto(500_000_000_000_000_000_000_000), "0.500 NEAR");
    }

    #[test]
    fn test_parse_near_amount() {
        assert_eq!(
            parse_near_amount("1").unwrap(),
            1_000_000_000_000_000_000_000_000
        );
        assert_eq!(
            parse_near_amount("0.5").unwrap(),
            500_000_000_000_000_000_000_000
        );
        assert_eq!(
            parse_near_amount("1.5 NEAR").unwrap(),
            1_500_000_000_000_000_000_000_000
        );
        assert_eq!(parse_near_amount("100 yoctoNEAR").unwrap(), 100);
        assert_eq!(parse_near_amount("0").unwrap(), 0);
    }

    // -- Borsh serialization tests --

    #[test]
    fn test_account_id_borsh() {
        let id = NearAccountId::new("alice.near").unwrap();
        let bytes = borsh::to_vec(&id).unwrap();
        // Length-prefixed string: 4 bytes length + 10 bytes "alice.near"
        assert_eq!(bytes.len(), 4 + 10);
    }

    #[test]
    fn test_public_key_borsh() {
        let key_str = "ed25519:6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp";
        let key = NearPublicKey::from_near_format(key_str).unwrap();
        let bytes = borsh::to_vec(&key).unwrap();
        // 1 byte key_type + 32 bytes data
        assert_eq!(bytes.len(), 33);
        assert_eq!(bytes[0], 0); // Ed25519 = 0
    }
}
