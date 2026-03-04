#!/usr/bin/env python3
"""
Compare benchmark results from multiple runs.

Reads summary.json files from all run directories in results/,
prints a markdown comparison table with percentiles, daemon overhead,
and swap warnings.

Usage:
    python3 -m analysis.compare [results-dir]
"""

import json
import sys
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent


def load_summaries(results_dir: Path) -> list:
    """Load all summary.json files from result directories."""
    summaries = []
    for run_dir in sorted(results_dir.iterdir()):
        summary_file = run_dir / "summary.json"
        if summary_file.exists():
            with open(summary_file) as f:
                summary = json.load(f)
                summary["run_dir"] = str(run_dir.name)
                summaries.append(summary)
    return summaries


def print_table(summaries: list) -> None:
    """Print a markdown comparison table."""
    if not summaries:
        print("No results found.")
        return

    # Check for any swap activity
    swap_runs = []
    for s in summaries:
        swap = s.get("swap", {})
        if swap.get("swap_occurred"):
            swap_runs.append(s.get("run_dir", "unknown"))

    if swap_runs:
        print("\nWARNING: Swap activity detected in the following runs:")
        for r in swap_runs:
            print(f"  - {r}")
        print("Results may not reflect true physical memory usage.\n")

    print()
    header = (
        "| Agents | Mode   | Approach         | Mean (MiB) | Peak (MiB) | "
        "p95 (MiB) | Avg Workers | Per-Agent (MiB) | Drift (KiB/s) | Daemon (MiB) |"
    )
    separator = (
        "|--------|--------|------------------|------------|------------|"
        "-----------|-------------|-----------------|---------------|--------------|"
    )
    print(header)
    print(separator)

    sort_key = lambda x: (
        x.get("mode", "loaded"),
        x.get("num_agents", 0),
        x.get("approach", ""),
    )

    for s in sorted(summaries, key=sort_key):
        approach = s.get("approach", "unknown")
        mode = s.get("mode", "loaded")
        agents = s.get("num_agents", 0)
        mean = s.get("steady_state_mean_mib", 0)
        peak = s.get("peak_mib", 0)
        p95 = s.get("p95_mib", 0)
        workers = s.get("avg_workers", 0)
        per_agent = s.get("per_agent_mean_mib", 0)
        drift = s.get("drift_kb_per_s", 0)

        workers_str = f"{workers:.1f}" if workers >= 0 else "N/A"

        # Daemon overhead (sum of all daemons' PSS, fall back to RSS)
        daemon_total = 0
        for d_name, d_data in s.get("daemon_overhead", {}).items():
            daemon_total += d_data.get("mean_pss_mib", d_data.get("mean_rss_mib", 0))
        daemon_str = f"{daemon_total:.0f}" if daemon_total > 0 else "-"

        # Flag drift
        drift_str = f"{drift:.1f}" if abs(drift) > 1 else "-"

        print(
            f"| {agents:>6} | {mode:<6} | {approach:<16} | {mean:>10.0f} | {peak:>10.0f} | "
            f"{p95:>9.0f} | {workers_str:>11} | {per_agent:>15.0f} | "
            f"{drift_str:>13} | {daemon_str:>12} |"
        )

    print()

    # Print worker spawn stats if available
    has_worker_stats = any(s.get("total_workers_spawned", 0) > 0 for s in summaries)
    if has_worker_stats:
        print("Worker Statistics:")
        print("| Run | Total Spawned | Max Concurrent |")
        print("|-----|---------------|----------------|")
        for s in sorted(summaries, key=sort_key):
            total = s.get("total_workers_spawned", 0)
            max_c = s.get("max_concurrent_workers", 0)
            if total > 0:
                name = f"{s.get('approach', '?')}-{s.get('mode', '?')}-n{s.get('num_agents', '?')}"
                print(f"| {name:<30} | {total:>13} | {max_c:>14} |")
        print()


def main():
    results_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else BENCH_DIR / "results"

    if not results_dir.exists():
        print(f"Results directory not found: {results_dir}")
        sys.exit(1)

    summaries = load_summaries(results_dir)
    print_table(summaries)


if __name__ == "__main__":
    main()
