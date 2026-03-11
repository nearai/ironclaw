# IronClaw Fuzz Targets

Fuzz testing for security-critical input parsing paths using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer).

## Targets

| Target | What it exercises |
|--------|-------------------|
| `fuzz_safety_sanitizer` | Prompt injection pattern detection (Aho-Corasick + regex) |
| `fuzz_safety_validator` | Input validation (length, encoding, forbidden patterns) |
| `fuzz_leak_detector` | Secret leak detection (API keys, tokens, credentials) |
| `fuzz_tool_params` | Tool parameter and schema JSON validation |
| `fuzz_config_env` | SafetyLayer end-to-end (sanitize, validate, policy check) |

## Setup

```bash
cargo install cargo-fuzz
rustup install nightly
```

## Running

```bash
# Run a specific target (runs until stopped or crash found)
cargo +nightly fuzz run fuzz_safety_sanitizer

# Run with a time limit (5 minutes)
cargo +nightly fuzz run fuzz_leak_detector -- -max_total_time=300

# Run all targets for 60 seconds each
for target in fuzz_safety_sanitizer fuzz_safety_validator fuzz_leak_detector fuzz_tool_params fuzz_config_env; do
    echo "==> $target"
    cargo +nightly fuzz run "$target" -- -max_total_time=60
done
```

## Adding New Targets

1. Create `fuzz/fuzz_targets/fuzz_<name>.rs` following the existing pattern
2. Add a `[[bin]]` entry in `fuzz/Cargo.toml`
3. Create `fuzz/corpus/fuzz_<name>/` for seed inputs
4. Exercise real IronClaw code paths, not just generic serde
