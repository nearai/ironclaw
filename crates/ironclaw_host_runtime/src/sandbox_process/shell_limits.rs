//! Model-adjustable clamp bounds for `builtin.shell` timeout and captured
//! output size.
//!
//! The model may request a `timeout` and `output_limit` per call (see
//! `first_party_tools::schemas` and `first_party_tools::shell_core`); this
//! module owns the operator ceilings both values are clamped to before they
//! reach the sandbox transport, plus the defaults used when unset.

use std::time::Duration;

/// Default shell command timeout when the model omits `timeout`.
pub const SHELL_TIMEOUT_DEFAULT_SECS: u64 = 120;
/// Operator ceiling for `timeout`. Requests above this are clamped down,
/// never rejected.
pub const SHELL_TIMEOUT_MAX_SECS: u64 = 600;
/// Floor for `timeout`; zero-second timeouts are rejected upstream by
/// `shell_core::parse_timeout`, but the clamp still enforces the floor for
/// any other caller of `clamp_shell_timeout_secs`.
pub const SHELL_TIMEOUT_MIN_SECS: u64 = 1;

/// Default captured-output cap (stdout+stderr) when the model omits
/// `output_limit`.
pub const SHELL_OUTPUT_LIMIT_DEFAULT_BYTES: u64 = 64 * 1024;
/// Operator ceiling for `output_limit`. Requests above this are clamped
/// down, never rejected.
pub const SHELL_OUTPUT_LIMIT_MAX_BYTES: u64 = 1024 * 1024;
/// Floor for `output_limit`; requests below this are clamped up.
pub const SHELL_OUTPUT_LIMIT_MIN_BYTES: u64 = 1024;

/// Clamp a model-requested shell timeout (seconds) to
/// `[SHELL_TIMEOUT_MIN_SECS, SHELL_TIMEOUT_MAX_SECS]`, defaulting to
/// `SHELL_TIMEOUT_DEFAULT_SECS` when unset.
pub fn clamp_shell_timeout_secs(requested: Option<u64>) -> Duration {
    let secs = requested
        .unwrap_or(SHELL_TIMEOUT_DEFAULT_SECS)
        .clamp(SHELL_TIMEOUT_MIN_SECS, SHELL_TIMEOUT_MAX_SECS);
    Duration::from_secs(secs)
}

/// Clamp a model-requested output cap (bytes) to
/// `[SHELL_OUTPUT_LIMIT_MIN_BYTES, SHELL_OUTPUT_LIMIT_MAX_BYTES]`,
/// defaulting to `SHELL_OUTPUT_LIMIT_DEFAULT_BYTES` when unset.
pub fn clamp_shell_output_limit_bytes(requested: Option<u64>) -> usize {
    let bytes = requested
        .unwrap_or(SHELL_OUTPUT_LIMIT_DEFAULT_BYTES)
        .clamp(SHELL_OUTPUT_LIMIT_MIN_BYTES, SHELL_OUTPUT_LIMIT_MAX_BYTES);
    bytes as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_unset_defaults_to_120() {
        assert_eq!(
            clamp_shell_timeout_secs(None),
            Duration::from_secs(SHELL_TIMEOUT_DEFAULT_SECS)
        );
        assert_eq!(clamp_shell_timeout_secs(None), Duration::from_secs(120));
    }

    #[test]
    fn timeout_within_range_is_honored() {
        assert_eq!(clamp_shell_timeout_secs(Some(45)), Duration::from_secs(45));
    }

    #[test]
    fn timeout_over_cap_is_clamped_to_600() {
        assert_eq!(
            clamp_shell_timeout_secs(Some(6_000)),
            Duration::from_secs(SHELL_TIMEOUT_MAX_SECS)
        );
        assert_eq!(
            clamp_shell_timeout_secs(Some(601)),
            Duration::from_secs(600)
        );
    }

    #[test]
    fn timeout_below_floor_is_clamped_up() {
        // parse_timeout rejects 0 before this is reached in production, but
        // the clamp itself must still hold the floor for any other caller.
        assert_eq!(
            clamp_shell_timeout_secs(Some(0)),
            Duration::from_secs(SHELL_TIMEOUT_MIN_SECS)
        );
    }

    #[test]
    fn output_limit_unset_defaults_to_64kib() {
        assert_eq!(
            clamp_shell_output_limit_bytes(None),
            SHELL_OUTPUT_LIMIT_DEFAULT_BYTES as usize
        );
        assert_eq!(clamp_shell_output_limit_bytes(None), 65536);
    }

    #[test]
    fn output_limit_over_cap_is_clamped_to_1mib() {
        assert_eq!(
            clamp_shell_output_limit_bytes(Some(10 * 1024 * 1024)),
            SHELL_OUTPUT_LIMIT_MAX_BYTES as usize
        );
        assert_eq!(clamp_shell_output_limit_bytes(Some(1_048_577)), 1_048_576);
    }

    #[test]
    fn output_limit_below_floor_is_clamped_up() {
        assert_eq!(
            clamp_shell_output_limit_bytes(Some(10)),
            SHELL_OUTPUT_LIMIT_MIN_BYTES as usize
        );
    }

    #[test]
    fn output_limit_within_range_is_honored() {
        assert_eq!(clamp_shell_output_limit_bytes(Some(200_000)), 200_000);
    }
}
