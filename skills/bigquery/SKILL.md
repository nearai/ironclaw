---
name: bigquery
version: "1.0.0"
description: BigQuery API — BigQuery is a serverless, highly scalable
activation:
  keywords:
    - "bigquery"
    - "ai"
  patterns:
    - "(?i)bigquery"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
---

# BigQuery API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://bigquery.googleapis.com/bigquery/v2`

## Actions

**List datasets:**
```
http(method="GET", url="https://bigquery.googleapis.com/bigquery/v2/projects/{project_id}/datasets")
```

**List tables:**
```
http(method="GET", url="https://bigquery.googleapis.com/bigquery/v2/projects/{project_id}/datasets/{dataset_id}/tables")
```

**Run query:**
```
http(method="POST", url="https://bigquery.googleapis.com/bigquery/v2/projects/{project_id}/queries", body={"query": "SELECT * FROM `project.dataset.table` LIMIT 10","useLegacySql": false})
```

**Get query results:**
```
http(method="GET", url="https://bigquery.googleapis.com/bigquery/v2/projects/{project_id}/queries/{job_id}")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Queries use Standard SQL by default (`useLegacySql: false`).
- Large results: check `jobComplete` field; poll with `getQueryResults` if `false`.
- Table references: `project.dataset.table`.
