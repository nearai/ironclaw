# Hermetic deterministic suite

The canonical complete local command for the merge-gating deterministic Reborn
test strategy is:

```bash
scripts/ci/run-hermetic-deterministic-suite.sh all
```

It composes the same checked-in discovery and entrypoints used by
`reborn-tests.yml` and `reborn-e2e.yml`: the production Reborn package closure
and allowlist with CI feature flags, root test partitions, shared-state group
suites, the complete registered Reborn integration tier, recorded-QA fixture
checks and replay, the Rust Reborn E2E gate, WebUI unit tests, standalone binary
build, and the merge-gating Python E2E/provider lanes. It deliberately excludes
live canaries, nightly Playwright shards, stress, release, and platform
compile-only jobs.

Install the CI-pinned toolchains first. In particular, build the Emulate
revision pinned in `.github/workflows/reborn-e2e.yml`, set
`IRONCLAW_EMULATE_CLI` to its built CLI, install the E2E Python package and
Chromium, install frontend dependencies, and prefetch locked Cargo
dependencies. Package installation is a build-tooling precondition. The
canonical script performs `cargo fetch --locked` before entering the guarded
process and then forces Cargo offline. Every command and descendant inside the
suite is guarded, so remote compiler wrappers are disabled rather than treated
as a network exception.

Every stage runs through `scripts/ci/run-hermetic-test-process.sh`. That boundary:

- removes real provider credentials and ambient provider/LLM behavior;
- gives the process a fresh temporary home, IronClaw base/reborn homes,
  workspace, XDG directories, and temporary directory;
- pins timezone, locale, and Python hash iteration order;
- suppresses the OS keychain and long provider retries; and
- denies non-loopback IP connections while permitting Unix sockets and
  deliberate IPv4/IPv6 localhost fakes. The syscall interposer records denied
  attempts on Linux and ordinary macOS binaries; macOS additionally uses the
  process sandbox so SIP-protected launchers remain fail-closed.

Rust wall-clock and random behavior is not overridden through process-global
environment variables. Time-sensitive domain tests use their owning typed
clock seams (for example `FakeClock`/`FixedClock`), configured Reborn jitter
defaults to zero in deterministic tests, and cryptographic or identity
randomness remains OS-backed.

CI may invoke a narrower stage or use `command` to retain its matrix sharding:

```bash
scripts/ci/run-hermetic-deterministic-suite.sh rust-e2e substrates
scripts/ci/run-hermetic-deterministic-suite.sh command cargo test -p ironclaw_network
```

The guard is mutation-tested by:

```bash
scripts/ci/test-hermetic-test-process.sh
```

Maintainers can locally sabotage one control and confirm the self-test turns
red with `IRONCLAW_HERMETIC_SELF_TEST_SABOTAGE` set to `env`, `temp`,
`python-seed`, or `network`.
