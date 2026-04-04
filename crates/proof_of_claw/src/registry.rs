//! Tool registry — tracks available tools and their capability hashes.
//!
//! IronClaw handles the actual WASM execution. This registry tracks tool
//! metadata and capability hashes used in policy checks and execution traces.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A registered tool with a deterministic capability hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// Content-addressable SHA-256 of (name | description | wasm_module).
    pub capability_hash: String,
    /// WASM bytecode for the tool (if applicable).
    pub wasm_module: Option<Vec<u8>>,
}

/// Registry of known tools and their capability hashes.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register_builtin_tools();
        registry
    }

    /// Register a tool, computing its capability hash from its definition.
    ///
    /// The hash changes if name, description, or WASM bytecode changes,
    /// providing tamper-evident capability tracking.
    pub fn register(&mut self, mut tool: Tool) {
        tool.capability_hash = compute_capability_hash(
            &tool.name,
            &tool.description,
            &tool.wasm_module,
        );
        self.tools.insert(tool.name.clone(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// List all registered tools.
    pub fn list(&self) -> Vec<&Tool> {
        self.tools.values().collect()
    }

    /// Returns true if the given tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    fn register_builtin_tools(&mut self) {
        for tool_def in [
            ("swap_tokens", "Swap tokens on a DEX"),
            ("transfer", "Transfer tokens to an address"),
            ("query", "Query blockchain state"),
        ] {
            self.register(Tool {
                name: tool_def.0.to_string(),
                description: tool_def.1.to_string(),
                capability_hash: String::new(),
                wasm_module: None,
            });
        }
    }
}

/// Compute a deterministic SHA-256 capability hash from tool metadata.
pub fn compute_capability_hash(
    name: &str,
    description: &str,
    wasm_module: &Option<Vec<u8>>,
) -> String {
    let mut h = Sha256::new();
    h.update(name.as_bytes());
    h.update(b"|");
    h.update(description.as_bytes());
    if let Some(wasm) = wasm_module {
        h.update(b"|");
        h.update(wasm);
    }
    format!("0x{}", hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_hash_deterministic() {
        let h1 = compute_capability_hash("swap", "swap tokens", &None);
        let h2 = compute_capability_hash("swap", "swap tokens", &None);
        assert_eq!(h1, h2);
        assert!(h1.starts_with("0x"));
        assert_eq!(h1.len(), 66);
    }

    #[test]
    fn test_wasm_changes_hash() {
        let h1 = compute_capability_hash("tool", "desc", &None);
        let h2 = compute_capability_hash("tool", "desc", &Some(vec![0, 1, 2]));
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_builtin_tools_registered() {
        let registry = ToolRegistry::new();
        assert!(registry.contains("swap_tokens"));
        assert!(registry.contains("transfer"));
        assert!(registry.contains("query"));
    }
}
