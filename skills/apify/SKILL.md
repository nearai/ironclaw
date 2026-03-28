---
name: apify
version: "1.0.0"
description: Apify API — Apify is a full‑stack web scraping and browser automation platform where develop
activation:
  keywords:
    - "apify"
    - "tools"
  patterns:
    - "(?i)apify"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [APIFY_API_KEY]
---

# Apify API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.apify.com/v2`

## Actions

**List actors:**
```
http(method="GET", url="https://api.apify.com/v2/acts?limit=10")
```

**Run actor:**
```
http(method="POST", url="https://api.apify.com/v2/acts/{actor_id}/runs", body={"memory": 256,"timeout": 60})
```

**Get run details:**
```
http(method="GET", url="https://api.apify.com/v2/acts/{actor_id}/runs/{run_id}")
```

**Get dataset items:**
```
http(method="GET", url="https://api.apify.com/v2/datasets/{dataset_id}/items?limit=100")
```

## Notes

- Actor IDs look like `username~actor-name` or a hash.
- Run status: `READY`, `RUNNING`, `SUCCEEDED`, `FAILED`, `TIMED-OUT`, `ABORTED`.
- Default dataset is created per run; check `defaultDatasetId` in run details.
