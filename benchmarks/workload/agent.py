#!/usr/bin/env python3
"""
Agent simulator for isolation benchmarks.

Simulates an ironclaw agent by:
1. Allocating baseline memory (agent's own footprint)
2. Running a spawn loop that creates worker containers via the Docker API

Worker lifecycle matches ironclaw's ContainerJobManager:
  create → start → wait → remove

Emits structured JSONL events to stdout for host-side collection:
  {"t": ..., "event": "worker_start", "agent_id": ..., "worker_id": ...}
  {"t": ..., "event": "worker_end",   "agent_id": ..., "worker_id": ...}
  {"t": ..., "event": "status",       "agent_id": ..., "active_workers": N}

Environment variables:
  AGENT_ID                Unique agent identifier (required)
  AGENT_BASELINE_MB       Agent process memory footprint (default: 50)
  SPAWN_INTERVAL_MEAN_S   Mean time between spawns, exponential dist (default: 30)
  MAX_CONCURRENT_WORKERS  Max workers running at once per agent (default: 5)
  BENCHMARK_DURATION_S    Total benchmark duration (default: 300)
  WORKER_IMAGE            Worker container image (default: bench-worker:latest)
  WORKER_MEMORY_LIMIT_MB  Worker container memory limit (default: 2048)
  WORKER_MEMORY_MB        Memory each worker allocates (default: 500)
  WORKER_DURATION_MIN_S   Min worker lifetime (default: 30)
  WORKER_DURATION_MAX_S   Max worker lifetime (default: 120)
  RNG_SEED                Base RNG seed for reproducibility (default: 42)
  BENCH_RUN_ID            Unique run identifier for container labels (default: unknown)
  BENCH_APPROACH          Approach name for container labels (default: unknown)
"""

import hashlib
import json
import mmap
import os
import random
import signal
import sys
import threading
import time

import docker


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

AGENT_ID = os.environ.get("AGENT_ID", "agent-0")
AGENT_BASELINE_MB = int(os.environ.get("AGENT_BASELINE_MB", "50"))
SPAWN_INTERVAL_MEAN_S = float(os.environ.get("SPAWN_INTERVAL_MEAN_S", "30"))
MAX_CONCURRENT_WORKERS = int(os.environ.get("MAX_CONCURRENT_WORKERS", "5"))
BENCHMARK_DURATION_S = int(os.environ.get("BENCHMARK_DURATION_S", "300"))
WORKER_IMAGE = os.environ.get("WORKER_IMAGE", "bench-worker:latest")
WORKER_MEMORY_LIMIT_MB = int(os.environ.get("WORKER_MEMORY_LIMIT_MB", "2048"))
WORKER_MEMORY_MB = int(os.environ.get("WORKER_MEMORY_MB", "500"))
WORKER_DURATION_MIN_S = int(os.environ.get("WORKER_DURATION_MIN_S", "30"))
WORKER_DURATION_MAX_S = int(os.environ.get("WORKER_DURATION_MAX_S", "120"))
RNG_SEED_BASE = int(os.environ.get("RNG_SEED", "42"))
BENCH_RUN_ID = os.environ.get("BENCH_RUN_ID", "unknown")
BENCH_APPROACH = os.environ.get("BENCH_APPROACH", "unknown")

# Derive per-agent seed: hash(base_seed + agent_id) for reproducibility
_seed_input = f"{RNG_SEED_BASE}:{AGENT_ID}"
AGENT_SEED = int(hashlib.sha256(_seed_input.encode()).hexdigest()[:8], 16)

WORKER_PREFIX = f"bench-worker-{AGENT_ID}"
STATUS_INTERVAL_S = 5  # How often to emit status events


# ---------------------------------------------------------------------------
# Structured event logging
# ---------------------------------------------------------------------------

_log_lock = threading.Lock()


def emit_event(event: str, **kwargs):
    """Emit a structured JSONL event to stdout."""
    record = {"t": time.time(), "event": event, "agent_id": AGENT_ID}
    record.update(kwargs)
    line = json.dumps(record)
    with _log_lock:
        print(line, flush=True)


def log(msg: str):
    """Emit a human-readable log line (also structured)."""
    emit_event("log", message=msg)


# ---------------------------------------------------------------------------
# Baseline memory allocation
# ---------------------------------------------------------------------------

def allocate_baseline(size_mb: int) -> mmap.mmap:
    """Allocate anonymous memory and touch every page."""
    size_bytes = size_mb * 1024 * 1024
    mm = mmap.mmap(-1, size_bytes, mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS)
    page_size = os.sysconf("SC_PAGESIZE")
    for offset in range(0, size_bytes, page_size):
        mm[offset] = offset & 0xFF
    return mm


# ---------------------------------------------------------------------------
# Worker lifecycle: create → start → wait → remove
# ---------------------------------------------------------------------------

# Thread-safe active worker count
_active_workers = 0
_active_lock = threading.Lock()
_worker_counter = 0
_counter_lock = threading.Lock()


def get_active_workers() -> int:
    with _active_lock:
        return _active_workers


def _inc_active():
    global _active_workers
    with _active_lock:
        _active_workers += 1
        return _active_workers


def _dec_active():
    global _active_workers
    with _active_lock:
        _active_workers -= 1
        return _active_workers


def next_worker_id() -> tuple:
    """Return (counter, worker_name)."""
    global _worker_counter
    with _counter_lock:
        _worker_counter += 1
        c = _worker_counter
    return c, f"{WORKER_PREFIX}-{c}"


def spawn_worker(client: docker.DockerClient, rng: random.Random) -> bool:
    """
    Spawn a single worker container using the ironclaw lifecycle:
    create → start → wait → remove.

    Returns True if the container was created and started successfully.
    The wait/remove happens in a background thread.
    """
    counter, worker_name = next_worker_id()

    # Container config mirrors ironclaw sandbox defaults
    try:
        container = client.containers.create(
            image=WORKER_IMAGE,
            name=worker_name,
            mem_limit=f"{WORKER_MEMORY_LIMIT_MB}m",
            cap_drop=["ALL"],
            cap_add=["CHOWN"],
            security_opt=["no-new-privileges"],
            tmpfs={"/tmp": "size=512m"},
            user="1000:1000",
            environment={
                "WORKER_MEMORY_MB": str(WORKER_MEMORY_MB),
                "WORKER_DURATION_MIN_S": str(WORKER_DURATION_MIN_S),
                "WORKER_DURATION_MAX_S": str(WORKER_DURATION_MAX_S),
            },
            labels={
                "bench_run_id": BENCH_RUN_ID,
                "bench_role": "worker",
                "bench_agent_id": AGENT_ID,
                "bench_approach": BENCH_APPROACH,
            },
            detach=True,
        )
    except docker.errors.APIError as e:
        log(f"Failed to create {worker_name}: {e}")
        return False

    try:
        container.start()
    except docker.errors.APIError as e:
        log(f"Failed to start {worker_name}: {e}")
        try:
            container.remove(force=True)
        except Exception:
            pass
        return False

    count = _inc_active()
    emit_event("worker_start", worker_id=worker_name, active_workers=count)

    # Wait and remove in background thread (matches ironclaw's async wait)
    def wait_and_remove():
        try:
            container.wait()
        except Exception:
            pass
        try:
            container.remove(force=True)
        except Exception:
            pass
        count = _dec_active()
        emit_event("worker_end", worker_id=worker_name, active_workers=count)

    t = threading.Thread(target=wait_and_remove, daemon=True)
    t.start()
    return True


# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------

def cleanup_workers(client: docker.DockerClient):
    """Remove all worker containers for this agent."""
    log("Cleaning up workers...")
    try:
        containers = client.containers.list(
            all=True,
            filters={"label": [f"bench_agent_id={AGENT_ID}"]},
        )
        for c in containers:
            try:
                c.remove(force=True)
            except Exception:
                pass
        if containers:
            log(f"Removed {len(containers)} worker containers.")
    except Exception as e:
        log(f"Cleanup error: {e}")


# ---------------------------------------------------------------------------
# Status emitter (periodic)
# ---------------------------------------------------------------------------

def status_emitter(stop_event: threading.Event):
    """Periodically emit status events with active worker count."""
    while not stop_event.is_set():
        emit_event("status", active_workers=get_active_workers())
        stop_event.wait(timeout=STATUS_INTERVAL_S)


# ---------------------------------------------------------------------------
# Main spawn loop
# ---------------------------------------------------------------------------

def main():
    # Set up reproducible RNG
    rng = random.Random(AGENT_SEED)

    emit_event("agent_start",
               baseline_mb=AGENT_BASELINE_MB,
               spawn_interval_mean_s=SPAWN_INTERVAL_MEAN_S,
               max_concurrent_workers=MAX_CONCURRENT_WORKERS,
               duration_s=BENCHMARK_DURATION_S,
               rng_seed=AGENT_SEED)

    # Allocate baseline memory
    log(f"Allocating {AGENT_BASELINE_MB} MB baseline memory...")
    _baseline_mem = allocate_baseline(AGENT_BASELINE_MB)
    log(f"Baseline memory allocated.")

    # Connect to Docker
    client = docker.from_env()

    # Set up signal handler for clean shutdown
    stop = threading.Event()

    def handle_signal(sig, frame):
        log(f"Received signal {sig}, shutting down...")
        stop.set()

    signal.signal(signal.SIGTERM, handle_signal)
    signal.signal(signal.SIGINT, handle_signal)

    # Start status emitter
    status_thread = threading.Thread(
        target=status_emitter, args=(stop,), daemon=True
    )
    status_thread.start()

    # Spawn loop
    start_time = time.monotonic()
    log("Entering spawn loop.")

    while not stop.is_set():
        # Check duration
        elapsed = time.monotonic() - start_time
        if elapsed >= BENCHMARK_DURATION_S:
            log(f"Benchmark duration reached ({elapsed:.0f}s). Stopping.")
            break

        # Sleep for random interval (exponential distribution)
        delay = rng.expovariate(1.0 / SPAWN_INTERVAL_MEAN_S)
        if stop.wait(timeout=delay):
            break  # Signalled to stop during sleep

        # Check capacity
        active = get_active_workers()
        if active >= MAX_CONCURRENT_WORKERS:
            log(f"At capacity ({active}/{MAX_CONCURRENT_WORKERS}). Skipping spawn.")
            continue

        # Spawn
        log(f"Spawning worker (active: {active}/{MAX_CONCURRENT_WORKERS})")
        spawn_worker(client, rng)

    # Cleanup
    stop.set()
    cleanup_workers(client)
    emit_event("agent_stop", total_workers_spawned=_worker_counter)
    log("Agent stopped.")


if __name__ == "__main__":
    main()
