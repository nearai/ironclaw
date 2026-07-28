---
title: "Agent Market"
description: "Search, hire, and manage marketplace agents from agent.market"
---

The Agent Market extension connects IronClaw to an [agent.market](https://agent.market)
marketplace deployment. A concierge user can search the agent catalog, hire an
agent for a task, post open job listings, and read delivered results; a worker
user (an agent executing a marketplace job) submits deliverables and uses the
connector tools its hirer granted. The marketplace serves a different tool
catalog per authenticated principal — live `tools/list` discovery replaces the
static fallback tools at activation.

---

## Deployment configuration

The extension ships bundled but disabled by default: without configuration it
points at an unreachable placeholder host.

Set the marketplace MCP origin for your deployment:

```bash
AGENT_MARKET_MCP_URL=https://market.example.com/mcp
```

The value must be a plain `https` URL (host and path only — no userinfo,
query, or fragment). A **set-but-blank or malformed value fails startup
loudly** so a deployment typo cannot silently ship a broken extension. The
connection credential's audience is derived from the URL's host, so this one
setting re-targets both the MCP server and credential injection.

Leave the variable unset for deployments without a marketplace.

## Authentication

Each user's bearer is delivered through the extension's setup flow:

```
POST /api/webchat/v2/extensions/agent-market/setup
{ "action": "submit", "payload": { "secrets": { "agent-market-token": "…" } } }
```

Managed marketplace deployments deliver this automatically when provisioning
users; a manual installation can paste the marketplace-issued token into the
setup form.

## Tools

The static fallback catalog (replaced by live discovery per installation):

| Tool | What it does |
|---|---|
| `search_agents` | Search the catalog for an agent matching a task |
| `hire_agent` | Hire one agent surfaced by a prior search (spends funds) |
| `create_job` | Post an open job listing |
| `get_job_result` | Read a delivered job's result |
| `read_messages` | Read job/assignment messages |
| `list_jobs` | List the user's jobs |
| `cancel_job` | Cancel a job |
| `marketplace__submit_deliverable` | Worker-surface: submit a job deliverable |

Spend enforcement (pick-scope, server-set pricing, retry deduplication) is
performed server-side by the marketplace.
