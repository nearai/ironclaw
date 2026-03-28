---
name: shopify-admin
version: "1.0.0"
description: Shopify Admin API — Shopify Admin is the backend interface for managing your Shopify store
activation:
  keywords:
    - "shopify-admin"
    - "shopify admin"
    - "ecommerce"
  patterns:
    - "(?i)shopify.?admin"
  tags:
    - "tools"
    - "ecommerce"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SHOPIFY_STORE_NAME, SHOPIFY_ADMIN_ACCESS_TOKEN]
---

# Shopify Admin API

Use the `http` tool. API key is automatically injected via `X-Shopify-Access-Token` header — **never construct auth headers manually**.

## Base URL

`https://{SHOPIFY_STORE}.myshopify.com/admin/api/2024-10`

## Actions

**List products:**
```
http(method="GET", url="https://{SHOPIFY_STORE}.myshopify.com/admin/api/2024-10/products.json?limit=10")
```

**Get product:**
```
http(method="GET", url="https://{SHOPIFY_STORE}.myshopify.com/admin/api/2024-10/products/{product_id}.json")
```

**Create product:**
```
http(method="POST", url="https://{SHOPIFY_STORE}.myshopify.com/admin/api/2024-10/products.json", body={"product": {"title": "Widget","body_html": "<p>Description</p>","vendor": "Acme","product_type": "Gadgets","variants": [{"price": "29.99","sku": "WIDGET-001"}]}})
```

**List orders:**
```
http(method="GET", url="https://{SHOPIFY_STORE}.myshopify.com/admin/api/2024-10/orders.json?status=any&limit=10")
```

**List customers:**
```
http(method="GET", url="https://{SHOPIFY_STORE}.myshopify.com/admin/api/2024-10/customers.json?limit=10")
```

## Notes

- Auth via `X-Shopify-Access-Token` header (auto-injected).
- All responses wrapped in resource key: `{"products": [...]}`.
- Pagination: `Link` header with `rel="next"` URL.
- API version in URL path (e.g., `2024-10`).
