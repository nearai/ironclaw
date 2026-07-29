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
dependencies. Cargo and package installation are build-tooling preconditions;
the non-loopback guard is applied to executed test binaries and Python/Node test
processes, not dependency download tooling or remote compiler caches.

Every stage runs through `scripts/ci/run-hermetic-test-process.sh`. That boundary:

- removes real provider credentials and ambient provider/LLM behavior;
- gives the process a fresh temporary home, IronClaw base/reborn homes,
  workspace, XDG directories, and temporary directory;
- pins timezone, locale, Python hash seed, the test random seed, and the
  test-clock epoch;
- suppresses the OS keychain and long provider retries; and
- records and fails on any non-loopback IP connection while permitting Unix
  sockets and deliberate IPv4/IPv6 localhost fakes.

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
`clock-seed`, or `network`.
