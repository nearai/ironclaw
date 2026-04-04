//! ENS name resolution and DM3 encrypted messaging integration.

use crate::types::AgentMessage;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// ENS Registry on mainnet and Sepolia.
const ENS_REGISTRY: &str = "0x00000000000C2e074eC69A0dFb2997BA6C7d2e1e";

/// DM3 client — sends and receives encrypted inter-agent messages.
pub struct DM3Client {
    client: Client,
    delivery_service_url: String,
    rpc_url: String,
    sender: mpsc::Sender<AgentMessage>,
    #[allow(dead_code)]
    receiver: mpsc::Receiver<AgentMessage>,
}

/// Public DM3 profile for an ENS name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DM3Profile {
    pub public_encryption_key: String,
    pub public_signing_key: String,
    pub delivery_service_url: String,
}

/// Wire format for a DM3 delivery service envelope.
#[derive(Debug, Serialize, Deserialize)]
struct DM3Envelope {
    to: String,
    from: String,
    message: String,
    #[serde(rename = "encryptionEnvelopeType")]
    encryption_type: String,
    timestamp: i64,
}

impl DM3Client {
    pub async fn new(delivery_service_url: String, rpc_url: String) -> Result<Self> {
        let (tx, rx) = mpsc::channel(100);
        Ok(Self {
            client: Client::new(),
            delivery_service_url,
            rpc_url,
            sender: tx,
            receiver: rx,
        })
    }

    pub fn sender(&self) -> mpsc::Sender<AgentMessage> {
        self.sender.clone()
    }

    /// Send an `AgentMessage` to a recipient via their DM3 delivery service.
    pub async fn send_message(&self, recipient_ens: &str, message: &AgentMessage) -> Result<()> {
        let profile = self.resolve_dm3_profile(recipient_ens).await?;
        let delivery_url = if profile.delivery_service_url.is_empty() {
            &self.delivery_service_url
        } else {
            &profile.delivery_service_url
        };

        let envelope = DM3Envelope {
            to: recipient_ens.to_string(),
            from: String::new(),
            message: serde_json::to_string(message)?,
            encryption_type: "x25519-xsalsa20-poly1305".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        let resp = self
            .client
            .post(format!("{}/messages", delivery_url))
            .json(&envelope)
            .send()
            .await
            .context(format!("Failed to deliver DM3 message to {recipient_ens}"))?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "DM3 returned {} for {recipient_ens}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }

        tracing::info!("DM3 message delivered to {recipient_ens}");
        Ok(())
    }

    /// Resolve a DM3 profile for an ENS name.
    ///
    /// Resolution order: on-chain ENS text record → delivery service HTTP API
    /// → bare fallback (uses configured delivery service).
    pub async fn resolve_dm3_profile(&self, ens_name: &str) -> Result<DM3Profile> {
        // 1. On-chain ENS text record
        if let Ok(Some(profile)) = self.resolve_dm3_profile_from_ens(ens_name).await {
            tracing::debug!("Resolved DM3 profile for {ens_name} from ENS");
            return Ok(profile);
        }

        // 2. HTTP fallback via delivery service
        let resp = self
            .client
            .get(format!("{}/profile/{ens_name}", self.delivery_service_url))
            .send()
            .await;

        if let Ok(r) = resp {
            if r.status().is_success() {
                return r.json().await.context("Failed to parse DM3 profile");
            }
        }

        // 3. Bare fallback
        Ok(DM3Profile {
            public_encryption_key: String::new(),
            public_signing_key: String::new(),
            delivery_service_url: self.delivery_service_url.clone(),
        })
    }

    /// Low-level eth_call via JSON-RPC.
    async fn eth_call(&self, to: &str, data: &str) -> Result<String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{ "to": to, "data": data }, "latest"],
            "id": 1
        });
        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("eth_call request failed")?;

        let json: serde_json::Value = resp.json().await.context("eth_call response not JSON")?;
        if let Some(err) = json.get("error") {
            anyhow::bail!("RPC error: {err}");
        }
        json.get("result")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("eth_call returned no result"))
    }

    async fn resolve_dm3_profile_from_ens(&self, ens_name: &str) -> Result<Option<DM3Profile>> {
        let node = namehash(ens_name);
        let node_hex = hex::encode(node);

        // Get resolver from ENS Registry
        let calldata = format!("0x0178b8bf{node_hex}");
        let resolver_result = self.eth_call(ENS_REGISTRY, &calldata).await?;
        let resolver_addr = parse_address(&resolver_result)?;

        if resolver_addr == "0x0000000000000000000000000000000000000000" {
            return Ok(None);
        }

        // Query resolver.text(node, "network.dm3.profile")
        let key = "network.dm3.profile";
        let text_calldata = format!(
            "0x59d1d43c{node_hex}0000000000000000000000000000000000000000000000000000000000000040{}{}",
            format!("{:064x}", key.len()),
            right_pad_hex(&hex::encode(key.as_bytes()), 64)
        );
        let text_result = self.eth_call(&resolver_addr, &text_calldata).await?;
        let profile_json = decode_abi_string(&text_result)?;

        let profile_json_str = match profile_json {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(None),
        };

        let profile: DM3Profile = serde_json::from_str(&profile_json_str)
            .context("Failed to parse DM3 profile JSON from ENS")?;
        Ok(Some(profile))
    }

    /// Poll the delivery service for new incoming messages.
    pub async fn poll_messages(&self) -> Result<Vec<AgentMessage>> {
        let resp = self
            .client
            .get(format!("{}/messages/incoming", self.delivery_service_url))
            .send()
            .await
            .context("Failed to poll DM3 delivery service")?;

        if !resp.status().is_success() {
            anyhow::bail!("DM3 poll returned {}", resp.status());
        }

        Ok(resp.json().await.unwrap_or_default())
    }
}

// ── ENS helpers ──────────────────────────────────────────────────────────────

/// Compute the ENS namehash of `name`.
fn namehash(name: &str) -> [u8; 32] {
    use ethers::utils::keccak256;
    let mut node = [0u8; 32];
    if name.is_empty() {
        return node;
    }
    for label in name.rsplit('.') {
        let label_hash = keccak256(label.as_bytes());
        let mut combined = [0u8; 64];
        combined[..32].copy_from_slice(&node);
        combined[32..].copy_from_slice(&label_hash);
        node = keccak256(&combined);
    }
    node
}

fn parse_address(hex_result: &str) -> Result<String> {
    let clean = hex_result.strip_prefix("0x").unwrap_or(hex_result);
    if clean.len() < 40 {
        anyhow::bail!("result too short for address");
    }
    let addr_hex = &clean[clean.len() - 40..];
    Ok(format!("0x{addr_hex}"))
}

/// Decode an ABI-encoded `string` return value.
fn decode_abi_string(hex_result: &str) -> Result<Option<String>> {
    let clean = hex_result.strip_prefix("0x").unwrap_or(hex_result);
    if clean.len() < 128 {
        return Ok(None);
    }
    let offset = match usize::from_str_radix(&clean[..64], 16) {
        Ok(v) => v * 2,
        Err(_) => return Ok(None),
    };
    if offset + 64 > clean.len() {
        return Ok(None);
    }
    let str_len = match usize::from_str_radix(&clean[offset..offset + 64], 16) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if str_len == 0 || str_len > 100_000 {
        return Ok(None);
    }
    let data_start = offset + 64;
    let data_end = data_start + str_len * 2;
    if data_end > clean.len() {
        return Ok(None);
    }
    let bytes = hex::decode(&clean[data_start..data_end])
        .context("hex decode failed in decode_abi_string")?;
    String::from_utf8(bytes)
        .map(Some)
        .context("UTF-8 decode failed")
}

fn right_pad_hex(hex_str: &str, chunk: usize) -> String {
    let remainder = hex_str.len() % chunk;
    if remainder == 0 {
        hex_str.to_string()
    } else {
        format!("{}{}", hex_str, "0".repeat(chunk - remainder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namehash_empty() {
        assert_eq!(namehash(""), [0u8; 32]);
    }

    #[test]
    fn test_namehash_eth() {
        let expected =
            hex::decode("93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae")
                .unwrap();
        assert_eq!(namehash("eth"), expected.as_slice());
    }

    #[test]
    fn test_namehash_vitalik() {
        let expected =
            hex::decode("ee6c4522aab0003e8d14cd40a6af439055fd2577951148c14b6cea9a53475835")
                .unwrap();
        assert_eq!(namehash("vitalik.eth"), expected.as_slice());
    }
}
