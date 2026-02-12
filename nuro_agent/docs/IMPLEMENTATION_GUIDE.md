# Implementation Guide

## 1) Copy the config template

- Start from: `ironclaw/config/openclaw.nuro.safe.template.json5`
- Apply it to your OpenClaw config path.
- Replace placeholders:
  - `OPENCLAW_GATEWAY_PASSWORD`
  - `REPLACE_OWNER_ID`
  - Optional channel target IDs

## 2) Prepare workspace files

Use the files in `ironclaw/workspace/` as your agent workspace baseline.

## 3) Run hardening preflight

```bash
bash ironclaw/scripts/preflight_hardening.sh
```

Optional fix mode:

```bash
bash ironclaw/scripts/preflight_hardening.sh --fix
```

## 4) Install cron pack

Dry run first:

```bash
bash ironclaw/scripts/setup_cron_jobs_example.sh
```

Apply jobs:

```bash
TZ_NAME=America/Los_Angeles \
DELIVERY_CHANNEL=telegram \
DELIVERY_TO=<your-target-id> \
bash ironclaw/scripts/setup_cron_jobs_example.sh --apply
```

## 5) Validate runtime posture

- DM policy should be pairing.
- Group policy should be allowlist + mention-gated.
- Elevated exec should remain disabled.
- Non-main sessions should be sandboxed.
- Logs should be redacted for tool summaries.

## 6) Open nuro interface

```bash
bash /Users/nuro/Documents/dev/ironclaw/nuro_agent/scripts/run_nuro_interface.sh
```

Default URL: `http://localhost:8099`
