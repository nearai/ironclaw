"""
Approach A: VM-per-agent with QEMU/KVM.

Each agent runs in its own lightweight VM (Alpine Linux with Docker).
Inside each VM: Docker daemon + agent process + worker containers.
The VM overhead (guest kernel, guest Docker daemon, QEMU process) is what
we're measuring compared to the container approach.

Note on VM memory behavior: Once the guest touches pages, host pages
generally remain allocated to the VM even after the process inside the
guest frees them (unless ballooning/free-page reporting is enabled).
This means VM memory acts as a high-water mark, which is a real property
of the isolation approach and affects density economics.
"""

import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Dict, List

from approaches.base import Approach, BenchmarkConfig

BENCH_DIR = Path(__file__).resolve().parent.parent
VM_DIR = BENCH_DIR / "vm"
VM_IMAGE = VM_DIR / "alpine-agent.qcow2"
LAUNCH_SCRIPT = VM_DIR / "launch.sh"


class VmQemuApproach(Approach):
    """VM-per-agent isolation using QEMU/KVM."""

    def __init__(self):
        self._agent_ids: List[str] = []
        self._config: BenchmarkConfig = None

    @property
    def name(self) -> str:
        return "vm-qemu"

    def setup(self, config: BenchmarkConfig) -> None:
        """Verify QEMU and VM image are available."""
        self._config = config

        # Check for QEMU
        result = subprocess.run(
            ["qemu-system-x86_64", "--version"],
            capture_output=True,
        )
        if result.returncode != 0:
            raise RuntimeError("qemu-system-x86_64 not found. Install QEMU first.")

        # Check for KVM
        if not os.path.exists("/dev/kvm"):
            raise RuntimeError("/dev/kvm not found. KVM support required.")

        # Check for VM image
        if not VM_IMAGE.exists():
            raise RuntimeError(
                f"VM image not found at {VM_IMAGE}. "
                "Run 'make vm-image' first."
            )

        print(f"[{self.name}] Setup complete. QEMU and VM image verified.")

    def start_agents(self, n: int, config: BenchmarkConfig) -> List[str]:
        """Start N QEMU VMs, each running an agent."""
        self._agent_ids = []

        for i in range(n):
            agent_id = f"agent-{i}"

            # Build environment variables to pass into the VM
            env_vars = [
                f"AGENT_BASELINE_MB={config.agent_baseline_mb}",
                f"SPAWN_INTERVAL_MEAN_S={config.spawn_interval_mean_s}",
                f"MAX_CONCURRENT_WORKERS={config.max_concurrent_workers}",
                f"BENCHMARK_DURATION_S={config.benchmark_duration_s}",
                f"WORKER_IMAGE={config.worker_image}",
                f"WORKER_MEMORY_LIMIT_MB={config.worker_memory_limit_mb}",
                f"WORKER_MEMORY_MB={config.worker_memory_mb}",
                f"WORKER_DURATION_MIN_S={config.worker_duration_min_s}",
                f"WORKER_DURATION_MAX_S={config.worker_duration_max_s}",
                f"RNG_SEED={config.rng_seed}",
                f"BENCH_RUN_ID={config.run_id}",
                f"BENCH_APPROACH={self.name}",
            ]

            cmd = [
                str(LAUNCH_SCRIPT), "start",
                agent_id,
                str(config.agent_memory_mb),
                "2",  # CPUs per VM
                str(VM_IMAGE),
            ] + env_vars

            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode != 0:
                raise RuntimeError(
                    f"Failed to start VM {agent_id}: {result.stderr.strip()}"
                )

            self._agent_ids.append(agent_id)
            print(f"[{self.name}] Started VM {agent_id}")

        # Wait for VMs to boot and Docker to start
        print(f"[{self.name}] Waiting 30s for {n} VMs to boot + start Docker...")
        time.sleep(30)

        print(f"[{self.name}] {n} VMs started.")
        return list(self._agent_ids)

    def get_agent_pids(self) -> Dict[str, int]:
        """Get QEMU process PIDs from pidfiles."""
        pids = {}
        vm_base = Path(os.environ.get("VM_BASE_DIR", "/tmp/bench-vms"))
        for agent_id in self._agent_ids:
            pidfile = vm_base / agent_id / "qemu.pid"
            try:
                if pidfile.exists():
                    pid = int(pidfile.read_text().strip())
                    # Verify process still exists
                    os.kill(pid, 0)
                    pids[agent_id] = pid
            except (ValueError, ProcessLookupError, PermissionError):
                pass
        return pids

    def count_active_workers(self) -> int:
        """
        Count workers across all VMs.
        Since workers run inside VMs, we can't count them from the host.
        Return -1 to indicate "unknown" — accurate counts come from
        agent JSONL logs collected after the run.
        """
        return -1

    def collect_agent_logs(self, agent_ids: List[str], output_dir) -> None:
        """Collect agent logs from QEMU serial console output files."""
        output_dir = Path(output_dir)
        vm_base = Path(os.environ.get("VM_BASE_DIR", "/tmp/bench-vms"))
        for agent_id in agent_ids:
            console_log = vm_base / agent_id / "console.log"
            dest = output_dir / f"{agent_id}.log"
            if console_log.exists():
                try:
                    shutil.copy2(console_log, dest)
                    print(f"[{self.name}] Collected console log for {agent_id}")
                except Exception as e:
                    print(f"[{self.name}] Failed to collect log for {agent_id}: {e}")

    def stop_agents(self) -> None:
        """Stop all VMs."""
        result = subprocess.run(
            [str(LAUNCH_SCRIPT), "stop-all"],
            capture_output=True,
            text=True,
        )
        self._agent_ids = []
        print(f"[{self.name}] All VMs stopped.")

    def cleanup(self) -> None:
        """Stop VMs (image cleanup is manual via make clean)."""
        self.stop_agents()
