# ironclaw_safety Fuzz Targets

Fuzz testing for the `ironclaw_safety` crate using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer).

## Targets

| Target | What it exercises |
|--------|-------------------|
| `fuzz_safety_sanitizer` | Prompt injection pattern detection (Aho-Corasick + regex) |
| `fuzz_safety_validator` | Input validation (length, encoding, forbidden patterns) |
| `fuzz_leak_detector` | Secret leak detection (API keys, tokens, credentials) |
| `fuzz_credential_detect` | HTTP request credential detection |
| `fuzz_config_env` | SafetyLayer end-to-end (sanitize, validate, policy check) |

## Setup

```bash
cargo install cargo-fuzz
rustup install nightly
```

## Running

```bash
cd crates/ironclaw_safety

# Run a specific target (runs until stopped or crash found)
cargo +nightly fuzz run fuzz_safety_sanitizer

# Run with a time limit (5 minutes)
cargo +nightly fuzz run fuzz_leak_detector -- -max_total_time=300

# Run all targets for 60 seconds each
for target in fuzz_safety_sanitizer fuzz_safety_validator fuzz_leak_detector fuzz_credential_detect fuzz_config_env; do
    echo "==> $target"
    cargo +nightly fuzz run "$target" -- -max_total_time=60
done
```

## Seed Corpus

Each target has a seed corpus in `corpus/<target>/` with representative inputs covering the major pattern families. The fuzzer uses these as starting points for mutation.
