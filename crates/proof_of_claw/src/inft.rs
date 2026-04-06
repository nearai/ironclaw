//! ERC-7857 Intelligent NFT (iNFT) integration for agent identity.
//!
//! ERC-7857 iNFTs represent provable agent identity on 0G Chain.
//! This client handles minting, querying, and metadata management.

use crate::config::AgentConfig;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub struct INFTClient {
    client: Client,
    zero_g_storage_endpoint: String,
    rpc_url: String,
    inft_contract: String,
}

/// Agent metadata stored off-chain (on 0G Storage) and referenced by the iNFT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub name: String,
    pub ens_name: String,
    pub policy: PolicyMetadata,
    pub risc_zero_image_id: String,
    pub capabilities: Vec<String>,
    pub dm3_endpoint: String,
    pub inference_model: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMetadata {
    pub allowed_tools: Vec<String>,
    pub max_value_autonomous_wei: u64,
    pub endpoint_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintResult {
    pub token_id: u64,
    pub encrypted_uri: String,
    pub metadata_hash: String,
    pub tx_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct INFTData {
    pub token_id: u64,
    pub owner: String,
    pub agent_id: String,
    pub policy_hash: String,
    pub risc_zero_image_id: String,
    pub encrypted_uri: String,
    pub metadata_hash: String,
    pub ens_name: String,
    pub reputation_score: u64,
    pub total_proofs: u64,
    pub minted_at: u64,
    pub active: bool,
}

impl INFTClient {
    pub async fn new(config: &AgentConfig) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            zero_g_storage_endpoint: config.zero_g_indexer_rpc.clone(),
            rpc_url: config.rpc_url.clone(),
            inft_contract: config.inft_contract.clone().unwrap_or_default(),
        })
    }

    /// Build agent metadata from the agent config.
    pub fn build_metadata(config: &AgentConfig) -> AgentMetadata {
        AgentMetadata {
            name: config.agent_id.clone(),
            ens_name: config.ens_name.clone(),
            policy: PolicyMetadata {
                allowed_tools: config.policy.allowed_tools.clone(),
                max_value_autonomous_wei: config.policy.max_value_autonomous_wei,
                endpoint_allowlist: config.policy.endpoint_allowlist.clone(),
            },
            risc_zero_image_id: config.risc_zero_image_id.clone().unwrap_or_default(),
            capabilities: config.policy.allowed_tools.clone(),
            dm3_endpoint: config.dm3_delivery_service_url.clone(),
            inference_model: "0g-compute".to_string(),
            version: "1.0.0".to_string(),
        }
    }

    /// Upload agent metadata to 0G Storage.
    ///
    /// Returns `(storage_uri, metadata_hash)`.
    pub async fn upload_metadata(&self, metadata: &AgentMetadata) -> Result<(String, String)> {
        let plaintext = serde_json::to_string(metadata)?;
        let mut h = Sha256::new();
        h.update(plaintext.as_bytes());
        let metadata_hash = format!("0x{}", hex::encode(h.finalize()));

        let resp = self
            .client
            .post(format!("{}/upload", self.zero_g_storage_endpoint))
            .json(&serde_json::json!({
                "data": plaintext,
                "tags": {
                    "type": "inft-metadata",
                    "agent": metadata.name,
                    "ens": metadata.ens_name,
                }
            }))
            .send()
            .await;

        let encrypted_uri = match resp {
            Ok(r) if r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                if body.starts_with("0x") {
                    format!("0g://{}", body)
                } else {
                    format!("0g://{}", &metadata_hash[2..])
                }
            }
            _ => format!("0g://{}", &metadata_hash[2..]),
        };

        Ok((encrypted_uri, metadata_hash))
    }

    /// Build calldata for `ProofOfClawINFT.mint(...)`.
    pub fn build_mint_calldata(
        agent_id: &str,
        policy_hash: &str,
        risc_zero_image_id: &str,
        encrypted_uri: &str,
        metadata_hash: &str,
        ens_name: &str,
    ) -> Vec<u8> {
        use ethers::abi::{encode, Token};
        use ethers::utils::keccak256;

        let agent_hash = {
            let mut h = Sha256::new();
            h.update(agent_id.as_bytes());
            h.finalize()
        };

        let selector = &keccak256(b"mint(bytes32,bytes32,bytes32,string,bytes32,string)")[..4];
        let encoded = encode(&[
            Token::FixedBytes(agent_hash.to_vec()),
            Token::FixedBytes(hex_to_bytes32(policy_hash).to_vec()),
            Token::FixedBytes(hex_to_bytes32(risc_zero_image_id).to_vec()),
            Token::String(encrypted_uri.to_string()),
            Token::FixedBytes(hex_to_bytes32(metadata_hash).to_vec()),
            Token::String(ens_name.to_string()),
        ]);

        [selector.to_vec(), encoded].concat()
    }

    /// Query full iNFT data for an agent from the contract.
    pub async fn get_agent_inft(&self, agent_id: &str) -> Result<Option<INFTData>> {
        if self.inft_contract.is_empty() {
            anyhow::bail!("INFT_CONTRACT not configured");
        }

        let agent_hash = {
            let mut h = Sha256::new();
            h.update(agent_id.as_bytes());
            format!("0x{}", hex::encode(h.finalize()))
        };
        let agent_bytes = hex_to_bytes32(&agent_hash);

        // Check token exists
        let selector = &ethers::utils::keccak256(b"getTokenByAgent(bytes32)")[..4];
        let calldata = [selector.to_vec(), ethers::abi::encode(&[
            ethers::abi::Token::FixedBytes(agent_bytes.to_vec())
        ])].concat();

        let result_bytes = self.eth_call(&calldata).await?;
        let token_id = parse_u256(&result_bytes[..32]).unwrap_or(0);

        if token_id == 0 {
            return Ok(None);
        }

        // Fetch full agent data
        let data_selector = &ethers::utils::keccak256(b"getAgentData(bytes32)")[..4];
        let data_calldata = [data_selector.to_vec(), ethers::abi::encode(&[
            ethers::abi::Token::FixedBytes(agent_bytes.to_vec())
        ])].concat();

        let data_bytes = self.eth_call(&data_calldata).await?;

        if data_bytes.len() < 352 {
            return Ok(Some(INFTData {
                token_id,
                owner: String::new(),
                agent_id: agent_hash,
                policy_hash: String::new(),
                risc_zero_image_id: String::new(),
                encrypted_uri: String::new(),
                metadata_hash: String::new(),
                ens_name: String::new(),
                reputation_score: 0,
                total_proofs: 0,
                minted_at: 0,
                active: true,
            }));
        }

        let tokens = ethers::abi::decode(
            &[
                ethers::abi::ParamType::Uint(256),
                ethers::abi::ParamType::Address,
                ethers::abi::ParamType::FixedBytes(32),
                ethers::abi::ParamType::FixedBytes(32),
                ethers::abi::ParamType::String,
                ethers::abi::ParamType::FixedBytes(32),
                ethers::abi::ParamType::String,
                ethers::abi::ParamType::Uint(256),
                ethers::abi::ParamType::Uint(256),
                ethers::abi::ParamType::Uint(256),
                ethers::abi::ParamType::Bool,
            ],
            &data_bytes,
        )
        .context("Failed to decode iNFT agent data")?;

        Ok(Some(INFTData {
            token_id,
            owner: tokens[1].clone().into_address()
                .map(|a| format!("0x{}", hex::encode(a.as_bytes())))
                .unwrap_or_default(),
            agent_id: agent_hash,
            policy_hash: tokens[2].clone().into_fixed_bytes()
                .map(|b| format!("0x{}", hex::encode(b)))
                .unwrap_or_default(),
            risc_zero_image_id: tokens[3].clone().into_fixed_bytes()
                .map(|b| format!("0x{}", hex::encode(b)))
                .unwrap_or_default(),
            encrypted_uri: tokens[4].clone().into_string().unwrap_or_default(),
            metadata_hash: tokens[5].clone().into_fixed_bytes()
                .map(|b| format!("0x{}", hex::encode(b)))
                .unwrap_or_default(),
            ens_name: tokens[6].clone().into_string().unwrap_or_default(),
            reputation_score: tokens[7].clone().into_uint().unwrap_or_default().as_u64(),
            total_proofs: tokens[8].clone().into_uint().unwrap_or_default().as_u64(),
            minted_at: tokens[9].clone().into_uint().unwrap_or_default().as_u64(),
            active: tokens[10].clone().into_bool().unwrap_or(true),
        }))
    }

    async fn eth_call(&self, calldata: &[u8]) -> Result<Vec<u8>> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{
                "to": self.inft_contract,
                "data": format!("0x{}", hex::encode(calldata))
            }, "latest"],
            "id": 1
        });
        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("eth_call failed")?;

        let json: serde_json::Value = resp.json().await?;
        if let Some(err) = json.get("error") {
            anyhow::bail!("RPC error: {err}");
        }
        let hex_str = json["result"].as_str().unwrap_or("0x");
        hex::decode(hex_str.trim_start_matches("0x"))
            .context("hex decode failed")
    }
}

fn hex_to_bytes32(hex_str: &str) -> [u8; 32] {
    let clean = hex_str.trim_start_matches("0x");
    let decoded = hex::decode(clean).unwrap_or_default();
    let mut bytes = [0u8; 32];
    bytes[32 - decoded.len().min(32)..].copy_from_slice(&decoded[..decoded.len().min(32)]);
    bytes
}

/// Extract a `u64` from the last 8 bytes of a 32-byte ABI-encoded U256 word.
fn parse_u256(data: &[u8]) -> Option<u64> {
    if data.len() < 32 {
        return None;
    }
    let last_8 = &data[data.len() - 8..];
    Some(u64::from_be_bytes([
        last_8[0], last_8[1], last_8[2], last_8[3],
        last_8[4], last_8[5], last_8[6], last_8[7],
    ]))
}
