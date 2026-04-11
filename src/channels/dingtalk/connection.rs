//! Connection lifecycle management for DingTalk Stream mode.
//!
//! Tracks connection state, heartbeat health, and bounded reconnection with
//! exponential backoff. Prevents infinite reconnect loops via cycle limits
//! and a wall-clock deadline.

use std::time::{Duration, Instant, UNIX_EPOCH};

/// Current state of the DingTalk Stream connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // variants are design-forward; not all are used yet
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Failed,
}

/// Manages connection health, heartbeat tracking, and bounded reconnection.
pub struct ConnectionManager {
    pub state: ConnectionState,
    /// Number of consecutive heartbeat intervals with no message received.
    pub consecutive_heartbeat_misses: u32,
    /// Number of reconnect cycles attempted in this failure run.
    pub reconnect_cycles: u32,
    /// Maximum reconnect cycles before giving up.
    pub max_reconnect_cycles: u32,
    /// How long (ms) to keep retrying before declaring failure.
    pub reconnect_deadline_ms: u64,
    /// Current backoff delay for the next reconnect sleep.
    pub backoff_delay: Duration,
    /// Wall-clock deadline for the current reconnect run. Set on first reconnect,
    /// cleared on successful connection.
    pub reconnect_deadline: Option<Instant>,
}

#[allow(dead_code)] // used in on_heartbeat_miss; production wiring comes in a later unit
const HEARTBEAT_MISS_THRESHOLD: u32 = 3;
const BACKOFF_BASE_MS: u64 = 2_000;
const BACKOFF_MAX_MS: u64 = 60_000;

impl ConnectionManager {
    /// Create a new manager with the given policy limits.
    pub fn new(max_reconnect_cycles: u32, reconnect_deadline_ms: u64) -> Self {
        Self {
            state: ConnectionState::Disconnected,
            consecutive_heartbeat_misses: 0,
            reconnect_cycles: 0,
            max_reconnect_cycles,
            reconnect_deadline_ms,
            backoff_delay: Duration::from_millis(BACKOFF_BASE_MS),
            reconnect_deadline: None,
        }
    }

    /// Called when a WebSocket connection is fully established.
    ///
    /// Resets all reconnect counters and clears the deadline.
    pub fn on_connected(&mut self) {
        self.state = ConnectionState::Connected;
        self.consecutive_heartbeat_misses = 0;
        self.reconnect_cycles = 0;
        self.backoff_delay = Duration::from_millis(BACKOFF_BASE_MS);
        self.reconnect_deadline = None;
    }

    /// Called whenever any message is received over the WebSocket.
    ///
    /// Resets the heartbeat miss counter, confirming the connection is alive.
    pub fn on_message_received(&mut self) {
        self.consecutive_heartbeat_misses = 0;
    }

    /// Called when a heartbeat interval passes with no message.
    ///
    /// Returns `true` if the miss count reaches the threshold, signalling that
    /// the connection should be treated as dead and a reconnect triggered.
    #[allow(dead_code)] // called by heartbeat task wiring in a later unit
    pub fn on_heartbeat_miss(&mut self) -> bool {
        self.consecutive_heartbeat_misses += 1;
        tracing::debug!(
            misses = self.consecutive_heartbeat_misses,
            threshold = HEARTBEAT_MISS_THRESHOLD,
            "DingTalk heartbeat miss"
        );
        self.consecutive_heartbeat_misses >= HEARTBEAT_MISS_THRESHOLD
    }

    /// Decide whether a reconnect attempt should proceed.
    ///
    /// On the **first** call after a disconnect this sets the reconnect deadline.
    /// Returns `false` (and transitions to `Failed`) when either:
    /// - `reconnect_cycles >= max_reconnect_cycles`, or
    /// - the deadline has been exceeded.
    ///
    /// On success increments `reconnect_cycles` and sets state to `Reconnecting`.
    pub fn should_reconnect(&mut self) -> bool {
        // Set the deadline on the very first reconnect attempt.
        if self.reconnect_deadline.is_none() {
            self.reconnect_deadline =
                Some(Instant::now() + Duration::from_millis(self.reconnect_deadline_ms));
            tracing::debug!(
                deadline_ms = self.reconnect_deadline_ms,
                "DingTalk: reconnect deadline set"
            );
        }

        // Check cycle limit.
        if self.reconnect_cycles >= self.max_reconnect_cycles {
            tracing::debug!(
                cycles = self.reconnect_cycles,
                max = self.max_reconnect_cycles,
                "DingTalk: reconnect cycle limit reached"
            );
            self.state = ConnectionState::Failed;
            return false;
        }

        // Check wall-clock deadline.
        if let Some(deadline) = self.reconnect_deadline {
            if Instant::now() > deadline {
                tracing::debug!("DingTalk: reconnect deadline exceeded");
                self.state = ConnectionState::Failed;
                return false;
            }
        }

        self.reconnect_cycles += 1;
        self.state = ConnectionState::Reconnecting;
        true
    }

    /// Compute the next backoff duration and advance the internal counter.
    ///
    /// Uses exponential backoff capped at `BACKOFF_MAX_MS`, with ±30% jitter
    /// derived from the current system time's sub-second nanoseconds — no
    /// external crate required.
    pub fn next_backoff(&mut self) -> Duration {
        let delay_ms = self.backoff_delay.as_millis() as u64;

        // Jitter: ±30% of delay_ms, derived from subsecond nanos (cheap hash).
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as f64;
        // Maps nanos to [-0.3, +0.3] × delay_ms
        let jitter_ms = (nanos / u32::MAX as f64 - 0.5) * 0.6 * delay_ms as f64;
        let jittered_ms = (delay_ms as f64 + jitter_ms).max(0.0) as u64;

        let result = Duration::from_millis(jittered_ms);

        // Advance backoff for next call: double, cap at max.
        self.backoff_delay =
            Duration::from_millis((delay_ms * 2).min(BACKOFF_MAX_MS));

        tracing::debug!(
            delay_ms = jittered_ms,
            next_delay_ms = self.backoff_delay.as_millis(),
            "DingTalk: next backoff computed"
        );

        result
    }

    /// Called when a reconnect attempt itself fails (registration or WS connect error).
    ///
    /// Transitions to `Failed` if limits are exceeded; otherwise stays in `Reconnecting`.
    pub fn on_reconnect_failed(&mut self) {
        let cycles_exhausted = self.reconnect_cycles >= self.max_reconnect_cycles;
        let deadline_exceeded = self
            .reconnect_deadline
            .map(|d| Instant::now() > d)
            .unwrap_or(false);

        if cycles_exhausted || deadline_exceeded {
            tracing::debug!(
                cycles = self.reconnect_cycles,
                "DingTalk: reconnect failed, transitioning to Failed"
            );
            self.state = ConnectionState::Failed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> ConnectionManager {
        ConnectionManager::new(5, 60_000)
    }

    #[test]
    fn initial_state_is_disconnected() {
        let cm = manager();
        assert_eq!(cm.state, ConnectionState::Disconnected);
        assert_eq!(cm.consecutive_heartbeat_misses, 0);
        assert_eq!(cm.reconnect_cycles, 0);
    }

    // ── Backoff ──────────────────────────────────────────────────────────────

    #[test]
    fn backoff_increases_exponentially() {
        let mut cm = manager();
        // Force-advance without jitter by inspecting the stored backoff_delay
        // (actual returned values carry jitter; test the internal progression).
        let _d1 = cm.next_backoff();
        // After first call delay doubles: 2 s → 4 s stored
        assert_eq!(cm.backoff_delay, Duration::from_millis(4_000));

        let _d2 = cm.next_backoff();
        // 4 s → 8 s
        assert_eq!(cm.backoff_delay, Duration::from_millis(8_000));

        let _d3 = cm.next_backoff();
        // 8 s → 16 s
        assert_eq!(cm.backoff_delay, Duration::from_millis(16_000));
    }

    #[test]
    fn backoff_caps_at_max() {
        let mut cm = manager();
        // Wind delay up to max
        cm.backoff_delay = Duration::from_millis(32_000);
        let _d = cm.next_backoff();
        // 32 s → 60 s (capped)
        assert_eq!(cm.backoff_delay, Duration::from_millis(BACKOFF_MAX_MS));

        let _d2 = cm.next_backoff();
        // Still capped
        assert_eq!(cm.backoff_delay, Duration::from_millis(BACKOFF_MAX_MS));
    }

    // ── Heartbeat ────────────────────────────────────────────────────────────

    #[test]
    fn heartbeat_miss_triggers_at_threshold() {
        let mut cm = manager();
        assert!(!cm.on_heartbeat_miss()); // miss 1
        assert!(!cm.on_heartbeat_miss()); // miss 2
        assert!(cm.on_heartbeat_miss());  // miss 3 → trigger
    }

    #[test]
    fn message_received_resets_heartbeat_miss_count() {
        let mut cm = manager();
        cm.on_heartbeat_miss();
        cm.on_heartbeat_miss();
        cm.on_message_received();
        assert_eq!(cm.consecutive_heartbeat_misses, 0);
        // After reset, need another 3 misses
        assert!(!cm.on_heartbeat_miss());
        assert!(!cm.on_heartbeat_miss());
        assert!(cm.on_heartbeat_miss());
    }

    // ── Reconnect cycles ─────────────────────────────────────────────────────

    #[test]
    fn reconnect_cycles_respected() {
        let mut cm = ConnectionManager::new(3, 3_600_000); // 1 h deadline
        assert!(cm.should_reconnect()); // cycle 1
        assert!(cm.should_reconnect()); // cycle 2
        assert!(cm.should_reconnect()); // cycle 3
        // Now at limit
        assert!(!cm.should_reconnect());
        assert_eq!(cm.state, ConnectionState::Failed);
    }

    #[test]
    fn on_connected_resets_cycles_and_deadline() {
        let mut cm = ConnectionManager::new(3, 3_600_000);
        cm.should_reconnect();
        cm.should_reconnect();
        cm.on_connected();
        assert_eq!(cm.state, ConnectionState::Connected);
        assert_eq!(cm.reconnect_cycles, 0);
        assert!(cm.reconnect_deadline.is_none());

        // Should be able to reconnect again fresh
        assert!(cm.should_reconnect());
    }

    // ── Deadline ─────────────────────────────────────────────────────────────

    #[test]
    fn deadline_exceeded_stops_reconnection() {
        // Create manager with a deadline already in the past
        let mut cm = ConnectionManager::new(100, 1); // 1 ms deadline
        // Set deadline explicitly to a past instant
        cm.reconnect_deadline = Some(Instant::now() - Duration::from_secs(1));
        // should_reconnect should detect expired deadline
        assert!(!cm.should_reconnect());
        assert_eq!(cm.state, ConnectionState::Failed);
    }

    #[test]
    fn deadline_set_on_first_reconnect() {
        let mut cm = manager();
        assert!(cm.reconnect_deadline.is_none());
        cm.should_reconnect();
        assert!(cm.reconnect_deadline.is_some());
    }
}
