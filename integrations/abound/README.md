# Abound Integration

Customer-specific integration for Abound's remittance platform. This directory is designed to be separable into its own repo.

## Contents

```
integrations/abound/
├── credentials.json                     # Credential mappings for Abound's API hosts
├── skills/abound-remittance/SKILL.md    # Agent skill for remittance workflows
└── tests/
    ├── test_abound_api_direct.py        # Direct tests against Abound's dev API
    └── test_abound_e2e.py              # E2E tests through IronClaw's Responses API
```

## Deployment

Set these env vars in Railway (or your deployment):

```bash
# Scans for all *.json credential mappings in this directory
INTEGRATION_CREDENTIALS_DIR=/app/integrations

# Skills directory (picks up abound-api + smart-remittance + others)
SKILLS_DIR=/app/skills

# Auto-approve tool calls (no interactive approval in API-driven deployments)
AGENT_AUTO_APPROVE_TOOLS=true
```

## Per-User Credential Setup

After creating a user via the Admin API, inject their Abound credentials:

```bash
# Inject read token (per-user, for account info, exchange rate, notifications)
PUT /api/admin/users/{user_id}/secrets/abound_read_token
{"value": "<user's abound read token>", "provider": "abound"}

# Inject write token (per-user, for send-wire only)
PUT /api/admin/users/{user_id}/secrets/abound_write_token
{"value": "<user's abound write token>", "provider": "abound"}

# Inject shared API key
PUT /api/admin/users/{user_id}/secrets/abound_api_key
{"value": "<shared X-API-KEY>", "provider": "abound"}
```

Credentials are path-scoped: `abound_read_token` is injected for read endpoints, `abound_write_token` only for send-wire. `X-API-KEY` is injected for all Abound hosts.

## Running Tests

```bash
# Direct API tests (no IronClaw needed)
python integrations/abound/tests/test_abound_api_direct.py

# E2E tests (requires running IronClaw deployment)
python integrations/abound/tests/test_abound_e2e.py
```

## Abound Dev Endpoints

| Endpoint | Method | URL |
|----------|--------|-----|
| Account Info | GET | `devneobank.timesclub.co/times/bank/remittance/agent/account/info` |
| Exchange Rate | GET | `devneobank.timesclub.co/times/bank/remittance/agent/exchange-rate` |
| Send Wire | POST | `devneobank.timesclub.co/times/bank/remittance/agent/send-wire` |
| Notification | POST | `dev.timesclub.co/times/users/agent/create-notification` |
