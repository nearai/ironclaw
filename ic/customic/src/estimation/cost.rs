//! Cost estimation.

use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Estimates costs for tools and operations.
pub struct CostEstimator {
    /// Base costs per tool.
    tool_costs: HashMap<String, Decimal>,
    /// LLM cost per 1K tokens.
    llm_cost_per_1k: Decimal,
}

impl CostEstimator {
    /// Create a new cost estimator.
    pub fn new() -> Self {
        let mut tool_costs = HashMap::new();

        // Default tool costs (in USD or equivalent)
        tool_costs.insert("http".to_string(), dec!(0.0001)); // API call
        tool_costs.insert("echo".to_string(), dec!(0.0)); // Free
        tool_costs.insert("time".to_string(), dec!(0.0)); // Free
        tool_costs.insert("json".to_string(), dec!(0.0)); // Free

        Self {
            tool_costs,
            llm_cost_per_1k: dec!(0.01), // Approximate
        }
    }

    /// Estimate cost for a tool call.
    pub fn estimate_tool(&self, tool_name: &str) -> Decimal {
        self.tool_costs
            .get(tool_name)
            .copied()
            .unwrap_or(dec!(0.001)) // Default for unknown tools
    }

    /// Estimate LLM cost for tokens.
    pub fn estimate_llm_tokens(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        let total_tokens = Decimal::from(input_tokens + output_tokens);
        (total_tokens / dec!(1000)) * self.llm_cost_per_1k
    }

    /// Set a tool's base cost.
    pub fn set_tool_cost(&mut self, tool_name: impl Into<String>, cost: Decimal) {
        self.tool_costs.insert(tool_name.into(), cost);
    }

    /// Get all tool costs.
    pub fn all_tool_costs(&self) -> &HashMap<String, Decimal> {
        &self.tool_costs
    }
}

impl Default for CostEstimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_cost_estimation() {
        let estimator = CostEstimator::new();

        assert_eq!(estimator.estimate_tool("echo"), dec!(0.0));
        assert_eq!(estimator.estimate_tool("http"), dec!(0.0001));
        assert!(estimator.estimate_tool("unknown") > dec!(0.0));
    }

    #[test]
    fn test_llm_cost_estimation() {
        let estimator = CostEstimator::new();

        let cost = estimator.estimate_llm_tokens(1000, 500);
        assert!(cost > dec!(0.0));
    }
}
