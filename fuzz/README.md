# IronClaw Fuzz Targets

Fuzz testing for IronClaw code paths that depend on the full crate, using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer).

> **Note:** Safety-specific fuzz targets (sanitizer, validator, leak detector, credential detect) have moved to `crates/ironclaw_safety/fuzz/`. See that directory's README for details.

## Targets

| Target | What it exercises |
|--------|-------------------|
| `fuzz_tool_params` | Tool parameter and schema JSON validation |

## Setup

```bash
cargo install cargo-fuzz
rustup install nightly
```

## Running

```bash
# Run a specific target (runs until stopped or crash found)
cargo +nightly fuzz run fuzz_tool_params

# Run with a time limit (5 minutes)
cargo +nightly fuzz run fuzz_tool_params -- -max_total_time=300
```

## Adding New Targets

1. Create `fuzz/fuzz_targets/fuzz_<name>.rs` following the existing pattern
2. Add a `[[bin]]` entry in `fuzz/Cargo.toml`
3. Create `fuzz/corpus/fuzz_<name>/` for seed inputs
4. Exercise real IronClaw code paths, not just generic serde

For safety-only targets, add them to `crates/ironclaw_safety/fuzz/` instead.
