//! Minimal NEAR transaction types with borsh serialization.
//!
//! Hand-rolled types that produce byte-identical borsh output to near-primitives,
//! without pulling in the massive nearcore dependency tree.
//!
//! # Serialization Format
//!
//! NEAR transactions are borsh-serialized, then SHA-256 hashed for signing.
//! The signed transaction includes the original transaction + ed25519 signature.

use borsh::BorshSerialize;

use crate::keys::signer::sha256_hash;
use crate::keys::types::{KeyType, NearAccountId, NearPublicKey};

/// A NEAR transaction ready for signing.
#[derive(Debug, Clone, BorshSerialize)]
pub struct Transaction {
    pub signer_id: NearAccountId,
    pub public_key: NearPublicKey,
    pub nonce: u64,
    pub receiver_id: NearAccountId,
    pub block_hash: BlockHash,
    pub actions: Vec<Action>,
}

impl Transaction {
    /// Borsh-serialize and SHA-256 hash for signing.
    pub fn hash_for_signing(&self) -> Result<[u8; 32], crate::keys::KeyError> {
        let bytes = borsh::to_vec(self).map_err(|e| {
            crate::keys::KeyError::SerializationFailed(format!(
                "failed to serialize transaction: {}",
                e
            ))
        })?;
        Ok(sha256_hash(&bytes))
    }
}

/// A signed NEAR transaction with ed25519 signature.
#[derive(Debug, Clone)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: Signature,
}

impl SignedTransaction {
    /// Encode as base64 for RPC submission.
    pub fn to_base64(&self) -> Result<String, crate::keys::KeyError> {
        let bytes = self.to_borsh()?;
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        ))
    }

    /// Borsh-serialize the signed transaction.
    pub fn to_borsh(&self) -> Result<Vec<u8>, crate::keys::KeyError> {
        let mut buf = Vec::new();
        borsh::BorshSerialize::serialize(&self.transaction, &mut buf).map_err(|e| {
            crate::keys::KeyError::SerializationFailed(format!(
                "failed to serialize signed transaction: {}",
                e
            ))
        })?;
        borsh::BorshSerialize::serialize(&self.signature, &mut buf).map_err(|e| {
            crate::keys::KeyError::SerializationFailed(format!(
                "failed to serialize signature: {}",
                e
            ))
        })?;
        Ok(buf)
    }

    /// Get the transaction hash (the hash that was signed).
    pub fn tx_hash(&self) -> Result<[u8; 32], crate::keys::KeyError> {
        self.transaction.hash_for_signing()
    }
}

/// Block hash (32 bytes), used as recent block reference for transaction validity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHash(pub [u8; 32]);

impl BorshSerialize for BlockHash {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(&self.0)
    }
}

impl BlockHash {
    pub fn from_base58(s: &str) -> Result<Self, crate::keys::KeyError> {
        let bytes =
            bs58::decode(s)
                .into_vec()
                .map_err(|e| crate::keys::KeyError::InvalidKeyFormat {
                    reason: format!("invalid base58 block hash: {}", e),
                })?;
        if bytes.len() != 32 {
            return Err(crate::keys::KeyError::InvalidKeyFormat {
                reason: format!("block hash must be 32 bytes, got {}", bytes.len()),
            });
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bytes);
        Ok(Self(hash))
    }
}

/// Ed25519 signature (NEAR uses key_type prefix for borsh serialization).
#[derive(Debug, Clone)]
pub struct Signature {
    pub key_type: KeyType,
    pub data: [u8; 64],
}

impl BorshSerialize for Signature {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.key_type, writer)?;
        writer.write_all(&self.data)?;
        Ok(())
    }
}

/// NEAR transaction action variants.
///
/// Only includes the variants we actually need for key management operations.
/// Borsh enum discriminants MUST match near-primitives exactly.
#[derive(Debug, Clone)]
pub enum Action {
    CreateAccount,                  // 0
    DeployContract(DeployContract), // 1
    FunctionCall(FunctionCall),     // 2
    Transfer(Transfer),             // 3
    Stake(Stake),                   // 4
    AddKey(AddKey),                 // 5
    DeleteKey(DeleteKey),           // 6
    DeleteAccount(DeleteAccount),   // 7
}

impl BorshSerialize for Action {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Action::CreateAccount => {
                BorshSerialize::serialize(&0u8, writer)?;
            }
            Action::DeployContract(v) => {
                BorshSerialize::serialize(&1u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
            Action::FunctionCall(v) => {
                BorshSerialize::serialize(&2u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
            Action::Transfer(v) => {
                BorshSerialize::serialize(&3u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
            Action::Stake(v) => {
                BorshSerialize::serialize(&4u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
            Action::AddKey(v) => {
                BorshSerialize::serialize(&5u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
            Action::DeleteKey(v) => {
                BorshSerialize::serialize(&6u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
            Action::DeleteAccount(v) => {
                BorshSerialize::serialize(&7u8, writer)?;
                BorshSerialize::serialize(v, writer)?;
            }
        }
        Ok(())
    }
}

/// Deploy contract action.
#[derive(Debug, Clone, BorshSerialize)]
pub struct DeployContract {
    pub code: Vec<u8>,
}

/// Function call action.
#[derive(Debug, Clone, BorshSerialize)]
pub struct FunctionCall {
    pub method_name: String,
    pub args: Vec<u8>,
    pub gas: u64,
    pub deposit: u128,
}

/// Transfer action.
#[derive(Debug, Clone, BorshSerialize)]
pub struct Transfer {
    pub deposit: u128,
}

/// Stake action.
#[derive(Debug, Clone, BorshSerialize)]
pub struct Stake {
    pub stake: u128,
    pub public_key: NearPublicKey,
}

/// Add key action.
#[derive(Debug, Clone)]
pub struct AddKey {
    pub public_key: NearPublicKey,
    pub access_key: AccessKeyBorsh,
}

impl BorshSerialize for AddKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.public_key, writer)?;
        BorshSerialize::serialize(&self.access_key, writer)?;
        Ok(())
    }
}

/// Delete key action.
#[derive(Debug, Clone)]
pub struct DeleteKey {
    pub public_key: NearPublicKey,
}

impl BorshSerialize for DeleteKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.public_key, writer)?;
        Ok(())
    }
}

/// Delete account action.
#[derive(Debug, Clone, BorshSerialize)]
pub struct DeleteAccount {
    pub beneficiary_id: NearAccountId,
}

/// Borsh-serializable access key (for AddKey actions).
#[derive(Debug, Clone)]
pub struct AccessKeyBorsh {
    pub nonce: u64,
    pub permission: AccessKeyPermissionBorsh,
}

impl BorshSerialize for AccessKeyBorsh {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.nonce, writer)?;
        BorshSerialize::serialize(&self.permission, writer)?;
        Ok(())
    }
}

/// Borsh-serializable access key permission.
#[derive(Debug, Clone)]
pub enum AccessKeyPermissionBorsh {
    FunctionCall(FunctionCallPermissionBorsh),
    FullAccess,
}

impl BorshSerialize for AccessKeyPermissionBorsh {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            AccessKeyPermissionBorsh::FunctionCall(fc) => {
                BorshSerialize::serialize(&0u8, writer)?;
                BorshSerialize::serialize(fc, writer)?;
            }
            AccessKeyPermissionBorsh::FullAccess => {
                BorshSerialize::serialize(&1u8, writer)?;
            }
        }
        Ok(())
    }
}

/// Borsh-serializable function call permission.
#[derive(Debug, Clone, BorshSerialize)]
pub struct FunctionCallPermissionBorsh {
    /// Allowance in yoctoNEAR (None = unlimited within key scope).
    pub allowance: Option<u128>,
    pub receiver_id: String,
    pub method_names: Vec<String>,
}

/// Standard gas amounts.
pub const TGAS: u64 = 1_000_000_000_000;

/// 300 TGas, the maximum per transaction.
pub const MAX_GAS: u64 = 300 * TGAS;

/// 1 yoctoNEAR, commonly used as a deposit to indicate "attached" value.
pub const ONE_YOCTO: u128 = 1;

/// 1 NEAR in yoctoNEAR.
pub const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

#[cfg(test)]
mod tests {
    use crate::keys::transaction::{
        AccessKeyBorsh, AccessKeyPermissionBorsh, Action, BlockHash, FunctionCall,
        FunctionCallPermissionBorsh, MAX_GAS, ONE_NEAR, ONE_YOCTO, Signature, TGAS, Transaction,
        Transfer,
    };
    use crate::keys::types::{KeyType, NearAccountId, NearPublicKey};

    fn test_public_key() -> NearPublicKey {
        NearPublicKey::from_near_format("ed25519:6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp")
            .unwrap()
    }

    #[test]
    fn test_transfer_action_borsh() {
        let action = Action::Transfer(Transfer { deposit: ONE_NEAR });
        let bytes = borsh::to_vec(&action).unwrap();
        // Discriminant (1 byte) + u128 (16 bytes)
        assert_eq!(bytes.len(), 1 + 16);
        assert_eq!(bytes[0], 3); // Transfer = discriminant 3
    }

    #[test]
    fn test_function_call_action_borsh() {
        let action = Action::FunctionCall(FunctionCall {
            method_name: "deposit".to_string(),
            args: b"{}".to_vec(),
            gas: 30 * TGAS,
            deposit: ONE_YOCTO,
        });
        let bytes = borsh::to_vec(&action).unwrap();
        assert_eq!(bytes[0], 2); // FunctionCall = discriminant 2
        // Verify it serializes without error
        assert!(bytes.len() > 1);
    }

    #[test]
    fn test_transaction_hash_for_signing() {
        let tx = Transaction {
            signer_id: NearAccountId::new("alice.near").unwrap(),
            public_key: test_public_key(),
            nonce: 1,
            receiver_id: NearAccountId::new("bob.near").unwrap(),
            block_hash: BlockHash([0u8; 32]),
            actions: vec![Action::Transfer(Transfer { deposit: ONE_NEAR })],
        };

        let hash = tx.hash_for_signing().unwrap();
        assert_eq!(hash.len(), 32);

        // Same transaction should produce same hash
        let hash2 = tx.hash_for_signing().unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_transaction_different_nonce_different_hash() {
        let tx1 = Transaction {
            signer_id: NearAccountId::new("alice.near").unwrap(),
            public_key: test_public_key(),
            nonce: 1,
            receiver_id: NearAccountId::new("bob.near").unwrap(),
            block_hash: BlockHash([0u8; 32]),
            actions: vec![Action::Transfer(Transfer { deposit: ONE_NEAR })],
        };

        let tx2 = Transaction {
            nonce: 2,
            ..tx1.clone()
        };

        assert_ne!(
            tx1.hash_for_signing().unwrap(),
            tx2.hash_for_signing().unwrap()
        );
    }

    #[test]
    fn test_signed_transaction_to_base64() {
        let tx = Transaction {
            signer_id: NearAccountId::new("alice.near").unwrap(),
            public_key: test_public_key(),
            nonce: 1,
            receiver_id: NearAccountId::new("bob.near").unwrap(),
            block_hash: BlockHash([0u8; 32]),
            actions: vec![Action::Transfer(Transfer { deposit: ONE_NEAR })],
        };

        let signed = crate::keys::transaction::SignedTransaction {
            transaction: tx,
            signature: Signature {
                key_type: KeyType::Ed25519,
                data: [0u8; 64],
            },
        };

        let b64 = signed.to_base64().unwrap();
        assert!(!b64.is_empty());

        // Should be valid base64
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).unwrap();
        assert!(!decoded.is_empty());
    }

    #[test]
    fn test_block_hash_from_base58() {
        let hash_str = "11111111111111111111111111111111"; // 32 zero bytes in base58
        let hash = BlockHash::from_base58(hash_str).unwrap();
        assert_eq!(hash.0, [0u8; 32]);
    }

    #[test]
    fn test_access_key_borsh_full_access() {
        let ak = AccessKeyBorsh {
            nonce: 0,
            permission: AccessKeyPermissionBorsh::FullAccess,
        };
        let bytes = borsh::to_vec(&ak).unwrap();
        // u64 (8 bytes) + discriminant (1 byte)
        assert_eq!(bytes.len(), 9);
    }

    #[test]
    fn test_access_key_borsh_function_call() {
        let ak = AccessKeyBorsh {
            nonce: 0,
            permission: AccessKeyPermissionBorsh::FunctionCall(FunctionCallPermissionBorsh {
                allowance: Some(ONE_NEAR),
                receiver_id: "contract.near".to_string(),
                method_names: vec!["deposit".to_string()],
            }),
        };
        let bytes = borsh::to_vec(&ak).unwrap();
        assert!(!bytes.is_empty());
        // First 8 bytes = nonce, then discriminant 0 for FunctionCall
        assert_eq!(bytes[8], 0);
    }

    #[test]
    fn test_gas_constants() {
        assert_eq!(TGAS, 1_000_000_000_000);
        assert_eq!(MAX_GAS, 300_000_000_000_000);
    }
}
