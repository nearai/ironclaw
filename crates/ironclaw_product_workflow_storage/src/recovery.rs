//! Shared recovery-lease constant for the durable idempotency ledger.
//!
//! Both `LibSqlProductIdempotencyLedger` and `PostgresProductIdempotencyLedger`
//! reclaim non-terminal `Received`/`Dispatched` rows whose `received_at` is
//! older than this TTL. The `IdempotencyLedger` trait contract requires this
//! reclaim path so a crashed or timed-out workflow does not permanently
//! wedge subsequent retries for the same fingerprint (Henry's PR #3590
//! review item #1).
//!
//! The default is deliberately generous relative to
//! `NativeProductAdapterRunnerConfig::workflow_timeout` (currently 15s) so
//! an honest slow dispatch doesn't get clobbered, but short enough that a
//! stuck row recovers within a few Telegram retry waves.

use std::time::Duration;

pub const DEFAULT_RECOVERY_LEASE: Duration = Duration::from_secs(300);
