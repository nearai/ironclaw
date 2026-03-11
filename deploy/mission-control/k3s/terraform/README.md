# Single-Node k3s Stage

This is the phase-after-benchmark path for WSL Ubuntu.

The local benchmark-first path is now green again on the current workstation, but this
stage remains intentionally staged until the same KPI contract is carried into the cluster.

## Assumptions

- k3s is already installed and reachable through `~/.kube/config`
- local or remote vLLM upstreams are already running
- the router image is published or built locally and pushed to a registry k3s can pull from

## Apply

```bash
cd deploy/mission-control/k3s/terraform
terraform init
terraform apply
```

This stages:

- a dedicated namespace
- Prometheus/Grafana via `kube-prometheus-stack`
- a single `llm-cluster-router` deployment
- a config map for the baseline mixed-GPU routing topology

Before promoting this stage, confirm:

1. the local router benchmark is repeatable
2. the canonical Mission Control benchmark artifact reflects a healthy run
3. the paired GPU probe confirms the intended device binding for the active model lane
4. the same router health and model inventory contract is preserved in-cluster

Current proof baseline before promotion:

- router benchmark: `target/mission-control-benchmark.json`
- GPU probe: `target/mission-control-gpu-probe.json`
- direct Ollama comparison: `target/mission-control-benchmark-ollama-direct.json`
