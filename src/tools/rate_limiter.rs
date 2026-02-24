//! Shared rate limiter for built-in and WASM tool invocations.
//!
//! Provides per-tool, per-user rate limiting using a sliding window counter.
//! Built-in tools (shell, http, file write, etc.) are throttled here before
//! `tool.execute()` is called in the agent loop. WASM tools re-export these
//! types for HTTP-level rate limiting inside host functions.
//!
//! # Rate Limit Algorithm
//!
//! Uses a simplified sliding window counter:
//! - Track request counts for current minute and hour windows
//! - Reset counters when window expires
//! - Increment counter and check against limits
//!
//! # Persistence
//!
//! Rate limit state is in-memory only. Limits reset on process restart.
//! This is acceptable for v1; future versions may persist to the database.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::tools::tool::ToolRateLimitConfig;

const MINUTE_SECS: u64 = 60;
const HOUR_SECS: u64 = 3600;

/// Result of a rate limit check.
#[derive(Debug, Clone)]
pub enum RateLimitResult {
    /// Request is allowed.
    Allowed {
        /// Remaining requests in the current minute.
        remaining_minute: u32,
        /// Remaining requests in the current hour.
        remaining_hour: u32,
    },
    /// Request is rate limited.
    Limited {
        /// When the rate limit will reset.
        retry_after: Duration,
        /// Which limit was exceeded.
        limit_type: LimitType,
    },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitResult::Allowed { .. })
    }
}

/// Which rate limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitType {
    PerMinute,
    PerHour,
}

impl std::fmt::Display for LimitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitType::PerMinute => write!(f, "per-minute"),
            LimitType::PerHour => write!(f, "per-hour"),
        }
    }
}

/// State for a single rate limit window.
#[derive(Debug, Clone)]
struct WindowState {
    window_start: Instant,
    count: u32,
}

impl WindowState {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            count: 0,
        }
    }

    /// Check if the window has expired and reset if needed.
    fn maybe_reset(&mut self, window_duration: Duration) {
        if self.window_start.elapsed() >= window_duration {
            self.window_start = Instant::now();
            self.count = 0;
        }
    }

    /// Time until window resets.
    fn time_until_reset(&self, window_duration: Duration) -> Duration {
        let elapsed = self.window_start.elapsed();
        if elapsed >= window_duration {
            Duration::ZERO
        } else {
            window_duration - elapsed
        }
    }
}

/// Rate limit state for a single (user, tool) pair.
#[derive(Debug)]
struct ToolRateLimitState {
    minute_window: WindowState,
    hour_window: WindowState,
}

impl ToolRateLimitState {
    fn new() -> Self {
        Self {
            minute_window: WindowState::new(),
            hour_window: WindowState::new(),
        }
    }
}

/// In-memory rate limiter for tool invocations.
///
/// Keyed by `(user_id, tool_name)` so each user has independent limits.
/// Shared via `Arc` — a single instance lives in `ToolRegistry` and is
/// checked before every built-in tool execution.
pub struct RateLimiter {
    state: RwLock<HashMap<(String, String), ToolRateLimitState>>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    pub fn new() -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
        }
    }

    /// Shared logic: reset windows, check limits, and optionally record the request.
    async fn check_internal(
        &self,
        user_id: &str,
        tool_name: &str,
        config: &ToolRateLimitConfig,
        record: bool,
    ) -> RateLimitResult {
        let key = (user_id.to_string(), tool_name.to_string());

        let mut state = self.state.write().await;
        let tool_state = state.entry(key).or_insert_with(ToolRateLimitState::new);

        // Reset windows if expired.
        tool_state
            .minute_window
            .maybe_reset(Duration::from_secs(MINUTE_SECS));
        tool_state
            .hour_window
            .maybe_reset(Duration::from_secs(HOUR_SECS));

        // Check minute limit.
        if tool_state.minute_window.count >= config.requests_per_minute {
            return RateLimitResult::Limited {
                retry_after: tool_state
                    .minute_window
                    .time_until_reset(Duration::from_secs(MINUTE_SECS)),
                limit_type: LimitType::PerMinute,
            };
        }

        // Check hour limit.
        if tool_state.hour_window.count >= config.requests_per_hour {
            return RateLimitResult::Limited {
                retry_after: tool_state
                    .hour_window
                    .time_until_reset(Duration::from_secs(HOUR_SECS)),
                limit_type: LimitType::PerHour,
            };
        }

        if record {
            tool_state.minute_window.count += 1;
            tool_state.hour_window.count += 1;
        }

        RateLimitResult::Allowed {
            remaining_minute: config.requests_per_minute - tool_state.minute_window.count,
            remaining_hour: config.requests_per_hour - tool_state.hour_window.count,
        }
    }

    /// Check if a request is allowed and record it if so.
    pub async fn check_and_record(
        &self,
        user_id: &str,
        tool_name: &str,
        config: &ToolRateLimitConfig,
    ) -> RateLimitResult {
        self.check_internal(user_id, tool_name, config, true).await
    }

    /// Check without recording (for preview/estimation).
    pub async fn check(
        &self,
        user_id: &str,
        tool_name: &str,
        config: &ToolRateLimitConfig,
    ) -> RateLimitResult {
        self.check_internal(user_id, tool_name, config, false).await
    }

    /// Get current usage for a (user, tool) pair.
    pub async fn get_usage(&self, user_id: &str, tool_name: &str) -> Option<(u32, u32)> {
        let key = (user_id.to_string(), tool_name.to_string());
        let state = self.state.read().await;
        state
            .get(&key)
            .map(|s| (s.minute_window.count, s.hour_window.count))
    }

    /// Clear rate limit state for a specific (user, tool) pair.
    pub async fn clear(&self, user_id: &str, tool_name: &str) {
        let key = (user_id.to_string(), tool_name.to_string());
        self.state.write().await.remove(&key);
    }

    /// Clear all rate limit state.
    pub async fn clear_all(&self) {
        self.state.write().await.clear();
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Error when rate limited.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Rate limited ({limit_type}), retry after {retry_after:?}")]
pub struct RateLimitError {
    pub retry_after: Duration,
    pub limit_type: LimitType,
}

impl From<RateLimitResult> for Result<(), RateLimitError> {
    fn from(result: RateLimitResult) -> Self {
        match result {
            RateLimitResult::Allowed { .. } => Ok(()),
            RateLimitResult::Limited {
                retry_after,
                limit_type,
            } => Err(RateLimitError {
                retry_after,
                limit_type,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool::ToolRateLimitConfig;

    #[tokio::test]
    async fn test_allowed_within_limits() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(10, 100);

        let result = limiter.check_and_record("user1", "shell", &config).await;

        match result {
            RateLimitResult::Allowed {
                remaining_minute,
                remaining_hour,
            } => {
                assert_eq!(remaining_minute, 9);
                assert_eq!(remaining_hour, 99);
            }
            _ => panic!("Expected allowed"),
        }
    }

    #[tokio::test]
    async fn test_minute_limit_exceeded() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(2, 100);

        // Use up the minute limit
        limiter.check_and_record("user1", "shell", &config).await;
        limiter.check_and_record("user1", "shell", &config).await;

        // Third request should be limited
        let result = limiter.check_and_record("user1", "shell", &config).await;

        match result {
            RateLimitResult::Limited {
                limit_type,
                retry_after,
            } => {
                assert_eq!(limit_type, LimitType::PerMinute);
                assert!(retry_after.as_secs() <= 60);
            }
            _ => panic!("Expected limited"),
        }
    }

    #[tokio::test]
    async fn test_hour_limit_exceeded() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(100, 2);

        // Use up the hour limit
        limiter.check_and_record("user1", "shell", &config).await;
        limiter.check_and_record("user1", "shell", &config).await;

        // Third request should be limited
        let result = limiter.check_and_record("user1", "shell", &config).await;

        match result {
            RateLimitResult::Limited { limit_type, .. } => {
                assert_eq!(limit_type, LimitType::PerHour);
            }
            _ => panic!("Expected limited"),
        }
    }

    #[tokio::test]
    async fn test_user_isolation() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(1, 10);

        // User1 uses their limit
        limiter.check_and_record("user1", "shell", &config).await;
        let result1 = limiter.check_and_record("user1", "shell", &config).await;

        // User2 should still have their limit
        let result2 = limiter.check_and_record("user2", "shell", &config).await;

        assert!(!result1.is_allowed());
        assert!(result2.is_allowed());
    }

    #[tokio::test]
    async fn test_tool_isolation() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(1, 10);

        // shell uses its limit
        limiter.check_and_record("user1", "shell", &config).await;
        let result1 = limiter.check_and_record("user1", "shell", &config).await;

        // http should still have its limit
        let result2 = limiter.check_and_record("user1", "http", &config).await;

        assert!(!result1.is_allowed());
        assert!(result2.is_allowed());
    }

    #[tokio::test]
    async fn test_get_usage() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(30, 300);

        limiter.check_and_record("user1", "shell", &config).await;
        limiter.check_and_record("user1", "shell", &config).await;
        limiter.check_and_record("user1", "shell", &config).await;

        let usage = limiter.get_usage("user1", "shell").await;
        assert_eq!(usage, Some((3, 3)));
    }

    #[tokio::test]
    async fn test_clear() {
        let limiter = RateLimiter::new();
        let config = ToolRateLimitConfig::new(1, 10);

        limiter.check_and_record("user1", "shell", &config).await;
        let result1 = limiter.check_and_record("user1", "shell", &config).await;
        assert!(!result1.is_allowed());

        limiter.clear("user1", "shell").await;

        let result2 = limiter.check_and_record("user1", "shell", &config).await;
        assert!(result2.is_allowed());
    }

    #[tokio::test]
    async fn test_read_only_tools_have_no_config() {
        // Read-only tools return None from rate_limit_config() —
        // verified in the individual tool tests, but assert the config
        // type we'd use for write tools has sensible defaults here.
        let write_config = ToolRateLimitConfig::new(20, 200);
        assert_eq!(write_config.requests_per_minute, 20);
        assert_eq!(write_config.requests_per_hour, 200);
    }
}
