//! Small shared constants and time helpers used across the executor
//! submodules.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) const INPUT_POLL_LIMIT: usize = 16;
pub(super) const NO_PROGRESS_WINDOW: usize = 5;
pub(super) const NO_PROGRESS_THRESHOLD: usize = 3;
/// Defense-in-depth cap on the inner retry loop. The default
/// `RecoveryStrategy` returns `Abort` once its own per-class budget is
/// exhausted; this constant only guards against a custom strategy that
/// indefinitely returns `Retry`.
pub(super) const MAX_RETRIES_PER_CALL: u32 = 8;

/// Wall-clock distance from `start` to `Instant::now()`. Exists so the
/// executor's tick prologue stays readable and so tests on a paused tokio
/// clock can validate the wall-clock budget path.
pub(super) fn elapsed_since(start: tokio::time::Instant) -> Duration {
    tokio::time::Instant::now().saturating_duration_since(start)
}

/// Current wall clock as milliseconds since the Unix epoch.
///
/// Captures and compares the persisted
/// `LoopExecutionState::started_at_unix_ms` anchor so a resumed run
/// retains its time budget across process restart. A clock reading
/// before `UNIX_EPOCH` saturates to `0`; the wall-clock comparator then
/// treats elapsed time as `0`, which is conservative — it never
/// spuriously trips the cap.
pub(super) fn system_time_now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| u64::try_from(delta.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Whether the wall-clock budget has been exceeded.
///
/// Combines the in-process `tokio::time::Instant` cap (so test code with
/// `start_paused = true` still works) with the persisted `SystemTime`
/// anchor (so a run that resumes after process restart immediately
/// observes its already-elapsed budget). The cap fires if EITHER source
/// agrees the limit has been reached.
///
/// Clock-skew note: if the OS clock jumps backward, `SystemTime` elapsed
/// underflows to `Duration::ZERO`, and the in-process `Instant` cap takes
/// over for the remainder of this `execute()` call. Wall-clock budgets
/// are a defense-in-depth limiter, not a correctness invariant.
pub(super) fn wall_clock_limit_exceeded(
    in_process_start: tokio::time::Instant,
    persisted_start_unix_ms: Option<u64>,
    limit: Duration,
) -> bool {
    if elapsed_since(in_process_start) >= limit {
        return true;
    }
    let Some(started_at_unix_ms) = persisted_start_unix_ms else {
        return false;
    };
    let now_ms = system_time_now_unix_ms();
    let elapsed_ms = now_ms.saturating_sub(started_at_unix_ms);
    Duration::from_millis(elapsed_ms) >= limit
}
