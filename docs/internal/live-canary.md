# Reborn Live Canary

The supported live regression path is the canonical Reborn WebUI QA lane. It
runs `ironclaw serve` with the prepared PR or default-branch binary and covers
the selected live provider and browser cases.

The implementation lives in:

- `.github/workflows/live-canary.yml` for scheduling, approval, preparation,
  execution, and reporting;
- `scripts/reborn_webui_v2_live_qa/` for the QA harness;
- `scripts/live-canary/run.sh` for local and CI dispatch;
- `scripts/live-canary/scrub-artifacts.sh` for artifact scanning.

Legacy auth, workflow, public-smoke, persona, provider-matrix, and upgrade
canaries depended on the retired v1 runtime. They were removed rather than left
as unreachable jobs. Equivalent coverage must use the canonical Reborn harness
before a new lane is introduced.

## Run Locally

```bash
LANE=reborn-webui-v2-live-qa \
CASES=qa_3b_endpoint_status_live_chat \
scripts/live-canary/run.sh
```

Use `CASES=all` for all non-Telegram QA cases. Artifacts are written under
`artifacts/live-canary/` and must pass `scrub-artifacts.sh` before upload.

## Repository Configuration

The workflow materializes live credentials into runner-local files and passes
only their paths to the QA process. PR-targeted runs require the
`reborn-live-canary-pr` environment gate and approval for the exact head SHA by
a collaborator with write access.

Slack setup uses `PUT /api/webchat/v2/channels/slack/setup` after `ironclaw
serve` starts. Google OAuth cases mint a short-lived access token during the
run. Required secrets and variables are named in `.github/workflows/live-canary.yml`.

Do not commit live secrets, tokens, storage state, screenshots containing PII,
or unsanitized provider responses.
