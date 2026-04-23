---
name: composio
version: "1.0.0"
description: Connect to 250+ apps (Gmail, GitHub, Slack, Notion, etc.) via the Composio integration platform using the HTTP tool with automatic credential injection
activation:
  keywords:
    - "composio"
    - "connect app"
    - "integration"
  patterns:
    - "(?i)composio"
    - "(?i)connect.*(app|tool|integration)"
  tags:
    - "integration"
    - "composio"
  max_context_tokens: 2000
credentials:
  - name: composio_api_key
    provider: composio
    location:
      type: header
      name: x-api-key
    hosts:
      - "backend.composio.dev"
    setup_instructions: "Get an API key at app.composio.dev — go to Settings > API Keys to generate one"
http:
  allowed_hosts:
    - "backend.composio.dev"
---

# Composio Skill

You have access to the Composio API via the `http` tool. Credentials are automatically injected — **never construct `x-api-key` headers manually**. When the URL host is `backend.composio.dev`, the system injects the header transparently.

Composio provides a unified API to 250+ apps. Before using an app, you must connect it via OAuth.

## API Patterns

Base URL: `https://backend.composio.dev/api/v3`

### List available tools for an app

```
http(method="GET", url="https://backend.composio.dev/api/v3/tools?toolkit_slug=gmail&limit=50&toolkit_versions=latest")
```

Returns available tool actions for the specified app.

### Execute a tool action

```
http(method="POST", url="https://backend.composio.dev/api/v3/tools/execute/{tool_slug}", body={"connected_account_id": "{account_id}", "params": {"to": "user@example.com", "subject": "Hello", "body": "World"}})
```

- `tool_slug`: e.g. `GMAIL_SEND_EMAIL`, `GITHUB_CREATE_ISSUE`
- `params`: Action-specific parameters (check list endpoint for schema)
- `connected_account_id`: Omit to auto-resolve if only one account connected

### Connect an app (OAuth)

```
http(method="GET", url="https://backend.composio.dev/api/v3/auth/configs?toolkit_slug=gmail")
```

Extract `auth_config_id`, then:
```
http(method="POST", url="https://backend.composio.dev/api/v3/connected_accounts/link", body={"auth_config_id": "{auth_config_id}", "user_id": "default"})
```

Returns a `redirect_url` for the user to complete OAuth.

### List connected accounts

```
http(method="GET", url="https://backend.composio.dev/api/v3/connected_accounts?user_ids[]=default")
```

## Common Mistakes

- Do NOT add an `x-api-key` header — it is injected automatically.
- You must connect an app before executing its tools. Check `connected_accounts` first.
- The `tool_slug` is uppercase with underscores (e.g. `GMAIL_SEND_EMAIL`), not lowercase.
- For apps with native IronClaw skills (Gmail, GitHub, Slack, etc.), prefer those skills — they have richer documentation and direct API access.
