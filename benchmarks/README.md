# Agent Isolation Benchmarks

Compares memory overhead of different isolation approaches for hosting
ironclaw agents with worker sandboxing.

## Approaches

| Approach | Isolation boundary | Daemon | Worker isolation |
|---|---|---|---|
| `container-docker` | cgroups/namespaces | dockerd + containerd | Shared kernel, shared daemon |
| `container-gvisor` | cgroups/namespaces + gVisor sentry | dockerd + containerd | gVisor syscall interception, shared daemon |
| `container-gvisor-dind` | gVisor sandbox per agent | Per-agent dockerd inside gVisor | gVisor kernel isolation + private daemon |
| `podman-rootless` | cgroups/namespaces per user | None (socket-activated) | Separate user namespaces, no persistent daemon |
| `vm-qemu` | Full VM per agent | QEMU process + inner dockerd | Full VM (heavy) |
| `hybrid-firecracker` | Firecracker microVM per worker | dockerd (for agents) | Hardware-level KVM, lightweight |

## Quick Start

```bash
# Build Docker images (required for all approaches)
make images

# Run the container approach with 5 agents (stochastic workload)
make run APPROACH=container-docker AGENTS=5

# Run idle mode (no workers — measures pure isolation overhead)
make run-idle APPROACH=container-docker AGENTS=50

# Run an idle sweep at multiple scales
make run-sweep APPROACH=container-docker

# Compare results
make compare

# Generate charts (requires matplotlib)
make plot
```

### gVisor approaches (requires runsc)

```bash
# container-gvisor uses gVisor for workers only
make run APPROACH=container-gvisor AGENTS=5

# container-gvisor-dind runs each agent in its own gVisor sandbox
# with a private Docker daemon (Docker-in-gVisor)
make run APPROACH=container-gvisor-dind AGENTS=3
```

Requires `runsc` runtime registered in `/etc/docker/daemon.json` with
`--net-raw` and `--allow-packet-socket-write` flags (for the DinD variant).

### Podman rootless (requires podman + systemd-container)

```bash
make podman-setup
make run APPROACH=podman-rootless AGENTS=3
```

Creates ephemeral OS users per agent. Requires `podman`, `uidmap`, and
`systemd-container` (for `machinectl` / `systemd-run --machine`).

### VM approach (requires KVM + libguestfs)

```bash
# Build the VM image (one-time)
make vm-image

# Run
make run APPROACH=vm-qemu AGENTS=5
```

### Firecracker hybrid (requires firecracker + KVM)

```bash
make fc-setup
make run APPROACH=hybrid-firecracker AGENTS=3
```

Agents run in Docker containers with `/dev/kvm` passthrough. Workers are
Firecracker microVMs spawned directly by the agent.

### GCP VM Setup

```bash
# Create a GCP VM with nested virtualization
gcloud compute instances create bench-vm \
  --zone=us-central1-a \
  --machine-type=n2-standard-16 \
  --enable-nested-virtualization \
  --image-family=ubuntu-2204-lts \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=50GB

# SSH in and run setup
gcloud compute ssh bench-vm
sudo bash benchmarks/setup-gcp.sh
```

## Prerequisites

- **All approaches**: Linux, Docker daemon, Python 3.8+, `docker` Python SDK
- **gVisor approaches**: `runsc` runtime registered in Docker daemon config
- **Podman rootless**: `podman`, `uidmap`, `systemd-container`
- **VM approach**: QEMU (`qemu-system-x86_64`), KVM (`/dev/kvm`), libguestfs-tools
- **Firecracker hybrid**: `firecracker` binary, KVM (`/dev/kvm`)
- **Charts**: `pip install matplotlib`
- **GCP**: Use `setup-gcp.sh` to install everything

## Modes

| Mode | Description | Use case |
|------|-------------|----------|
| `loaded` | Stochastic workload — agents spawn workers randomly (default) | Realistic memory profile under load |
| `idle` | No workers — agents sit idle | Measure pure isolation overhead per agent |

```bash
# Explicit mode selection
make run APPROACH=container-docker AGENTS=5 MODE=loaded
make run APPROACH=container-docker AGENTS=100 MODE=idle
```

## Configuration

Edit `config.env` to tune parameters:

```
RNG_SEED=42                   # Base seed for reproducible randomness
BENCHMARK_DURATION_S=300      # How long to run (seconds)
SPAWN_INTERVAL_MEAN_S=30      # Mean time between worker spawns per agent
MAX_CONCURRENT_WORKERS=5      # Max workers per agent
WORKER_MEMORY_MB=500          # Memory each worker allocates
WORKER_DURATION_MIN_S=30      # Min worker lifetime
WORKER_DURATION_MAX_S=120     # Max worker lifetime
```

## Host Tuning

For accurate measurements, run `setup-gcp.sh` or manually apply:

```bash
# Disable transparent huge pages
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled

# Disable kernel same-page merging
echo 0 | sudo tee /sys/kernel/mm/ksm/run

# Disable swap
sudo swapoff -a

# Drop page caches
echo 3 | sudo tee /proc/sys/vm/drop_caches
```

The orchestrator will warn at startup if swap is enabled, and will
report if any swap activity occurred during the benchmark.

## Output

Each run creates a directory under `results/`:

```
results/container-docker-loaded-n5-20260304T143022/
├── params.json        # Full configuration for reproducibility
├── timeseries.jsonl   # Memory samples (one JSON object per line)
├── summary.json       # Aggregated statistics
├── agent-0.jsonl      # Agent event log (worker_start/worker_end events)
├── agent-1.jsonl      # ...
└── ...
```

### What's in the JSONL

Each sample includes:
- Host memory consumed (MemTotal - MemAvailable)
- Full `/proc/meminfo` breakdown (Cached, Slab, AnonPages, Shmem, Swap, etc.)
- Per-agent RSS and PSS from `/proc/<pid>/smaps_rollup`
- Daemon (dockerd, containerd) RSS and PSS
- Active worker count
- Swap activity counters (pswpin/pswpout)
- Memory pressure (PSI) if available

### Summary statistics

- Baseline-subtracted mean, peak, and p50/p95/p99
- Per-agent mean overhead
- Memory drift slope (KiB/s) to detect leaks
- Daemon overhead breakdown
- Total workers spawned and max concurrent

## Worker Lifecycle

The agent uses the Docker Python SDK to match ironclaw's `ContainerJobManager` path:

```
create → start → wait (background) → remove
```

Each worker container gets labels for tracking and cleanup:
- `bench_run_id`: unique per benchmark run
- `bench_role`: "agent" or "worker"
- `bench_agent_id`: which agent spawned it
- `bench_approach`: which approach is running

## Adding New Approaches

1. Create `approaches/my_approach.py`
2. Implement a class extending `approaches.base.Approach`
3. Run: `make run APPROACH=my-approach AGENTS=5`

See `approaches/base.py` for the interface and `approaches/container_docker.py`
for a reference implementation.
