---
name: click-house
version: "1.0.0"
description: ClickHouse API — ClickHouse is an ultra-fast, column-oriented database designed for real-time ana
activation:
  keywords:
    - "click-house"
    - "clickhouse"
    - "analytics"
  patterns:
    - "(?i)click.?house"
  tags:
    - "analytics"
    - "data"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CLICK_HOUSE_API_KEY, CLICK_HOUSE_SECRET_KEY]
---

# ClickHouse API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://{CLICKHOUSE_HOST}:8443`

## Actions

**Run query:**
```
http(method="POST", url="https://{CLICKHOUSE_HOST}:8443/?query=SELECT+1")
```

**List tables:**
```
http(method="POST", url="https://{CLICKHOUSE_HOST}:8443/?query=SHOW+TABLES+FROM+default")
```

**Insert data:**
```
http(method="POST", url="https://{CLICKHOUSE_HOST}:8443/?query=INSERT+INTO+table+FORMAT+JSONEachRow", body=[{"col1": "value1","col2": 42}])
```

## Notes

- ClickHouse uses HTTP interface on port 8443 (HTTPS) or 8123 (HTTP).
- Pass SQL in `query` parameter or POST body.
- Output formats: `JSON`, `JSONEachRow`, `CSV`, `TSV`.
- Auth: Basic auth or `X-ClickHouse-User`/`X-ClickHouse-Key` headers.
