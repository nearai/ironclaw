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

    // === QA Plan P2 - 4.4: Value estimator boundary tests ===

    #[test]
    fn test_profitability_negative_cost() {
        let estimator = ValueEstimator::new();
        // Negative cost means we get paid to do the work -- always profitable
        // with any positive price.
        assert!(estimator.is_profitable(dec!(100.0), dec!(-50.0)));
        assert!(estimator.is_profitable(dec!(1.0), dec!(-0.01)));
    }

    #[test]
    fn test_profitability_cost_exceeds_price() {
        let estimator = ValueEstimator::new();
        // Cost exceeds price → negative margin → not profitable.
        assert!(!estimator.is_profitable(dec!(10.0), dec!(100.0)));
    }

    #[test]
    fn test_margin_zero_earnings() {
        let estimator = ValueEstimator::new();
        // Zero earnings → margin should be zero, not panic from divide-by-zero.
        assert_eq!(
            estimator.calculate_margin(Decimal::ZERO, dec!(50.0)),
            Decimal::ZERO
        );
        assert_eq!(
            estimator.calculate_margin(Decimal::ZERO, Decimal::ZERO),
            Decimal::ZERO
        );
    }

    #[test]
    fn test_estimate_zero_cost() {
        let estimator = ValueEstimator::new();
        // Zero cost → value estimate should be zero (cost + 30% of zero).
        let value = estimator.estimate("free task", Decimal::ZERO);
        assert_eq!(value, Decimal::ZERO);
    }

    #[test]
    fn test_minimum_vs_ideal_bid() {
        let estimator = ValueEstimator::new();
        let cost = dec!(100.0);
        let min_bid = estimator.minimum_bid(cost);
        let ideal_bid = estimator.ideal_bid(cost);
        // Minimum bid should always be less than ideal bid.
        assert!(min_bid < ideal_bid);
        // Both should be above cost.
        assert!(min_bid > cost);
        assert!(ideal_bid > cost);
    }

    #[test]
    fn test_profit_calculation() {
        let estimator = ValueEstimator::new();
        assert_eq!(
            estimator.calculate_profit(dec!(150.0), dec!(100.0)),
            dec!(50.0)
        );
        // Negative profit (loss).
        assert_eq!(
            estimator.calculate_profit(dec!(50.0), dec!(100.0)),
            dec!(-50.0)
        );
    }
}
