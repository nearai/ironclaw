//! Statistical learning for estimation improvement.

use std::collections::HashMap;
use std::time::Duration;

use rust_decimal::Decimal;

/// Learning model for estimation adjustments.
#[derive(Debug, Clone)]
pub struct LearningModel {
    /// Cost adjustment factor (multiplier).
    pub cost_factor: f64,
    /// Time adjustment factor (multiplier).
    pub time_factor: f64,
    /// Number of samples.
    pub sample_count: u64,
    /// Running error rate for cost.
    pub cost_error_rate: f64,
    /// Running error rate for time.
    pub time_error_rate: f64,
}

impl Default for LearningModel {
    fn default() -> Self {
        Self {
            cost_factor: 1.0,
            time_factor: 1.0,
            sample_count: 0,
            cost_error_rate: 0.0,
            time_error_rate: 0.0,
        }
    }
}

/// Learner that improves estimates over time.
pub struct EstimationLearner {
    /// Models per category.
    models: HashMap<String, LearningModel>,
    /// Exponential moving average alpha.
    alpha: f64,
    /// Minimum samples before adjusting.
    min_samples: u64,
}

impl EstimationLearner {
    /// Create a new estimation learner.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            alpha: 0.1, // EMA smoothing factor
            min_samples: 5,
        }
    }

    /// Record actual results and update the model.
    pub fn record(
        &mut self,
        category: &str,
        estimated_cost: Decimal,
        actual_cost: Decimal,
        estimated_time: Duration,
        actual_time: Duration,
    ) {
        let model = self.models.entry(category.to_string()).or_default();
        model.sample_count += 1;

        // Calculate errors
        let cost_ratio = if !estimated_cost.is_zero() {
            (actual_cost / estimated_cost)
                .to_string()
                .parse::<f64>()
                .unwrap_or(1.0)
        } else {
            1.0
        };

        let time_ratio = if !estimated_time.is_zero() {
            actual_time.as_secs_f64() / estimated_time.as_secs_f64()
        } else {
            1.0
        };

        // Update factors using exponential moving average
        model.cost_factor = model.cost_factor * (1.0 - self.alpha) + cost_ratio * self.alpha;
        model.time_factor = model.time_factor * (1.0 - self.alpha) + time_ratio * self.alpha;

        // Update error rates
        let cost_error = (cost_ratio - 1.0).abs();
        let time_error = (time_ratio - 1.0).abs();

        model.cost_error_rate =
            model.cost_error_rate * (1.0 - self.alpha) + cost_error * self.alpha;
        model.time_error_rate =
            model.time_error_rate * (1.0 - self.alpha) + time_error * self.alpha;
    }

    /// Adjust estimates based on learned factors.
    pub fn adjust(&self, category: &str, cost: Decimal, time: Duration) -> (Decimal, Duration) {
        let model = self.models.get(category);

        match model {
            Some(m) if m.sample_count >= self.min_samples => {
                let adjusted_cost = cost * Decimal::try_from(m.cost_factor).unwrap_or(Decimal::ONE);
                let adjusted_time = Duration::from_secs_f64(time.as_secs_f64() * m.time_factor);
                (adjusted_cost, adjusted_time)
            }
            _ => (cost, time), // Not enough data, use original estimates
        }
    }

    /// Get confidence for a category (based on sample count and error rate).
    pub fn confidence(&self, category: &str) -> f64 {
        match self.models.get(category) {
            Some(m) if m.sample_count >= self.min_samples => {
                // Higher samples and lower error = higher confidence
                let sample_factor = (m.sample_count as f64 / 100.0).min(1.0);
                let error_factor = 1.0 - ((m.cost_error_rate + m.time_error_rate) / 2.0).min(1.0);
                0.5 + (sample_factor * 0.3) + (error_factor * 0.2)
            }
            Some(_) => 0.3, // Some data but not enough
            None => 0.2,    // No data
        }
    }

    /// Get the model for a category.
    pub fn get_model(&self, category: &str) -> Option<&LearningModel> {
        self.models.get(category)
    }

    /// Get all models.
    pub fn all_models(&self) -> &HashMap<String, LearningModel> {
        &self.models
    }

    /// Set the EMA alpha.
    pub fn set_alpha(&mut self, alpha: f64) {
        self.alpha = alpha.clamp(0.01, 0.5);
    }

    /// Set minimum samples.
    pub fn set_min_samples(&mut self, min: u64) {
        self.min_samples = min;
    }

    /// Clear all learned data.
    pub fn clear(&mut self) {
        self.models.clear();
    }
}

impl Default for EstimationLearner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_learning_model_update() {
        let mut learner = EstimationLearner::new();
        learner.set_min_samples(2);

        // Record some results where actuals are 20% higher than estimates
        for _ in 0..5 {
            learner.record(
                "test",
                dec!(100.0),
                dec!(120.0),
                Duration::from_secs(60),
                Duration::from_secs(72),
            );
        }

        let model = learner.get_model("test").unwrap();
        assert!(model.cost_factor > 1.0);
        assert!(model.time_factor > 1.0);
    }

    #[test]
    fn test_adjustment() {
        let mut learner = EstimationLearner::new();
        learner.set_min_samples(2);

        // Train with consistent 50% underestimation
        for _ in 0..10 {
            learner.record(
                "test",
                dec!(100.0),
                dec!(150.0),
                Duration::from_secs(60),
                Duration::from_secs(90),
            );
        }

        let (adjusted_cost, adjusted_time) =
            learner.adjust("test", dec!(100.0), Duration::from_secs(60));

        // Should adjust upward
        assert!(adjusted_cost > dec!(100.0));
        assert!(adjusted_time > Duration::from_secs(60));
    }

    #[test]
    fn test_confidence() {
        let mut learner = EstimationLearner::new();

        // No data = low confidence
        assert!(learner.confidence("unknown") < 0.5);

        // Add data
        for _ in 0..20 {
            learner.record(
                "known",
                dec!(100.0),
                dec!(100.0), // Perfect estimates
                Duration::from_secs(60),
                Duration::from_secs(60),
            );
        }

        // More data with good accuracy = higher confidence
        assert!(learner.confidence("known") > 0.5);
    }
}
