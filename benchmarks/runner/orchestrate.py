#!/usr/bin/env python3
"""
Benchmark orchestrator.

Runs a single approach with a single agent count. The user runs this
separately for each approach and agent count, then compares results
offline with analysis/compare.py.

Usage:
    python3 -m runner.orchestrate --approach container-docker --agents 5
    python3 -m runner.orchestrate --approach vm-qemu --agents 10
    python3 -m runner.orchestrate --approach container-docker --agents 100 --mode idle

Modes:
    loaded  (default) — stochastic workload with workers
    idle    — no workers spawned, measures pure isolation overhead per agent

Results are saved to results/<approach>-<mode>-<agents>-<timestamp>/.
"""

import argparse
import importlib
import json
import os
import sys
import time
from datetime import datetime
from pathlib import Path

# Add parent directory to path so we can import approaches/runner modules
BENCH_DIR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(BENCH_DIR))

from approaches.base import Approach, BenchmarkConfig
from runner.collect import Collector, read_meminfo, read_vmstat_swap

# Registry of available approaches
APPROACHES = {}


def register_approach(cls: type) -> None:
    """Register an approach class."""
    instance = cls()
    APPROACHES[instance.name] = instance


def discover_approaches() -> None:
    """Auto-discover approach modules in approaches/."""
    approaches_dir = BENCH_DIR / "approaches"
    for py_file in approaches_dir.glob("*.py"):
        if py_file.name.startswith("_") or py_file.name == "base.py":
            continue
        module_name = f"approaches.{py_file.stem}"
        try:
            mod = importlib.import_module(module_name)
            # Find Approach subclasses in the module
            for attr_name in dir(mod):
                attr = getattr(mod, attr_name)
                if (
                    isinstance(attr, type)
                    and issubclass(attr, Approach)
                    and attr is not Approach
                ):
                    register_approach(attr)
        except Exception as e:
            print(f"Warning: could not load approach from {py_file.name}: {e}")


def load_config_env() -> dict:
    """Load config.env file as environment variable defaults."""
    config_path = BENCH_DIR / "config.env"
    env = {}
    if config_path.exists():
        with open(config_path) as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#"):
                    if "=" in line:
                        key, value = line.split("=", 1)
                        env[key.strip()] = value.strip()
    return env


def check_swap_status() -> dict:
    """Check if swap is enabled and return status info."""
    meminfo = read_meminfo()
    swap_total = meminfo.get("SwapTotal", 0)
    swap_free = meminfo.get("SwapFree", 0)
    vmstat = read_vmstat_swap()

    status = {
        "swap_enabled": swap_total > 0,
        "swap_total_kb": swap_total,
        "swap_used_kb": swap_total - swap_free,
        "pswpin_start": vmstat.get("pswpin", 0),
        "pswpout_start": vmstat.get("pswpout", 0),
    }

    if swap_total > 0:
        print(f"WARNING: Swap is enabled ({swap_total // 1024} MiB total, "
              f"{(swap_total - swap_free) // 1024} MiB used).")
        print("  For accurate density measurements, disable swap: sudo swapoff -a")
        print("  Continuing anyway, but results may be distorted by swap activity.\n")

    return status


def check_swap_activity(start_counters: dict) -> dict:
    """Check if swap was used during the benchmark."""
    vmstat = read_vmstat_swap()
    pswpin_delta = vmstat.get("pswpin", 0) - start_counters.get("pswpin_start", 0)
    pswpout_delta = vmstat.get("pswpout", 0) - start_counters.get("pswpout_start", 0)

    result = {
        "pswpin_delta": pswpin_delta,
        "pswpout_delta": pswpout_delta,
        "swap_occurred": (pswpin_delta > 0 or pswpout_delta > 0),
    }

    if result["swap_occurred"]:
        print(f"\nWARNING: Swap activity detected during benchmark!")
        print(f"  Pages swapped in:  {pswpin_delta}")
        print(f"  Pages swapped out: {pswpout_delta}")
        print(f"  Results may not reflect true physical memory usage.")

    return result


def parse_agent_logs(run_dir: Path) -> dict:
    """Parse agent JSONL logs for worker event statistics."""
    stats = {
        "total_workers_spawned": 0,
        "max_concurrent_workers": 0,
    }

    for log_file in sorted(run_dir.glob("agent-*.jsonl")):
        active = 0
        try:
            with open(log_file) as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if event.get("event") == "worker_start":
                        stats["total_workers_spawned"] += 1
                        if "active_workers" in event:
                            active = event["active_workers"]
                            stats["max_concurrent_workers"] = max(
                                stats["max_concurrent_workers"], active
                            )
                    elif event.get("event") == "worker_end":
                        if "active_workers" in event:
                            active = event["active_workers"]
        except Exception:
            pass

    return stats


def compute_percentiles(values: list, percentiles: list = None) -> dict:
    """Compute percentiles from a list of values."""
    if not values:
        return {}
    if percentiles is None:
        percentiles = [50, 95, 99]
    sorted_vals = sorted(values)
    n = len(sorted_vals)
    result = {}
    for p in percentiles:
        idx = int(p / 100.0 * (n - 1))
        result[f"p{p}"] = sorted_vals[idx]
    return result


def compute_drift_slope(timestamps: list, values: list) -> float:
    """Compute linear slope (KiB/s) via simple linear regression."""
    n = len(timestamps)
    if n < 2:
        return 0.0
    mean_t = sum(timestamps) / n
    mean_v = sum(values) / n
    numerator = sum((t - mean_t) * (v - mean_v) for t, v in zip(timestamps, values))
    denominator = sum((t - mean_t) ** 2 for t in timestamps)
    if denominator == 0:
        return 0.0
    return numerator / denominator


def run_benchmark(
    approach: Approach,
    num_agents: int,
    config: BenchmarkConfig,
    output_dir: Path,
    sample_interval_ms: int,
    mode: str,
) -> Path:
    """Execute a single benchmark run."""
    timestamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    run_dir = output_dir / f"{approach.name}-{mode}-n{num_agents}-{timestamp}"
    run_dir.mkdir(parents=True, exist_ok=True)

    # Save parameters
    params = {
        "approach": approach.name,
        "mode": mode,
        "num_agents": num_agents,
        "config": {
            "agent_memory_mb": config.agent_memory_mb,
            "agent_baseline_mb": config.agent_baseline_mb,
            "max_concurrent_workers": config.max_concurrent_workers,
            "spawn_interval_mean_s": config.spawn_interval_mean_s,
            "benchmark_duration_s": config.benchmark_duration_s,
            "worker_image": config.worker_image,
            "worker_memory_limit_mb": config.worker_memory_limit_mb,
            "worker_memory_mb": config.worker_memory_mb,
            "worker_duration_min_s": config.worker_duration_min_s,
            "worker_duration_max_s": config.worker_duration_max_s,
        },
        "run_id": config.run_id,
        "rng_seed": config.rng_seed,
        "sample_interval_ms": sample_interval_ms,
        "timestamp": timestamp,
    }
    with open(run_dir / "params.json", "w") as f:
        json.dump(params, f, indent=2)

    print(f"\n{'=' * 60}")
    print(f"Benchmark: {approach.name} with {num_agents} agents (mode={mode})")
    print(f"Run ID: {config.run_id}")
    print(f"RNG seed: {config.rng_seed}")
    print(f"Output: {run_dir}")
    print(f"Duration: {config.benchmark_duration_s}s + 20s baseline/settle")
    print(f"{'=' * 60}\n")

    # Pre-flight checks
    swap_status = check_swap_status()

    # Setup approach
    print("Setting up approach...")
    approach.setup(config)

    # Open timeseries output
    ts_file = open(run_dir / "timeseries.jsonl", "w")

    # Create collector
    collector = Collector(interval_ms=sample_interval_ms)
    agent_ids = []

    try:
        # Phase 1: Baseline (10s, no agents)
        print("Recording baseline memory (10s)...")
        collector_thread = collector.run_in_thread(
            ts_file,
            get_agent_pids=lambda: {},
            get_daemon_pids=approach.get_daemon_pids,
            count_workers=lambda: 0,
        )
        time.sleep(10)

        # Phase 2: Start agents
        print(f"Starting {num_agents} agents...")
        agent_ids = approach.start_agents(num_agents, config)
        print(f"Agents started: {agent_ids}")

        # Update collector to include agent PIDs
        collector.stop()
        collector_thread.join(timeout=5)

        collector = Collector(interval_ms=sample_interval_ms)
        collector_thread = collector.run_in_thread(
            ts_file,
            get_agent_pids=approach.get_agent_pids,
            get_daemon_pids=approach.get_daemon_pids,
            count_workers=approach.count_active_workers,
        )

        # Phase 3: Run for benchmark duration
        duration = config.benchmark_duration_s
        print(f"Collecting data for {duration}s...")
        start = time.monotonic()
        while time.monotonic() - start < duration:
            elapsed = int(time.monotonic() - start)
            workers = approach.count_active_workers()
            workers_str = str(workers) if workers >= 0 else "N/A (inside VMs)"
            sys.stdout.write(
                f"\r  [{elapsed}/{duration}s] Active workers: {workers_str}    "
            )
            sys.stdout.flush()
            time.sleep(5)
        print()

        # Phase 4: Collect agent logs before stopping
        print("Collecting agent logs...")
        approach.collect_agent_logs(agent_ids, run_dir)

        # Phase 5: Teardown
        print("Stopping agents...")
        approach.stop_agents()

        # Phase 6: Settle (10s)
        print("Waiting for memory to settle (10s)...")
        time.sleep(10)

    finally:
        collector.stop()
        collector_thread.join(timeout=5)
        ts_file.close()

    # Post-flight: check swap activity
    swap_result = check_swap_activity(swap_status)

    # Generate summary
    summary = generate_summary(run_dir, swap_result)
    with open(run_dir / "summary.json", "w") as f:
        json.dump(summary, f, indent=2)

    print(f"\nResults saved to {run_dir}")
    print(f"  Steady-state mean: {summary.get('steady_state_mean_mib', 0):.0f} MiB")
    print(f"  Peak: {summary.get('peak_mib', 0):.0f} MiB")
    print(f"  p95: {summary.get('p95_mib', 0):.0f} MiB")
    avg_w = summary.get('avg_workers', 0)
    print(f"  Avg workers: {avg_w:.1f}" if avg_w >= 0 else "  Avg workers: N/A")
    print(f"  Per-agent: {summary.get('per_agent_mean_mib', 0):.0f} MiB")
    drift = summary.get('drift_kb_per_s', 0)
    if abs(drift) > 10:
        print(f"  Drift: {drift:.1f} KiB/s (potential leak)")

    return run_dir


def generate_summary(run_dir: Path, swap_result: dict = None) -> dict:
    """Generate summary statistics from a benchmark run."""
    with open(run_dir / "params.json") as f:
        params = json.load(f)

    samples = []
    with open(run_dir / "timeseries.jsonl") as f:
        for line in f:
            if line.strip():
                samples.append(json.loads(line))

    if not samples:
        return {"error": "No samples collected"}

    num_agents = params["num_agents"]

    # Compute baseline: first 10 seconds (before agents start)
    baseline_samples = [s for s in samples if s["timestamp_s"] < 10]
    baseline_kb = (
        sum(s["host_consumed_kb"] for s in baseline_samples) / len(baseline_samples)
        if baseline_samples
        else 0
    )

    # Steady state: skip first 70s (10s baseline + 60s warmup)
    warmup_end = 70
    steady_samples = [s for s in samples if s["timestamp_s"] >= warmup_end]

    if not steady_samples:
        steady_samples = [s for s in samples if s["timestamp_s"] >= 10]

    consumed_values = [s["host_consumed_kb"] for s in steady_samples]
    worker_values = [s["active_workers"] for s in steady_samples]

    mean_consumed_kb = sum(consumed_values) / len(consumed_values)
    peak_consumed_kb = max(consumed_values)
    mean_workers = sum(worker_values) / len(worker_values)

    # Subtract baseline for net overhead
    net_consumed_kb = mean_consumed_kb - baseline_kb

    # Percentiles (baseline-subtracted)
    net_values = [v - baseline_kb for v in consumed_values]
    pcts = compute_percentiles(net_values, [50, 95, 99])

    # Drift detection (slope of consumed memory during steady state)
    timestamps = [s["timestamp_s"] for s in steady_samples]
    drift = compute_drift_slope(timestamps, consumed_values)

    # Daemon overhead (average during steady state)
    daemon_overhead = {}
    for s in steady_samples:
        for daemon_name, daemon_data in s.get("daemons", {}).items():
            if daemon_name not in daemon_overhead:
                daemon_overhead[daemon_name] = {"rss_sum": 0, "pss_sum": 0, "count": 0}
            daemon_overhead[daemon_name]["rss_sum"] += daemon_data.get("rss_kb", 0)
            daemon_overhead[daemon_name]["pss_sum"] += daemon_data.get("pss_kb", 0)
            daemon_overhead[daemon_name]["count"] += 1

    daemon_summary = {}
    for name, data in daemon_overhead.items():
        if data["count"] > 0:
            daemon_summary[name] = {
                "mean_rss_mib": (data["rss_sum"] / data["count"]) / 1024,
                "mean_pss_mib": (data["pss_sum"] / data["count"]) / 1024,
            }

    # Agent log stats
    agent_log_stats = parse_agent_logs(run_dir)

    summary = {
        "approach": params["approach"],
        "mode": params.get("mode", "loaded"),
        "num_agents": num_agents,
        "run_id": params.get("run_id", "unknown"),
        "rng_seed": params.get("rng_seed"),
        "baseline_mib": baseline_kb / 1024,
        "steady_state_mean_mib": net_consumed_kb / 1024,
        "peak_mib": (peak_consumed_kb - baseline_kb) / 1024,
        "p50_mib": pcts.get("p50", 0) / 1024,
        "p95_mib": pcts.get("p95", 0) / 1024,
        "p99_mib": pcts.get("p99", 0) / 1024,
        "avg_workers": mean_workers,
        "per_agent_mean_mib": (net_consumed_kb / 1024) / max(num_agents, 1),
        "drift_kb_per_s": round(drift, 2),
        "total_samples": len(samples),
        "steady_state_samples": len(steady_samples),
        "daemon_overhead": daemon_summary,
        **agent_log_stats,
    }

    # Swap info
    if swap_result:
        summary["swap"] = swap_result

    return summary


def main():
    parser = argparse.ArgumentParser(
        description="Run a single isolation benchmark."
    )
    parser.add_argument(
        "--approach",
        required=True,
        help="Approach name (e.g., container-docker, vm-qemu)",
    )
    parser.add_argument(
        "--agents",
        type=int,
        default=None,
        help="Number of agents (default: from config.env)",
    )
    parser.add_argument(
        "--mode",
        choices=["loaded", "idle"],
        default="loaded",
        help="Benchmark mode: 'loaded' (default, stochastic workload) "
             "or 'idle' (no workers, measures pure isolation overhead)",
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=None,
        help="Output directory (default: benchmarks/results/)",
    )
    parser.add_argument(
        "--sample-interval-ms",
        type=int,
        default=None,
        help="Collection interval in ms (default: from config.env or 500)",
    )
    args = parser.parse_args()

    # Load config
    config_env = load_config_env()
    # Apply config.env as defaults, then let BenchmarkConfig read from os.environ
    for k, v in config_env.items():
        os.environ.setdefault(k, v)

    config = BenchmarkConfig.from_env()
    num_agents = args.agents or int(config_env.get("DEFAULT_AGENTS", "5"))
    sample_interval = args.sample_interval_ms or int(
        config_env.get("SAMPLE_INTERVAL_MS", "500")
    )
    output_dir = Path(args.output_dir) if args.output_dir else BENCH_DIR / "results"

    # Apply mode overrides
    if args.mode == "idle":
        config.max_concurrent_workers = 0
        # Shorter duration for idle mode — overhead stabilizes fast
        if config.benchmark_duration_s > 120:
            config.benchmark_duration_s = 120
            print(f"Idle mode: reduced duration to {config.benchmark_duration_s}s")

    # Discover and select approach
    discover_approaches()
    if args.approach not in APPROACHES:
        available = ", ".join(sorted(APPROACHES.keys())) or "(none found)"
        print(f"Error: unknown approach '{args.approach}'")
        print(f"Available: {available}")
        sys.exit(1)

    approach = APPROACHES[args.approach]

    # Run
    run_benchmark(approach, num_agents, config, output_dir, sample_interval, args.mode)


if __name__ == "__main__":
    main()
