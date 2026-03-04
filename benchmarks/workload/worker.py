#!/usr/bin/env python3
"""
Worker memory simulator for isolation benchmarks.

Simulates a oneshot worker task: allocates a configurable amount of memory,
touches every page to ensure physical allocation, holds for a random duration,
then exits. Designed to run inside a Docker container with --rm.

Environment variables:
    WORKER_MEMORY_MB       Memory to allocate and touch (default: 500)
    WORKER_DURATION_MIN_S  Minimum hold duration in seconds (default: 30)
    WORKER_DURATION_MAX_S  Maximum hold duration in seconds (default: 120)
"""

import mmap
import os
import random
import sys
import time

PAGE_SIZE = os.sysconf("SC_PAGESIZE")  # Usually 4096


def get_rss_kb() -> int:
    """Read RSS from /proc/self/statm in KiB."""
    try:
        with open("/proc/self/statm") as f:
            parts = f.read().split()
            rss_pages = int(parts[1])
            return rss_pages * PAGE_SIZE // 1024
    except (FileNotFoundError, IndexError):
        return -1


def allocate_and_touch(size_mb: int) -> mmap.mmap:
    """Allocate anonymous memory and touch every page to force physical allocation."""
    size_bytes = size_mb * 1024 * 1024
    # MAP_ANONYMOUS + MAP_PRIVATE = anonymous private mapping
    mm = mmap.mmap(-1, size_bytes, mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS)
    # Touch every page to ensure RSS reflects real physical allocation
    for offset in range(0, size_bytes, PAGE_SIZE):
        mm[offset] = offset & 0xFF
    return mm


def main():
    memory_mb = int(os.environ.get("WORKER_MEMORY_MB", "500"))
    duration_min = int(os.environ.get("WORKER_DURATION_MIN_S", "30"))
    duration_max = int(os.environ.get("WORKER_DURATION_MAX_S", "120"))

    duration = random.uniform(duration_min, duration_max)

    print(f"[worker] Allocating {memory_mb} MB...", flush=True)
    mem = allocate_and_touch(memory_mb)
    rss = get_rss_kb()
    print(
        f"[worker] Allocated. RSS={rss} KiB ({rss // 1024} MiB). "
        f"Holding for {duration:.0f}s.",
        flush=True,
    )

    time.sleep(duration)

    mem.close()
    print("[worker] Done, exiting.", flush=True)


if __name__ == "__main__":
    main()
