# Reborn Live Canary

This directory contains the operator entrypoints for the canonical Reborn
WebUI live QA lane:

- `run.sh` dispatches `reborn-webui-v2-live-qa`.
- `scrub-artifacts.sh` scans artifacts before upload.
- `notify_slack.py` summarizes the resulting QA artifacts.
- `validate_reborn_binary_artifact.py` verifies the prepared `ironclaw` binary.

The retired v1 auth, workflow, public-smoke, and upgrade runners are no longer
dispatchable. Their coverage must be implemented against the canonical
`ironclaw serve` harness before a new lane is added.

Run selected cases from the repository root:

```bash
LANE=reborn-webui-v2-live-qa \
CASES=qa_3b_endpoint_status_live_chat,qa_8b_hn_keyword_live_chat \
scripts/live-canary/run.sh
```

Run all non-Telegram QA cases:

```bash
LANE=reborn-webui-v2-live-qa CASES=all scripts/live-canary/run.sh
```

Artifacts are written under
`artifacts/live-canary/reborn-webui-v2-live-qa/<provider>/<timestamp>/`.
GitHub Actions uses `.github/workflows/live-canary.yml` for scheduled and manual
runs, including PR approval, OAuth preflight, binary preparation, QA, artifact
scrubbing, and reporting.
