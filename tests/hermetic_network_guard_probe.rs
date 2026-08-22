//! Negative-control proof that cargo-nextest's per-test child processes are
//! still wrapped by the hermetic network guard
//! (`CARGO_TARGET_<TRIPLE>_RUNNER` -> scripts/ci/hermetic-network-runner.sh).
//! `scripts/ci/test-hermetic-test-process.sh`'s existing checks prove the
//! guard blocks a directly-launched compiled probe; they never invoke
//! `cargo`/`cargo nextest`, so they prove nothing about whether nextest's
//! test-process spawn path inherits the same env. This test does: run it
//! once under `cargo nextest` inside the hermetic wrapper with the guard
//! ACTIVE (must PASS -- connection refused), and once with
//! `IRONCLAW_HERMETIC_SABOTAGE=network` (guard DISABLED, must FAIL --
//! connection not refused within the timeout). Only the differential
//! between those two runs is the actual proof; see
//! scripts/ci/test-hermetic-test-process.sh for the orchestration.
//!
//! Deliberately NOT `reborn_`-prefixed (precedent: trace_format.rs,
//! trace_llm_tests.rs) so it is invisible to the root-partition, group,
//! and planner `tests/reborn_*` globs -- it must never run as part of an
//! ordinary PR's test selection, only via this file's explicit invocation.
#[test]
#[ignore = "network-guard negative control; run explicitly via cargo \
            nextest inside scripts/ci/run-hermetic-test-process.sh, see \
            scripts/ci/test-hermetic-test-process.sh"]
fn nextest_child_process_is_network_guarded() {
    use std::io::ErrorKind;
    use std::net::TcpStream;
    use std::time::Duration;

    // TEST-NET-1 (RFC 5737): non-routable, never answers. Un-guarded, a
    // real connect() to it times out or reports unreachable; guarded, the
    // interposer's connect() wrapper sets errno=EPERM immediately
    // (scripts/ci/hermetic-network-guard.c) -- so PermissionDenied is a
    // guard-specific signature, not "any connection error".
    let outcome = TcpStream::connect_timeout(
        &"192.0.2.1:80".parse().expect("literal address parses"),
        Duration::from_secs(5),
    );
    match outcome {
        Ok(_) => panic!(
            "connected to a non-loopback TEST-NET-1 address; the network \
             guard did not intercept this nextest-run test process"
        ),
        Err(err) => {
            assert_eq!(
                err.kind(),
                ErrorKind::PermissionDenied,
                "connect() failed but not via EPERM (the hermetic network \
                 guard's interposer sets EPERM on a blocked connect; a \
                 plain timeout or unreachable-host error means this \
                 process was NOT guarded): {err}"
            );
        }
    }
}
