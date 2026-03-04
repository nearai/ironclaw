#!/usr/bin/env python3
"""
Generate comparison charts from benchmark results.

Reads timeseries.jsonl and summary.json from all run directories,
generates:
  - Per-run time-series plots (memory over time + worker count)
  - Cross-approach comparison chart (steady-state mean at each N)
  - Per-agent scaling chart

Usage:
    python3 -m analysis.plot [results-dir]

Requires: pip install matplotlib
"""

import json
import sys
from collections import defaultdict
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent


def load_run(run_dir: Path) -> dict:
    """Load a single run's data."""
    params_file = run_dir / "params.json"
    ts_file = run_dir / "timeseries.jsonl"
    summary_file = run_dir / "summary.json"

    if not all(f.exists() for f in [params_file, ts_file]):
        return None

    with open(params_file) as f:
        params = json.load(f)

    samples = []
    with open(ts_file) as f:
        for line in f:
            if line.strip():
                samples.append(json.loads(line))

    summary = {}
    if summary_file.exists():
        with open(summary_file) as f:
            summary = json.load(f)

    return {
        "params": params,
        "samples": samples,
        "summary": summary,
        "run_dir": run_dir.name,
    }


def plot_timeseries(run: dict, output_path: Path) -> None:
    """Plot time-series for a single run."""
    import matplotlib.pyplot as plt

    samples = run["samples"]
    params = run["params"]

    times = [s["timestamp_s"] for s in samples]
    mem_mib = [s["host_consumed_kb"] / 1024 for s in samples]
    workers = [s["active_workers"] for s in samples]

    fig, ax1 = plt.subplots(figsize=(14, 6))

    # Memory on left axis
    color1 = "tab:blue"
    ax1.set_xlabel("Time (seconds)")
    ax1.set_ylabel("Host Memory Consumed (MiB)", color=color1)
    ax1.plot(times, mem_mib, color=color1, alpha=0.8, linewidth=0.8)
    ax1.tick_params(axis="y", labelcolor=color1)

    # Workers on right axis (if available and meaningful)
    if any(w >= 0 for w in workers):
        ax2 = ax1.twinx()
        color2 = "tab:orange"
        ax2.set_ylabel("Active Workers", color=color2)
        ax2.plot(times, workers, color=color2, alpha=0.5, linewidth=0.8)
        ax2.tick_params(axis="y", labelcolor=color2)

    # Plot daemon overhead if present
    daemon_times = []
    daemon_rss = defaultdict(list)
    for s in samples:
        for d_name, d_data in s.get("daemons", {}).items():
            daemon_times.append(s["timestamp_s"])
            daemon_rss[d_name].append(d_data.get("pss_kb", d_data.get("rss_kb", 0)) / 1024)

    approach = params.get("approach", "unknown")
    mode = params.get("mode", "loaded")
    n = params.get("num_agents", "?")
    plt.title(f"{approach} — {n} agents ({mode})")

    fig.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close()
    print(f"  Saved: {output_path}")


def plot_comparison(runs: list, output_dir: Path) -> None:
    """Plot cross-approach comparison charts."""
    import matplotlib.pyplot as plt

    # Group by (approach, mode)
    by_key = defaultdict(list)
    for run in runs:
        if run["summary"]:
            approach = run["summary"].get("approach", "unknown")
            mode = run["summary"].get("mode", "loaded")
            key = f"{approach} ({mode})"
            by_key[key].append(run["summary"])

    if len(by_key) < 2:
        print("  Need results from at least 2 (approach, mode) combos for comparison chart.")
        return

    # Steady-state comparison bar chart
    fig, ax = plt.subplots(figsize=(12, 6))

    keys = sorted(by_key.keys())
    all_n_values = sorted(
        set(
            s["num_agents"]
            for summaries in by_key.values()
            for s in summaries
        )
    )

    bar_width = 0.8 / len(keys)
    for i, key in enumerate(keys):
        summaries = {s["num_agents"]: s for s in by_key[key]}
        xs = []
        ys = []
        for j, n in enumerate(all_n_values):
            if n in summaries:
                xs.append(j + i * bar_width)
                ys.append(summaries[n]["steady_state_mean_mib"])
        ax.bar(xs, ys, bar_width, label=key, alpha=0.8)

    ax.set_xlabel("Number of Agents")
    ax.set_ylabel("Steady-State Memory (MiB)")
    ax.set_title("Memory Consumption by Approach")
    ax.set_xticks([j + bar_width * (len(keys) - 1) / 2 for j in range(len(all_n_values))])
    ax.set_xticklabels([str(n) for n in all_n_values])
    ax.legend()
    ax.grid(axis="y", alpha=0.3)

    fig.tight_layout()
    output_path = output_dir / "comparison-bar.png"
    plt.savefig(output_path, dpi=150)
    plt.close()
    print(f"  Saved: {output_path}")

    # Per-agent scaling chart
    fig, ax = plt.subplots(figsize=(10, 6))

    for key in keys:
        summaries = sorted(by_key[key], key=lambda s: s["num_agents"])
        ns = [s["num_agents"] for s in summaries]
        per_agent = [s["per_agent_mean_mib"] for s in summaries]
        ax.plot(ns, per_agent, marker="o", label=key)

    ax.set_xlabel("Number of Agents")
    ax.set_ylabel("Per-Agent Memory (MiB)")
    ax.set_title("Per-Agent Overhead Scaling")
    ax.legend()
    ax.grid(alpha=0.3)

    fig.tight_layout()
    output_path = output_dir / "per-agent-scaling.png"
    plt.savefig(output_path, dpi=150)
    plt.close()
    print(f"  Saved: {output_path}")


def main():
    results_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else BENCH_DIR / "results"

    if not results_dir.exists():
        print(f"Results directory not found: {results_dir}")
        sys.exit(1)

    # Load all runs
    runs = []
    for run_dir in sorted(results_dir.iterdir()):
        if run_dir.is_dir() and (run_dir / "params.json").exists():
            run = load_run(run_dir)
            if run:
                runs.append(run)

    if not runs:
        print("No benchmark results found.")
        sys.exit(1)

    print(f"Found {len(runs)} benchmark run(s).")

    # Generate per-run time-series plots
    print("\nGenerating time-series plots...")
    for run in runs:
        output_path = results_dir / run["run_dir"] / "timeseries.png"
        plot_timeseries(run, output_path)

    # Generate comparison charts
    print("\nGenerating comparison charts...")
    plot_comparison(runs, results_dir)

    print("\nDone.")


if __name__ == "__main__":
    main()
