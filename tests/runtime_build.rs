//! Tests for `ironclaw::bootstrap::build_runtime`.
//!
//! The runtime builder is invoked at binary startup with the parsed
//! `TOKIO_WORKER_THREADS` env var. The critical regression to guard
//! against is slipping back to a `current_thread` runtime in slim mode,
//! which would silently break any call to `tokio::task::block_in_place`
//! in channel or provider code.

use ironclaw::bootstrap::build_runtime;

/// Slim mode (`TOKIO_WORKER_THREADS=1`) must produce a `multi_thread`
/// runtime, not `current_thread`, so `block_in_place` works without
/// panicking.
#[test]
fn slim_mode_runtime_supports_block_in_place() {
    let runtime = build_runtime(Some(1)).expect("runtime should build");
    runtime.block_on(async {
        // `block_in_place` panics on a `current_thread` runtime. If the
        // runtime builder ever regresses to `new_current_thread()` in
        // slim mode, this call panics and the test fails.
        tokio::task::block_in_place(|| {
            // Minimal sync work — the call itself is the assertion.
        });
    });
}

/// Sanity check: the default runtime (no override) also supports
/// `block_in_place`.
#[test]
fn default_runtime_supports_block_in_place() {
    let runtime = build_runtime(None).expect("runtime should build");
    runtime.block_on(async {
        tokio::task::block_in_place(|| {});
    });
}
