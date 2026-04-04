//! Configuration for Proof of Claw — loaded from environment variables.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Human-readable agent identifier (also used as ENS subname).
    pub agent_id: String,
    /// ENS name for this agent (e.g. `myagent.proofclaw.eth`).
    pub ens_name: String,
    /// Ethereum private key (hex, with or without `0x` prefix).
    pub private_key: String,
    /// Ethereum RPC endpoint (e.g. `https://eth-sepolia.g.alchemy.com/v3/...`).
    pub rpc_url: String,
    /// 0G Storage indexer RPC endpoint.
    pub zero_g_indexer_rpc: String,
    /// 0G Compute inference endpoint.
    pub zero_g_compute_endpoint: String,
    /// DM3 delivery service URL for encrypted messaging.
    pub dm3_delivery_service_url: String,
    /// Ledger origin token (ERC-7730 clear-signing). Optional.
    pub ledger_origin_token: Option<String>,
    // ── EIP-8004 registries ──────────────────────────────────────────────
    pub eip8004_identity_registry: Option<String>,
    pub eip8004_reputation_registry: Option<String>,
    pub eip8004_validation_registry: Option<String>,
    pub eip8004_integration_contract: Option<String>,
    /// ERC-7857 iNFT contract address.
    pub inft_contract: Option<String>,
    /// RISC Zero image ID for the proof circuit.
    pub risc_zero_image_id: Option<String>,
    pub policy: PolicyConfig,
}

/// Policy thresholds and tool allowlist for this agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Tools this agent is allowed to invoke. Empty = allow all.
    pub allowed_tools: Vec<String>,
    /// Approved HTTP endpoints. Empty = allow all.
    pub endpoint_allowlist: Vec<String>,
    /// Maximum value (in wei) for autonomous actions. Actions above this
    /// require physical Ledger approval.
    pub max_value_autonomous_wei: u64,
}

impl AgentConfig {
    /// Load all config from environment variables.
    ///
    /// # Required env vars
    /// - `AGENT_ID`, `ENS_NAME`, `PRIVATE_KEY`, `RPC_URL`
    /// - `ZERO_G_INDEXER_RPC`, `ZERO_G_COMPUTE_ENDPOINT`
    /// - `DM3_DELIVERY_SERVICE_URL`
    /// - `ALLOWED_TOOLS`, `ENDPOINT_ALLOWLIST`, `MAX_VALUE_AUTONOMOUS_WEI`
    ///
    /// # Optional env vars
    /// - `LEDGER_ORIGIN_TOKEN`
    /// - `EIP8004_IDENTITY_REGISTRY`, `EIP8004_REPUTATION_REGISTRY`,
    ///   `EIP8004_VALIDATION_REGISTRY`, `EIP8004_INTEGRATION_CONTRACT`
    /// - `INFT_CONTRACT`, `RISC_ZERO_IMAGE_ID`
    pub fn from_env() -> Result<Self> {
        let private_key = env_private_key()?;

        Ok(Self {
            agent_id: env("AGENT_ID")?,
            ens_name: env("ENS_NAME")?,
            private_key,
            rpc_url: env("RPC_URL")?,
            zero_g_indexer_rpc: env("ZERO_G_INDEXER_RPC")?,
            zero_g_compute_endpoint: env("ZERO_G_COMPUTE_ENDPOINT")?,
            dm3_delivery_service_url: env("DM3_DELIVERY_SERVICE_URL")?,
            ledger_origin_token: env_opt("LEDGER_ORIGIN_TOKEN"),
            eip8004_identity_registry: env_address("EIP8004_IDENTITY_REGISTRY"),
            eip8004_reputation_registry: env_address("EIP8004_REPUTATION_REGISTRY"),
            eip8004_validation_registry: env_address("EIP8004_VALIDATION_REGISTRY"),
            eip8004_integration_contract: env_address("EIP8004_INTEGRATION_CONTRACT"),
            inft_contract: env_address("INFT_CONTRACT"),
            risc_zero_image_id: env_hash("RISC_ZERO_IMAGE_ID"),
            policy: PolicyConfig {
                allowed_tools: env("ALLOWED_TOOLS")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                endpoint_allowlist: env("ENDPOINT_ALLOWLIST")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                max_value_autonomous_wei: env("MAX_VALUE_AUTONOMOUS_WEI")?
                    .parse()
                    .context("MAX_VALUE_AUTONOMOUS_WEI must be a valid u64")?,
            },
        })
    }

    /// Returns true when EIP-8004 registries are configured.
    pub fn has_eip8004(&self) -> bool {
        self.eip8004_identity_registry.is_some()
    }

    /// Returns true when the iNFT contract is configured.
    pub fn has_inft(&self) -> bool {
        self.inft_contract.is_some()
    }
}

// ── Env helpers ───────────────────────────────────────────────────────────────

fn env(var: &str) -> Result<String> {
    std::env::var(var).context(format!("${var} not set"))
}

fn env_opt(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

/// Read a hex address env var. Returns `None` if unset or all-zeros.
fn env_address(key: &str) -> Option<String> {
    env_opt(key).and_then(|v| {
        let stripped = v.trim_start_matches("0x");
        if stripped.chars().all(|c| c == '0') {
            None
        } else {
            Some(v)
        }
    })
}

/// Read a hex hash env var. Returns `None` if unset or all-zeros.
fn env_hash(key: &str) -> Option<String> {
    env_opt(key).and_then(|v| {
        let stripped = v.trim_start_matches("0x");
        if stripped.is_empty() || stripped.chars().all(|c| c == '0') {
            None
        } else {
            Some(v)
        }
    })
}

/// Read `PRIVATE_KEY`, rejecting all-zeros placeholder values.
fn env_private_key() -> Result<String> {
    let key = env("PRIVATE_KEY")?;
    let stripped = key.trim_start_matches("0x");
    if stripped.chars().all(|c| c == '0') {
        anyhow::bail!(
            "PRIVATE_KEY is set to all zeros — configure a real key in .env"
        );
    }
    Ok(key)
}
