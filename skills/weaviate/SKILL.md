---
name: weaviate
version: "1.0.0"
description: Weaviate API — Weaviate is an open-source vector database designed for storing and searching la
activation:
  keywords:
    - "weaviate"
    - "ai"
  patterns:
    - "(?i)weaviate"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
    - "communication"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [WEAVIATE_API_KEY, WEAVIATE_REST_ENDPOINT]
---

# Weaviate API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{WEAVIATE_HOST}`

## Actions

**List collections:**
```
http(method="GET", url="https://{WEAVIATE_HOST}/v1/schema")
```

**Create collection:**
```
http(method="POST", url="https://{WEAVIATE_HOST}/v1/schema", body={"class": "Article","vectorizer": "text2vec-openai","properties": [{"name": "title","dataType": ["text"]},{"name": "content","dataType": ["text"]}]})
```

**Add object:**
```
http(method="POST", url="https://{WEAVIATE_HOST}/v1/objects", body={"class": "Article","properties": {"title": "My Article","content": "Article content"}})
```

**GraphQL query:**
```
http(method="POST", url="https://{WEAVIATE_HOST}/v1/graphql", body={"query": "{ Get { Article(limit: 5, nearText: {concepts: [\"AI\"]}) { title content } } }"})
```

## Notes

- Weaviate is a vector database.
- Classes define collections with properties and vectorizers.
- Vectorizers: `text2vec-openai`, `text2vec-cohere`, `text2vec-huggingface`.
- Search: `nearText` (semantic), `nearVector`, `bm25` (keyword), `hybrid`.
