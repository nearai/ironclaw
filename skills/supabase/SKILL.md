---
name: supabase
version: "1.0.0"
description: Supabase API — database (PostgREST), auth, storage, edge functions
activation:
  keywords:
    - "supabase"
    - "supabase table"
    - "supabase auth"
  exclude_keywords:
    - "firebase"
    - "planetscale"
  patterns:
    - "(?i)supabase.*(table|query|auth|storage|bucket)"
  tags:
    - "database"
    - "backend"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [SUPABASE_URL, SUPABASE_SERVICE_KEY]
---

# Supabase API

Use the `http` tool. Include `apikey` header on all requests. The service key grants full access (bypasses RLS).

## Base URL

`https://{SUPABASE_URL}.supabase.co`

Required header:
```
headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}, {"name": "Prefer", "value": "return=representation"}]
```

## Database (PostgREST)

**Select rows:**
```
http(method="GET", url="https://{project}.supabase.co/rest/v1/users?select=id,name,email&order=created_at.desc&limit=20", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}])
```

**Select with filter:**
```
http(method="GET", url="https://{project}.supabase.co/rest/v1/orders?select=*&status=eq.active&amount=gt.100&order=created_at.desc", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}])
```

**Insert row:**
```
http(method="POST", url="https://{project}.supabase.co/rest/v1/users", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}, {"name": "Prefer", "value": "return=representation"}], body={"name": "Alice", "email": "alice@example.com"})
```

**Update rows:**
```
http(method="PATCH", url="https://{project}.supabase.co/rest/v1/users?id=eq.123", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}, {"name": "Prefer", "value": "return=representation"}], body={"name": "Updated Name"})
```

**Delete rows:**
```
http(method="DELETE", url="https://{project}.supabase.co/rest/v1/users?id=eq.123", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}])
```

**RPC (call database function):**
```
http(method="POST", url="https://{project}.supabase.co/rest/v1/rpc/my_function", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}], body={"param1": "value"})
```

## Storage

**List buckets:**
```
http(method="GET", url="https://{project}.supabase.co/storage/v1/bucket", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}])
```

**List files in bucket:**
```
http(method="POST", url="https://{project}.supabase.co/storage/v1/object/list/<bucket_name>", headers=[{"name": "apikey", "value": "{SUPABASE_SERVICE_KEY}"}], body={"prefix": "", "limit": 20})
```

## Notes

- PostgREST filter operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `in`, `is`.
- Select specific columns: `?select=col1,col2`. Relations: `?select=*,orders(*)`.
- `Prefer: return=representation` returns the created/updated rows.
- Use `anon` key for client-side (respects RLS) or `service_role` key for admin (bypasses RLS).
- Pagination: `limit` + `offset` or `Range` header.
