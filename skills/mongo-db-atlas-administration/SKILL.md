---
name: mongo-db-atlas-administration
version: "1.0.0"
description: MongoDB Atlas Administration API — MongoDB Atlas Administration is a fully managed cloud database service that enab
activation:
  keywords:
    - "mongo-db-atlas-administration"
    - "mongodb atlas administration"
    - "database"
  patterns:
    - "(?i)mongo.?db.?atlas.?administration"
  tags:
    - "database"
    - "data-storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MONGO_DB_ACCESS_TOKEN]
---

# MongoDB Atlas Administration API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://cloud.mongodb.com/api/atlas/v2`

## Actions

**List projects:**
```
http(method="GET", url="https://cloud.mongodb.com/api/atlas/v2/groups?itemsPerPage=10")
```

**List clusters:**
```
http(method="GET", url="https://cloud.mongodb.com/api/atlas/v2/groups/{project_id}/clusters")
```

**Get cluster:**
```
http(method="GET", url="https://cloud.mongodb.com/api/atlas/v2/groups/{project_id}/clusters/{cluster_name}")
```

**List database users:**
```
http(method="GET", url="https://cloud.mongodb.com/api/atlas/v2/groups/{project_id}/databaseUsers")
```

## Notes

- Uses Digest auth or API key pair.
- `groups` = projects in Atlas terminology.
- Cluster names are unique within a project.
- Requires `Accept: application/vnd.atlas.2025-01-01+json` header for latest API version.
