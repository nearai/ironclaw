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
    /// Base URL of the 0G inference **service** (OpenAI-compatible), e.g. from the marketplace
    /// “Build” flow — not necessarily the broker hostname alone. Requests use
    /// `{zero_g_compute_endpoint}/v1/proxy/chat/completions`.
    pub zero_g_compute_endpoint: String,
    /// Bearer token for 0G Compute authentication.
    /// Set via `ZERO_G_COMPUTE_AUTH_TOKEN` env var.
    pub zero_g_compute_auth_token: Option<String>,
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
    /// Agent description/persona (SOUL metadata).
    pub description: Option<String>,
    /// Comma-separated skills list.
    pub skills: Option<String>,
    /// RISC Zero image ID for the proof circuit.
    pub risc_zero_image_id: Option<String>,
    pub policy: PolicyConfig,
    /// True when loaded via `AgentConfig::mock()` — external services may be unavailable.
    #[serde(default)]
    pub is_mock: bool,
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

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            allowed_tools: vec!["query".to_string(), "read".to_string()],
            endpoint_allowlist: vec![],
            max_value_autonomous_wei: 1_000_000_000_000_000_000u64, // 1 ETH
        }
    }
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
        Self::from_env_inner(false)
    }

    /// Load config in mock mode — uses placeholder values for missing/invalid
    /// environment variables so the agent can run locally without external deps.
    ///
    /// Call this instead of `from_env()` when running in development or when
    /// external services (RPC, 0G, DM3) are unavailable.
    pub fn mock() -> Result<Self> {
        Self::from_env_inner(true)
    }

    fn from_env_inner(mock: bool) -> Result<Self> {
        let private_key = env_private_key(mock)?;

        Ok(Self {
            agent_id: env_or("AGENT_ID", "mock-agent", mock),
            ens_name: env_or("ENS_NAME", "mock.proofclaw.eth", mock),
            private_key,
            rpc_url: env_or(
                "RPC_URL",
                "https://eth-sepolia.g.alchemy.com/v3/placeholder",
                mock,
            ),
            zero_g_indexer_rpc: env_or(
                "ZERO_G_INDEXER_RPC",
                "https://indexer-storage-testnet.0g.ai",
                mock,
            ),
            zero_g_compute_endpoint: env_or(
                "ZERO_G_COMPUTE_ENDPOINT",
                "https://broker-testnet.0g.ai",
                mock,
            ),
            zero_g_compute_auth_token: env_opt("ZERO_G_COMPUTE_AUTH_TOKEN"),
            dm3_delivery_service_url: env_or(
                "DM3_DELIVERY_SERVICE_URL",
                "http://localhost:3001",
                mock,
            ),
            ledger_origin_token: env_opt("LEDGER_ORIGIN_TOKEN"),
            eip8004_identity_registry: env_address("EIP8004_IDENTITY_REGISTRY"),
            eip8004_reputation_registry: env_address("EIP8004_REPUTATION_REGISTRY"),
            eip8004_validation_registry: env_address("EIP8004_VALIDATION_REGISTRY"),
            eip8004_integration_contract: env_address("EIP8004_INTEGRATION_CONTRACT"),
            inft_contract: env_address("INFT_CONTRACT"),
            description: env_opt("AGENT_DESCRIPTION"),
            skills: env_opt("AGENT_SKILLS"),
            risc_zero_image_id: env_hash("RISC_ZERO_IMAGE_ID"),
            policy: PolicyConfig {
                allowed_tools: env_or("ALLOWED_TOOLS", "query,read,swap_tokens,transfer", mock)
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                endpoint_allowlist: env_or("ENDPOINT_ALLOWLIST", "", mock)
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                max_value_autonomous_wei: env_or("MAX_VALUE_AUTONOMOUS_WEI", "1000000000000000000", mock)
                    .parse()
                    .context("MAX_VALUE_AUTONOMOUS_WEI must be a valid u64")?,
            },
            is_mock: mock,
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

/// Read an env var, falling back to `default` when `mock` is true and the
/// variable is not set.
fn env_or(var: &str, default: &str, mock: bool) -> String {
    std::env::var(var).unwrap_or_else(|_| {
        if !mock {
            eprintln!(
                "warning: {var} not set — using default '{default}'. \
                 Set MOCK_MODE=1 to suppress this warning."
            );
        }
        default.to_string()
    })
}

fn env_opt(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

/// Read `PRIVATE_KEY`, accepting placeholder values when `mock` is true.
fn env_private_key(mock: bool) -> Result<String> {
    let key = std::env::var("PRIVATE_KEY").unwrap_or_else(|_| {
        if !mock {
            eprintln!("warning: PRIVATE_KEY not set — using placeholder");
        }
        "0x0000000000000000000000000000000000000000000000000000000000000001".to_string()
    });
    let stripped = key.trim_start_matches("0x");
    if stripped.chars().all(|c| c == '0') {
        if mock {
            Ok(key)
        } else {
            anyhow::bail!(
                "PRIVATE_KEY is set to all zeros — configure a real key in .env"
            );
        }
    } else {
        Ok(key)
    }
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
