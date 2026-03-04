"""
Abstract base class for benchmark approaches.

Each approach implements a different isolation strategy for running agents.
The orchestrator uses this interface to start/stop agents and collect PIDs
for memory measurement, without knowing the details of the isolation mechanism.

To add a new approach (e.g., Podman rootless, Firecracker):
  1. Create a new file in approaches/ (e.g., podman_rootless.py)
  2. Implement a class that extends Approach
  3. Register it in approaches/__init__.py
"""

import uuid
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Dict, List, Optional


@dataclass
class BenchmarkConfig:
    """Configuration passed to approaches from config.env / CLI."""

    # Agent settings
    agent_memory_mb: int = 4096
    agent_baseline_mb: int = 50
    max_concurrent_workers: int = 5
    spawn_interval_mean_s: int = 30
    benchmark_duration_s: int = 300

    # Worker settings
    worker_image: str = "bench-worker:latest"
    worker_memory_limit_mb: int = 2048
    worker_memory_mb: int = 500
    worker_duration_min_s: int = 30
    worker_duration_max_s: int = 120

    # Run metadata
    run_id: str = field(default_factory=lambda: uuid.uuid4().hex[:12])
    rng_seed: int = 42

    @classmethod
    def from_env(cls, env: Optional[Dict[str, str]] = None) -> "BenchmarkConfig":
        """Load config from environment variables (or a dict)."""
        import os

        e = env or os.environ
        return cls(
            agent_memory_mb=int(e.get("AGENT_MEMORY_MB", "4096")),
            agent_baseline_mb=int(e.get("AGENT_BASELINE_MB", "50")),
            max_concurrent_workers=int(e.get("MAX_CONCURRENT_WORKERS", "5")),
            spawn_interval_mean_s=int(e.get("SPAWN_INTERVAL_MEAN_S", "30")),
            benchmark_duration_s=int(e.get("BENCHMARK_DURATION_S", "300")),
            worker_image=e.get("WORKER_IMAGE", "bench-worker:latest"),
            worker_memory_limit_mb=int(e.get("WORKER_MEMORY_LIMIT_MB", "2048")),
            worker_memory_mb=int(e.get("WORKER_MEMORY_MB", "500")),
            worker_duration_min_s=int(e.get("WORKER_DURATION_MIN_S", "30")),
            worker_duration_max_s=int(e.get("WORKER_DURATION_MAX_S", "120")),
            rng_seed=int(e.get("RNG_SEED", "42")),
        )


class Approach(ABC):
    """Abstract base class for isolation approaches."""

    @property
    @abstractmethod
    def name(self) -> str:
        """Short identifier, e.g. 'vm-qemu', 'container-docker'."""
        ...

    @abstractmethod
    def setup(self, config: BenchmarkConfig) -> None:
        """One-time setup: build images, VM disks, etc."""
        ...

    @abstractmethod
    def start_agents(self, n: int, config: BenchmarkConfig) -> List[str]:
        """
        Start N agents. Returns a list of agent IDs.
        Agents should begin their spawn loops immediately.
        """
        ...

    @abstractmethod
    def get_agent_pids(self) -> Dict[str, int]:
        """
        Return a mapping of agent_id -> host PID for memory measurement.
        For VMs: the QEMU process PID.
        For containers: the container init PID on the host.
        """
        ...

    def get_daemon_pids(self) -> Dict[str, int]:
        """
        Return PIDs for container runtime daemons (dockerd, containerd, etc.)
        for memory measurement. Default returns empty dict.

        Override in approaches where daemons are on the host
        (e.g., container-docker). For VM approaches, daemons run inside
        the guest and are not directly measurable from the host.
        """
        return {}

    @abstractmethod
    def count_active_workers(self) -> int:
        """Count the total number of active worker containers across all agents."""
        ...

    def collect_agent_logs(self, agent_ids: List[str], output_dir) -> None:
        """
        Collect agent stdout logs (JSONL events) after benchmark completes.
        Override per approach: docker logs for containers, console logs for VMs.
        Default is a no-op.
        """
        pass

    @abstractmethod
    def stop_agents(self) -> None:
        """Stop all agents and clean up their workers."""
        ...

    def cleanup(self) -> None:
        """Optional: remove build artifacts (images, VM disks, etc.)."""
        pass
