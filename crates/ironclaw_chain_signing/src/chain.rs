//! The chain identity newtype shared across the keystore, custodial signer, and
//! per-chain modules.

use ironclaw_attestation::DecodedTransaction;
use serde::{Deserialize, Serialize};

/// A chain / network identity string, e.g. `eip155:1`, `solana:mainnet-beta`,
/// `near:mainnet`.
///
/// This is the value bound into the secrets AAD ([`ironclaw_secrets::chain_key_aad`])
/// and compared against a transaction's [`DecodedTransaction::chain_network`].
/// It is a strong newtype so a raw `String` chain id cannot be confused with an
/// account id or any other identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ChainKeyId(String);

/// Why a `ChainKeyId` failed validation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChainKeyIdError {
    /// The identity string was empty.
    #[error("chain key id must not be empty")]
    Empty,
}

impl ChainKeyId {
    /// Shared validation routed through both `new` and `try_from` so the wire
    /// contract matches explicit construction. Fail-closed: an empty identity is
    /// rejected (it could never match a real chain network and the coarse
    /// [`family`](Self::family) check already treats unknown prefixes as a
    /// mismatch).
    fn validate(s: &str) -> Result<(), ChainKeyIdError> {
        if s.is_empty() {
            return Err(ChainKeyIdError::Empty);
        }
        Ok(())
    }

    /// Wrap a chain identity string, validating it first.
    pub fn new(value: impl Into<String>) -> Result<Self, ChainKeyIdError> {
        let s = value.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    /// Borrow the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the newtype, yielding the underlying string.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// The coarse chain family for this identity (`evm`, `solana`, `near`, or
    /// `unknown`). Used to reject wrong-chain confusion before any key access.
    pub fn family(&self) -> ChainFamily {
        if self.0.starts_with("eip155:") {
            ChainFamily::Evm
        } else if self.0.starts_with("solana:") {
            ChainFamily::Solana
        } else if self.0.starts_with("near:") {
            ChainFamily::Near
        } else {
            ChainFamily::Unknown
        }
    }
}

impl TryFrom<String> for ChainKeyId {
    type Error = ChainKeyIdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for ChainKeyId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<ChainKeyId> for String {
    fn from(id: ChainKeyId) -> Self {
        id.0
    }
}

impl std::fmt::Display for ChainKeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Coarse chain family, derived from a [`ChainKeyId`] or a
/// [`DecodedTransaction`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainFamily {
    /// EVM family (`eip155:*`).
    Evm,
    /// Solana family (`solana:*`).
    Solana,
    /// NEAR family (`near:*`).
    Near,
    /// Unrecognized — always treated as a mismatch (fail closed).
    Unknown,
}

impl ChainFamily {
    /// The chain family a decoded transaction belongs to.
    pub fn of_transaction(tx: &DecodedTransaction) -> Self {
        match tx {
            DecodedTransaction::Evm(_) => ChainFamily::Evm,
            DecodedTransaction::Solana(_) => ChainFamily::Solana,
            DecodedTransaction::Near(_) => ChainFamily::Near,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{Bytes32, NearAction, NearPublicKey, NearTransaction};

    #[test]
    fn family_maps_prefixes_and_unknown_fails_closed() {
        assert_eq!(
            ChainKeyId::new("eip155:1").unwrap().family(),
            ChainFamily::Evm
        );
        assert_eq!(
            ChainKeyId::new("solana:mainnet-beta").unwrap().family(),
            ChainFamily::Solana
        );
        assert_eq!(
            ChainKeyId::new("near:mainnet").unwrap().family(),
            ChainFamily::Near
        );
        // Anything without a recognized prefix is Unknown (fail closed).
        assert_eq!(
            ChainKeyId::new("btc:1").unwrap().family(),
            ChainFamily::Unknown
        );
        assert_eq!(
            ChainKeyId::new("eip155").unwrap().family(),
            ChainFamily::Unknown
        );
    }

    #[test]
    fn chain_key_id_rejects_empty_on_new_and_wire() {
        assert_eq!(ChainKeyId::new(""), Err(ChainKeyIdError::Empty));
        // serde wire deserialization routes through the same validation.
        let err = serde_json::from_str::<ChainKeyId>("\"\"");
        assert!(err.is_err());
        // A non-empty value round-trips.
        let id: ChainKeyId = serde_json::from_str("\"eip155:1\"").unwrap();
        assert_eq!(id.as_str(), "eip155:1");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"eip155:1\"");
    }

    #[test]
    fn of_transaction_maps_near_variant() {
        let tx = DecodedTransaction::Near(NearTransaction {
            network: "mainnet".into(),
            signer_id: "alice.near".into(),
            public_key: NearPublicKey {
                key_type: 0,
                data: vec![7u8; 32],
            },
            receiver_id: "bob.near".into(),
            nonce: 1,
            block_hash: Bytes32([3u8; 32]),
            actions: vec![NearAction::Transfer { deposit: vec![1] }],
        });
        assert_eq!(ChainFamily::of_transaction(&tx), ChainFamily::Near);
    }
}
