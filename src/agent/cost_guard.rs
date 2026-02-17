//! Cost enforcement guardrails for the agent.
//!
//! Tracks LLM spending and action rates, enforcing configurable limits
//! to prevent runaway agents from burning through API credits. Especially
//! important for daemon/heartbeat modes where the agent acts autonomously.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::Mutex;

use crate::llm::costs;

/// Configuration for cost guardrails.
#[derive(Debug, Clone, Default)]
pub struct CostGuardConfig {
    /// Maximum spend per day in cents (e.g. 10000 = $100). None = unlimited.
    pub max_cost_per_day_cents: Option<u64>,
    /// Maximum LLM calls per hour. None = unlimited.
    pub max_actions_per_hour: Option<u64>,
}

/// Error returned when a cost limit is exceeded.
#[derive(Debug, Clone)]
pub enum CostLimitExceeded {
    /// Daily spending cap reached.
    DailyBudget { spent_cents: u64, limit_cents: u64 },
    /// Hourly action rate limit reached.
    HourlyRate { actions: u64, limit: u64 },
}

impl std::fmt::Display for CostLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DailyBudget {
                spent_cents,
                limit_cents,
            } => write!(
                f,
                "Daily cost limit exceeded: spent ${:.2} of ${:.2} allowed",
                *spent_cents as f64 / 100.0,
                *limit_cents as f64 / 100.0
            ),
            Self::HourlyRate { actions, limit } => write!(
                f,
                "Hourly action limit exceeded: {} actions of {} allowed per hour",
                actions, limit
            ),
        }
    }
}

/// Tracks costs and action rates, enforcing configurable limits.
///
/// Thread-safe; designed to be shared via `Arc<CostGuard>`.
pub struct CostGuard {
    config: CostGuardConfig,

    /// Running cost total for the current day (in USD, not cents).
    daily_cost: Mutex<DailyCost>,

    /// Sliding window of action timestamps for rate limiting.
    action_window: Mutex<VecDeque<Instant>>,

    /// Flag set when daily budget is exceeded to short-circuit checks.
    budget_exceeded: AtomicBool,
}

struct DailyCost {
    total: Decimal,
    /// Day boundary (midnight UTC) for resetting the counter.
    reset_date: chrono::NaiveDate,
}

impl CostGuard {
    pub fn new(config: CostGuardConfig) -> Self {
        Self {
            config,
            daily_cost: Mutex::new(DailyCost {
                total: Decimal::ZERO,
                reset_date: chrono::Utc::now().date_naive(),
            }),
            action_window: Mutex::new(VecDeque::new()),
            budget_exceeded: AtomicBool::new(false),
        }
    }

    /// Check whether the next action is allowed under the configured limits.
    ///
    /// Call this BEFORE making an LLM call. Does NOT record the action yet,
    /// call `record_action` after the action completes.
    pub async fn check_allowed(&self) -> Result<(), CostLimitExceeded> {
        // Fast path: if budget already blown, skip the lock
        if self.budget_exceeded.load(Ordering::Relaxed) {
            let daily = self.daily_cost.lock().await;
            let spent_cents = to_cents(daily.total);
            return Err(CostLimitExceeded::DailyBudget {
                spent_cents,
                limit_cents: self.config.max_cost_per_day_cents.unwrap_or(0),
            });
        }

        // Check daily budget
        if let Some(limit_cents) = self.config.max_cost_per_day_cents {
            let daily = self.daily_cost.lock().await;
            let spent_cents = to_cents(daily.total);
            if spent_cents >= limit_cents {
                self.budget_exceeded.store(true, Ordering::Relaxed);
                return Err(CostLimitExceeded::DailyBudget {
                    spent_cents,
                    limit_cents,
                });
            }
        }

        // Check hourly rate
        if let Some(limit) = self.config.max_actions_per_hour {
            let mut window = self.action_window.lock().await;
            let cutoff = Instant::now() - std::time::Duration::from_secs(3600);
            // Drain expired entries
            while window.front().is_some_and(|t| *t < cutoff) {
                window.pop_front();
            }
            let count = window.len() as u64;
            if count >= limit {
                return Err(CostLimitExceeded::HourlyRate {
                    actions: count,
                    limit,
                });
            }
        }

        Ok(())
    }

    /// Record a completed LLM action: its token costs and the action timestamp.
    ///
    /// Call this AFTER an LLM call completes so that costs are tracked.
    pub async fn record_llm_call(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Decimal {
        let (input_rate, output_rate) =
            costs::model_cost(model).unwrap_or_else(costs::default_cost);
        let cost =
            input_rate * Decimal::from(input_tokens) + output_rate * Decimal::from(output_tokens);

        // Update daily cost (reset if new day)
        {
            let mut daily = self.daily_cost.lock().await;
            let today = chrono::Utc::now().date_naive();
            if today != daily.reset_date {
                daily.total = Decimal::ZERO;
                daily.reset_date = today;
                self.budget_exceeded.store(false, Ordering::Relaxed);
                tracing::info!("Cost guard: daily counter reset for {}", today);
            }
            daily.total += cost;

            // Check if we just crossed the threshold
            if let Some(limit_cents) = self.config.max_cost_per_day_cents {
                let spent_cents = to_cents(daily.total);
                if spent_cents >= limit_cents {
                    self.budget_exceeded.store(true, Ordering::Relaxed);
                    tracing::warn!(
                        "Daily cost limit reached: ${:.2} of ${:.2}",
                        daily.total,
                        Decimal::from(limit_cents) / dec!(100)
                    );
                }
                // Warn at 80% threshold
                let warn_threshold = limit_cents * 80 / 100;
                if spent_cents >= warn_threshold && spent_cents < limit_cents {
                    tracing::warn!(
                        "Approaching daily cost limit: ${:.2} of ${:.2} ({}%)",
                        daily.total,
                        Decimal::from(limit_cents) / dec!(100),
                        spent_cents * 100 / limit_cents
                    );
                }
            }
        }

        // Record action in sliding window
        {
            let mut window = self.action_window.lock().await;
            window.push_back(Instant::now());
        }

        cost
    }

    /// Current daily spend in USD (as Decimal).
    pub async fn daily_spend(&self) -> Decimal {
        let daily = self.daily_cost.lock().await;
        let today = chrono::Utc::now().date_naive();
        if today != daily.reset_date {
            Decimal::ZERO
        } else {
            daily.total
        }
    }

    /// Number of actions in the current hourly window.
    pub async fn actions_this_hour(&self) -> u64 {
        let mut window = self.action_window.lock().await;
        let cutoff = Instant::now() - std::time::Duration::from_secs(3600);
        while window.front().is_some_and(|t| *t < cutoff) {
            window.pop_front();
        }
        window.len() as u64
    }
}

/// Convert a Decimal USD amount to whole cents (truncated).
fn to_cents(usd: Decimal) -> u64 {
    let cents = (usd * dec!(100)).trunc();
    cents.to_string().parse::<u64>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unlimited_allows_everything() {
        let guard = CostGuard::new(CostGuardConfig::default());

        // No limits set, should always be allowed
        assert!(guard.check_allowed().await.is_ok());

        // Record a big call, still allowed
        guard.record_llm_call("gpt-4o", 100_000, 100_000).await;
        assert!(guard.check_allowed().await.is_ok());
    }

    #[tokio::test]
    async fn test_daily_budget_enforcement() {
        let guard = CostGuard::new(CostGuardConfig {
            max_cost_per_day_cents: Some(1), // $0.01 limit
            max_actions_per_hour: None,
        });

        // First call allowed
        assert!(guard.check_allowed().await.is_ok());

        // Record a call that costs more than $0.01
        // gpt-4o: input=$0.0000025/tok, output=$0.00001/tok
        // 10000 input + 10000 output = $0.025 + $0.10 = $0.125
        guard.record_llm_call("gpt-4o", 10_000, 10_000).await;

        // Now should be blocked
        let result = guard.check_allowed().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CostLimitExceeded::DailyBudget { limit_cents, .. } => {
                assert_eq!(limit_cents, 1);
            }
            other => panic!("Expected DailyBudget, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_hourly_rate_enforcement() {
        let guard = CostGuard::new(CostGuardConfig {
            max_cost_per_day_cents: None,
            max_actions_per_hour: Some(3),
        });

        // First 3 actions allowed
        for _ in 0..3 {
            assert!(guard.check_allowed().await.is_ok());
            guard.record_llm_call("gpt-4o", 10, 10).await;
        }

        // 4th should be blocked
        let result = guard.check_allowed().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CostLimitExceeded::HourlyRate { actions, limit } => {
                assert_eq!(actions, 3);
                assert_eq!(limit, 3);
            }
            other => panic!("Expected HourlyRate, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_daily_spend_tracking() {
        let guard = CostGuard::new(CostGuardConfig::default());

        assert_eq!(guard.daily_spend().await, Decimal::ZERO);

        let cost = guard.record_llm_call("gpt-4o", 1000, 500).await;
        assert!(cost > Decimal::ZERO);
        assert_eq!(guard.daily_spend().await, cost);
    }

    #[tokio::test]
    async fn test_actions_this_hour() {
        let guard = CostGuard::new(CostGuardConfig::default());

        assert_eq!(guard.actions_this_hour().await, 0);

        guard.record_llm_call("gpt-4o", 10, 10).await;
        guard.record_llm_call("gpt-4o", 10, 10).await;

        assert_eq!(guard.actions_this_hour().await, 2);
    }

    #[test]
    fn test_to_cents() {
        assert_eq!(to_cents(dec!(1.50)), 150);
        assert_eq!(to_cents(dec!(0.01)), 1);
        assert_eq!(to_cents(Decimal::ZERO), 0);
    }

    #[test]
    fn test_cost_limit_display() {
        let budget = CostLimitExceeded::DailyBudget {
            spent_cents: 1050,
            limit_cents: 1000,
        };
        assert!(budget.to_string().contains("$10.50"));
        assert!(budget.to_string().contains("$10.00"));

        let rate = CostLimitExceeded::HourlyRate {
            actions: 101,
            limit: 100,
        };
        assert!(rate.to_string().contains("101 actions"));
        assert!(rate.to_string().contains("100 allowed"));
    }
}
