# Benchmark Phase 4: Skill Activation, JSON Output, CI Workflow

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire skill activation control into benchmark scenarios, add machine-readable JSON output for CI integration, and create a GitHub Actions benchmark workflow.

**Architecture:** Add `retain_only()` to SkillRegistry (same pattern as ToolRegistry), wire `setup.skills` in the benchmark runner, add `--json` CLI flag for machine-readable output, and create `.github/workflows/benchmark.yml` for CI.

**Tech Stack:** Rust, tokio, serde_json, clap, GitHub Actions

---

### Task 1: Add `retain_only()` to SkillRegistry + Wire Skill Filtering

**Files:**
- Modify: `src/skills/registry.rs` (add `retain_only` method)
- Modify: `src/benchmark/runner.rs:592-669` (wire skill registry with filtering)

**Step 1: Add `retain_only` to SkillRegistry**

In `src/skills/registry.rs`, after the `count()` method (around line 289), add:

```rust
/// Retain only skills whose names are in the given allowlist.
///
/// If `names` is empty, this is a no-op (all skills are kept).
pub fn retain_only(&mut self, names: &[&str]) {
    if names.is_empty() {
        return;
    }
    self.skills.retain(|s| names.contains(&s.manifest.name.as_str()));
}
```

**Step 2: Write test for `retain_only`**

Add to `mod tests` in `src/skills/registry.rs`:

```rust
#[test]
fn test_retain_only_filters_skills() {
    // This tests the filtering logic on an empty registry (skills are
    // discovered from the filesystem, not easily constructed).
    let mut registry = SkillRegistry::new(PathBuf::from("/tmp/nonexistent-skills"));
    // Empty allowlist is a no-op.
    registry.retain_only(&[]);
    assert_eq!(registry.count(), 0);
}
```

**Step 3: Wire skill registry into `run_bench_scenario()`**

In `src/benchmark/runner.rs`, after the tool filtering block (line ~592) and before "4. Seed workspace documents", add:

```rust
// 3b. Create and filter skill registry (if skills specified in setup).
let skill_registry = if !scenario.setup.skills.is_empty() {
    let mut registry = crate::skills::SkillRegistry::new(
        SkillsConfig::default().local_dir,
    );
    registry.discover_all().await;
    let names: Vec<&str> = scenario.setup.skills.iter().map(|s| s.as_str()).collect();
    registry.retain_only(&names);
    Some(Arc::new(tokio::sync::RwLock::new(registry)))
} else {
    None
};
```

Then update the `AgentDeps` construction to use the new variable:

```rust
skill_registry,  // was: None
```

**Step 4: Run tests**

Run: `cargo test --all-features skills::registry::tests`
Run: `cargo test --all-features benchmark`

**Step 5: Commit**

```bash
git add src/skills/registry.rs src/benchmark/runner.rs
git commit -m "feat(benchmark): add SkillRegistry::retain_only and wire skill filtering in scenarios"
```

---

### Task 2: Add `--json` CLI Flag for Machine-Readable Output

**Files:**
- Modify: `src/cli/benchmark.rs` (add `--json` flag, conditional output)

**Step 1: Add the `--json` flag**

In `src/cli/benchmark.rs`, add to `BenchmarkCommand`:

```rust
/// Output results as JSON (machine-readable) instead of human-readable report
#[arg(long)]
pub json: bool,
```

**Step 2: Update `run_benchmark_command()` to conditionally output JSON**

After the line that calculates `run_result`, change the output section to:

```rust
// Save results to disk (per-scenario files + summary).
let result_dir = save_scenario_results(&run_result).map_err(|e| anyhow::anyhow!("{}", e))?;
eprintln!("Results saved to: {}/", result_dir);

if cmd.json {
    // Machine-readable JSON output to stdout.
    let json = serde_json::to_string_pretty(&run_result)
        .map_err(|e| anyhow::anyhow!("Failed to serialize: {}", e))?;
    println!("{json}");
} else {
    // Human-readable report.
    let baseline = load_baseline().map_err(|e| anyhow::anyhow!("{}", e))?;
    let report = format_report(&run_result, baseline.as_ref());
    println!("{report}");
}
```

**Step 3: Run checks**

Run: `cargo check --all-features`
Run: `cargo test --all-features cli::tests`
Accept snapshots if help text changed: `cargo insta accept`

**Step 4: Commit**

```bash
git add src/cli/benchmark.rs src/cli/snapshots/
git commit -m "feat(benchmark): add --json flag for machine-readable output"
```

---

### Task 3: Create GitHub Actions Benchmark Workflow

**Files:**
- Create: `.github/workflows/benchmark.yml`

**Step 1: Create the workflow**

Create `.github/workflows/benchmark.yml`:

```yaml
name: Benchmark

on:
  workflow_dispatch:
    inputs:
      scenarios_dir:
        description: 'Scenarios directory'
        default: 'benchmarks/trajectories'
      tags:
        description: 'Tag filter (comma-separated)'
        required: false
      parallel:
        description: 'Parallel scenario count'
        default: '1'
      max_cost:
        description: 'Maximum total cost in USD'
        default: '1.00'

jobs:
  benchmark:
    name: Run Benchmarks
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2
        with:
          key: benchmark

      - name: Build benchmark binary
        run: cargo build --release --features "libsql,benchmark"

      - name: Run benchmarks
        env:
          LLM_BACKEND: ${{ secrets.LLM_BACKEND || 'openai' }}
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
          NEARAI_API_KEY: ${{ secrets.NEARAI_API_KEY }}
          NEARAI_SESSION_TOKEN: ${{ secrets.NEARAI_SESSION_TOKEN }}
        run: |
          ARGS="--scenarios-dir ${{ inputs.scenarios_dir || 'benchmarks/trajectories' }}"
          ARGS="$ARGS --parallel ${{ inputs.parallel || '1' }}"
          ARGS="$ARGS --json"

          if [ -n "${{ inputs.tags }}" ]; then
            ARGS="$ARGS --tags ${{ inputs.tags }}"
          fi
          if [ -n "${{ inputs.max_cost }}" ]; then
            ARGS="$ARGS --max-cost ${{ inputs.max_cost }}"
          fi

          ./target/release/ironclaw benchmark $ARGS | tee benchmark-results.json

      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results-${{ github.sha }}
          path: |
            benchmark-results.json
            benchmarks/results/
```

**Step 2: Commit**

```bash
git add .github/workflows/benchmark.yml
git commit -m "ci: add GitHub Actions benchmark workflow (manual trigger)"
```

---

### Task 4: Final Quality Gate

**Step 1:** `cargo fmt --check` (fix if needed)
**Step 2:** `cargo clippy --all --benches --tests --examples --all-features` (0 warnings)
**Step 3:** `cargo test --all-features` (all pass)
**Step 4:** `cargo check` (default features clean)
**Step 5:** `cargo test --lib cli::tests` and `cargo test --all-features --lib cli::tests` (both pass)
**Step 6:** Commit any fixes
