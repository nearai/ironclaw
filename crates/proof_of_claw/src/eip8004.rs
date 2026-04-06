//! EIP-8004 Trustless Agents — identity, reputation, and validation
//! registry integration for Proof of Claw agents.

use crate::config::AgentConfig;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// EIP-8004 client for querying and writing to on-chain agent registries.
pub struct EIP8004Client {
    client: Client,
    rpc_url: String,
    identity_registry: String,
    reputation_registry: String,
    validation_registry: String,
    integration_contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    #[serde(rename = "type")]
    pub registration_type: String,
    pub name: String,
    pub description: String,
    pub image: String,
    pub services: Vec<ServiceEndpoint>,
    pub x402_support: bool,
    pub active: bool,
    pub registrations: Vec<RegistrationEntry>,
    pub supported_trust: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEndpoint {
    pub name: String,
    pub endpoint: String,
    pub version: Option<String>,
    pub skills: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationEntry {
    pub agent_id: u64,
    pub agent_registry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationSummary {
    pub count: u64,
    pub summary_value: i128,
    pub summary_value_decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub count: u64,
    pub average_response: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationFeedback {
    pub value: i128,
    pub value_decimals: u8,
    pub tag1: String,
    pub tag2: String,
    pub endpoint: String,
    pub feedback_uri: String,
}

impl EIP8004Client {
    pub async fn new(config: &AgentConfig) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            rpc_url: config.rpc_url.clone(),
            identity_registry: config.eip8004_identity_registry.clone().unwrap_or_default(),
            reputation_registry: config
                .eip8004_reputation_registry
                .clone()
                .unwrap_or_default(),
            validation_registry: config
                .eip8004_validation_registry
                .clone()
                .unwrap_or_default(),
            integration_contract: config
                .eip8004_integration_contract
                .clone()
                .unwrap_or_default(),
        })
    }

    /// Build the agent's EIP-8004 registration metadata.
    pub fn build_registration(
        &self,
        config: &AgentConfig,
        token_id: u64,
        chain_id: u64,
    ) -> AgentRegistration {
        AgentRegistration {
            registration_type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1"
                .to_string(),
            name: config.ens_name.clone(),
            description: "Proof of Claw agent with policy-compliant execution "
                .to_string(),
            image: String::new(),
            services: vec![
                ServiceEndpoint {
                    name: "ENS".to_string(),
                    endpoint: config.ens_name.clone(),
                    version: None,
                    skills: None,
                },
                ServiceEndpoint {
                    name: "DM3".to_string(),
                    endpoint: config.dm3_delivery_service_url.clone(),
                    version: Some("1.0".to_string()),
                    skills: None,
                },
            ],
            x402_support: false,
            active: true,
            registrations: vec![RegistrationEntry {
                agent_id: token_id,
                agent_registry: format!("eip155:{}:{}", chain_id, self.identity_registry),
            }],
            supported_trust: vec!["reputation".to_string(), "validation-zk".to_string()],
        }
    }

    /// Query an agent's reputation from the EIP-8004 Reputation Registry.
    ///
    /// Calls `getSummary(bytes32 agentId, string tag)`.
    pub async fn get_reputation(
        &self,
        agent_id: &[u8; 32],
        tag: &str,
    ) -> Result<ReputationSummary> {
        if self.reputation_registry.is_empty() {
            anyhow::bail!("EIP-8004 reputation registry not configured");
        }

        let selector = &ethers::utils::keccak256(b"getSummary(bytes32,string)")[..4];
        let encoded = ethers::abi::encode(&[
            ethers::abi::Token::FixedBytes(agent_id.to_vec()),
            ethers::abi::Token::String(tag.to_string()),
        ]);
        let calldata = [selector.to_vec(), encoded].concat();
        let result_hex = self.eth_call(&self.reputation_registry, &calldata).await?;
        let result_bytes = hex::decode(result_hex.trim_start_matches("0x")).unwrap_or_default();

        if result_bytes.len() < 96 {
            return Ok(ReputationSummary {
                count: 0,
                summary_value: 0,
                summary_value_decimals: 0,
            });
        }

        let tokens = ethers::abi::decode(
            &[
                ethers::abi::ParamType::Uint(256),
                ethers::abi::ParamType::Int(256),
                ethers::abi::ParamType::Uint(8),
            ],
            &result_bytes,
        )
        .context("Failed to decode reputation summary")?;

        let count = tokens[0]
            .clone()
            .into_uint()
            .unwrap_or_default()
            .as_u64();

        // i128 conversion
        let raw_int = tokens[1].clone().into_int().unwrap_or_default();
        let summary_value = i128::try_from(raw_int).unwrap_or(0);

        let summary_value_decimals = tokens[2]
            .clone()
            .into_uint()
            .unwrap_or_default()
            .as_u64() as u8;

        Ok(ReputationSummary {
            count,
            summary_value,
            summary_value_decimals,
        })
    }

    /// Query an agent's validation summary from the EIP-8004 Validation Registry.
    ///
    /// Calls `getSummary(bytes32 agentId)`.
    pub async fn get_validation_summary(
        &self,
        agent_id: &[u8; 32],
    ) -> Result<ValidationSummary> {
        if self.validation_registry.is_empty() {
            anyhow::bail!("EIP-8004 validation registry not configured");
        }

        let selector = &ethers::utils::keccak256(b"getSummary(bytes32)")[..4];
        let encoded = ethers::abi::encode(&[ethers::abi::Token::FixedBytes(agent_id.to_vec())]);
        let calldata = [selector.to_vec(), encoded].concat();
        let result_hex = self.eth_call(&self.validation_registry, &calldata).await?;
        let result_bytes = hex::decode(result_hex.trim_start_matches("0x")).unwrap_or_default();

        if result_bytes.len() < 64 {
            return Ok(ValidationSummary {
                count: 0,
                average_response: 0,
            });
        }

        let tokens = ethers::abi::decode(
            &[
                ethers::abi::ParamType::Uint(256),
                ethers::abi::ParamType::Uint(8),
            ],
            &result_bytes,
        )
        .context("Failed to decode validation summary")?;

        let count = tokens[0]
            .clone()
            .into_uint()
            .unwrap_or_default()
            .as_u64();
        let average_response = tokens[1]
            .clone()
            .into_uint()
            .unwrap_or_default()
            .as_u64() as u8;

        Ok(ValidationSummary {
            count,
            average_response,
        })
    }

    /// Submit reputation feedback on behalf of the agent.
    ///
    /// Calls `submitReputation(bytes32,int256,uint8,string,string,string,string)`.
    pub async fn submit_feedback(
        &self,
        agent_id: &[u8; 32],
        feedback: &ReputationFeedback,
    ) -> Result<String> {
        if self.integration_contract.is_empty() {
            anyhow::bail!("EIP-8004 integration contract not configured");
        }

        let selector =
            &ethers::utils::keccak256(b"submitReputation(bytes32,int256,uint8,string,string,string,string)")[..4];

        // Encode signed value using ethers I256
        let value_uint: ethers::types::U256 = if feedback.value >= 0 {
            ethers::types::U256::from(feedback.value as u128)
        } else {
            // Two's complement for negative
            ethers::types::U256::MAX
                - ethers::types::U256::from((-feedback.value) as u128)
                + 1
        };
        let value_token = ethers::abi::Token::Int(value_uint);

        let encoded = ethers::abi::encode(&[
            ethers::abi::Token::FixedBytes(agent_id.to_vec()),
            value_token,
            ethers::abi::Token::Uint(ethers::types::U256::from(feedback.value_decimals)),
            ethers::abi::Token::String(feedback.tag1.clone()),
            ethers::abi::Token::String(feedback.tag2.clone()),
            ethers::abi::Token::String(feedback.endpoint.clone()),
            ethers::abi::Token::String(feedback.feedback_uri.clone()),
        ]);

        let calldata = [selector.to_vec(), encoded].concat();
        self.send_transaction(&self.integration_contract, &calldata)
            .await
    }

    /// Check if an agent meets minimum trust thresholds.
    ///
    /// New agents with no history pass by default (bootstrap phase).
    pub async fn meets_trust_threshold(
        &self,
        agent_id: &[u8; 32],
        min_reputation: i128,
        min_validation_score: u8,
    ) -> Result<bool> {
        let reputation = self.get_reputation(agent_id, "policyCompliance").await?;
        let validation = self.get_validation_summary(agent_id).await?;

        if reputation.count == 0 && validation.count == 0 {
            return Ok(true);
        }

        let rep_ok = reputation.count == 0 || reputation.summary_value >= min_reputation;
        let val_ok = validation.count == 0 || validation.average_response >= min_validation_score;
        Ok(rep_ok && val_ok)
    }

    // ── RPC helpers ─────────────────────────────────────────────────────────

    async fn eth_call(&self, to: &str, calldata: &[u8]) -> Result<String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{
                "to": to,
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
        Ok(json["result"].as_str().unwrap_or("0x").to_string())
    }

    async fn send_transaction(&self, to: &str, calldata: &[u8]) -> Result<String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendTransaction",
            "params": [{
                "to": to,
                "data": format!("0x{}", hex::encode(calldata))
            }],
            "id": 1
        });
        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("eth_sendTransaction failed")?;

        let json: serde_json::Value = resp.json().await?;
        if let Some(err) = json.get("error") {
            anyhow::bail!("RPC error: {err}");
        }
        json["result"]
            .as_str()
            .map(String::from)
            .context("No tx hash in response")
    }
}
