---
name: runpod
version: "1.0.0"
description: Runpod API — RunPod is a cloud infrastructure platform designed for running GPU-accelerated w
activation:
  keywords:
    - "runpod"
    - "cloud computing"
  patterns:
    - "(?i)runpod"
  tags:
    - "tools"
    - "cloud-computing"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [RUNPOD_API_KEY]
---

# Runpod API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.runpod.io/v2`

## Actions

**Run endpoint:**
```
http(method="POST", url="https://api.runpod.io/v2/{endpoint_id}/run", body={"input": {"prompt": "Hello"}})
```

**Run sync:**
```
http(method="POST", url="https://api.runpod.io/v2/{endpoint_id}/runsync", body={"input": {"prompt": "Hello"}})
```

**Get status:**
```
http(method="GET", url="https://api.runpod.io/v2/{endpoint_id}/status/{request_id}")
```

## Notes

- `/run` is async (returns job ID); `/runsync` waits for result.
- Status values: `IN_QUEUE`, `IN_PROGRESS`, `COMPLETED`, `FAILED`.
- Input schema depends on the deployed model.
- Auth via `Authorization: Bearer {key}` header.
