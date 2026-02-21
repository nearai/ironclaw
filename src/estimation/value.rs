//! Value/earnings estimation.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Estimates the value/earnings potential of jobs.
pub struct ValueEstimator {
    /// Minimum profit margin to aim for.
    min_margin: Decimal,
    /// Target profit margin.
    target_margin: Decimal,
}

impl ValueEstimator {
    /// Create a new value estimator.
    pub fn new() -> Self {
        Self {
            min_margin: dec!(0.1),    // 10% minimum
            target_margin: dec!(0.3), // 30% target
        }
    }

    /// Estimate value for a job based on description and cost.
    pub fn estimate(&self, _description: &str, estimated_cost: Decimal) -> Decimal {
        // Simple formula: value = cost + margin
        // In practice, this would analyze the description to estimate complexity
        let margin = estimated_cost * self.target_margin;
        estimated_cost + margin
    }

    /// Calculate minimum acceptable bid.
    pub fn minimum_bid(&self, estimated_cost: Decimal) -> Decimal {
        estimated_cost + (estimated_cost * self.min_margin)
    }

    /// Calculate ideal bid.
    pub fn ideal_bid(&self, estimated_cost: Decimal) -> Decimal {
        estimated_cost + (estimated_cost * self.target_margin)
    }

    /// Check if a job is profitable at a given price.
    pub fn is_profitable(&self, price: Decimal, estimated_cost: Decimal) -> bool {
        if price.is_zero() {
            // With a zero price, the job is only profitable if the cost is negative.
            // This results in a positive profit and an effectively infinite margin.
            return estimated_cost < Decimal::ZERO;
        }
        let margin = (price - estimated_cost) / price;
        margin >= self.min_margin
    }

    /// Calculate profit for a completed job.
    pub fn calculate_profit(&self, earnings: Decimal, actual_cost: Decimal) -> Decimal {
        earnings - actual_cost
    }

    /// Calculate profit margin.
    pub fn calculate_margin(&self, earnings: Decimal, actual_cost: Decimal) -> Decimal {
        if earnings.is_zero() {
            return Decimal::ZERO;
        }
        (earnings - actual_cost) / earnings
    }

    /// Set minimum margin.
    pub fn set_min_margin(&mut self, margin: Decimal) {
        self.min_margin = margin;
    }

    /// Set target margin.
    pub fn set_target_margin(&mut self, margin: Decimal) {
        self.target_margin = margin;
    }
}

impl Default for ValueEstimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_estimation() {
        let estimator = ValueEstimator::new();

        let cost = dec!(10.0);
        let value = estimator.estimate("test job", cost);

        assert!(value > cost);
    }

    #[test]
    fn test_profitability() {
        let estimator = ValueEstimator::new();

        let cost = dec!(10.0);
        assert!(estimator.is_profitable(dec!(15.0), cost));
        assert!(!estimator.is_profitable(dec!(10.5), cost)); // Only 5% margin
    }

    #[test]
    fn test_margin_calculation() {
        let estimator = ValueEstimator::new();

        let margin = estimator.calculate_margin(dec!(100.0), dec!(70.0));
        assert_eq!(margin, dec!(0.30)); // 30%
    }

    #[test]
    fn test_profitability_zero_price() {
        let estimator = ValueEstimator::new();

        // Zero price should return false, not panic
        assert!(!estimator.is_profitable(Decimal::ZERO, dec!(10.0)));
        assert!(!estimator.is_profitable(Decimal::ZERO, Decimal::ZERO));
        // Negative cost with zero price is profitable (we get paid to do it)
        assert!(estimator.is_profitable(Decimal::ZERO, dec!(-10.0)));
    }
}
