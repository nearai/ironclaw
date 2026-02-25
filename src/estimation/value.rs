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

    // === Additional boundary / edge-case tests (QA Plan 4.4) ===

    #[test]
    fn is_profitable_with_very_large_values() {
        let estimator = ValueEstimator::new();
        // rust_decimal::Decimal max is ~79_228_162_514_264_337_593_543_950_335.
        // Use values large enough to stress multiplication but within Decimal range.
        let big = Decimal::new(i64::MAX, 0); // 9_223_372_036_854_775_807
        let small = Decimal::new(1, 0);

        // Large price, small cost -- clearly profitable, must not overflow.
        assert!(estimator.is_profitable(big, small));

        // Large cost, small price -- clearly unprofitable.
        assert!(!estimator.is_profitable(small, big));

        // Large equal values: margin = 0, which is < 10% min -- not profitable.
        assert!(!estimator.is_profitable(big, big));
    }

    #[test]
    fn estimate_value_with_very_large_cost() {
        let estimator = ValueEstimator::new();
        let big = Decimal::new(i64::MAX / 2, 0);
        let value = estimator.estimate("big job", big);
        // value = cost + cost * 0.3 = cost * 1.3, should not overflow.
        assert!(value > big);
    }

    #[test]
    fn is_profitable_with_negative_price() {
        let estimator = ValueEstimator::new();
        // Negative price is an unusual edge case. The current formula
        // margin = (price - cost) / price can produce misleading results
        // because dividing two negatives yields a positive.
        //
        // price = -10, cost = 5: margin = (-10 - 5) / -10 = 1.5 >= 0.1
        // The formula says "profitable" even though the scenario is nonsensical.
        // We document the current behavior here; a guard for negative prices
        // could be added in a future hardening pass.
        assert!(estimator.is_profitable(dec!(-10.0), dec!(5.0)));

        // price = -10, cost = -20: margin = (-10 - (-20)) / -10 = -1.0 < 0.1.
        assert!(!estimator.is_profitable(dec!(-10.0), dec!(-20.0)));
    }

    #[test]
    fn calculate_margin_with_negative_earnings() {
        let estimator = ValueEstimator::new();
        // Negative earnings -- margin formula still computes without panic.
        let margin = estimator.calculate_margin(dec!(-100.0), dec!(50.0));
        // (earnings - cost) / earnings = (-100 - 50) / -100 = 1.5
        assert_eq!(margin, dec!(1.5));
    }

    #[test]
    fn calculate_margin_with_both_negative() {
        let estimator = ValueEstimator::new();
        // Both negative: earnings = -50, cost = -100.
        // margin = (-50 - (-100)) / -50 = 50 / -50 = -1.0
        let margin = estimator.calculate_margin(dec!(-50.0), dec!(-100.0));
        assert_eq!(margin, dec!(-1.0));
    }

    #[test]
    fn minimum_bid_with_zero_cost() {
        let estimator = ValueEstimator::new();
        // Zero cost -- both bids should be zero.
        assert_eq!(estimator.minimum_bid(Decimal::ZERO), Decimal::ZERO);
        assert_eq!(estimator.ideal_bid(Decimal::ZERO), Decimal::ZERO);
    }

    #[test]
    fn minimum_bid_with_negative_cost() {
        let estimator = ValueEstimator::new();
        // Negative cost -- the bid formulas still compute (cost + cost * margin),
        // producing a negative bid (we'd pay them).
        let min_bid = estimator.minimum_bid(dec!(-100.0));
        let ideal_bid = estimator.ideal_bid(dec!(-100.0));
        assert!(min_bid < Decimal::ZERO);
        assert!(ideal_bid < Decimal::ZERO);
        // With negative values, ideal (more negative) < minimum (less negative).
        assert!(ideal_bid < min_bid);
    }

    #[test]
    fn estimate_with_negative_cost() {
        let estimator = ValueEstimator::new();
        // Negative cost: value = cost + cost * 0.3 = -100 + (-30) = -130.
        let value = estimator.estimate("refund task", dec!(-100.0));
        assert_eq!(value, dec!(-130.0));
    }

    #[test]
    fn custom_margins_affect_profitability() {
        let mut estimator = ValueEstimator::new();
        let price = dec!(110.0);
        let cost = dec!(100.0);

        // Default 10% min margin: (110 - 100) / 110 ~= 9.09% < 10% -> not profitable.
        assert!(!estimator.is_profitable(price, cost));

        // Lower min margin to 5% -> now 9.09% >= 5% -> profitable.
        estimator.set_min_margin(dec!(0.05));
        assert!(estimator.is_profitable(price, cost));

        // Raise min margin to 50% -> 9.09% < 50% -> not profitable.
        estimator.set_min_margin(dec!(0.50));
        assert!(!estimator.is_profitable(price, cost));
    }

    #[test]
    fn custom_target_margin_affects_bids() {
        let mut estimator = ValueEstimator::new();
        let cost = dec!(100.0);

        let default_ideal = estimator.ideal_bid(cost);
        assert_eq!(default_ideal, dec!(130.0)); // 100 + 30%

        estimator.set_target_margin(dec!(0.5));
        let new_ideal = estimator.ideal_bid(cost);
        assert_eq!(new_ideal, dec!(150.0)); // 100 + 50%
    }

    #[test]
    fn is_profitable_at_exact_margin_boundary() {
        let estimator = ValueEstimator::new();
        // min_margin = 0.1 (10%). Price = 100, cost = 90 -> margin = 10/100 = 0.1.
        // Exactly at boundary -- should be profitable (>=).
        assert!(estimator.is_profitable(dec!(100.0), dec!(90.0)));

        // Slightly below boundary: cost = 90.01 -> margin = 9.99/100 = 0.0999 < 0.1.
        assert!(!estimator.is_profitable(dec!(100.0), dec!(90.01)));
    }

    #[test]
    fn profit_with_zero_values() {
        let estimator = ValueEstimator::new();
        assert_eq!(
            estimator.calculate_profit(Decimal::ZERO, Decimal::ZERO),
            Decimal::ZERO
        );
        assert_eq!(
            estimator.calculate_profit(Decimal::ZERO, dec!(100.0)),
            dec!(-100.0)
        );
        assert_eq!(
            estimator.calculate_profit(dec!(100.0), Decimal::ZERO),
            dec!(100.0)
        );
    }

    #[test]
    fn default_impl_matches_new() {
        let from_new = ValueEstimator::new();
        let from_default = ValueEstimator::default();
        let cost = dec!(100.0);

        // Both should produce identical results.
        assert_eq!(from_new.estimate("x", cost), from_default.estimate("x", cost));
        assert_eq!(from_new.minimum_bid(cost), from_default.minimum_bid(cost));
        assert_eq!(from_new.ideal_bid(cost), from_default.ideal_bid(cost));
        assert_eq!(
            from_new.is_profitable(dec!(150.0), cost),
            from_default.is_profitable(dec!(150.0), cost)
        );
    }
}
