# ic-infrastructure-health-check — Deep Analysis Report (Draft)

## 1. Executive summary
The **ic‑infrastructure‑health‑check** subproject provides a Bash‑based orchestration layer that runs health diagnostics across key OpenClaw/Ironclaw services (gateway, XMPP bridge, ClickHouse, TensorZero, model APIs, etc.). Each diagnostic is a self‑contained script that emits structured JSON, which the main driver (`infrastructure-health-check.sh`) aggregates, writes a timestamped report, produces a markdown summary, and optionally sends a Gotify notification when the overall status is degraded or critical. The system is triggered via a systemd timer (`ironclaw-watchdog.timer`) and can also be run manually or from cron.

Key strengths:
- Simple, dependency‑light implementation (pure Bash, `jq`, `curl`, `nc`).
- JSON‑first output makes downstream processing easy.
- Extensible – adding a new component is a matter of creating a `health‑<name>.sh` script and wiring it into the driver.
- Integrated with existing OpenClaw heartbeat mechanisms.

Areas for improvement include robust parallel execution, refactoring duplicated run‑check calls, moving configuration to a central file, and adding proper testing/CI coverage.

## 2. Project structure
```
ic-infrastructure-health-check/
├─ README.md                     # High‑level design & usage
├─ infrastructure-health-check.sh# Main driver (aggregates checks)
├─ send-notification.sh         # Gotify wrapper
├─ cron-wrapper.sh               # Simple wrapper for cron jobs
├─ ironclaw-watchdog.timer       # systemd timer (hourly)
├─ icservices/                   # Systemd unit files (watchdog, xmpp‑bridge)
│   ├─ ironclaw.service
│   └─ xmpp-bridge.service
├─ icscripts/                    # CI helper scripts (quality_gate, coverage, …)
├─ health‑gateway.sh            # Checks OpenClaw gateway session health
├─ health‑xmpp.sh                # XMPP bridge connectivity/latency
├─ health‑omemo.sh               # OMEMO encryption health (placeholder)
├─ health‑ratelimit.sh           # Rate‑limit status
├─ health‑clickhouse.sh          # ClickHouse query latency & resource usage
├─ health‑tensorzero.sh          # TensorZero latency, GPU utilisation
├─ health‑models.sh              # Model API availability
├─ health‑systemd.sh             # Systemd unit health checks
├─ health‑irc.sh (not present)   # Mentioned in README but not shipped
└─ reports/ (runtime)           # Generated JSON & markdown per run
```
No compiled languages are used – the whole stack is Bash scripts.

## 3. Build system & CI/CD
- No compiled artifacts; the project is shipped as‑is.
- CI helper scripts live under `icscripts/` (e.g., `quality_gate.sh`, `coverage.sh`).
- The repository appears to use a generic CI pipeline (not visible in this subproject) that likely runs those scripts.
- No Dockerfile or explicit packaging – deployment relies on cloning the repo into the user's workspace and enabling the systemd timer.

## 4. Dependencies
| Category | Tool | Reason |
|----------|------|--------|
| Shell | `bash >=4.0` | Core scripting language |
| JSON | `jq` | Validation & extraction of JSON output |
| HTTP | `curl` | Gotify notifications and external API checks |
| TCP | `nc` (netcat) | Connectivity tests for services |
| DB | `clickhouse-client` (optional) | ClickHouse diagnostics |
| GPU | `nvidia-smi` / `rocm-smi` (optional) | TensorZero GPU utilisation |
| Systemd | `systemd` timers/services | Scheduling |
| Logging | Standard output + log file in `$HOME/.ironclaw/workspace/reports/health` |

No external language runtimes (Python, Rust, Go) are required.

## 5. Core components & runtime flow
1. **`infrastructure-health-check.sh`** (driver)
   - Sets environment (`SCRIPT_DIR`, `REPORT_DIR`).
   - Defines `run_check` helper that executes a health script with a 30 s timeout, validates JSON, logs timing, and returns the exit code.
   - Launches each component check in background, redirecting stdout to temporary files (`/tmp/check‑<name>.tmp`) and stderr to per‑component logs.
   - Waits for all background jobs, aggregates temporary files, builds a JSON report (`$REPORT_DIR/<timestamp>.json`) and a markdown summary (`<timestamp>-summary.md`).
   - Sends a Gotify notification if overall status is not `healthy`.
2. **Individual `health‑*.sh` scripts** – each follows the same contract:
   - Emits a JSON object with fields `component`, `status`, `timestamp`, `metrics`, and `issues`.
   - Returns an exit code (0 = healthy, 1 = degraded, 2 = critical).
   - Example: `health-gateway.sh` inspects the `$HOME/.ironclaw/agents` directory for stale lock files, orphan transcripts, oversized sessions, and active sessions.
3. **Notification** – `send-notification.sh` formats a concise message (overall status + component list) and posts it to Gotify if `GOTIFY_TOKEN` is set.
4. **Scheduling** – `ironclaw-watchdog.timer` fires hourly and calls the driver via `cron-wrapper.sh` (or directly via the systemd service `ironclaw.service`).

## 6. Monitoring & observability capabilities
- **JSON reports** stored under `$HOME/.ironclaw/workspace/reports/health/` – easy to ingest into Grafana Loki, Prometheus exporters, or custom dashboards.
- **Markdown summaries** provide human‑readable status for quick console inspection.
- **Gotify integration** pushes alerts to a user‑configured notification endpoint.
- **Exit codes** enable downstream orchestration (e.g., a CI job can fail on non‑healthy status).
- **Systemd timer** guarantees regular execution without additional cron management.

Missing observability aspects:
- No Prometheus metrics endpoint – health data is file‑based only.
- No structured log aggregation (logs are appended to a single file, but not exported).
- No alert throttling – repeated failures will spam Gotify.

## 7. Extensibility assessment
| Aspect | Current state | Suggested improvement |
|--------|----------------|-----------------------|
| **Adding a new component** | Create `health‑<name>.sh` that obeys JSON contract and add a `run_check` line in the driver. | Automate discovery: driver could iterate over `health-*.sh` files instead of hard‑coded list, reducing maintenance.|
| **Configuration** | Hard‑coded thresholds inside each script (e.g., `STALE_LOCK_MINUTES`). | Centralised configuration file (YAML/ENV) that all scripts source, enabling fleet‑wide tuning.|
| **Parallel execution** | Background jobs are launched but some lines are duplicated/mistyped (see lines 73‑78 where the same `run_check` appears twice). | Refactor to a loop over an array of `{script, component}` pairs; ensure proper quoting and error handling.|
| **Testing** | No unit or integration tests; CI scripts only lint the repo. | Add BATS (Bash Automated Testing System) suites for each health script, validate JSON schema, and integrate into CI pipeline.|
| **Language** | Pure Bash – easy to read but limited in robustness. | Consider moving core orchestration to Rust or Go for stronger typing, better concurrency, and easier packaging.|
| **Packaging** | Deployed via source checkout. | Provide a Docker image or a installable package (e.g., a Homebrew formula) for reproducible deployments.|

## 8. Integration points with Ironclaw services
- **Workspace paths** – All scripts read/write under `$HOME/.ironclaw/workspace/`, sharing the same directory layout as the rest of OpenClaw.
- **Environment variables** – `GOTIFY_URL`, `GOTIFY_TOKEN`, and various API keys (`OPENAI_API_KEY`, etc.) are expected to be exported by the host environment, mirroring other services.
- **Systemd & Heartbeat** – The timer is part of the OpenClaw heartbeat documentation (`HEARTBEAT.md`). Other services can depend on its successful run via systemd `After=` relationships.
- **Reporting directory** – Other services can consume the JSON files for cross‑service health dashboards.
- **Notification channel** – Gotify is also used by other Ironclaw components, providing a unified alerting layer.

## 9. Risks, gaps, and potential improvements
1. **Duplicate/background launch bugs** – Lines 73‑78 contain duplicated `run_check` commands that may spawn extra processes or overwrite temp files.
2. **Hard‑coded paths** – Scripts assume a specific home directory layout; moving the workspace would break them.
3. **Missing health‑irc.sh** – The README mentions an IRC health check, but the script is absent, leading to a runtime error.
4. **No schema validation** – While each script validates its own JSON, the driver does not enforce a global schema for the aggregated report.
5. **Limited error handling** – Timeouts return a generic `unknown` status; richer error messages would aid debugging.
6. **Security** – API keys are passed via environment variables; consider using a secret manager (Vault, AWS Secrets Manager) for production.
7. **Scalability** – As more components are added, the flat list of `run_check` calls becomes unmaintainable.
8. **Observability** – No direct Prometheus exporter; integrating one would allow Grafana dashboards with real‑time alerts.

## 10. Recommended roadmap
| Milestone | Description | Owner | ETA |
|-----------|-------------|-------|-----|
| **M1 – Refactor driver** | Replace hard‑coded `run_check` calls with a loop that auto‑discovers `health-*.sh` scripts; fix duplicated lines; add error‑count aggregation. | Dev team | 2 weeks |
| **M2 – Central configuration** | Introduce `config.yaml` (or `.env`) that defines thresholds, report directory, Gotify credentials; source it from all scripts. | DevOps | 3 weeks |
| **M3 – Add unit tests** | Write BATS test suites for each health script; validate JSON schema and exit codes. Integrate into CI (`ci/quality_gate.sh`). | QA | 4 weeks |
| **M4 – Prometheus exporter** | Implement a thin Go/Rust exporter that reads the latest JSON report and exposes `/metrics` for Prometheus. | Infra | 6 weeks |
| **M5 – Docker packaging** | Build a lightweight Docker image that bundles all scripts and runs the driver on a schedule (cron + Gotify env). | Infra | 8 weeks |
| **M6 – Secret management** | Replace plain‑env API keys with Vault look‑ups; add fallback to env for local dev. | Security | 10 weeks |
| **M7 – Documentation refresh** | Update README with new configuration flow, add diagrams of data flow, and list supported services. | Docs | 10 weeks |

---
*Report generated by Admiral Starforce Nebula*

