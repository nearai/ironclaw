---
name: 1-password
version: "1.0.0"
description: 1Password API — 1Password is a secure password manager that consolidates credentials
activation:
  keywords:
    - "1-password"
    - "1password"
    - "security"
  patterns:
    - "(?i)1.?password"
  tags:
    - "security"
    - "identity"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [1PASSWORD_CONNECT_SERVER_URL, 1PASSWORD_CONNECT_ACCESS_TOKEN]
---

# 1Password API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`{1PASSWORD_CONNECT_SERVER_URL}/v1`

## Actions

**List vaults:**
```
http(method="GET", url="{1PASSWORD_CONNECT_SERVER_URL}/v1/vaults")
```

**List items in vault:**
```
http(method="GET", url="{1PASSWORD_CONNECT_SERVER_URL}/v1/vaults/{vault_id}/items")
```

**Get item details:**
```
http(method="GET", url="{1PASSWORD_CONNECT_SERVER_URL}/v1/vaults/{vault_id}/items/{item_id}")
```

**Create item:**
```
http(method="POST", url="{1PASSWORD_CONNECT_SERVER_URL}/v1/vaults/{vault_id}/items", body={"vault": {"id": "<vault_id>"},"category": "LOGIN","title": "My Login","fields": [{"purpose": "USERNAME","value": "user@example.com"},{"purpose": "PASSWORD","value": "secret"}]})
```

**Delete item:**
```
http(method="DELETE", url="{1PASSWORD_CONNECT_SERVER_URL}/v1/vaults/{vault_id}/items/{item_id}")
```

## Notes

- Item categories: `LOGIN`, `PASSWORD`, `SECURE_NOTE`, `CREDIT_CARD`, `IDENTITY`, `DOCUMENT`.
- Fields have `purpose`: `USERNAME`, `PASSWORD`, `NOTES`.
- The Connect server must be running and accessible at the configured URL.
