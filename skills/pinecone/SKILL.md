---
name: pinecone
version: "1.0.0"
description: Pinecone API — Pinecone is a vector database designed for building AI applications with fast an
activation:
  keywords:
    - "pinecone"
    - "ai"
  patterns:
    - "(?i)pinecone"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PINECONE_INDEX_HOST, PINECONE_API_KEY, PINECONE_ASSISTANT_HOST]
---

# Pinecone API

Use the `http` tool. API key is automatically injected via `Api-Key` header — **never construct auth headers manually**.

## Base URL

`https://api.pinecone.io`

## Actions

**List indexes:**
```
http(method="GET", url="https://api.pinecone.io/indexes")
```

**Create index:**
```
http(method="POST", url="https://api.pinecone.io/indexes", body={"name": "my-index","dimension": 1536,"metric": "cosine","spec": {"serverless": {"cloud": "aws","region": "us-east-1"}}})
```

**Describe index:**
```
http(method="GET", url="https://api.pinecone.io/indexes/{index_name}")
```

**Upsert vectors:**
```
http(method="POST", url="https://{index_host}/vectors/upsert", body={"vectors": [{"id": "vec1","values": [0.1,0.2,0.3]}]})
```

**Query vectors:**
```
http(method="POST", url="https://{index_host}/query", body={"vector": [0.1,0.2,0.3],"topK": 5,"includeMetadata": true})
```

## Notes

- Control plane: `api.pinecone.io` (manage indexes).
- Data plane: `{index_host}` from describe index response (upsert/query).
- Metrics: `cosine`, `euclidean`, `dotproduct`.
- Vectors can include `metadata` for filtering.
