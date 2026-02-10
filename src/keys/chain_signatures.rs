//! Cross-chain signing via v1.signer MPC contract.
//!
//! Enables signing payloads for other chains (Ethereum, Bitcoin, etc.)
//! using NEAR's chain signatures MPC network.

use crate::keys::KeyError;
use crate::keys::policy::SignatureDomain;
use crate::keys::transaction::{Action, FunctionCall, MAX_GAS, ONE_YOCTO};

/// The chain signatures MPC contract on mainnet.
pub const CHAIN_SIGNATURES_CONTRACT_MAINNET: &str = "v1.signer";

/// The chain signatures MPC contract on testnet.
pub const CHAIN_SIGNATURES_CONTRACT_TESTNET: &str = "v1.signer-prod.testnet";

/// Build a FunctionCall action for requesting a chain signature.
pub fn build_chain_signature_action(
    payload: &[u8],
    derivation_path: &str,
    _domain: SignatureDomain,
) -> Result<Action, KeyError> {
    let args = serde_json::json!({
        "request": {
            "payload": payload.iter().map(|b| *b as u32).collect::<Vec<u32>>(),
            "path": derivation_path,
            "key_version": 0,
        },
    });

    let args_bytes = serde_json::to_vec(&args).map_err(|e| KeyError::ChainSignatureError {
        reason: format!("failed to serialize chain sig args: {}", e),
    })?;

    Ok(Action::FunctionCall(FunctionCall {
        method_name: "sign".to_string(),
        args: args_bytes,
        gas: MAX_GAS,
        deposit: ONE_YOCTO,
    }))
}

/// Parse the result of a chain signature request from the transaction outcome.
pub fn parse_chain_signature_result(
    outcome: &serde_json::Value,
) -> Result<ChainSignatureResult, KeyError> {
    // The result is in the SuccessValue field, base64-encoded
    let success_value = outcome
        .get("SuccessValue")
        .and_then(|v| v.as_str())
        .ok_or_else(|| KeyError::ChainSignatureError {
            reason: "no SuccessValue in chain signature outcome".to_string(),
        })?;

    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, success_value)
        .map_err(|e| KeyError::ChainSignatureError {
            reason: format!("failed to decode chain sig result: {}", e),
        })?;

    let result_str = String::from_utf8(decoded).map_err(|e| KeyError::ChainSignatureError {
        reason: format!("chain sig result is not UTF-8: {}", e),
    })?;

    let result_json: serde_json::Value =
        serde_json::from_str(&result_str).map_err(|e| KeyError::ChainSignatureError {
            reason: format!("failed to parse chain sig result JSON: {}", e),
        })?;

    // Extract big_r and s components
    let big_r = result_json
        .get("big_r")
        .and_then(|v| v.get("affine_point"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| KeyError::ChainSignatureError {
            reason: "missing big_r.affine_point in chain sig result".to_string(),
        })?
        .to_string();

    let s = result_json
        .get("s")
        .and_then(|v| v.get("scalar"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| KeyError::ChainSignatureError {
            reason: "missing s.scalar in chain sig result".to_string(),
        })?
        .to_string();

    let recovery_id = result_json
        .get("recovery_id")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8);

    Ok(ChainSignatureResult {
        big_r,
        s,
        recovery_id,
    })
}

/// Result from a chain signature request.
#[derive(Debug, Clone)]
pub struct ChainSignatureResult {
    /// The R component (affine point, hex-encoded).
    pub big_r: String,
    /// The s component (scalar, hex-encoded).
    pub s: String,
    /// Recovery ID for ECDSA (relevant for Ethereum).
    pub recovery_id: Option<u8>,
}

/// Get the chain signatures contract address for a network.
pub fn chain_sig_contract(network: &crate::keys::types::NearNetwork) -> &str {
    match network {
        crate::keys::types::NearNetwork::Mainnet => CHAIN_SIGNATURES_CONTRACT_MAINNET,
        crate::keys::types::NearNetwork::Testnet => CHAIN_SIGNATURES_CONTRACT_TESTNET,
        crate::keys::types::NearNetwork::Custom(_) => CHAIN_SIGNATURES_CONTRACT_TESTNET,
    }
}

#[cfg(test)]
mod tests {
    use crate::keys::chain_signatures::{
        build_chain_signature_action, chain_sig_contract, parse_chain_signature_result,
    };
    use crate::keys::policy::SignatureDomain;
    use crate::keys::transaction::{Action, MAX_GAS, ONE_YOCTO};
    use crate::keys::types::NearNetwork;

    #[test]
    fn test_build_chain_signature_action() {
        let payload = vec![0u8; 32];
        let action =
            build_chain_signature_action(&payload, "ethereum-1", SignatureDomain::Secp256k1)
                .unwrap();

        match action {
            Action::FunctionCall(fc) => {
                assert_eq!(fc.method_name, "sign");
                assert_eq!(fc.gas, MAX_GAS);
                assert_eq!(fc.deposit, ONE_YOCTO);

                // Verify args parse correctly
                let args: serde_json::Value = serde_json::from_slice(&fc.args).unwrap();
                assert!(args.get("request").is_some());
                let path = args["request"]["path"].as_str().unwrap();
                assert_eq!(path, "ethereum-1");
            }
            _ => panic!("expected FunctionCall action"),
        }
    }

    #[test]
    fn test_parse_chain_signature_result() {
        let result_json = serde_json::json!({
            "big_r": {"affine_point": "02abc123"},
            "s": {"scalar": "def456"},
            "recovery_id": 0
        });

        let result_str = serde_json::to_string(&result_json).unwrap();
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            result_str.as_bytes(),
        );

        let outcome = serde_json::json!({"SuccessValue": encoded});
        let result = parse_chain_signature_result(&outcome).unwrap();

        assert_eq!(result.big_r, "02abc123");
        assert_eq!(result.s, "def456");
        assert_eq!(result.recovery_id, Some(0));
    }

    #[test]
    fn test_chain_sig_contract_addresses() {
        assert_eq!(chain_sig_contract(&NearNetwork::Mainnet), "v1.signer");
        assert_eq!(
            chain_sig_contract(&NearNetwork::Testnet),
            "v1.signer-prod.testnet"
        );
    }
}
