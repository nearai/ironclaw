"""
Approach B: Container-per-agent with shared Docker daemon.

Each agent runs in its own Docker container with the host Docker socket
bind-mounted. The agent spawns worker containers as siblings on the host
Docker daemon — the same path ironclaw uses when SANDBOX_ENABLED=true.
"""

import subprocess
import time
from pathlib import Path
from typing import Dict, List

from approaches.base import Approach, BenchmarkConfig


class ContainerDockerApproach(Approach):
    """Container-per-agent isolation using shared Docker daemon."""

    def __init__(self):
        self._agent_ids: List[str] = []
        self._agent_image = "bench-agent:latest"
        self._run_id: str = "unknown"

    @property
    def name(self) -> str:
        return "container-docker"

    def setup(self, config: BenchmarkConfig) -> None:
        """Verify that required Docker images exist."""
        self._run_id = config.run_id
        for image in [self._agent_image, config.worker_image]:
            result = subprocess.run(
                ["docker", "image", "inspect", image],
                capture_output=True,
            )
            if result.returncode != 0:
                raise RuntimeError(
                    f"Docker image '{image}' not found. Run 'make images' first."
                )
        print(f"[{self.name}] Setup complete. Images verified. run_id={self._run_id}")

    def start_agents(self, n: int, config: BenchmarkConfig) -> List[str]:
        """Start N agent containers, each with Docker socket access."""
        self._agent_ids = []

        for i in range(n):
            agent_id = f"agent-{i}"
            container_name = f"bench-agent-{i}"

            cmd = [
                "docker", "run", "-d",
                "--name", container_name,
                "--memory", f"{config.agent_memory_mb}m",
                # Labels for identification and cleanup
                "--label", f"bench_run_id={config.run_id}",
                "--label", "bench_role=agent",
                "--label", f"bench_agent_id={agent_id}",
                "--label", f"bench_approach={self.name}",
                # Bind-mount Docker socket so agent can spawn sibling containers
                "-v", "/var/run/docker.sock:/var/run/docker.sock",
                # Pass configuration via environment
                "-e", f"AGENT_ID={agent_id}",
                "-e", f"AGENT_BASELINE_MB={config.agent_baseline_mb}",
                "-e", f"SPAWN_INTERVAL_MEAN_S={config.spawn_interval_mean_s}",
                "-e", f"MAX_CONCURRENT_WORKERS={config.max_concurrent_workers}",
                "-e", f"BENCHMARK_DURATION_S={config.benchmark_duration_s}",
                "-e", f"WORKER_IMAGE={config.worker_image}",
                "-e", f"WORKER_MEMORY_LIMIT_MB={config.worker_memory_limit_mb}",
                "-e", f"WORKER_MEMORY_MB={config.worker_memory_mb}",
                "-e", f"WORKER_DURATION_MIN_S={config.worker_duration_min_s}",
                "-e", f"WORKER_DURATION_MAX_S={config.worker_duration_max_s}",
                "-e", f"RNG_SEED={config.rng_seed}",
                "-e", f"BENCH_RUN_ID={config.run_id}",
                "-e", f"BENCH_APPROACH={self.name}",
                self._agent_image,
            ]

            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode != 0:
                raise RuntimeError(
                    f"Failed to start agent {agent_id}: {result.stderr.strip()}"
                )

            self._agent_ids.append(agent_id)
            print(f"[{self.name}] Started {container_name}")

        print(f"[{self.name}] {n} agents started.")
        return list(self._agent_ids)

    def get_agent_pids(self) -> Dict[str, int]:
        """Get host PIDs for agent containers via docker inspect."""
        pids = {}
        for i, agent_id in enumerate(self._agent_ids):
            container_name = f"bench-agent-{i}"
            try:
                result = subprocess.run(
                    ["docker", "inspect", "--format", "{{.State.Pid}}", container_name],
                    capture_output=True,
                    text=True,
                )
                if result.returncode == 0:
                    pid = int(result.stdout.strip())
                    if pid > 0:
                        pids[agent_id] = pid
            except (ValueError, subprocess.SubprocessError):
                pass
        return pids

    def get_daemon_pids(self) -> Dict[str, int]:
        """Get PIDs for dockerd and containerd on the host."""
        pids = {}
        for daemon_name in ["dockerd", "containerd"]:
            try:
                result = subprocess.run(
                    ["pgrep", "-x", daemon_name],
                    capture_output=True,
                    text=True,
                )
                if result.returncode == 0 and result.stdout.strip():
                    # pgrep may return multiple PIDs; take the first (main process)
                    pid = int(result.stdout.strip().split("\n")[0])
                    pids[daemon_name] = pid
            except (ValueError, subprocess.SubprocessError):
                pass
        return pids

    def count_active_workers(self) -> int:
        """Count worker containers across all agents using labels."""
        try:
            result = subprocess.run(
                [
                    "docker", "ps", "-q",
                    "--filter", f"label=bench_run_id={self._run_id}",
                    "--filter", "label=bench_role=worker",
                ],
                capture_output=True,
                text=True,
            )
            if result.returncode == 0 and result.stdout.strip():
                return len(result.stdout.strip().split("\n"))
        except subprocess.SubprocessError:
            pass
        return 0

    def collect_agent_logs(self, agent_ids: List[str], output_dir) -> None:
        """Collect agent stdout logs via docker logs."""
        output_dir = Path(output_dir)
        for i, agent_id in enumerate(agent_ids):
            container_name = f"bench-agent-{i}"
            log_file = output_dir / f"{agent_id}.jsonl"
            try:
                result = subprocess.run(
                    ["docker", "logs", container_name],
                    capture_output=True,
                    text=True,
                )
                if result.returncode == 0:
                    log_file.write_text(result.stdout)
                    print(f"[{self.name}] Collected logs for {agent_id}")
            except subprocess.SubprocessError as e:
                print(f"[{self.name}] Failed to collect logs for {agent_id}: {e}")

    def stop_agents(self) -> None:
        """Stop all agent containers and their workers."""
        # First stop agents (their cleanup will remove their workers)
        for i in range(len(self._agent_ids)):
            container_name = f"bench-agent-{i}"
            subprocess.run(
                ["docker", "rm", "-f", container_name],
                capture_output=True,
            )

        # Wait a moment for agent cleanup to fire
        time.sleep(2)

        # Force-remove any remaining containers from this run using labels
        result = subprocess.run(
            [
                "docker", "ps", "-aq",
                "--filter", f"label=bench_run_id={self._run_id}",
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode == 0 and result.stdout.strip():
            container_ids = result.stdout.strip().split("\n")
            if container_ids:
                subprocess.run(
                    ["docker", "rm", "-f"] + container_ids,
                    capture_output=True,
                )

        self._agent_ids = []
        print(f"[{self.name}] All agents and workers stopped.")

    def cleanup(self) -> None:
        """Remove Docker images."""
        self.stop_agents()
