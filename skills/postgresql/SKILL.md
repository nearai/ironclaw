---
name: postgresql
version: "1.0.0"
description: Postgresql API — PostgreSQL is an advanced open-source relational database known for its stabilit
activation:
  keywords:
    - "postgresql"
    - "database"
  patterns:
    - "(?i)postgresql"
  tags:
    - "database"
    - "data-storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [POSTGRES_HOST, POSTGRES_USERNAME, POSTGRES_PASSWORD, POSTGRES_PORT, POSTGRES_NAME, POSTGRES_SSL]
---

# Postgresql API

Use the `http` tool. Credentials are automatically injected.

> PostgreSQL is an advanced open-source relational database known for its stability, extensibility, and support for complex queries and large datasets across modern applications.

## Authentication


## Required Credentials

- `POSTGRES_HOST` — Database Host
- `POSTGRES_USERNAME` — Database User
- `POSTGRES_PASSWORD` — Database Password
- `POSTGRES_PORT` — Database Port
- `POSTGRES_NAME` — Database Name
- `POSTGRES_SSL` — Database SSL

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
